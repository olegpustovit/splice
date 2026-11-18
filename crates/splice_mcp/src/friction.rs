use super::{
    Json, object,
    project::Projects,
    text,
    wire::{count, now},
};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const FILE: &str = "friction.jsonl";

pub(super) fn state_dir() -> io::Result<PathBuf> {
    let home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "HOME does not name an absolute path",
                )
            })
    };
    if cfg!(target_os = "macos") {
        Ok(home()?.join("Library/Application Support/Splice"))
    } else if let Some(state) = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        Ok(state.join("splice"))
    } else {
        Ok(home()?.join(".local/state/splice"))
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Scope {
    project: Option<PathBuf>,
    client: String,
}

struct Pending {
    id: String,
    scope: Scope,
    session: Option<String>,
    intent: Json,
    remaining: u8,
    commit: bool,
}

pub(super) struct Recorder {
    global: Option<PathBuf>,
    client: String,
    sessions: Vec<(Scope, String)>,
    pending: Vec<Pending>,
}

impl Recorder {
    pub(super) fn new(global: Option<PathBuf>) -> Self {
        Self {
            global,
            client: "mcp".into(),
            sessions: Vec::new(),
            pending: Vec::new(),
        }
    }

    pub(super) fn initialize(&mut self, info: &Json) {
        self.client = client_name(info).unwrap_or_else(|| "mcp".into());
    }

    pub(super) fn call(
        &mut self,
        projects: &Projects,
        tool: &str,
        args: &Json,
        result: &Json,
        modern: Option<&Json>,
    ) {
        let Some(payload) = result.field("structuredContent") else {
            return;
        };
        let error = payload.field("error");
        let note = args.field("friction").and_then(Json::as_str);
        if error.is_none()
            && note.is_none()
            && self.pending.is_empty()
            && args.field("sessionId").is_none()
            && !matches!(tool, "session.start" | "history.undo")
        {
            return;
        }
        let store = projects.context(args, payload);
        let scope = Scope {
            client: match modern {
                Some(meta) => meta
                    .field("io.modelcontextprotocol/clientInfo")
                    .and_then(client_name)
                    .unwrap_or_else(|| "mcp".into()),
                None => self.client.clone(),
            },
            project: store.map(|store| store.path().to_owned()),
        };
        let revision = store.map_or(Json::Null, |store| Json::Unsigned(store.project().rev));
        let mut session = args.field("sessionId").and_then(Json::as_str);
        if tool == "session.start" {
            session = payload.field("sessionId").and_then(Json::as_str);
        }
        if let Some(session) = session
            && store.is_some_and(|store| {
                store
                    .sessions()
                    .iter()
                    .any(|s| s.open && s.id.to_string() == session)
            })
            && !self
                .sessions
                .iter()
                .any(|(known, id)| *known == scope && id == session)
        {
            self.sessions.push((scope.clone(), session.into()));
            if tool != "session.start"
                && let Some(project) = &scope.project
            {
                self.resume(&scope, session, &project.join(FILE));
            }
        }
        let inferred = || {
            let mut sessions = self.sessions.iter().filter(|(known, _)| *known == scope);
            let (_, only) = sessions.next()?;
            sessions.next().is_none().then_some(only.as_str())
        };
        let session = session.or_else(inferred).map(str::to_owned);
        let success = result.field("isError").and_then(Json::as_bool) == Some(false);
        let intent = (error.is_some() || !self.pending.is_empty()).then(|| call_intent(tool, args));
        let dry_run = args.field("dryRun").and_then(Json::as_bool) == Some(true);
        self.pending.retain_mut(|entry| {
            if entry.scope == scope && entry.session == session {
                let recovered =
                    success && intent.as_ref() == Some(&entry.intent) && !(entry.commit && dry_run);
                let update = object(vec![
                    ("id", text(&entry.id)),
                    (
                        "follow",
                        object(vec![
                            ("tool", text(tool)),
                            ("args", args.clone()),
                            ("ok", Json::Bool(success)),
                        ]),
                    ),
                    ("recovered", Json::Bool(recovered)),
                ]);
                persist(self.global.as_deref(), scope.project.as_deref(), &update);
                entry.remaining -= 1;
            }
            entry.remaining != 0
        });
        let undo = success
            && tool == "history.undo"
            && !dry_run
            && (payload
                .field("restored")
                .is_some_and(|value| *value != Json::Null)
                || payload
                    .field("reverted")
                    .and_then(Json::as_array)
                    .is_some_and(|entries| !entries.is_empty()));
        if error.is_none() && note.is_none() && !undo {
            return;
        }
        let base = || {
            object(vec![
                ("time", Json::Unsigned(now())),
                ("client", text(&scope.client)),
                ("tool", text(tool)),
                ("args", args.clone()),
                ("revision", revision.clone()),
                ("session", session.as_deref().map_or(Json::Null, text)),
                (
                    "project",
                    scope
                        .project
                        .as_ref()
                        .map_or(Json::Null, |path| text(&path.to_string_lossy())),
                ),
                ("next", Json::Array(Vec::new())),
                ("recovered", Json::Bool(false)),
                ("note", note.map_or(Json::Null, text)),
            ])
        };
        if let Some(error) = error {
            let record = entry(base(), error, "ERROR", "");
            if persist(self.global.as_deref(), scope.project.as_deref(), &record) {
                let id = record.field("id").and_then(Json::as_str).unwrap().into();
                self.pending.push(Pending {
                    id,
                    scope: scope.clone(),
                    session: session.clone(),
                    intent: intent.expect("error keeps its intent"),
                    remaining: 3,
                    commit: !dry_run,
                });
            }
        }
        if let Some(note) = note {
            let record = entry(base(), &Json::Null, "NOTE", note);
            persist(self.global.as_deref(), scope.project.as_deref(), &record);
        }
        if undo {
            let record = entry(
                base(),
                &Json::Null,
                "AGENT_UNDO",
                "The agent undid an edit.",
            );
            persist(self.global.as_deref(), scope.project.as_deref(), &record);
        }
    }
    fn resume(&mut self, scope: &Scope, session: &str, path: &Path) {
        let entries = match read_entries(path) {
            Ok(entries) => entries,
            Err(error) => {
                eprintln!("splice friction: {}: {error}", path.display());
                return;
            }
        };
        for entry in entries {
            if entry.field("session").and_then(Json::as_str) != Some(session)
                || entry.field("resolved").is_some()
                || !entry
                    .field("error")
                    .is_some_and(|error| error.as_object().is_some())
            {
                continue;
            }
            let next = entry
                .field("next")
                .and_then(Json::as_array)
                .map_or(0, <[Json]>::len);
            let remaining = 3usize.saturating_sub(next) as u8;
            let id = entry.field("id").and_then(Json::as_str);
            let tool = entry.field("tool").and_then(Json::as_str);
            if let (Some(id), Some(tool), Some(args), true) =
                (id, tool, entry.field("args"), remaining != 0)
                && !self.pending.iter().any(|entry| entry.id == id)
            {
                self.pending.push(Pending {
                    id: id.into(),
                    scope: scope.clone(),
                    session: Some(session.into()),
                    intent: call_intent(tool, args),
                    remaining,
                    commit: args.field("dryRun").and_then(Json::as_bool) != Some(true),
                });
            }
        }
    }
}

fn client_name(info: &Json) -> Option<String> {
    let name = info.field("name")?.as_str()?;
    Some(match info.field("version").and_then(Json::as_str) {
        Some(version) if !version.is_empty() => format!("{name} {version}"),
        _ => name.into(),
    })
}

fn call_intent(tool: &str, args: &Json) -> Json {
    let selectors = |object: &Json, fields: &[&str]| {
        Json::Array(
            fields
                .iter()
                .map(|field| object.field(field).cloned().unwrap_or(Json::Null))
                .collect(),
        )
    };
    let target = if tool == "edit.apply" {
        args.field("ops")
            .and_then(Json::as_array)
            .map_or(Json::Null, |ops| {
                Json::Array(
                    ops.iter()
                        .map(|op| {
                            selectors(
                                op,
                                &[
                                    "cmd", "item", "items", "clip", "marker", "lane", "target",
                                    "media", "path",
                                ],
                            )
                        })
                        .collect(),
                )
            })
    } else {
        selectors(args, &["id", "clip", "ids", "jobId", "path", "scope"])
    };
    Json::Array(vec![text(tool), target])
}

fn new_id() -> String {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "fr_{stamp:032x}{:08x}{:016x}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    )
}

fn entry(base: Json, error: &Json, fallback_code: &str, fallback_message: &str) -> Json {
    let op = error
        .field("op")
        .and_then(Json::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0);
    let command = base
        .field("args")
        .and_then(|args| args.field("ops"))
        .and_then(Json::as_array)
        .and_then(|ops| ops.get(op))
        .and_then(|op| op.field("cmd"))
        .cloned()
        .unwrap_or(Json::Null);
    super::wire::extend(
        base,
        vec![
            ("id", Json::String(new_id())),
            ("command", command),
            (
                "code",
                text(
                    error
                        .field("code")
                        .and_then(Json::as_str)
                        .unwrap_or(fallback_code),
                ),
            ),
            (
                "message",
                text(
                    error
                        .field("message")
                        .and_then(Json::as_str)
                        .unwrap_or(fallback_message),
                ),
            ),
            (
                "fix",
                text(
                    error
                        .field("fix")
                        .and_then(Json::as_str)
                        .unwrap_or_default(),
                ),
            ),
            ("error", error.clone()),
        ],
    )
}

fn persist(global: Option<&Path>, project: Option<&Path>, record: &Json) -> bool {
    let mut stored = false;
    for dir in [global, project].into_iter().flatten() {
        let path = dir.join(FILE);
        let result = if Some(dir) == global {
            fs::create_dir_all(dir)
        } else {
            Ok(())
        }
        .and_then(|()| append(&path, record));
        match result {
            Ok(()) => stored = true,
            Err(error) => eprintln!("splice friction: {}: {error}", path.display()),
        }
    }
    stored
}

fn log_file(path: &Path, write: bool) -> io::Result<File> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.nlink() != 1 => {
            return Err(bad_log("expected a regular single-link file"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound && write => {}
        Err(error) => return Err(error),
    }
    let file = OpenOptions::new()
        .read(true)
        .append(write)
        .create(write)
        .mode(0o600)
        .open(path)?;
    let actual = file.metadata()?;
    let named = fs::symlink_metadata(path)?;
    if !named.is_file()
        || actual.nlink() != 1
        || actual.ino() != named.ino()
        || actual.dev() != named.dev()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "friction log must be a regular, single-link file",
        ));
    }
    if write {
        file.lock()?;
    } else {
        file.lock_shared()?;
    }
    Ok(file)
}

