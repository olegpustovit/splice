use super::*;

#[test]
fn capture_permission_denial_requires_human_action() {
    let error = splice_core::history::StoreError::new(
        "TCC_DENIED",
        "Screen & System Audio Recording is unavailable.",
        "Run splice permissions --request screen.",
    );
    let payload = wire::error_payload(error);
    assert_eq!(
        payload.field("needsHuman").and_then(Json::as_bool),
        Some(true)
    );
}

#[test]
fn viewer_projects_pin_the_single_open_store() {
    let dir = std::env::temp_dir().join(format!("splice-viewer-pin-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("Selected.splice");
    let store = splice_core::history::Store::create(&path, &Default::default()).unwrap();
    let mut projects = project::Projects::viewer(store);
    let opened = projects
        .info(&object(vec![("path", text(path.to_str().unwrap()))]))
        .unwrap();
    assert_eq!(
        opened.field("projectId").and_then(Json::as_str),
        Some("prj_1")
    );
    let other = dir.join("Other.splice");
    let error = projects
        .info(&object(vec![
            ("path", text(other.to_str().unwrap())),
            ("create", Json::Bool(true)),
        ]))
        .unwrap_err();
    assert_eq!(error.code, "NOT_FOUND");
    assert!(!other.exists());
    projects.close().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
use splice_testkit::transcript;
use std::fs;
use std::io::{self, BufRead, Cursor, Write};
use std::path::{Path, PathBuf};

fn isolated_serve(input: &mut (dyn BufRead + Send), output: &mut dyn Write) -> io::Result<()> {
    rpc::serve_with_log(input, output, None, false)
}

const LEGACY_PRELUDE: &str = concat!(
    r#"> {"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fixtures","version":"1"}}}"#,
    "\n",
    r#"< {"jsonrpc":"2.0","id":0,"result":{"protocolVersion":"2025-11-25"}}"#,
    "\n",
    r#"> {"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    "\n",
);

fn mcp_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/mcp")
}

fn replay_headless(input: &[u8]) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    isolated_serve(&mut Cursor::new(input), &mut output)?;
    Ok(output)
}

#[test]
fn stdio_conformance_transcript() {
    transcript::run(
        &mcp_fixtures().join("rpc-conformance.jsonl"),
        "",
        &[],
        &mut replay_headless,
    )
    .unwrap();
}

#[test]
fn tools_list_golden() {
    let mut input = Cursor::new(concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        "\n"
    ));
    let mut output = Vec::new();
    isolated_serve(&mut input, &mut output).unwrap();
    let output = String::from_utf8(output).unwrap();
    let response = output.lines().nth(1).unwrap();
    let message = splice_json::parse(response).unwrap();
    let result = message.field("result").unwrap();
    let tools = result.field("tools").unwrap().as_array().unwrap();
    assert_eq!(tools.len(), 21);
    for tool in tools {
        for field in ["inputSchema", "outputSchema"] {
            assert_eq!(
                tool.field(field).unwrap().field("type").unwrap().as_str(),
                Some("object")
            );
        }
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/mcp/tools-list.json");
    let mut json = String::new();
    splice_json::write(result, &mut json).unwrap();
    splice_testkit::golden(&path, json.as_bytes()).unwrap();
}

#[test]
fn stdio_recovers_after_oversized_and_invalid_utf8_lines() {
    use std::io::{BufReader, Read, repeat};

    let input = repeat(b' ').take(16 * 1024 * 1024 + 128).chain(Cursor::new(
        b"\n\xff\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\r\n",
    ));
    let mut output = Vec::new();
    isolated_serve(&mut BufReader::new(input), &mut output).unwrap();
    let output = String::from_utf8(output).unwrap();
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        [
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Message exceeds 16 MiB"}}"#,
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
        ]
    );
}

#[test]
fn tool_transcripts_run_against_live_store() {
    let fixtures = mcp_fixtures();
    let test_runs = TestDirectory::new("tool-transcripts");
    let mut cases: Vec<_> = fs::read_dir(&fixtures)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
        .collect();
    cases.sort();
    let list =
        splice_json::parse(&fs::read_to_string(fixtures.join("tools-list.json")).unwrap()).unwrap();
    for tool in list.field("tools").unwrap().as_array().unwrap() {
        let name = tool.field("name").unwrap().as_str().unwrap();
        assert!(
            cases.contains(&fixtures.join(format!("{name}.jsonl"))),
            "missing transcript for {name}"
        );
    }

    let template = test_runs.0.join("template.splice");
    copy_fixture(&fixtures.join("../projects/sample.splice"), &template).unwrap();
    let take = template.join("takes/med_screen");
    fs::create_dir_all(&take).unwrap();
    fs::write(take.join("events.json"), r#"{"events":[{"t":0.2,"k":"move","x":0.25,"y":0.5},{"t":0.4,"k":"down","b":0},{"t":0.5,"k":"key","ch":"private","keyCode":4}]}"#).unwrap();

    for case in cases {
        let name = case.file_stem().and_then(|stem| stem.to_str()).unwrap();
        // These cases exercise explicit protocol handshakes and reconnect faults.
        if matches!(name, "rpc-conformance" | "friction-recovery") {
            continue;
        }
        // Native media is covered by the macOS CLI and MCP subprocess acceptance tests.
        if cfg!(target_os = "macos")
            && matches!(
                name,
                "export.start" | "view.frames" | "record.start" | "record.stop"
            )
        {
            continue;
        }
        let directory = test_runs.0.join(name);
        let sample = directory.join("sample.splice");
        let new = directory.join("new.splice");
        copy_fixture(&template, &sample).unwrap();

        let seed = case.with_extension("splice");
        let project = directory.join("repro.splice");
        let mut paths = vec![("$sample", sample.as_path()), ("$new", new.as_path())];
        if seed.is_dir() {
            copy_fixture(&seed, &project).unwrap();
            paths.push(("$project", &project));
        }
        transcript::run(&case, LEGACY_PRELUDE, &paths, &mut |input| {
            let output = replay_headless(input)?;
            if name == "takes.events" || name == "media.analyze" {
                let wire = std::str::from_utf8(&output).map_err(io::Error::other)?;
                assert!(!wire.contains("private"), "{name} disclosed key text");
            }
            Ok(output)
        })
        .unwrap();

        if name == "edit.apply" {
            let reopened = splice_core::history::Store::open(&sample).unwrap();
            assert_eq!(reopened.project().rev, 1);
            assert_eq!(reopened.records().len(), 1);
        }
        if name == "session.end" {
            use splice_core::history::{Scope, Store, WriteOptions};

            let mut reopened = Store::open(&sample).unwrap();
            let session = &reopened.sessions()[0];
            assert!(!session.open);
            let undo = reopened
                .undo(
                    Scope::Session,
                    &WriteOptions {
                        session: Some(session.id),
                        ..WriteOptions::default()
                    },
                )
                .unwrap();
            assert_eq!(undo.reverted, [1]);
            assert_eq!(undo.revision, 2);
        }
        if name == "record.start" {
            assert!(!new.exists(), "capture dry run created a project");
        }
    }
}

fn copy_fixture(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let copy = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_fixture(&entry.path(), &copy)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), copy)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "fixture contains a nonregular file at {}",
                    entry.path().display()
                ),
            ));
        }
    }
    Ok(())
}

pub(super) struct TestDirectory(pub(super) PathBuf);

impl TestDirectory {
    pub(super) fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("splice-{name}-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn bundle_paths_follow_the_installed_executable_symlink() {
    use std::os::unix::fs::symlink;

    let root = TestDirectory::new("bundle-paths");
    let bundle = root.0.join("Applications/Splice Test.app");
    let executable = bundle.join("Contents/MacOS/splice");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, b"").unwrap();
    let bin = root.0.join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    let link = bin.join("splice");
    symlink(
        "../../Applications/Splice Test.app/Contents/MacOS/splice",
        &link,
    )
    .unwrap();

    let expected = Some(bundle.canonicalize().unwrap());
    assert_eq!(
        paths::bundle_from_executable(&executable).unwrap(),
        expected
    );
    assert_eq!(paths::bundle_from_executable(&link).unwrap(), expected);