fn append(path: &Path, record: &Json) -> io::Result<()> {
    let mut wire = String::new();
    splice_json::write(record, &mut wire).map_err(io::Error::other)?;
    wire.push('\n');
    let mut file = log_file(path, true)?;
    let mut end = file.metadata()?.len();
    // A process interrupted mid-write can leave only the final line incomplete.
    // Remove that uncommitted tail before adding a new record.
    let mut buffer = [0; 8192];
    while end > 0 {
        let start = end.saturating_sub(buffer.len() as u64);
        let slice = &mut buffer[..(end - start) as usize];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(slice)?;
        if let Some(index) = slice.iter().rposition(|&byte| byte == b'\n') {
            end = start + index as u64 + 1;
            break;
        }
        end = start;
    }
    if end != file.metadata()?.len() {
        file.set_len(end)?;
    }
    file.write_all(wire.as_bytes())
}

pub(super) fn read_entries(path: &Path) -> io::Result<Vec<Json>> {
    let file = match log_file(path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut reader = BufReader::new(file);
    let mut wire = Vec::new();
    let mut positions: BTreeMap<String, usize> = BTreeMap::new();
    let mut entries = Vec::new();
    while reader.read_until(b'\n', &mut wire)? != 0 && wire.ends_with(b"\n") {
        let row = splice_json::parse(str::from_utf8(&wire).map_err(io::Error::other)?)
            .map_err(io::Error::other)?;
        let id = row
            .field("id")
            .and_then(Json::as_str)
            .ok_or_else(|| bad_log("missing record ID"))?;
        if let Some(update) = row.field("follow").or_else(|| row.field("resolved")) {
            if let Some(&index) = positions.get(id) {
                let Json::Object(fields) = &mut entries[index] else {
                    return Err(bad_log("entry is not an object"));
                };
                if row.field("follow").is_some() {
                    let Some((_, Json::Array(next))) =
                        fields.iter_mut().find(|(key, _)| key == "next")
                    else {
                        return Err(bad_log("entry has no follow-up array"));
                    };
                    if next.len() < 3 {
                        next.push(update.clone());
                    }
                    if row.field("recovered") == Some(&Json::Bool(true)) {
                        update_field(fields, "recovered", Json::Bool(true));
                    }
                } else {
                    update_field(fields, "resolved", update.clone());
                }
            }
        } else if !positions.contains_key(id) {
            positions.insert(id.to_owned(), entries.len());
            entries.push(row);
        }
        wire.clear();
    }
    Ok(entries)
}

fn update_field(fields: &mut Vec<(String, Json)>, name: &str, value: Json) {
    if let Some((_, current)) = fields.iter_mut().find(|(key, _)| key == name) {
        *current = value;
    } else {
        fields.push((name.into(), value));
    }
}

fn bad_log(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("invalid friction log: {message}"),
    )
}

/// Runs `splice friction` against the user's local state and writes results to the supplied stream.
pub fn command(args: &[String], output: &mut dyn Write) -> io::Result<()> {
    run(&state_dir()?, Path::new("fixtures/mcp"), args, output)
}

pub(super) fn run(
    state: &Path,
    fixtures: &Path,
    args: &[String],
    output: &mut dyn Write,
) -> io::Result<()> {
    let path = state.join(FILE);
    let entries = read_entries(&path)?;
    match args
        .split_first()
        .map(|(first, rest)| (first.as_str(), rest))
    {
        None | Some(("list", [])) => list(&entries, false, output),
        Some(("list", [flag])) if flag == "--open" => list(&entries, true, output),
        Some(("show", [id])) => write_json(output, &lookup(&entries, id)?),
        Some(("repro", [id])) => reproduce(fixtures, &lookup(&entries, id)?, output),
        Some(("resolve", [id, sha])) => {
            if !(7..=64).contains(&sha.len()) || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "pass a Git commit SHA",
                ));
            }
            let entry = lookup(&entries, id)?;
            let resolution = object(vec![("id", text(id)), ("resolved", text(sha))]);
            append(&path, &resolution)?;
            if let Some(project) = entry.field("project").and_then(Json::as_str) {
                let package = Path::new(project);
                if package.is_dir() {
                    append(&package.join(FILE), &resolution)?;
                }
            }
            writeln!(output, "Resolved {id} at {sha}")
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: splice friction [list [--open] | show <id> | repro <id> | resolve <id> <sha>]",
        )),
    }
}