    let loose = root.0.join("target/debug/splice");
    fs::create_dir_all(loose.parent().unwrap()).unwrap();
    fs::write(&loose, b"").unwrap();
    assert_eq!(paths::bundle_from_executable(&loose).unwrap(), None);
    assert_eq!(
        paths::bundle_from_executable(&bin.join("missing"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound,
    );
}

#[test]
fn friction_transcript_tracks_session_recovery_and_writes_repro() {
    let root = TestDirectory::new("friction");
    let global = root.0.join("state");
    let fixtures = root.0.join("fixtures");
    let project = root.0.join("new.splice");
    let mut connections = 0;
    transcript::run(
        &mcp_fixtures().join("friction-recovery.jsonl"),
        LEGACY_PRELUDE,
        &[("$new", &project)],
        &mut |input| {
            let mut output = Vec::new();
            rpc::serve_with_log(
                &mut Cursor::new(input),
                &mut output,
                Some(global.clone()),
                false,
            )?;
            connections += 1;
            if connections == 1 {
                for path in [
                    global.join("friction.jsonl"),
                    project.join("friction.jsonl"),
                ] {
                    fs::OpenOptions::new()
                        .append(true)
                        .open(path)?
                        .write_all(b"{\"partial\":\"\xff")?;
                }
            }
            Ok(output)
        },
    )
    .unwrap();
    assert_eq!(connections, 2);

    let entries = friction::read_entries(&global.join("friction.jsonl")).unwrap();
    assert_eq!(
        entries,
        friction::read_entries(&project.join("friction.jsonl")).unwrap()
    );
    assert_eq!(entries.len(), 3);
    let error = &entries[0];
    assert_eq!(
        error.field("client").and_then(Json::as_str),
        Some("fixtures 1")
    );
    assert_eq!(error.field("session").and_then(Json::as_str), Some("ses_1"));
    assert_eq!(error.field("recovered").and_then(Json::as_bool), Some(true));
    assert_eq!(
        error.field("command").and_then(Json::as_str),
        Some("marker.add")
    );
    let next: Vec<_> = error
        .field("next")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|call| call.field("tool").and_then(Json::as_str).unwrap())
        .collect();
    assert_eq!(next, ["timeline.snapshot", "edit.apply", "history.undo"]);
    assert_eq!(
        entries[1].field("code").and_then(Json::as_str),
        Some("NOTE")
    );
    assert_eq!(
        entries[2].field("code").and_then(Json::as_str),
        Some("AGENT_UNDO")
    );

    let id = error.field("id").and_then(Json::as_str).unwrap().to_owned();
    let mut cli = Vec::new();
    friction::run(
        &global,
        &fixtures,
        &["list".into(), "--open".into()],
        &mut cli,
    )
    .unwrap();
    assert!(
        String::from_utf8(cli.clone())
            .unwrap()
            .contains("marker.add")
    );
    cli.clear();
    friction::run(&global, &fixtures, &["show".into(), id.clone()], &mut cli).unwrap();
    assert_eq!(
        splice_json::parse(std::str::from_utf8(&cli).unwrap()).unwrap(),
        *error
    );
    let arguments = ["repro".into(), id.clone()];
    cli.clear();
    friction::run(&global, &fixtures, &arguments, &mut cli).unwrap();
    let repro = fixtures.join(format!("friction-{id}.jsonl"));
    let reproduced = fs::read_to_string(&repro).unwrap();
    assert!(reproduced.contains(r#""name":"edit.apply""#));
    assert!(reproduced.contains(r#""code":"BAD_ARGS""#));
    assert!(reproduced.contains(r#""cmd":"marker.add""#));
    assert!(!reproduced.contains(project.to_str().unwrap()));
    let seed = root.0.join("repro.splice");
    let mut store =
        splice_core::history::Store::create(&seed, &splice_model::Project::default()).unwrap();
    store.start_session("first", 0).unwrap();
    store.close().unwrap();
    transcript::run(
        &repro,
        LEGACY_PRELUDE,
        &[("$project", &seed)],
        &mut replay_headless,
    )
    .unwrap();

    assert_eq!(
        friction::run(&global, &fixtures, &arguments, &mut cli)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );

    friction::run(
        &global,
        &fixtures,
        &["resolve".into(), id.clone(), "1234abc".into()],
        &mut cli,
    )
    .unwrap();
    let updated = friction::read_entries(&global.join("friction.jsonl")).unwrap();
    assert_eq!(
        updated[0].field("resolved").and_then(Json::as_str),
        Some("1234abc")
    );
    assert_eq!(
        updated,
        friction::read_entries(&project.join("friction.jsonl")).unwrap()
    );
    cli.clear();
    friction::run(
        &global,
        &fixtures,
        &["list".into(), "--open".into()],
        &mut cli,
    )
    .unwrap();
    assert!(!std::str::from_utf8(&cli).unwrap().contains(&id));
}

#[test]
fn export_commit_keeps_a_concurrent_destination_and_removes_staging() {
    let temp = TestDirectory::new("export-commit");
    let file = temp.0.join("movie.mp4");
    let staged = super::export_file::Output::prepare(file.clone(), false).unwrap();
    fs::write(staged.staged(), b"finished export").unwrap();
    fs::write(&file, b"another writer").unwrap();
    assert_eq!(staged.publish().unwrap_err().code, "EXISTS");
    drop(staged);
    assert_eq!(fs::read(&file).unwrap(), b"another writer");
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);

    let staged = super::export_file::Output::prepare(file.clone(), true).unwrap();
    fs::write(staged.staged(), b"replacement").unwrap();
    staged.publish().unwrap();
    drop(staged);
    assert_eq!(fs::read(&file).unwrap(), b"replacement");
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
}

#[test]
fn frame_selection_checks_bounds_and_rounds_filmstrip_samples() {
    use super::platform::FrameSelection;
    let args = splice_json::parse(r#"{"count":4}"#).unwrap();
    let frames = FrameSelection::parse(&args).unwrap().resolve(11).unwrap();
    assert_eq!(frames, [0, 3, 7, 10]);
    let args = splice_json::parse(r#"{"at":[4,0,4,2]}"#).unwrap();
    assert_eq!(
        FrameSelection::parse(&args).unwrap().resolve(5).unwrap(),
        [4, 0, 4, 2]
    );
    assert_eq!(
        FrameSelection::parse(&args)
            .unwrap()
            .resolve(4)
            .unwrap_err()
            .code,
        "OUT_OF_RANGE"
    );
    for invalid in [
        r#"{"at":[-1]}"#,
        r#"{"at":[]}"#,
        r#"{"count":0}"#,
        r#"{"count":17}"#,
        r#"{"count":1,"at":[0]}"#,
    ] {
        let args = splice_json::parse(invalid).unwrap();
        assert!(FrameSelection::parse(&args).is_err(), "{invalid}");
    }
}

#[test]
fn code_profile_transcript_runs_tools_in_one_call() {
    let root = TestDirectory::new("code-profile");
    let sample = root.0.join("sample.splice");
    copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &sample).unwrap();
    // Paths inside JavaScript are not `path` fields, so bind them as text.
    let fixture = fs::read_to_string(mcp_fixtures().join("code/code.run.jsonl")).unwrap();
    let case = root.0.join("code.run.jsonl");
    fs::write(&case, fixture.replace("$sample", sample.to_str().unwrap())).unwrap();
    transcript::run(&case, LEGACY_PRELUDE, &[], &mut |input| {
        let mut output = Vec::new();
        rpc::serve_with_log(&mut Cursor::new(input), &mut output, None, true)?;
        Ok(output)
    })
    .unwrap();
    let reopened = splice_core::history::Store::open(&sample).unwrap();
    assert_eq!(reopened.project().rev, 1);
}

#[test]
fn code_mode_declarations_golden() {
    let text = typescript::declarations(Some("*"));
    for row in splice_core::commands::COMMANDS {
        if row.tier == splice_core::commands::Tier::Planned {
            // Planned rows only fail; code mode leaves them out (commands.describe lists them).
            assert!(
                !text.contains(&format!("cmd: \"{}\";", row.id)),
                "{}",
                row.id
            );
            continue;
        }
        assert!(
            text.contains(&format!("cmd: \"{}\";", row.id)),
            "{}",
            row.id
        );
        for (name, hint) in row.params {
            let optional = if hint.starts_with('!') { "" } else { "?" };
            assert!(
                text.contains(&format!("      {name}{optional}: ")),
                "{} {name}",
                row.id
            );
        }
    }
    assert!(typescript::declarations(Some("item.split")).contains("interface ItemSplit"));
    assert!(!typescript::declarations(Some("item.split")).contains("interface ItemTrim"));
    let path = mcp_fixtures().join("code/splice.d.ts");
    splice_testkit::golden(&path, text.as_bytes()).unwrap();
}

/// A polling loop reports one trace row with a count, not one row per call.
#[test]
fn code_run_collapses_repeated_calls_in_the_trace() {
    let args = splice_json::parse(
        r#"{"code":"for (let i = 0; i < 50; i++) splice.jobs.status({jobId: \"job_1\"}); splice.project.info({path: \"a.splice\"}); return 1;"}"#,
    )
    .unwrap();
    let mut seen = 0;
    let reply = code::run(&args, &mut |name, _| {
        seen += 1;
        Ok(object(vec![(
            "structuredContent",
            object(vec![("tool", text(name))]),
        )]))
    })
    .unwrap();
    assert_eq!(seen, 51, "{reply:?}");
    let calls: Vec<_> = reply
        .field("calls")
        .and_then(Json::as_array)
        .unwrap()
        .iter()
        .map(|call| {
            (
                call.field("tool")
                    .and_then(Json::as_str)
                    .unwrap()
                    .to_owned(),
                call.field("count").and_then(Json::as_u64),
            )
        })
        .collect();
    assert_eq!(
        calls,
        [
            ("jobs.status".to_owned(), Some(50)),
            ("project.info".to_owned(), None)
        ]
    );
}

/// An absent revision says "Missing" and names the value to pass; a malformed one says "Invalid".
#[test]
fn write_arguments_distinguish_missing_from_invalid() {
    use super::wire::Args;
    let missing = splice_json::parse("{}")
        .unwrap()
        .revision("ifRevision")
        .unwrap_err();
    assert_eq!(missing.message, "Missing ifRevision.");
    assert!(missing.fix.contains("current revision"), "{}", missing.fix);
    let invalid = splice_json::parse(r#"{"ifRevision":"rx"}"#)
        .unwrap()
        .revision("ifRevision")
        .unwrap_err();
    assert_eq!(invalid.message, "Invalid ifRevision.");
    assert_eq!(
        splice_json::parse(r#"{"ifRevision":"r7"}"#)
            .unwrap()
            .revision("ifRevision")
            .unwrap(),
        7
    );
}

/// T14.6: the unfiltered declarations are an index: every tool's signature and every command's
/// name, at a fraction of the full surface; a comma-separated query returns full types for each.
#[test]
fn code_mode_declarations_index_and_multi_query() {
    let index = typescript::declarations(None);
    let full = typescript::declarations(Some("*"));
    // T23.1: discovery stays small as rows grow.
    assert!(index.len() <= 12_000, "index is {} chars", index.len());
    assert!(
        index.len() * 2 < full.len(),
        "index {} vs full {}",
        index.len(),
        full.len()
    );
    for row in splice_core::commands::COMMANDS {
        let listed = index.lines().any(|line| {
            line.starts_with("   *   ") && line.split([':', ',']).any(|id| id.trim() == row.id)
        });
        assert_eq!(
            listed,
            row.tier != splice_core::commands::Tier::Planned,
            "{}",
            row.id
        );
    }
    for tool in super::tools::TOOLS {
        let method = tool.name.split_once('.').unwrap().1;
        assert!(index.contains(&format!("    {method}(")), "{}", tool.name);
    }
    assert!(!index.contains("interface ClipCrop"));
    let slice = typescript::declarations(Some("clip.crop, effect.add"));
    assert!(slice.contains("interface ClipCrop") && slice.contains("interface EffectAdd"));
    assert!(!slice.contains("interface ItemSplit"));
}

/// Eval friction: sound items were labelled "Scale animation" and lanes were not inspectable.
#[test]
fn inspect_labels_sound_items_by_file_and_reads_lanes() {
    use splice_model::*;
    let mut p = Project::default();
    p.media.push(Media {
        id: MediaId::from_serial(2),
        kind: MediaKind::Audio,
        path: "music.wav".into(),
        bookmark: None,
        size: 1,
        mtime: 0,
        probe: Probe {
            fps: 30.0,
            frames: 90,
            width: 0,
            height: 0,
            audio: Vec::new(),
            codec: None,
            loudness: None,
        },
        take: None,
    });
    let lane = LaneId::from_serial(7);
    p.lanes.push(Lane {
        id: lane,
        name: "Music".into(),
        kind: LaneKind::Audio,
        effects: Vec::new(),
    });
    p.items
        .insert(Item {
            id: ItemId::from_serial(5),
            timing: ItemTiming {
                anchor: Anchor::Pinned(0),
                length: 90,
            },
            data: ItemData {
                lane,
                kind: ItemKind::Sound(Sound {
                    media: MediaId::from_serial(2),
                    source: 0,
                    volume: 1.0,
                    duck: None,
                }),
                appearance: Appearance::default(),
            },
        })
        .unwrap();
    let args = splice_json::parse(r#"{"ids":["ovl_5","lan_7"]}"#).unwrap();
    let reply = timeline::inspect(&p, &args).unwrap();
    let items = reply.field("items").and_then(Json::as_array).unwrap();
    let label = items[0].field("label").and_then(Json::as_str);
    assert_eq!(label, Some("music.wav"), "{:?}", items[0]);
    assert_eq!(
        items[1].field("laneKind").and_then(Json::as_str),
        Some("audio"),
        "{:?}",
        items[1]
    );
    assert_eq!(items[1].field("name").and_then(Json::as_str), Some("Music"));
}

/// A program that starts a job and then fails still reports the job, so it can be polled.
#[test]
fn code_run_reports_started_jobs_even_when_the_program_fails() {
    let args = splice_json::parse(
        r#"{"code":"splice.export.start({ifRevision: 1}); throw new Error(\"later\");"}"#,
    )
    .unwrap();
    let reply = code::run(&args, &mut |_, _| {
        Ok(object(vec![(
            "structuredContent",
            object(vec![("jobId", text("job_3")), ("state", text("running"))]),
        )]))
    })
    .unwrap();
    assert!(reply.field("error").is_some(), "{reply:?}");
    assert_eq!(
        reply.field("jobs"),
        Some(&Json::Array(vec![text("job_3")])),
        "{reply:?}"
    );
}

/// Eval friction: after project.info {path} and session.start in a program, later calls default to them.
#[test]
fn code_run_adopts_the_first_opened_project_and_session() {
    let args = splice_json::parse(
        r#"{"code":"splice.project.info({path: \"a.splice\"}); splice.session.start({}); splice.timeline.snapshot({}); return [splice.projectId, splice.sessionId];"}"#,
    )
    .unwrap();
    let mut seen = Vec::new();
    let reply = code::run(&args, &mut |name, args| {
        seen.push((
            name.to_owned(),
            args.field("projectId").cloned(),
            args.field("sessionId").cloned(),
        ));
        let body = match name {
            "project.info" => object(vec![("projectId", text("prj_4"))]),
            "session.start" => object(vec![("sessionId", text("ses_9"))]),
            _ => object(Vec::new()),
        };
        Ok(object(vec![("structuredContent", body)]))
    })
    .unwrap();
    assert_eq!(
        reply.field("result"),
        Some(&Json::Array(vec![text("prj_4"), text("ses_9")])),
        "{reply:?}"
    );
    assert_eq!(
        seen[2],
        ("timeline.snapshot".to_owned(), Some(text("prj_4")), None)
    );
    assert_eq!(seen[1].1, Some(text("prj_4")));
}

/// Found by the record-app-landscape code-mode eval: record.start opened ses_1 outside the
/// program, and edit.apply failed with NO_SESSION, then "Missing ifRevision".
#[test]
fn code_run_adopts_an_open_session_and_the_revision_it_read() {
    let args = splice_json::parse(
        r#"{"projectId":"prj_1","code":"splice.timeline.snapshot({}); splice.edit.apply({ops:[]}); splice.edit.apply({ops:[]}); return splice.sessionId;"}"#,
    )
    .unwrap();
    let mut seen = Vec::new();
    let mut edits = 0;
    let reply = code::run(&args, &mut |name, args| {
        seen.push((
            name.to_owned(),
            args.field("sessionId").cloned(),
            args.field("ifRevision").cloned(),
        ));
        let body = match name {
            "project.info" => object(vec![
                ("projectId", text("prj_1")),
                ("revision", text("r1")),
                ("openSession", text("ses_1")),
            ]),
            "timeline.snapshot" => object(vec![("revision", text("r1"))]),
            "edit.apply" => {
                edits += 1;
                object(vec![("revision", text(&format!("r{}", 1 + edits)))])
            }
            _ => object(Vec::new()),
        };
        Ok(object(vec![("structuredContent", body)]))
    })
    .unwrap();
    assert_eq!(reply.field("result"), Some(&text("ses_1")), "{reply:?}");
    let edits: Vec<_> = seen
        .iter()
        .filter(|(name, ..)| name == "edit.apply")
        .collect();
    assert_eq!(edits[0].1, Some(text("ses_1")));
    assert_eq!(edits[0].2, Some(text("r1")));
    // The next edit's precondition is the revision the first one made.
    assert_eq!(edits[1].2, Some(text("r2")));
}

/// Found by record-app-landscape (code mode): the program passed projectId on every call,
/// so no default followed and edit.apply failed with NO_SESSION; the agent filed friction.
#[test]
fn code_run_adopts_a_project_named_on_a_call() {
    let args = object(vec![(
        "code",
        text(
            r#"splice.call("edit.apply", {projectId: "prj_1", ops: []}); return splice.projectId;"#,
        ),
    )]);
    let mut seen = Vec::new();
    code::run(&args, &mut |name, args| {
        seen.push((name.to_owned(), args.field("sessionId").cloned()));
        let body = match name {
            "project.info" => object(vec![
                ("projectId", text("prj_1")),
                ("revision", text("r1")),
                ("openSession", text("ses_4")),
            ]),
            _ => object(vec![("revision", text("r2"))]),
        };
        Ok(object(vec![("structuredContent", body)]))
    })
    .unwrap();
    let edit = seen.iter().find(|(name, _)| name == "edit.apply").unwrap();
    assert_eq!(edit.1, Some(text("ses_4")), "{seen:?}");
}

/// Found by the evals: agents call project.open and job.status inside code.run.
#[test]
fn code_run_suggests_the_tool_an_unknown_name_meant() {
    // Names that mean one tool with the same arguments are that tool.
    for (alias, tool) in [
        ("project.open", "project.info"),
        ("job.status", "jobs.status"),
    ] {
        let args = object(vec![(
            "code",
            text(&format!(r#"splice.call("{alias}", {{}})"#)),
        )]);
        let mut called = Vec::new();
        code::run(&args, &mut |name, _| {
            called.push(name.to_owned());
            Ok(object(vec![("structuredContent", object(Vec::new()))]))
        })
        .unwrap();
        assert_eq!(called, [tool]);
    }
    for (code, near) in [
        (r#"splice.call("timeline.inspect", {})"#, "items.inspect"),
        (
            r#"splice.call("timeline.snapshots", {})"#,
            "timeline.snapshot",
        ),
    ] {
        let args = object(vec![("code", text(code))]);
        let reply = code::run(&args, &mut |_, _| unreachable!("not dispatched")).unwrap();
        let message = reply
            .field("error")
            .and_then(|e| e.field("message"))
            .unwrap();
        assert!(
            message
                .as_str()
                .unwrap()
                .contains(&format!("Did you mean {near:?}")),
            "{reply:?}"
        );
    }
}

/// The sample project, loaded through the package reader.
pub(crate) fn sample_project() -> splice_model::Project {
    let root = TestDirectory::new("sample-project");
    let sample = root.0.join("sample.splice");
    copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &sample).unwrap();
    splice_core::history::Store::open(&sample)
        .unwrap()
        .project()
        .clone()
}

#[test]
fn edit_apply_fills_cursor_crops_and_typing_zooms() {
    let root = TestDirectory::new("enrich");
    let sample = root.0.join("sample.splice");
    copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &sample).unwrap();
    let take = sample.join("takes/med_screen");
    fs::create_dir_all(&take).unwrap();
    fs::write(
        take.join("events.json"),
        r#"{"events":[{"t":0.2,"k":"move","x":0.9,"y":0.1},{"t":0.4,"k":"down","b":0,"x":0.9,"y":0.1},{"t":4.0,"k":"move","x":0.1,"y":0.9},{"t":5.0,"k":"key","ch":"a","keyCode":4}]}"#,
    )
    .unwrap();
    let store = splice_core::history::Store::open(&sample).unwrap();
    let ops = splice_json::parse(
        r#"[{"cmd":"crop.keys","item":"clp_take","follow":"cursor","aspect":"9:16"},
            {"cmd":"intent.autoZoom","item":"clp_take"}]"#,
    )
    .unwrap();
    let filled = super::enrich::ops(
        &store,
        ops.as_array().unwrap(),
        &mut splice_core::edit::missing_media,
    )
    .unwrap()
    .unwrap();
    let keys = filled[0].field("keys").unwrap().as_array().unwrap();
    assert!(keys.len() > 20 && filled[0].field("follow").is_none());
    let x = |k: &Json| k.field("x").unwrap().as_f64().unwrap();
    // The crop starts centered, moves right toward the early pointer, then left later.
    assert!(x(&keys[5]) > x(&keys[0]));
    assert!(x(keys.last().unwrap()) < x(&keys[5]));
    let clicks = filled[1].field("clicks").unwrap().as_array().unwrap();
    assert_eq!(
        clicks.len(),
        2,
        "one click and one typing burst: {clicks:?}"
    );
    // Applying the filled batch works.
    let mut project = store.project().clone();
    splice_core::edit::apply(
        &mut project,
        &filled,
        splice_core::package::journal::Source::Human,
        false,
    )
    .unwrap();
}

/// T45.4: caret samples (Accessibility) are listed by takes.events, and typing zooms land on the
/// caret instead of the resting pointer, unless a click since the caret's last sample moved
/// the focus. Takes without caret samples keep the pointer rule.
#[test]
fn typing_zooms_follow_the_text_caret() {
    let root = TestDirectory::new("enrich-caret");
    let sample = root.0.join("sample.splice");
    copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &sample).unwrap();
    let take = sample.join("takes/med_screen");
    fs::create_dir_all(&take).unwrap();
    let typing_focus = |events: &str| {
        fs::write(take.join("events.json"), events).unwrap();
        let store = splice_core::history::Store::open(&sample).unwrap();
        let ops = splice_json::parse(r#"[{"cmd":"intent.autoZoom","item":"clp_take"}]"#).unwrap();
        let filled = super::enrich::ops(
            &store,
            ops.as_array().unwrap(),
            &mut splice_core::edit::missing_media,
        )
        .unwrap()
        .unwrap();
        let clicks = filled[0].field("clicks").unwrap().as_array().unwrap();
        let burst = clicks.last().unwrap();
        let x = |key: &str| burst.field(key).unwrap().as_f64().unwrap();
        (x("x"), x("y"))
    };
    let base = r#"{"t":0.4,"k":"down","b":0,"x":0.9,"y":0.1},{"t":4.0,"k":"move","x":0.8,"y":0.8}"#;
    let caret = r#"{"t":4.5,"k":"caret","x":0.3,"y":0.59,"w":0.002,"h":0.02}"#;
    let key = r#"{"t":5.0,"k":"key","ch":"a","keyCode":4}"#;
    let (x, y) = typing_focus(&format!(r#"{{"events":[{base},{caret},{key}]}}"#));
    assert!(
        (x - 0.301).abs() < 1e-9 && (y - 0.6).abs() < 1e-9,
        "{x} {y}"
    );
    // A click after the caret's last sample moved the focus: back to the pointer.
    let click = r#"{"t":4.8,"k":"down","b":0,"x":0.7,"y":0.2}"#;
    let (x, y) = typing_focus(&format!(r#"{{"events":[{base},{caret},{click},{key}]}}"#));
    assert_eq!((x, y), (0.7, 0.2));
    // No caret samples (no Accessibility permission): the pointer, as before.
    let (x, y) = typing_focus(&format!(r#"{{"events":[{base},{key}]}}"#));
    assert_eq!((x, y), (0.8, 0.8));
    // takes.events lists the samples; a malformed one is a bad package, not a silent skip.
    fs::write(
        take.join("events.json"),
        format!(r#"{{"events":[{base},{caret},{key}]}}"#),
    )
    .unwrap();
    let store = splice_core::history::Store::open(&sample).unwrap();
    let args = splice_json::parse(r#"{"clip":"clp_take"}"#).unwrap();
    let listed = super::media::events(&sample, store.project(), &args).unwrap();
    let samples = listed.field("caret").unwrap().as_array().unwrap();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].field("w").and_then(Json::as_f64), Some(0.002));
    fs::write(
        take.join("events.json"),
        r#"{"events":[{"t":4.5,"k":"caret","x":0.3}]}"#,
    )
    .unwrap();
    let error = super::media::events(&sample, store.project(), &args).unwrap_err();
    assert!(format!("{error:?}").contains("BAD_PACKAGE"), "{error:?}");
}

/// Recorder's keystroke categories: takes.events names each key's category (a modifier pressed
/// alone is a `modifier` tap with no index) and marks the chips the clip's `show` draws.
/// Typed characters stay redacted; navigation glyphs are not typed text.
#[test]
fn takes_events_categorize_keys_and_mark_the_chips_each_show_draws() {
    let root = TestDirectory::new("key-categories");
    let sample = root.0.join("sample.splice");
    copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &sample).unwrap();
    let take = sample.join("takes/med_screen");
    fs::create_dir_all(&take).unwrap();
    fs::write(
        take.join("events.json"),
        r#"{"events":[
            {"t":1.0,"k":"key","ch":"a","keyCode":0,"mods":0},
            {"t":1.5,"k":"key","ch":"\u001c","keyCode":123,"mods":0},
            {"t":2.0,"k":"flags","keyCode":55,"mods":1048576},
            {"t":2.1,"k":"key","ch":"s","keyCode":1,"mods":1048576},
            {"t":2.2,"k":"flags","keyCode":55,"mods":0},
            {"t":3.0,"k":"flags","keyCode":56,"mods":131072},
            {"t":3.2,"k":"flags","keyCode":56,"mods":0},
            {"t":4.0,"k":"key","ch":"","keyCode":57,"mods":0}]}"#,
    )
    .unwrap();
    let store = splice_core::history::Store::open(&sample).unwrap();
    let listed = |project: &splice_model::Project| {
        let args = splice_json::parse(r#"{"clip":"clp_take","kinds":["keys"]}"#).unwrap();
        let listed = super::media::events(&sample, project, &args).unwrap();
        listed
            .field("keys")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|key| {
                (
                    key.field("index").and_then(Json::as_u64),
                    key.field("category")
                        .and_then(Json::as_str)
                        .unwrap()
                        .to_owned(),
                    key.field("chip") == Some(&Json::Bool(true)),
                    key.field("text").and_then(Json::as_str).map(str::to_owned),
                )
            })
            .collect::<Vec<_>>()
    };
    let mut project = store.project().clone();
    let show = |project: &mut splice_model::Project, show: &str| {
        let ops = splice_json::parse(&format!(
            r#"[{{"cmd":"keys.set","clip":"clp_take","show":"{show}"}}]"#
        ))
        .unwrap();
        splice_core::edit::apply(
            project,
            ops.as_array().unwrap(),
            splice_core::package::journal::Source::Agent,
            false,
        )
        .unwrap();
    };
    show(&mut project, "all");
    let all = listed(&project);
    let expected = [
        (Some(0), "typing", true, None),
        (Some(1), "navigation", true, Some("←")),
        (Some(2), "shortcut", true, None),
        (None, "modifier", true, Some("⇧")),
        // Caps lock types nothing: no chip, but it keeps its index.
        (Some(3), "typing", false, None),
    ];
    assert_eq!(all.len(), expected.len(), "{all:?}");
    for (got, want) in all.iter().zip(expected) {
        assert_eq!(
            (got.0, got.1.as_str(), got.2, got.3.as_deref()),
            want,
            "{all:?}"
        );
    }
    show(&mut project, "shortcuts+navigation");
    let chips: Vec<bool> = listed(&project).iter().map(|key| key.2).collect();
    assert_eq!(chips, [false, true, true, false, false]);
    show(&mut project, "modifiers");
    let chips: Vec<bool> = listed(&project).iter().map(|key| key.2).collect();
    assert_eq!(chips, [false, false, false, true, false]);
    show(&mut project, "none");
    assert!(listed(&project).iter().all(|key| !key.2));
}

/// The `keystrokes` effect's `show` options (effects.json, what apply validates) are exactly
/// the canonical category sets the renderer parses.
#[test]
fn keystroke_show_options_match_the_renderers_category_sets() {
    let options = splice_core::commands::effect_schemas()
        .field("keystrokes")
        .and_then(|e| e.field("params"))
        .and_then(|p| p.field("show"))
        .and_then(|s| s.field("options"))
        .and_then(Json::as_array)
        .unwrap();
    let mut names: Vec<&str> = options.iter().filter_map(Json::as_str).collect();
    names.sort_unstable();
    let mut canonical = splice_eval::chips::SHOW[1..].to_vec();
    canonical.sort_unstable();
    assert_eq!(names, canonical);
    let positions = splice_core::commands::effect_schemas()
        .field("keystrokes")
        .and_then(|e| e.field("params"))
        .and_then(|p| p.field("position"))
        .and_then(|s| s.field("options"))
        .and_then(Json::as_array)
        .unwrap();
    let names: Vec<&str> = positions.iter().filter_map(Json::as_str).collect();
    let grid: Vec<&str> = splice_eval::KEY_POSITIONS.iter().map(|p| p.0).collect();
    assert_eq!(names, grid);
}

/// T41: on a silent take, edit.apply fills intent.steps, intent.speedTyping and the smart cuts'
/// activity from the take's events; an `auto` basis with words on the timeline needs nothing.
#[test]
fn edit_apply_fills_activity_for_silent_takes() {
    let root = TestDirectory::new("enrich-activity");
    let package = |name: &str| {
        let path = root.0.join(name);
        copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &path).unwrap();
        fs::create_dir_all(path.join("takes/med_screen")).unwrap();
        path
    };
    let (sample, silent) = (package("sample.splice"), package("silent.splice"));
    fs::remove_file(silent.join("transcripts/med_screen.json")).unwrap();
    // A click, typing from 2 s to 4 s (and a shortcut at 5 s), a scroll at 9 s.
    let mut events = String::from(
        r#"{"events":[{"t":0.25,"k":"down","b":0,"x":0.5,"y":0.5},{"t":0.5,"k":"up","b":0}"#,
    );
    for i in 0..=8 {
        events.push_str(&format!(
            r#",{{"t":{},"k":"key","ch":"secret","keyCode":4}}"#,
            2.0 + f64::from(i) * 0.25
        ));
    }
    events.push_str(r#",{"t":5.0,"k":"key","mods":1048576,"keyCode":1},{"t":9.0,"k":"scroll"}]}"#);
    for path in [&sample, &silent] {
        fs::write(path.join("takes/med_screen/events.json"), &events).unwrap();
    }
    let ops = |text: &str| {
        splice_json::parse(text)
            .unwrap()
            .as_array()
            .unwrap()
            .to_vec()
    };
    let fill = |store: &splice_core::history::Store, batch: &[Json]| {
        super::enrich::ops(store, batch, &mut splice_core::edit::missing_media).unwrap()
    };
    // With words on the timeline, auto measures the transcript: nothing to fill.
    let spoken = splice_core::history::Store::open(&sample).unwrap();
    let auto = ops(r#"[{"cmd":"cut.pauses"}]"#);
    let filled = fill(&spoken, &auto).unwrap_or(auto.clone());
    assert!(filled[0].field("activity").is_none());
    let store = splice_core::history::Store::open(&silent).unwrap();
    let batch = ops(r#"[{"cmd":"intent.steps","item":"clp_take"},
            {"cmd":"intent.speedTyping","item":"clp_take","minRun":30},
            {"cmd":"cut.pauses"}]"#);
    let filled = fill(&store, &batch).unwrap();
    let text = |value: &Json| {
        let mut out = String::new();
        splice_json::write(value, &mut out).unwrap();
        out
    };
    assert!(!text(&Json::Array(filled.clone())).contains("secret"));
    let busy =
        r#"[{"from":7,"to":18},{"from":60,"to":123},{"from":150,"to":153},{"from":270,"to":273}]"#;
    assert_eq!(text(filled[0].field("activity").unwrap()), busy);
    assert_eq!(
        text(filled[1].field("typing").unwrap()),
        r#"[{"from":60,"to":123}]"#
    );
    assert_eq!(
        text(filled[2].field("activity").unwrap()),
        busy.replace("{\"from\"", "{\"media\":\"med_screen\",\"from\"")
    );
    // Each filled op applies to the silent take.
    for op in &filled {
        let mut project = store.project().clone();
        splice_core::edit::apply(
            &mut project,
            std::slice::from_ref(op),
            splice_core::package::journal::Source::Agent,
            false,
        )
        .unwrap_or_else(|error| panic!("{}: {error}", text(op)));
    }
    // A clip that is not a take is refused with the rule.
    let error = super::enrich::ops(
        &store,
        &ops(r#"[{"cmd":"intent.steps","item":"clp_open"}]"#),
        &mut splice_core::edit::missing_media,
    )
    .unwrap_err();
    assert_eq!(error.code, "WRONG_KIND");
    assert_eq!(error.rule.unwrap().param, "item");
    // T41.5: a free-text cue on the silent take exports as SRT and lists no words.
    let mut project = store.project().clone();
    splice_core::edit::apply(
        &mut project,
        &ops(r#"[{"cmd":"caption.add","text":"Open the settings","from":150,"to":210}]"#),
        splice_core::package::journal::Source::Agent,
        false,
    )
    .unwrap();
    let srt = super::media::captions(&project, &Json::Object(Vec::new()), "srt", None).unwrap();
    assert_eq!(
        srt.field("text").and_then(Json::as_str),
        Some("1\n00:00:05,000 --> 00:00:07,000\nOpen the settings\n\n")
    );
    let json = super::media::captions(&project, &Json::Object(Vec::new()), "json", None).unwrap();
    let cue = &json.field("cues").unwrap().as_array().unwrap()[0];
    assert_eq!(text(cue.field("words").unwrap()), "[]");
}

#[test]
fn m10_enrich_resolves_refs_and_refuses_overlays() {
    let root = TestDirectory::new("enrich-refs");
    let sample = root.0.join("sample.splice");
    copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &sample).unwrap();
    let take = sample.join("takes/med_screen");
    fs::create_dir_all(&take).unwrap();
    fs::write(
        take.join("events.json"),
        // Inside the sample's crop (x 0.36–0.82, y 0.1–0.9): clicks off it move no zoom (A5).
        r#"{"events":[{"t":0.4,"k":"down","b":0,"x":0.7,"y":0.2},{"t":4.0,"k":"move","x":0.4,"y":0.8},{"t":5.0,"k":"key","ch":"a","keyCode":4}]}"#,
    )
    .unwrap();
    let mut store = splice_core::history::Store::open(&sample).unwrap();
    let session = store.start_session("test", 1).unwrap().id;
    // The right half exists only inside the batch: its clicks come from the staged split.
    let ops = splice_json::parse(
        r#"[{"cmd":"item.split","item":"clp_take","at":210,"ref":"cut"},
            {"cmd":"intent.autoZoom","item":"$cut.right"},
            {"cmd":"crop.keys","item":"$cut","follow":"cursor","aspect":"9:16"}]"#,
    )
    .unwrap();
    let ops = ops.as_array().unwrap();
    let filled = super::enrich::ops(&store, ops, &mut splice_core::edit::missing_media)
        .unwrap()
        .unwrap();
    let clicks = filled[1].field("clicks").unwrap().as_array().unwrap();
    // Only the typing burst at 5 s lies in the right half (source frames 90 on).
    assert_eq!(clicks.len(), 1, "{clicks:?}");
    assert_eq!(
        filled[1].field("item").and_then(Json::as_str),
        Some("$cut.right")
    );
    assert!(filled[2].field("keys").is_some());
    // `splice apply`, the viewer and MCP share one path: the batch commits enriched.
    let options = splice_core::history::WriteOptions {
        source: splice_core::history::Source::Agent,
        session: Some(session),
        if_revision: Some(store.project().rev),
        ..Default::default()
    };
    super::history::apply_batch(&mut store, ops, &options).unwrap();
    // An overlay has no take events: refused with the rule, not silently skipped.
    let error = super::enrich::ops(
        &store,
        &[splice_json::parse(r#"{"cmd":"intent.autoZoom","item":"ovl_hook"}"#).unwrap()],
        &mut splice_core::edit::missing_media,
    )
    .unwrap_err();
    assert_eq!(error.code, "WRONG_KIND");
    assert_eq!(error.rule.unwrap().param, "item");
    store.close().unwrap();
}

#[test]
fn m13_consolidate_copies_only_on_commit_and_verifies_existing_files() {
    use splice_core::history::{Source, Store, WriteOptions};
    let root = TestDirectory::new("consolidate");
    let source = root.0.join("external.mov");
    fs::write(&source, b"the real media bytes").unwrap();
    let mut project = splice_model::Project::default();
    project.media.push(splice_model::Media {
        id: "med_1".parse().unwrap(),
        kind: splice_model::MediaKind::Video,
        path: source.to_str().unwrap().into(),
        bookmark: None,
        size: 20,
        mtime: 1,
        probe: splice_model::Probe {
            fps: 30.0,
            frames: 30,
            width: 96,
            height: 64,
            audio: Vec::new(),
            codec: None,
            loudness: None,
        },
        take: None,
    });
    project.serial = 1;
    let package = root.0.join("consolidate.splice");
    let mut store = Store::create(&package, &project).unwrap();
    let session = store.start_session("test", 1).unwrap().id;
    let copy = package.join("media/med_1.mov");
    let options = |rev: u64, dry_run: bool| WriteOptions {
        source: Source::Agent,
        session: Some(session),
        if_revision: Some(rev),
        dry_run,
        ..Default::default()
    };
    let consolidate = [splice_json::parse(r#"{"cmd":"media.consolidate"}"#).unwrap()];
    let failing = [
        consolidate[0].clone(),
        splice_json::parse(r#"{"cmd":"item.delete","items":"clp_missing"}"#).unwrap(),
    ];
    super::history::apply_batch(&mut store, &consolidate, &options(0, true)).unwrap();
    assert!(super::history::apply_batch(&mut store, &consolidate, &options(7, false)).is_err());
    assert!(super::history::apply_batch(&mut store, &failing, &options(0, false)).is_err());
    assert!(
        !copy.exists(),
        "dry-run, stale and failed batches copy nothing"
    );
    // A stale file at the destination is not trusted.
    fs::create_dir_all(copy.parent().unwrap()).unwrap();
    fs::write(&copy, b"something else entirely").unwrap();
    super::history::apply_batch(&mut store, &consolidate, &options(0, false)).unwrap();
    assert_eq!(fs::read(&copy).unwrap(), b"the real media bytes");
    assert_eq!(store.project().media[0].path, "media/med_1.mov");
    store.close().unwrap();
}

#[test]
fn m11_caption_sidecars_follow_the_export_range_and_overwrite() {
    let root = TestDirectory::new("sidecar");
    let sample = root.0.join("sample.splice");
    copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &sample).unwrap();
    let mut store = splice_core::history::Store::open(&sample).unwrap();
    let p = store.project();
    let all = super::media::captions(p, &object(vec![]), "json", None).unwrap();
    let all = all.field("cues").unwrap().as_array().unwrap();
    let frame = |cue: &Json, key: &str| cue.field(key).and_then(Json::as_i64).unwrap();
    // Export a window starting inside the second cue: the first is dropped, the second is
    // clipped to the window, and every time is relative to the window's start.
    let second = &all[1];
    let from = frame(second, "from") + 3;
    let window = [from as i32, frame(second, "to") as i32 + 30];
    let ranged = super::media::captions(p, &object(vec![]), "json", Some(window)).unwrap();
    let ranged = ranged.field("cues").unwrap().as_array().unwrap();
    assert_eq!(frame(&ranged[0], "from"), 0);
    assert_eq!(frame(&ranged[0], "to"), frame(second, "to") - from);
    assert!(
        ranged
            .iter()
            .all(|cue| frame(cue, "to") <= i64::from(window[1] - window[0]))
    );
    let srt = super::media::captions(p, &object(vec![]), "srt", Some(window)).unwrap();
    let srt = srt.field("text").and_then(Json::as_str).unwrap();
    assert!(srt.starts_with("1\n00:00:00,000 --> "), "{srt}");
    assert!(srt.contains("a frame 🎯 study"));
    let hide = splice_json::parse(
        r#"{"cmd":"caption.set","item":"ovl_cap2","words":{"w:screen:5":{"text":"shot","hidden":true}}}"#,
    )
    .unwrap();
    super::history::apply_batch(&mut store, &[hide], &Default::default()).unwrap();
    let args = object(vec![]);
    let hidden = super::media::captions(store.project(), &args, "json", Some(window)).unwrap();
    let cue = &hidden.field("cues").unwrap().as_array().unwrap()[0];
    let word = &cue.field("words").unwrap().as_array().unwrap()[1];
    assert_eq!(cue.field("text").and_then(Json::as_str), Some("a study"));
    assert_eq!(word.field("text").and_then(Json::as_str), Some("shot"));
    assert_eq!(word.field("hidden"), Some(&Json::Bool(true)));
    assert_eq!(word.field("emoji").and_then(Json::as_str), Some("🎯"));
    let srt = super::media::captions(store.project(), &args, "srt", Some(window)).unwrap();
    let srt = srt.field("text").and_then(Json::as_str).unwrap();
    assert!(srt.contains("\na study\n") && !srt.contains("shot") && !srt.contains('🎯'));
    // An existing sidecar is replaced only with overwrite.
    let path = root.0.join("out.srt");
    super::export_file::write_sidecar(&path, "first", false).unwrap();
    let error = super::export_file::write_sidecar(&path, "second", false).unwrap_err();
    assert_eq!(error.code, "EXISTS");
    assert_eq!(fs::read_to_string(&path).unwrap(), "first");
    super::export_file::write_sidecar(&path, "second", true).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "second");
}

#[test]
fn low_a_sessionless_failed_export_closes_its_session() {
    let root = TestDirectory::new("export-session");
    let package = root.0.join("Export.splice");
    let mut projects = project::Projects::default();
    let opened = projects
        .info(&object(vec![
            ("path", text(package.to_str().unwrap())),
            ("create", Json::Bool(true)),
        ]))
        .unwrap();
    let id = opened.field("projectId").and_then(Json::as_str).unwrap();
    for preset in ["no-such-preset", "youtube-1080"] {
        let args = object(vec![
            ("projectId", text(id)),
            ("ifRevision", Json::Unsigned(0)),
            ("preset", text(preset)),
        ]);
        assert!(platform::export(&mut projects, &args).is_err());
    }
    let store = projects.lookup_id(id).unwrap();
    assert!(!store.sessions().is_empty());
    assert!(store.sessions().iter().all(|session| !session.open));
    projects.close().unwrap();
}

#[test]
fn t36_10_capture_dry_run_omits_window_titles_unless_asked() {
    let reply = splice_json::parse(
        r#"{"sources":{"displays":[],"windows":[{"id":"win_7","app":"Mail","title":"Re: salary","pid":1}],"applications":[]}}"#,
    )
    .unwrap();
    let windows = |reply: &Json| reply.field("sources").unwrap().field("windows").cloned();
    let private = windows(&platform::window_titles(reply.clone(), false)).unwrap();
    let window = &private.as_array().unwrap()[0];
    assert!(window.field("title").is_none());
    assert_eq!(window.field("app").and_then(Json::as_str), Some("Mail"));
    let open = platform::window_titles(reply, true);
    assert!(
        windows(&open).unwrap().as_array().unwrap()[0]
            .field("title")
            .is_some()
    );
    // The selector errors teach window:{app}.
    let error = record_options::Options::parse(
        &splice_json::parse(r#"{"source":{"window":7.5}}"#).unwrap(),
    )
    .err()
    .unwrap();
    assert!(
        error.fix.contains("window:{app") || error.fix.contains("{app:"),
        "{}",
        error.fix
    );
}

#[test]
fn save_as_copies_the_package_and_opens_the_copy() {
    let root = TestDirectory::new("save-as");
    let sample = root.0.join("sample.splice");
    copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &sample).unwrap();
    let copy = root.0.join("copy.splice");
    let request = |id: u32, args: String| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"project.info","arguments":{args}}}}}"#
        )
    };
    let input = [
        r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#.to_owned(),
        request(1, format!(r#"{{"path":"{}"}}"#, sample.display())),
        request(2, format!(r#"{{"projectId":"prj_1","saveAs":"{}"}}"#, copy.display())),
        request(3, format!(r#"{{"projectId":"prj_1","saveAs":"{}"}}"#, copy.display())),
    ]
    .join("\n")
        + "\n";
    let output = String::from_utf8(replay_headless(input.as_bytes()).unwrap()).unwrap();
    let lines: Vec<_> = output.lines().collect();
    assert!(lines[2].contains(r#""projectId":"prj_2""#), "{}", lines[2]);
    assert!(lines[3].contains("EXISTS"), "{}", lines[3]);
    assert!(copy.join("project.json").is_file() && !copy.join("cache").exists());
    let reopened = splice_core::history::Store::open(&sample).unwrap();
    let copied = splice_core::history::Store::open(&copy);
    // The copy is held open by the closed server no longer; both packages load.
    assert!(copied.is_ok() && reopened.project().clips.order().len() == 3);
}

/// A bundled take uses clip.insert's descriptor provider and a folder independent of its ID.
#[test]
fn takes_events_follows_the_registered_primary_and_preserves_legacy_logs() {
    use splice_core::{commands::Target, history::Store};
    use splice_model::{Media, MediaId, MediaKind, Probe, TakeComponents, TimelineId};

    let root = TestDirectory::new("take-events-primary");
    let package = root.0.join("registered.splice");
    let mut store = Store::create(&package, &Default::default()).unwrap();
    let mut register = |path: &Path| {
        let path = path.to_str().unwrap();
        let media = Media {
            id: MediaId::from_serial(0),
            kind: MediaKind::Take,
            path: path.into(),
            bookmark: None,
            size: 5,
            mtime: 0,
            probe: Probe {
                fps: 30.0,
                frames: 90,
                width: 640,
                height: 400,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
            take: Some(TakeComponents {
                events: Some("events.json".into()),
                ..TakeComponents::default()
            }),
        };
        let op = object(vec![("cmd", text("clip.insert")), ("path", text(path))]);
        let edit = store
            .apply_with_media(&[op], &Default::default(), &mut |request| {
                assert_eq!(request, path);
                Ok(media.clone())
            })
            .unwrap();
        let Target::Timeline(TimelineId::Clip(clip)) = edit.report.changes[0].created[0] else {
            panic!("clip.insert did not create a clip");
        };
        let media = store.project().media.last().unwrap();
        assert_eq!(media.path, path);
        (clip.to_string(), media.id)
    };
    let bundled = Path::new("takes/v13-sample-screen-2/screen.mov");
    let linked = root.0.join("external/screen.mov");
    let directory = Path::new("takes/directory-reference");
    let (bundled_clip, bundled_id) = register(bundled);
    let (linked_clip, _) = register(&linked);
    let (directory_clip, _) = register(directory);
    store.close().unwrap();

    let mut projects = project::Projects::default();
    let call = |projects: &mut project::Projects, tool, args: Json| {
        tools::call(projects, tool, &args)
            .unwrap()
            .field("structuredContent")
            .unwrap()
            .clone()
    };
    let info = call(
        &mut projects,
        "project.info",
        object(vec![("path", text(package.to_str().unwrap()))]),
    );
    let project_id = info.field("projectId").unwrap();
    let events = |projects: &mut project::Projects, clip: &str| {
        call(
            projects,
            "takes.events",
            object(vec![
                ("projectId", project_id.clone()),
                ("clip", text(clip)),
            ]),
        )
    };
    let activity = |projects: &mut project::Projects, clip: &str| {
        call(
            projects,
            "media.analyze",
            object(vec![
                ("projectId", project_id.clone()),
                ("clip", text(clip)),
                ("kinds", Json::Array(vec![text("activity")])),
            ]),
        )
    };

    let write_log = |path: &Path, at: f64| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!(r#"{{"events":[{{"t":{at},"k":"down"}}]}}"#)).unwrap();
    };
    let assert_click = |reply: Json, source: i64| {
        let clicks = reply.field("clicks").unwrap_or_else(|| panic!("{reply:?}"));
        let clicks = clicks.as_array().unwrap();
        assert_eq!(clicks.len(), 1);
        assert_eq!(
            clicks[0].field("sourceFrame").and_then(Json::as_i64),
            Some(source)
        );
    };
    let assert_error = |reply: Json, code: &str| {
        assert_eq!(
            reply
                .field("error")
                .and_then(|e| e.field("code"))
                .and_then(Json::as_str),
            Some(code),
            "{reply:?}",
        );
    };
    let bundled = package.join(bundled);
    let bundled_log = bundled.with_file_name("events.json");
    let linked_log = linked.with_file_name("events.json");
    write_log(&bundled_log, 1.0);
    write_log(&linked_log, 1.5);
    write_log(&package.join(directory).join("events.json"), 2.0);
    fs::write(&bundled, b"media").unwrap();
    fs::write(&linked, b"media").unwrap();
    for (clip, source, start) in [
        (&bundled_clip, 30, 0),
        (&linked_clip, 45, 90),
        (&directory_clip, 60, 180),
    ] {
        assert_click(events(&mut projects, clip), source);
        let reply = activity(&mut projects, clip);
        let clicks = reply
            .field("activity")
            .and_then(|value| value.field("clicks"))
            .and_then(Json::as_array)
            .unwrap_or_else(|| panic!("{reply:?}"));
        assert_eq!(clicks.len(), 1);
        let from = clicks[0].field("from").and_then(Json::as_i64).unwrap();
        let to = clicks[0].field("to").and_then(Json::as_i64).unwrap();
        assert!((from..to).contains(&(start + source)), "{reply:?}");
    }

    let legacy = package
        .join("takes")
        .join(bundled_id.to_string())
        .join("events.json");
    write_log(&legacy, 0.5);
    assert_click(events(&mut projects, &bundled_clip), 30);
    fs::remove_file(&bundled_log).unwrap();
    assert_click(events(&mut projects, &bundled_clip), 15);

    // A declared sidecar must resolve inside its own take, even when a legacy copy exists.
    std::os::unix::fs::symlink(&linked_log, &bundled_log).unwrap();
    assert_error(events(&mut projects, &bundled_clip), "BAD_PACKAGE");
    assert_error(activity(&mut projects, &bundled_clip), "BAD_PACKAGE");
    fs::remove_file(&bundled_log).unwrap();
    fs::remove_file(&legacy).unwrap();
    assert_error(events(&mut projects, &bundled_clip), "MEDIA_OFFLINE");
    assert_error(activity(&mut projects, &bundled_clip), "MEDIA_OFFLINE");
    projects.close().unwrap();
}

#[test]
fn takes_events_stream_logs_past_the_old_16_mib_cap() {
    let root = TestDirectory::new("long-events");
    let sample = root.0.join("sample.splice");
    copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &sample).unwrap();
    let take = sample.join("takes/med_screen");
    fs::create_dir_all(&take).unwrap();
    // ~19 MB: 80 minutes of pointer moves at 100 Hz, one click at 2.0 s.
    let mut log = String::with_capacity(19_000_000);
    log.push_str(r#"{"t0":0,"source":{"kind":"display","rect":{"x":0,"y":0,"width":640,"height":360},"scale":1},"events":["#);
    for i in 0..480_000u32 {
        if i > 0 {
            log.push(',');
        }
        let t = f64::from(i) / 100.0;
        if i == 200 {
            log.push_str(r#"{"t":2.0,"k":"down","b":0,"x":0.5,"y":0.5},"#);
        }
        log.push_str(&format!(r#"{{"t":{t},"k":"move","x":0.25,"y":0.75}}"#));
    }
    log.push_str(r#"],"dropped":0}"#);
    assert!(log.len() > 16 * 1024 * 1024);
    fs::write(take.join("events.json"), &log).unwrap();
    drop(log);
    let store = splice_core::history::Store::open(&sample).unwrap();
    let read = |args: &str| {
        super::media::events(
            store.path(),
            store.project(),
            &splice_json::parse(args).unwrap(),
        )
        .unwrap()
    };
    let len = |events: &Json, kind: &str| events.field(kind).unwrap().as_array().unwrap().len();
    let events = read(r#"{"clip":"clp_take","from":120,"to":240,"stride":1}"#);
    assert_eq!(len(&events, "clicks"), 1);
    assert!(len(&events, "cursor") > 300);
    // A8: by default the path is thinned to one sample per half second (15 frames at 30 fps),
    // and counts say how many there were.
    let events = read(r#"{"clip":"clp_take","from":120,"to":240}"#);
    assert_eq!(len(&events, "cursor"), 8);
    assert_eq!(len(&events, "clicks"), 1);
    let counts = events.field("counts").unwrap();
    assert!(counts.field("cursor").and_then(Json::as_u64).unwrap() > 300);
    assert_eq!(counts.field("clicks").and_then(Json::as_u64), Some(1));
    let events = read(r#"{"clip":"clp_take","from":120,"to":240,"kinds":["clicks"]}"#);
    assert_eq!((len(&events, "cursor"), len(&events, "clicks")), (0, 1));
    for args in [
        r#"{"clip":"clp_take","kinds":["moves"]}"#,
        r#"{"clip":"clp_take","stride":0}"#,
    ] {
        let error = super::media::events(
            store.path(),
            store.project(),
            &splice_json::parse(args).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.code, "BAD_ARGS", "{args}");
    }
}

#[test]
fn code_run_reports_revisions_for_every_project_it_touched() {
    let args = splice_json::parse(
        r#"{"code":"splice.edit.apply({projectId:\"prj_1\", ifRevision:\"r3\", ops:[]}); splice.edit.apply({projectId:\"prj_2\", ifRevision:\"r7\", ops:[]}); splice.edit.apply({projectId:\"prj_1\", ifRevision:\"r4\", ops:[]}); return 1;"}"#,
    )
    .unwrap();
    let mut revision = 3;
    let reply = code::run(&args, &mut |name, args| {
        let project = args.field("projectId").and_then(Json::as_str).unwrap_or("");
        // The session lookup code mode makes reads; only edits move the revision.
        if name == "project.info" {
            return Ok(object(vec![(
                "structuredContent",
                object(vec![
                    ("projectId", text(project)),
                    ("revision", text(&format!("r{revision}"))),
                ]),
            )]));
        }
        revision += 1;
        let next = if project == "prj_2" { 8 } else { revision };
        Ok(object(vec![(
            "structuredContent",
            object(vec![("revision", text(&format!("r{next}")))]),
        )]))
    })
    .unwrap();
    let revisions = reply.field("revisions").unwrap();
    let get = |value: &Json, key: &str| value.field(key).and_then(Json::as_str).unwrap().to_owned();
    assert_eq!(
        (get(revisions, "from"), get(revisions, "to")),
        ("r3".into(), "r6".into())
    );
    let other = revisions.field("projects").unwrap().field("prj_2").unwrap();
    assert_eq!(
        (get(other, "from"), get(other, "to")),
        ("r7".into(), "r8".into())
    );
}

/// T24.1: every registry row, then a layered batch, then a session undo, in one code.run.
#[test]
fn capability_sweep_applies_every_row_and_restores_the_start() {
    let root = TestDirectory::new("sweep");
    let sample = root.0.join("sample.splice");
    copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &sample).unwrap();
    // media.relink probes the file on macOS: give the placeholder b-roll a real file, and relink
    // to a byte-identical copy (setup below), so the documented relink is the same media.
    let real = mcp_fixtures().join("../media/probe.mov");
    fs::create_dir_all(sample.join("assets")).unwrap();
    for name in ["broll.mov", "broll-v2.mov"] {
        fs::copy(&real, sample.join("assets").join(name)).unwrap();
    }
    // music.add's example imports assets/music.wav (probed on macOS).
    fs::copy(
        mcp_fixtures().join("../media/stereo.wav"),
        sample.join("assets/music.wav"),
    )
    .unwrap();
    let setup = |id: &str| -> &'static str {
        match id {
            "media.relink" => {
                r#"[{"cmd":"media.relink","media":"med_broll","path":"assets/broll.mov","force":true}]"#
            }
            "clip.join" => r#"[{"cmd":"item.split","item":"clp_take","at":300,"ref":"cut"}]"#,
            "clip.ungroup" => {
                r#"[{"cmd":"item.split","item":"clp_take","at":300,"ref":"cut"},{"cmd":"clip.join","items":["clp_take","$cut.right"]}]"#
            }
            "clip.restore" => r#"[{"cmd":"cut.range","from":200,"to":230}]"#,
            "mask.set" | "mask.track" => r#"[{"cmd":"mask.add","item":"clp_take","ref":"mask"}]"#,
            "audio.link" | "audio.sync" => {
                r#"[{"cmd":"audio.unlink","item":"clp_take","ref":"voice"}]"#
            }
            "clip.uncompound" => {
                r#"[{"cmd":"clip.compound","items":["clp_take","clp_end"],"ref":"compound"}]"#
            }
            "transition.set" | "transition.remove" => {
                r#"[{"cmd":"transition.add","item":"clp_end"}]"#
            }
            "template.set" => {
                r#"[{"cmd":"template.add","name":"lower-third","at":150,"ref":"lower"}]"#
            }
            "item.slide" => r#"[{"cmd":"item.trim","item":"clp_end","edge":"in","delta":12}]"#,
            "motion.set" => {
                r#"[{"cmd":"motion.add","at":180,"length":150,"from":1,"to":1.16,"ref":"motion"}]"#
            }
            "caption.set" | "caption.split" | "caption.merge" => "[]",
            _ => "[]",
        }
    };
    let mut rows = String::from("[");
    for (index, row) in splice_core::commands::COMMANDS.iter().enumerate() {
        if index > 0 {
            rows.push(',');
        }
        // The sweep renders no HTML: a movie the package already has stands in for the page's.
        let example = row
            .example
            .replace("media/html-5f1c.mov", "assets/broll.mov");
        rows.push_str(&format!("[\"{}\",{},{}]", row.id, example, setup(row.id)));
    }
    rows.push(']');
    let program = fs::read_to_string(mcp_fixtures().join("../../evals/sweep.js"))
        .unwrap()
        .replace("ROWS", &rows)
        .replace("SAMPLE", &format!("{:?}", sample.to_str().unwrap()));
    let mut code = String::new();
    splice_json::write(&Json::String(program), &mut code).unwrap();
    let call = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"code.run","arguments":{{"code":{code}}}}}}}"#
    );
    let init = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"sweep","version":"0"}}}"#;
    let mut output = Vec::new();
    rpc::serve_with_log(
        &mut Cursor::new(format!("{init}\n{call}\n").into_bytes()),
        &mut output,
        None,
        true,
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    let reply = splice_json::parse(output.lines().nth(1).unwrap()).unwrap();
    let content = reply
        .field("result")
        .and_then(|r| r.field("structuredContent"))
        .unwrap_or_else(|| panic!("{output}"));
    let result = content
        .field("result")
        .unwrap_or_else(|| panic!("{output}"));
    let count = |key: &str| result.field(key).and_then(Json::as_u64).unwrap();
    let failures = result
        .field("failures")
        .and_then(Json::as_array)
        .unwrap_or_else(|| panic!("{content:?}"));
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(count("rows"), splice_core::commands::COMMANDS.len() as u64);
    assert_eq!(count("applied"), count("rows"));
    assert!(count("batchChanges") >= 12);
    assert_eq!(
        result.field("restored"),
        Some(&Json::Bool(true)),
        "{output}"
    );
}

/// Eval friction: agents passed `items` to items.inspect, inspected effect IDs, and saw shapes
/// labelled "Scale animation".
#[test]
fn inspect_takes_items_reads_effects_and_labels_shapes() {
    let mut project = sample_project();
    let ops = splice_json::parse(
        r#"[{"cmd":"mask.add","item":"clp_take","ref":"mask"},{"cmd":"generator.add","shape":"arrow","at":200,"length":45}]"#,
    )
    .unwrap();
    splice_core::edit::apply(
        &mut project,
        ops.as_array().unwrap(),
        splice_core::history::Source::Agent,
        false,
    )
    .unwrap();
    let take = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let effects = &project.clips.data(take).unwrap().appearance.effects;
    let mask = effects
        .iter()
        .find(|e| e.kind == "mask")
        .unwrap()
        .id
        .to_string();
    let shape = project
        .items
        .order()
        .iter()
        .find(|&&slot| {
            matches!(
                project.items.data(slot).unwrap().kind,
                splice_model::ItemKind::Shape { .. }
            )
        })
        .map(|&slot| project.items.id(slot).unwrap().to_string())
        .unwrap();
    let args = splice_json::parse(&format!(r#"{{"items":["{mask}","{shape}"]}}"#)).unwrap();
    let reply = super::timeline::inspect(&project, &args).unwrap();
    let items = reply.field("items").and_then(Json::as_array).unwrap();
    assert_eq!(
        items[0].field("kind").and_then(Json::as_str),
        Some("effect")
    );
    assert_eq!(
        items[0].field("owner").and_then(Json::as_str),
        Some("clp_take")
    );
    assert_eq!(items[0].field("type").and_then(Json::as_str), Some("mask"));
    assert_eq!(
        items[1].field("label").and_then(Json::as_str),
        Some("arrow"),
        "{reply:?}"
    );
}

/// T23.5: code.run passes view.frames previews back as images (capped), so code-mode agents
/// do not read full-size frame PNGs from disk.
#[test]
fn code_run_returns_frame_previews_as_images() {
    let args = splice_json::parse(
        r#"{"code":"for (let i = 0; i < 4; i++) splice.view.frames({projectId:\"prj_1\", count:2}); return 1;"}"#,
    )
    .unwrap();
    let mut images = Vec::new();
    let payload = code::run_with_images(
        &args,
        &mut |_, _| {
            let image = object(vec![
                ("type", text("image")),
                ("mimeType", text("image/png")),
                ("data", text("AA==")),
            ]);
            Ok(object(vec![
                (
                    "structuredContent",
                    object(vec![("frames", Json::Array(vec![]))]),
                ),
                ("content", Json::Array(vec![image.clone(), image])),
            ]))
        },
        &mut images,
    );
    assert_eq!(images.len(), 6);
    let reply = code::reply(payload, images).unwrap();
    let content = reply.field("content").and_then(Json::as_array).unwrap();
    assert_eq!(
        content
            .iter()
            .filter(|c| c.field("type").and_then(Json::as_str) == Some("image"))
            .count(),
        6
    );
}

/// Sound items (voice-overs, podcast tracks) have transcripts too: transcript.get {item} reads
/// the item's media words on the timeline through the item's start.
#[test]
fn transcript_get_reads_a_sound_items_words() {
    let mut project = sample_project();
    let ops = splice_json::parse(r#"[{"cmd":"audio.unlink","item":"clp_take"}]"#).unwrap();
    splice_core::edit::apply(
        &mut project,
        ops.as_array().unwrap(),
        splice_core::history::Source::Agent,
        false,
    )
    .unwrap();
    let item = project
        .items
        .order()
        .iter()
        .find(|&&slot| {
            matches!(
                project.items.data(slot).unwrap().kind,
                splice_model::ItemKind::Sound(_)
            )
        })
        .map(|&slot| project.items.id(slot).unwrap().to_string())
        .unwrap();
    let args = splice_json::parse(&format!(r#"{{"item":"{item}","limit":5}}"#)).unwrap();
    let reply = super::media::transcript(&project, &args).unwrap();
    assert_eq!(
        reply.field("item").and_then(Json::as_str),
        Some(item.as_str())
    );
    let words = reply.field("words").and_then(Json::as_array).unwrap();
    assert_eq!(words.len(), 5);
    // The take starts at frame 120 with source 0, so a word at source frame s plays at 120 + s.
    let first = &words[0];
    let source = first.field("sourceFrom").and_then(Json::as_i64).unwrap();
    assert_eq!(
        first.field("frame").and_then(Json::as_i64),
        Some(120 + source)
    );
    let bad = splice_json::parse(r#"{"item":"ovl_hook"}"#).unwrap();
    assert_eq!(
        super::media::transcript(&project, &bad).unwrap_err().code,
        "BAD_ARGS"
    );
}

/// Eval record-pause-marker: record.start in an open session needs no ifRevision (it writes
/// nothing until record.stop, which publishes against the revision current then).
#[test]
fn record_start_joins_a_session_without_if_revision() {
    let temp = TestDirectory::new("record-session");
    let mut store = splice_core::history::Store::create(
        &temp.0.join("p.splice"),
        &splice_model::Project::default(),
    )
    .unwrap();
    let session = store.start_session("agent", 1).unwrap().id;
    let parse = |text: &str| splice_json::parse(text).unwrap();
    assert!(
        super::platform::record_write_options(&store, &parse("{}"))
            .unwrap()
            .is_none()
    );
    let joined = super::platform::record_write_options(
        &store,
        &parse(&format!(r#"{{"sessionId":"{session}"}}"#)),
    )
    .unwrap()
    .unwrap();
    assert_eq!(joined.session, Some(session));
    assert_eq!(joined.if_revision, Some(store.project().rev));
    store.check_write(&joined).unwrap();
    // ifRevision alone is a precondition (record_start_takes_if_revision_without_a_session).
    assert!(
        super::platform::record_write_options(&store, &parse(r#"{"ifRevision":"r0"}"#))
            .unwrap()
            .is_none()
    );
}

/// A5: on a cropped take, intent.autoZoom (clicks in captured-source coordinates, as
/// takes.events reports them) centers the camera on the clicked point of the source, with fit
/// and with fill; a click in the cropped-away part moves no zoom and the summary says so.
#[test]
fn auto_zoom_on_a_cropped_clip_lands_on_the_clicked_source_point() {
    let apply = |project: &mut splice_model::Project, ops: &str| {
        let ops = splice_json::parse(ops).unwrap();
        splice_core::edit::apply(
            project,
            ops.as_array().unwrap(),
            splice_core::package::journal::Source::Agent,
            false,
        )
        .unwrap()
    };
    // The visible source point at the middle of the camera's viewport on clp_take.
    let center = |project: &splice_model::Project, at: splice_model::Frame| {
        let evaluator = splice_eval::Evaluator::prepare(project).unwrap();
        let mut plan = evaluator.buffer();
        evaluator.frame(at, &mut plan);
        let picture = plan
            .layers
            .iter()
            .find_map(|layer| match layer {
                splice_eval::Layer::Picture(p) if p.item.to_string() == "clp_take" => Some(*p),
                _ => None,
            })
            .expect("the take's picture");
        let [cx, cy] = picture.framed_point(plan.size.map(f64::from));
        let s = picture.source;
        [s.x + cx * s.w, s.y + cy * s.h]
    };
    let click = [0.55, 0.42];
    for fit in ["fit", "fill"] {
        let mut project = sample_project();
        // The sample's take is cropped to {x 0.36, y 0.1, w 0.46, h 0.8}; drop its own zoom.
        apply(
            &mut project,
            &format!(
                r#"[{{"cmd":"zoom.remove","zoom":"ovl_detail"}},
                    {{"cmd":"clip.crop","items":["clp_take"],"fit":"{fit}"}}]"#
            ),
        );
        let report = apply(
            &mut project,
            &format!(
                r#"[{{"cmd":"intent.autoZoom","item":"clp_take","amount":2,
                     "clicks":[{{"at":300,"x":{},"y":{}}},{{"at":200,"x":0.1,"y":0.5}}]}}]"#,
                click[0], click[1]
            ),
        );
        let summary = format!("{:?}", report.changes[0].summary);
        assert!(summary.contains("1 click(s) outside the crop"), "{summary}");
        // clp_take starts at 120; the zoom opens 9 frames before the click at source 300.
        let at = 120 + 300 + 45;
        let got = center(&project, at);
        assert!(
            (got[0] - click[0]).abs() < 0.01 && (got[1] - click[1]).abs() < 0.01,
            "{fit}: camera centered on {got:?}, clicked {click:?}"
        );
    }
}

/// AC-CROP-2: a later re-crop (or fit change) keeps a blur mask over the same source rectangle
/// and a zoom's focus on the same source point, in fit and fill; the batch reports both as
/// `recropped`, flags content the new crop cuts away, and refuses a crop no mask could follow.
#[test]
fn re_cropping_keeps_masks_and_zoom_focus_on_the_same_source_content() {
    use splice_core::edit::{Recropped, SideEffect};
    let apply = |project: &mut splice_model::Project, ops: &str| {
        let ops = splice_json::parse(ops).unwrap();
        splice_core::edit::apply(
            project,
            ops.as_array().unwrap(),
            splice_core::package::journal::Source::Agent,
            false,
        )
    };
    // The picture and the canvas size.
    let picture = |project: &splice_model::Project, item: &str, at: splice_model::Frame| {
        let evaluator = splice_eval::Evaluator::prepare(project).unwrap();
        let mut plan = evaluator.buffer();
        evaluator.frame(at, &mut plan);
        let p = plan
            .layers
            .iter()
            .find_map(|layer| match layer {
                splice_eval::Layer::Picture(p) if p.item.to_string() == item => Some(*p),
                _ => None,
            })
            .expect("the picture");
        (p, plan.size.map(f64::from))
    };
    // The source rectangle under the mask, and the source point at the camera's center, with
    // the zoom settled (ovl_detail runs 180..330).
    let seen = |project: &splice_model::Project| {
        let (p, size) = picture(project, "clp_take", 320);
        let s = p.source;
        let [x, y, w, h] = p.look.masks[0].expect("the mask").rect;
        let [cx, cy] = p.framed_point(size);
        (
            [s.x + x * s.w, s.y + y * s.h, w * s.w, h * s.h],
            [s.x + cx * s.w, s.y + cy * s.h],
        )
    };
    let close = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 2e-3);
    for (fit, other) in [("fit", "fill"), ("fill", "fit")] {
        let mut project = sample_project();
        // The sample's take is cropped to {x 0.36, y 0.1, w 0.46, h 0.8}.
        apply(
            &mut project,
            &format!(
                r#"[{{"cmd":"clip.crop","items":["clp_take"],"fit":"{fit}"}},
                    {{"cmd":"zoom.set","zoom":"ovl_detail","amount":2,"focus":{{"x":0.5,"y":0.45}}}},
                    {{"cmd":"mask.add","item":"clp_take","mode":"blur","x":0.4,"y":0.3,"w":0.2,"h":0.1}}]"#
            ),
        )
        .unwrap();
        let (mask, center) = seen(&project);
        let report = apply(
            &mut project,
            r#"[{"cmd":"clip.crop","items":["clp_take"],"rect":{"x":0.3,"y":0.05,"w":0.6,"h":0.85}}]"#,
        )
        .unwrap();
        let (moved_mask, moved_center) = seen(&project);
        assert!(
            close(&mask, &moved_mask),
            "{fit}: mask {mask:?} -> {moved_mask:?}"
        );
        assert!(
            close(&center, &moved_center),
            "{fit}: zoom center {center:?} -> {moved_center:?}"
        );
        let recropped: Vec<_> = report
            .side_effects
            .iter()
            .filter_map(|e| match e {
                SideEffect::Recropped { id, outside, .. } => Some((*id, *outside)),
                _ => None,
            })
            .collect();
        assert!(
            recropped.len() == 2
                && recropped.iter().all(|(_, outside)| !outside)
                && recropped
                    .iter()
                    .any(|(id, _)| matches!(id, Recropped::Zoom(_)))
                && recropped
                    .iter()
                    .any(|(id, _)| matches!(id, Recropped::Mask(_))),
            "{fit}: {recropped:?}"
        );
        // Changing only the fit moves what the picture shows: the mask follows its content.
        apply(
            &mut project,
            &format!(r#"[{{"cmd":"clip.crop","items":["clp_take"],"fit":"{other}"}}]"#),
        )
        .unwrap();
        let (refit, _) = seen(&project);
        assert!(
            close(&mask, &refit),
            "{fit} -> {other}: mask {mask:?} -> {refit:?}"
        );
        // A crop that cuts the masked content away says so.
        let report = apply(
            &mut project,
            r#"[{"cmd":"clip.crop","items":["clp_take"],"rect":{"x":0.7,"y":0.05,"w":0.25,"h":0.85}}]"#,
        )
        .unwrap();
        assert!(
            report.side_effects.iter().any(|e| matches!(
                e,
                SideEffect::Recropped {
                    id: Recropped::Mask(_),
                    outside: true,
                    ..
                }
            )),
            "{fit}: {:?}",
            report.side_effects
        );
        // A crop so far off the mask that its numbers leave what a mask can hold is refused.
        let error = apply(
            &mut project,
            r#"[{"cmd":"clip.crop","items":["clp_take"],"rect":{"x":0.9,"y":0.9,"w":0.05,"h":0.05}}]"#,
        )
        .unwrap_err();
        assert_eq!(error.error.code, "OUT_OF_RANGE", "{fit}: {error}");
    }
    // A media overlay's masks follow its crop the same way (ovl_broll: a fill card, 420..495).
    let mut project = sample_project();
    let masked = |project: &splice_model::Project| {
        let (p, _) = picture(project, "ovl_broll", 450);
        let (s, [x, y, w, h]) = (p.source, p.look.masks[0].expect("the mask").rect);
        [s.x + x * s.w, s.y + y * s.h, w * s.w, h * s.h]
    };
    apply(
        &mut project,
        r#"[{"cmd":"mask.add","item":"ovl_broll","mode":"cover","x":0.3,"y":0.4,"w":0.3,"h":0.2}]"#,
    )
    .unwrap();
    let before = masked(&project);
    apply(
        &mut project,
        r#"[{"cmd":"clip.crop","items":["ovl_broll"],"rect":{"x":0.1,"y":0.2,"w":0.7,"h":0.7}}]"#,
    )
    .unwrap();
    let after = masked(&project);
    assert!(
        close(&before, &after),
        "overlay mask {before:?} -> {after:?}"
    );
}

/// A10: items.inspect on a fresh take lists what draws without any effect: the recorded cursor
/// as captured, the camera bubble and chips with their defaults, and each one's component.
#[test]
fn inspect_shows_the_implicit_look_of_a_fresh_take() {
    let mut project = sample_project();
    let ops = splice_json::parse(
        r#"[{"cmd":"effect.remove","effects":["fx_projcur"]},
            {"cmd":"clip.insert","media":"med_screen"}]"#,
    )
    .unwrap();
    splice_core::edit::apply(
        &mut project,
        ops.as_array().unwrap(),
        splice_core::history::Source::Agent,
        false,
    )
    .unwrap();
    let slot = *project.clips.order().last().unwrap();
    let id = project.clips.id(slot).unwrap().to_string();
    let args = splice_json::parse(&format!(r#"{{"ids":["{id}"]}}"#)).unwrap();
    let reply = super::timeline::inspect(&project, &args).unwrap();
    let look = reply.field("items").unwrap().as_array().unwrap()[0]
        .field("effectsResolved")
        .unwrap()
        .clone();
    let field = |kind: &str, key: &str| look.field(kind).and_then(|e| e.field(key)).cloned();
    // The project still has its window: resolved, not implicit.
    assert_eq!(field("window", "implicit"), None);
    // No cursor effect anywhere: the recorded cursor at size 1 without rings.
    assert_eq!(field("cursor", "implicit"), Some(Json::Bool(true)));
    let cursor = field("cursor", "params").unwrap();
    assert_eq!(cursor.field("size").and_then(Json::as_f64), Some(1.0));
    assert_eq!(cursor.field("clickRings"), Some(&Json::Bool(false)));
    assert_eq!(field("cursor", "component"), Some(Json::Bool(true)));
    // Chips: the effect's defaults, drawn only once keys.set turns the component on (A1).
    assert_eq!(field("keystrokes", "implicit"), Some(Json::Bool(true)));
    assert_eq!(field("keystrokes", "component"), Some(Json::Bool(false)));
    let chips = field("keystrokes", "params").unwrap();
    assert_eq!(chips.field("show").and_then(Json::as_str), Some("all"));
    assert_eq!(field("camera", "component"), Some(Json::Bool(true)));
}

/// A9: what docs/MCP.md and the splice skill name exists in the code, so drift fails the
/// build: every `group.name` in backticks is a tool, a registry command or an effect
/// parameter; the export preset list is exactly the presets; preset names in examples exist;
/// no legacy `w12` word IDs; every lint code is documented.
#[test]
fn docs_name_only_tools_commands_presets_and_lints_that_exist() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let docs = ["docs/MCP.md", ".agents/skills/splice/SKILL.md"]
        .map(|path| (path, fs::read_to_string(root.join(path)).unwrap()));
    let schemas = splice_core::commands::effect_schemas();
    // Backticked spans, and the leading identifier of each (`edit.apply {…}` names edit.apply).
    let spans = |text: &str| -> Vec<String> {
        text.split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    };
    let names = |span: &str| -> Option<String> {
        let head: String = span
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '.')
            .collect();
        let parts: Vec<&str> = head.split('.').collect();
        (parts.len() == 2
            && parts.iter().all(|p| !p.is_empty())
            && parts[0].chars().all(|c| c.is_ascii_lowercase())
            && parts[1]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase()))
        .then_some(head)
    };
    // Files, an alias and result fields the docs name in the same shape.
    const FILES: [&str; 8] = ["json", "jsonl", "md", "mov", "sock", "ts", "srt", "txt"];
    const OTHER: [&str; 4] = [
        "audio.summary",
        "range.from",
        "settings.credits",
        "performance.now",
    ];
    let mut unknown = Vec::new();
    for (path, text) in &docs {
        for span in spans(text) {
            let Some(name) = names(&span) else { continue };
            // JavaScript in code mode: `splice.…` and calls such as `window.seek(seconds)`.
            if name.starts_with("splice.") || span[name.len()..].starts_with('(') {
                continue;
            }
            let (group, last) = name.split_once('.').unwrap();
            let effect_param = schemas
                .field(group)
                .and_then(|schema| schema.field("params"))
                .and_then(|params| params.field(last))
                .is_some();
            if !(tools::known(&name, false)
                || tools::known(&name, true)
                || splice_core::commands::lookup(&name).is_some()
                || effect_param
                || code::SAME.iter().any(|(alias, _)| *alias == name)
                || FILES.contains(&last)
                || OTHER.contains(&name.as_str()))
            {
                unknown.push(format!("{path}: `{span}`"));
            }
        }
        // Words are `w:<media>:<n>` since 0.3.
        for span in spans(text) {
            let legacy = span.starts_with('w')
                && span.len() > 1
                && span[1..].chars().all(|c| c.is_ascii_digit());
            assert!(!legacy, "{path}: legacy word ID `{span}`");
        }
        // Preset names in examples (`preset:"…"`, `"preset":"…"`).
        for piece in text.split("preset").skip(1) {
            if let Some(name) = piece
                .strip_prefix("\":\"")
                .or_else(|| piece.strip_prefix(":\""))
            {
                let name: String = name.chars().take_while(|c| *c != '"').collect();
                assert!(
                    super::presets::PRESETS.iter().any(|p| p.name == name),
                    "{path}: unknown preset {name:?}"
                );
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "names the code does not have:\n{}",
        unknown.join("\n")
    );
    // The export preset list says exactly what the code has.
    let mcp = &docs[0].1;
    let list = mcp
        .split_once("`preset` is one of ")
        .and_then(|(_, rest)| rest.split_once("(a PNG still)"))
        .expect("the preset sentence")
        .0;
    let mut listed: Vec<String> = spans(list);
    let mut presets: Vec<String> = super::presets::PRESETS
        .iter()
        .map(|p| p.name.to_owned())
        .collect();
    listed.sort();
    presets.sort();
    assert_eq!(listed, presets, "docs/MCP.md preset list");
    // Every lint code the server emits is documented.
    let lints = include_str!("lints.rs");
    for code in lints.split("lint(\n").skip(1).filter_map(|call| {
        let start = call.find('"')? + 1;
        let end = start + call[start..].find('"')?;
        Some(&call[start..end])
    }) {
        assert!(
            mcp.contains(&format!("`{code}`")),
            "lint {code} is not in docs/MCP.md"
        );
    }
}

/// Found by record-app-landscape (code mode): code.run filled ifRevision from what it read,
/// and record.start without a session answered NO_SESSION. Alone, ifRevision is a
/// precondition and the take starts its own session.
#[test]
fn record_start_takes_if_revision_without_a_session() {
    let root = TestDirectory::new("record-revision");
    let store = splice_core::history::Store::create(
        &root.0.join("p.splice"),
        &splice_model::Project::default(),
    )
    .unwrap();
    let args = |revision: &str| object(vec![("ifRevision", text(revision))]);
    assert!(
        super::platform::record_write_options(&store, &args("r0"))
            .unwrap()
            .is_none()
    );
    let stale = super::platform::record_write_options(&store, &args("r5")).unwrap_err();
    assert!(format!("{stale:?}").contains("STALE_REVISION"), "{stale:?}");
}

/// Found by cursor-range-hide (code mode): the agent passed the package path as projectId.
#[test]
fn a_path_passed_as_project_id_is_pointed_at_path() {
    let mut projects = super::project::Projects::default();
    let error = projects
        .lookup(&object(vec![("projectId", text("/tmp/x/project.splice"))]))
        .map(|_| ())
        .unwrap_err();
    assert!(error.fix.contains("project.info {path:"), "{}", error.fix);
}

/// commands.describe {id} with a tool's name (record.start, edit.apply) describes the tool.
#[test]
fn describe_by_a_tool_id_describes_the_tool() {
    let mut projects = super::project::Projects::default();
    let reply = super::tools::call(
        &mut projects,
        "commands.describe",
        &object(vec![("id", text("record.start"))]),
    )
    .unwrap();
    let content = reply.field("structuredContent").unwrap();
    assert_eq!(
        content.field("tool").and_then(|t| t.field("name")),
        Some(&text("record.start")),
        "{reply:?}"
    );
}

/// From the final eval run: 26 calls failed on a missing, "undefined" or path-valued projectId
/// and 6 on a missing or invented sessionId. With one open project those mean it; edit.apply
/// without sessionId joins the open session or starts one, and says which.
#[test]
fn tools_default_the_only_project_and_its_session() {
    let root = TestDirectory::new("defaults");
    let sample = root.0.join("sample.splice");
    copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &sample).unwrap();
    let mut projects = super::project::Projects::default();
    let call = |projects: &mut super::project::Projects, name: &str, args: Json| {
        let reply = super::tools::call(projects, name, &args).unwrap();
        let ok = reply.field("isError") == Some(&Json::Bool(false));
        (ok, reply.field("structuredContent").cloned().unwrap())
    };
    let path = sample.to_str().unwrap();
    let (ok, info) = call(
        &mut projects,
        "project.info",
        object(vec![("path", text(path))]),
    );
    assert!(ok, "{info:?}");
    for args in [
        object(vec![]),
        object(vec![("projectId", text("undefined"))]),
        object(vec![("projectId", text(path))]),
    ] {
        let (ok, reply) = call(&mut projects, "timeline.snapshot", args);
        assert!(ok, "{reply:?}");
    }
    let edit = || {
        object(vec![
            ("ifRevision", info.field("revision").unwrap().clone()),
            (
                "ops",
                splice_json::parse(r#"[{"cmd":"marker.add","at":30,"name":"Intro"}]"#).unwrap(),
            ),
        ])
    };
    let (ok, reply) = call(&mut projects, "edit.apply", edit());
    assert!(ok, "{reply:?}");
    let note = reply.field("session").and_then(Json::as_str).unwrap();
    assert!(note.starts_with("Started session ses_"), "{note}");
    // The next edit joins that session instead of starting another.
    let mut next = edit();
    if let Json::Object(fields) = &mut next {
        fields[0].1 = reply.field("revision").unwrap().clone();
        fields[1].1 = splice_json::parse(r#"[{"cmd":"marker.add","at":60,"name":"Two"}]"#).unwrap();
    }
    let (ok, reply) = call(&mut projects, "edit.apply", next);
    assert!(ok, "{reply:?}");
    assert!(
        reply
            .field("session")
            .and_then(Json::as_str)
            .unwrap()
            .starts_with("Joined"),
        "{reply:?}"
    );
}

/// docs/RECORDER-PARITY.md stays true: every row has four cells and a known status, a
/// verified row names its evidence, and every test (`crate::module::test` as `cargo t` prints
/// it), eval (`eval:<id>`) and measuring script (`tool: tools/…`) it names exists.
#[test]
fn recorder_parity_evidence_exists() {
    use std::{fs, path::Path};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // `fn name(` with `#[test]` on one of the three lines above it.
    let has_test = |text: &str, name: &str| {
        let lines: Vec<&str> = text.lines().collect();
        let head = format!("fn {name}(");
        lines.iter().enumerate().any(|(i, line)| {
            line.trim_start().starts_with(&head)
                && lines[i.saturating_sub(3)..i]
                    .iter()
                    .any(|l| l.trim() == "#[test]")
        })
    };
    let test_exists = |name: &str| -> bool {
        let parts: Vec<&str> = name.split("::").collect();
        let (krate, modules, test) = (parts[0], &parts[1..parts.len() - 1], parts[parts.len() - 1]);
        let dir = root.join("crates").join(krate);
        // Integration targets print as `crate::test`: a #[test] fn, or a case a harness lists
        // by name.
        if modules.is_empty()
            && fs::read_dir(dir.join("tests")).is_ok_and(|entries| {
                entries.flatten().any(|entry| {
                    fs::read_to_string(entry.path()).is_ok_and(|text| {
                        has_test(&text, test) || text.contains(&format!("\"{test}\""))
                    })
                })
            })
        {
            return true;
        }
        // The longest module prefix that is a file; the rest are inline `mod name {` blocks.
        (0..=modules.len()).rev().any(|split| {
            let base = dir.join("src").join(modules[..split].join("/"));
            let files = if split == 0 {
                vec![dir.join("src/lib.rs")]
            } else {
                vec![base.with_extension("rs"), base.join("mod.rs")]
            };
            files.iter().any(|file| {
                fs::read_to_string(file).is_ok_and(|text| {
                    modules[split..]
                        .iter()
                        .all(|m| text.contains(&format!("mod {m} {{")))
                        && has_test(&text, test)
                })
            })
        })
    };
    let doc = fs::read_to_string(root.join("docs/RECORDER-PARITY.md")).unwrap();
    let mut problems = Vec::new();
    let mut rows = 0;
    for line in doc.lines().filter(|l| {
        l.starts_with("| ") && !l.starts_with("| ---") && !l.starts_with("| Recorder capability")
    }) {
        rows += 1;
        let cells: Vec<&str> = line.trim_matches('|').split(" | ").map(str::trim).collect();
        let [capability, call, evidence, status] = cells[..] else {
            problems.push(format!("not four cells: {line}"));
            continue;
        };
        if capability.is_empty() || call.is_empty() {
            problems.push(format!("empty capability or call: {line}"));
        }
        let spans: Vec<&str> = evidence.split('`').skip(1).step_by(2).collect();
        let verified = status.starts_with("verified");
        if !verified && !status.starts_with("UI-only") && !status.starts_with("GAP") {
            problems.push(format!(
                "status is not verified, UI-only or GAP: {capability}"
            ));
        }
        if verified && spans.is_empty() && !evidence.contains("live:") {
            problems.push(format!("verified without evidence: {capability}"));
        }
        for span in spans {
            let found = if let Some(id) = span.strip_prefix("eval:") {
                root.join("evals/tasks")
                    .join(format!("{id}.json"))
                    .is_file()
            } else if let Some(path) = span.strip_prefix("tool: ") {
                // A measuring script run by hand on the Mac (`tool: tools/…`).
                path.starts_with("tools/") && root.join(path).is_file()
            } else {
                span.contains("::") && test_exists(span)
            };
            if !found {
                problems.push(format!("{capability}: no {span}"));
            }
        }
    }
    assert!(rows > 80, "{rows} rows");
    assert!(problems.is_empty(), "{problems:#?}");
}

/// From the final eval run: export.start {path:"…teaser.gif"} in code mode lost the default
/// project ("Missing projectId"): path names the output there, not a project.
#[test]
fn an_output_path_keeps_the_default_project() {
    let args = object(vec![
        ("projectId", text("prj_1")),
        (
            "code",
            text(r#"splice.export.start({preset:"gif", path:"/tmp/teaser.gif"});"#),
        ),
    ]);
    let mut seen = Vec::new();
    code::run(&args, &mut |name, args| {
        seen.push((name.to_owned(), args.field("projectId").cloned()));
        Ok(object(vec![("structuredContent", object(Vec::new()))]))
    })
    .unwrap();
    let export = seen
        .iter()
        .find(|(name, _)| name == "export.start")
        .unwrap();
    assert_eq!(export.1, Some(text("prj_1")), "{seen:?}");
}

#[test]
fn v13_scale_motion_is_editable_inspectable_and_replayable_over_mcp() {
    use splice_core::history::Store;
    use splice_model::{ItemKind, ScaleMotion};

    let root = TestDirectory::new("scale-motion-mcp");
    let sample = root.0.join("sample.splice");
    copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &sample).unwrap();
    let mut store = Store::open(&sample).unwrap();
    let session = store.start_session("scale-motion", 1).unwrap().id;
    let request = |revision, ops: &str| {
        splice_json::parse(&format!(
            r#"{{"sessionId":"{session}","ifRevision":"r{revision}","ops":{ops}}}"#
        ))
        .unwrap()
    };
    history::apply(
        &mut store,
        &request(
            0,
            r#"[{"cmd":"motion.add","at":180,"length":150,"from":1,"to":1.16,"ref":"$motion"},
                {"cmd":"motion.set","item":"$motion","from":1}]"#,
        ),
    )
    .unwrap();
    let id = store
        .project()
        .items
        .order()
        .iter()
        .find_map(|&slot| {
            let items = &store.project().items;
            matches!(items.data(slot).unwrap().kind, ItemKind::Motion(_))
                .then(|| items.id(slot).unwrap())
        })
        .unwrap();
    let inspect = splice_json::parse(&format!(r#"{{"ids":["{id}"]}}"#)).unwrap();
    let assert_scales = |store: &Store, from, to| {
        let p = store.project();
        assert_eq!(
            p.items.data(p.items.find(id).unwrap()).unwrap().kind,
            ItemKind::Motion(ScaleMotion { from, to })
        );
        let result = timeline::inspect(p, &inspect).unwrap();
        let item = &result.field("items").unwrap().as_array().unwrap()[0];
        assert_eq!(item.field("kind").and_then(Json::as_str), Some("motion"));
        let params = item.field("params").unwrap();
        assert_eq!(params.field("from").and_then(Json::as_f64), Some(from));
        assert_eq!(params.field("to").and_then(Json::as_f64), Some(to));
    };
    assert_scales(&store, 1.0, 1.16);
    for (revision, args, from, to) in [
        (1, r#""from":1.08"#, 1.08, 1.16),
        (2, r#""to":1.24"#, 1.08, 1.24),
        (3, r#""from":1.24,"to":1.08"#, 1.24, 1.08),
    ] {
        let ops = format!(r#"[{{"cmd":"motion.set","item":"{id}",{args}}}]"#);
        history::apply(&mut store, &request(revision, &ops)).unwrap();
        assert_scales(&store, from, to);
    }
    drop(store);
    let replayed = Store::open(&sample).unwrap();
    assert_eq!(replayed.project().rev, 4);
    replayed.verify().unwrap();
    assert_scales(&replayed, 1.24, 1.08);
    replayed.close().unwrap();
    assert_scales(&Store::open(&sample).unwrap(), 1.24, 1.08);
}