fn lookup(entries: &[Json], id: &str) -> io::Result<Json> {
    if !id.strip_prefix("fr_").is_some_and(|value| {
        !value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pass a friction ID from splice friction list",
        ));
    }
    entries
        .iter()
        .find(|entry| entry.field("id").and_then(Json::as_str) == Some(id))
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no friction entry {id}")))
}

#[derive(Default)]
struct Cluster {
    hits: usize,
    recovered: usize,
    latest: u64,
    example: Option<Json>,
}

fn list(entries: &[Json], open: bool, output: &mut dyn Write) -> io::Result<()> {
    let mut clusters = BTreeMap::<(String, String, String), Cluster>::new();
    for entry in entries {
        if open && entry.field("resolved").is_some() {
            continue;
        }
        let key = |name| {
            entry
                .field(name)
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let cluster = clusters
            .entry((key("tool"), key("command"), key("code")))
            .or_default();
        cluster.hits += 1;
        cluster.recovered += usize::from(entry.field("recovered") == Some(&Json::Bool(true)));
        let time = entry.field("time").and_then(Json::as_u64).unwrap_or(0);
        if time >= cluster.latest {
            cluster.latest = time;
            cluster.example = Some(entry.clone());
        }
    }
    for ((tool, command, code), cluster) in clusters {
        let example = cluster.example.unwrap_or(Json::Null);
        let id = example
            .field("id")
            .and_then(Json::as_str)
            .unwrap_or_default();
        let message = example
            .field("message")
            .and_then(Json::as_str)
            .unwrap_or_default();
        let target = if command.is_empty() {
            tool
        } else {
            format!("{tool}/{command}")
        };
        let rate = 100.0 * cluster.recovered as f64 / cluster.hits as f64;
        writeln!(
            output,
            "{} {}  {} hits  {rate:.0}% recovered  {} {}",
            target.escape_debug(),
            code.escape_debug(),
            cluster.hits,
            id.escape_debug(),
            message.escape_debug()
        )?;
    }
    Ok(())
}

fn reproduce(directory: &Path, entry: &Json, output: &mut dyn Write) -> io::Result<()> {
    let id = entry
        .field("id")
        .and_then(Json::as_str)
        .ok_or_else(|| bad_log("missing ID"))?;
    let tool = entry
        .field("tool")
        .and_then(Json::as_str)
        .ok_or_else(|| bad_log("missing tool"))?;
    let mut args = entry
        .field("args")
        .cloned()
        .ok_or_else(|| bad_log("missing args"))?;
    let mut error = entry
        .field("error")
        .filter(|error| error.as_object().is_some())
        .cloned()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "repro requires a failing tool call",
            )
        })?;
    let mut transcript = Vec::new();
    let mut number = 1;
    if let Json::Object(fields) = &mut args
        && entry.field("project").and_then(Json::as_str).is_some()
        && tool != "project.info"
        && fields.iter().any(|(key, _)| key == "projectId")
    {
        transcript_line(
            &mut transcript,
            b'>',
            &request(
                number,
                "project.info",
                object(vec![("path", text("$project"))]),
            ),
        )?;
        transcript_line(
            &mut transcript,
            b'<',
            &response(number, object(vec![("projectId", text("prj_1"))]), false),
        )?;
        update_field(fields, "projectId", text("prj_1"));
        number += 1;
    }
    if matches!(tool, "project.info" | "record.start")
        && args
            .field("path")
            .and_then(Json::as_str)
            .is_some_and(|path| super::project::package_path(path).is_ok())
        && let Json::Object(fields) = &mut args
        && let Json::Object(error_fields) = &mut error
    {
        update_field(fields, "path", text("$project"));
        update_field(error_fields, "message", text("…"));
    }
    transcript_line(&mut transcript, b'>', &request(number, tool, args))?;
    transcript_line(
        &mut transcript,
        b'<',
        &response(number, object(vec![("error", error)]), true),
    )?;
    fs::create_dir_all(directory)?;
    let file = directory.join(format!("friction-{id}.jsonl"));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&file)?
        .write_all(&transcript)?;
    writeln!(output, "{}", file.display())
}

fn request(id: usize, tool: &str, args: Json) -> Json {
    object(vec![
        ("jsonrpc", text("2.0")),
        ("id", count(id)),
        ("method", text("tools/call")),
        (
            "params",
            object(vec![("name", text(tool)), ("arguments", args)]),
        ),
    ])
}

fn response(id: usize, content: Json, error: bool) -> Json {
    object(vec![
        ("jsonrpc", text("2.0")),
        ("id", count(id)),
        (
            "result",
            object(vec![
                ("structuredContent", content),
                ("isError", Json::Bool(error)),
            ]),
        ),
    ])
}

fn transcript_line(output: &mut Vec<u8>, prefix: u8, value: &Json) -> io::Result<()> {
    output.extend([prefix, b' ']);
    write_json(output, value)
}

fn write_json(output: &mut dyn Write, value: &Json) -> io::Result<()> {
    let mut wire = String::new();
    splice_json::write(value, &mut wire).map_err(io::Error::other)?;
    writeln!(output, "{wire}")
}
