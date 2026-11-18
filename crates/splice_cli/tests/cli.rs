use splice_json::{Value as Json, parse};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("splice-cli-{name}-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_splice"));
        command
            .current_dir(&self.0)
            .env("XDG_STATE_HOME", self.0.join("state"));
        #[cfg(target_os = "macos")]
        command.env("HOME", &self.0);
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn success(output: Output) -> Json {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap()
}

fn error(output: Output, code: &str) -> Json {
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let error = parse(std::str::from_utf8(&output.stderr).unwrap()).unwrap();
    assert_eq!(error.field("code").and_then(Json::as_str), Some(code));
    assert!(
        error
            .field("fix")
            .and_then(Json::as_str)
            .is_some_and(|fix| !fix.is_empty())
    );
    error
}

fn project_data(path: &Path) -> Json {
    parse(&fs::read_to_string(path.join("project.json")).unwrap()).unwrap()
}

#[test]
fn cli_creates_previews_commits_snapshots_and_checks_a_project() {
    let scratch = Scratch::new("edit");
    let project = scratch.0.join("Field Notes.splice");
    let package = project.to_str().unwrap();
    let created = success(scratch.run(&["new", package]));
    assert_eq!(created.field("revision").and_then(Json::as_str), Some("r0"));
    error(scratch.run(&["new", package]), "EXISTS");
    assert_eq!(
        project_data(&project).field("rev").and_then(Json::as_u64),
        Some(0)
    );

    let initial_checkpoint = fs::read(project.join("project.json")).unwrap();

    let operations = scratch.0.join("ops.json");
    fs::write(
        &operations,
        r#"[{"cmd":"title.add","at":0,"length":20,"text":"Field notes"}]"#,
    )
    .unwrap();
    let ops = operations.to_str().unwrap();
    let preview =
        success(scratch.run(&["apply", ops, package, "--dry-run", "--if-revision", "r0"]));
    assert_eq!(preview.field("dryRun").and_then(Json::as_bool), Some(true));
    assert_eq!(preview.field("revision").and_then(Json::as_str), Some("r0"));
    assert_eq!(
        preview
            .field("changes")
            .and_then(Json::as_array)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(fs::metadata(project.join("journal.ops")).unwrap().len(), 0);

    let live = success(scratch.run(&[
        "apply",
        "--idempotency-key",
        "title",
        "--if-revision",
        "0",
        ops,
        package,
    ]));
    assert_eq!(live.field("revision").and_then(Json::as_str), Some("r1"));
    assert_eq!(live.field("replayed").and_then(Json::as_bool), Some(false));
    let replay = success(scratch.run(&[
        "apply",
        ops,
        "--project",
        package,
        "--idempotency-key",
        "title",
        "--if-revision",
        "r0",
    ]));
    assert_eq!(replay.field("revision").and_then(Json::as_str), Some("r1"));
    assert_eq!(replay.field("replayed").and_then(Json::as_bool), Some(true));
    let stale = error(
        scratch.run(&["apply", ops, package, "--if-revision", "r0"]),
        "STALE_REVISION",
    );
    assert_eq!(stale.field("current").and_then(Json::as_str), Some("r1"));
    let journal = fs::read_to_string(project.join("journal.ops")).unwrap();
    assert_eq!(journal.lines().count(), 1);
    let record = parse(journal.lines().next().unwrap()).unwrap();
    assert_eq!(record.field("source").and_then(Json::as_str), Some("human"));

    // A verified older checkpoint can lag the durable journal after an interrupted close.
    fs::write(project.join("project.json"), initial_checkpoint).unwrap();
    let checked_pending = success(scratch.run(&["check", package]));
    assert_eq!(
        checked_pending.field("revision").and_then(Json::as_str),
        Some("r1")
    );
    assert_eq!(
        project_data(&project).field("rev").and_then(Json::as_u64),
        Some(0)
    );

    let snapshot = success(scratch.run(&["snapshot", package]));
    assert_eq!(snapshot, project_data(&project));
    assert_eq!(snapshot.field("rev").and_then(Json::as_u64), Some(1));
    let items = snapshot.field("items").and_then(Json::as_array).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0]
            .field("params")
            .unwrap()
            .field("text")
            .and_then(Json::as_str),
        Some("Field notes")
    );

    let checked = success(scratch.run(&["check", package]));
    assert_eq!(checked.field("ok").and_then(Json::as_bool), Some(true));
    assert_eq!(checked.field("revision").and_then(Json::as_str), Some("r1"));
    assert_eq!(
        checked
            .field("stateHash")
            .and_then(Json::as_str)
            .unwrap()
            .len(),
        16
    );

    let changed = fs::read_to_string(project.join("project.json"))
        .unwrap()
        .replace("Field notes", "Tampered note");
    fs::write(project.join("project.json"), changed).unwrap();
    error(scratch.run(&["check", package]), "JOURNAL_CORRUPT");
}

#[test]
fn cli_resolves_project_from_working_directory_and_describes_commands() {
    let scratch = Scratch::new("paths");
    let project = scratch.0.join("work.splice");
    let package = project.to_str().unwrap();
    success(scratch.run(&["new", package]));
    fs::write(
        project.join("ops.json"),
        r#"{"ops":[{"cmd":"marker.add","at":7,"name":"Beat"}]}"#,
    )
    .unwrap();
    let applied = success(
        scratch
            .command()
            .current_dir(&project)
            .args(["apply", "ops.json"])
            .output()
            .unwrap(),
    );
    assert_eq!(applied.field("revision").and_then(Json::as_str), Some("r1"));

    fs::write(
        project.join("ops.json"),
        r#"{"dryRun":true,"ops":[{"cmd":"marker.add","at":10}]}"#,
    )
    .unwrap();
    error(
        scratch
            .command()
            .current_dir(&project)
            .args(["apply", "ops.json"])
            .output()
            .unwrap(),
        "BAD_ARGS",
    );
    let snapshot = success(
        scratch
            .command()
            .current_dir(&project)
            .arg("snapshot")
            .output()
            .unwrap(),
    );
    assert_eq!(snapshot.field("rev").and_then(Json::as_u64), Some(1));

    let desc = success(scratch.run(&["describe", "item.trim"]));
    assert_eq!(
        desc.field("example")
            .unwrap()
            .field("cmd")
            .and_then(Json::as_str),
        Some("item.trim")
    );
    let catalog = success(scratch.run(&["describe", "--query", "marker"]));
    assert!(
        !catalog
            .field("commands")
            .and_then(Json::as_array)
            .unwrap()
            .is_empty()
    );
    let version = scratch.run(&["version"]);
    assert!(version.status.success());
    assert!(
        std::str::from_utf8(&version.stdout)
            .unwrap()
            .starts_with("splice ")
    );
}

#[test]
fn cli_dispatches_mcp_friction_and_native_commands() {
    let scratch = Scratch::new("dispatch");
    if cfg!(target_os = "linux") {
        for args in [
            vec!["open", "sample.splice"],
            vec!["open", "sample.splice", "--snapshot", "out.png"],
            vec!["sample.splice"],
            vec!["export", "sample.splice"],
            vec!["frame", "--at", "30", "out.png", "sample.splice"],
            vec!["--smoke-test"],
            Vec::new(),
        ] {
            error(scratch.run(&args), "UNSUPPORTED_PLATFORM");
        }
        assert!(!scratch.0.join("out.png").exists());
    }

    let mut mcp = scratch
        .command()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    mcp.stdin.take().unwrap().write_all(concat!(
        r#"{"jsonrpc":"2.0","id":"init","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"cli-test","version":"1"}}}"#, "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#, "\n",
    ).as_bytes()).unwrap();
    let output = mcp.wait_with_output().unwrap();
    assert!(output.status.success());
    let wire = std::str::from_utf8(&output.stdout).unwrap();
    let catalog = parse(wire.lines().nth(1).unwrap()).unwrap();
    assert_eq!(
        catalog
            .field("result")
            .unwrap()
            .field("tools")
            .and_then(Json::as_array)
            .unwrap()
            .len(),
        19
    );
    assert!(
        scratch
            .run(&["friction", "list", "--open"])
            .status
            .success()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn native_viewer_smoke_opens_and_closes_a_hidden_window() {
    use std::os::unix::ffi::OsStrExt;

    let scratch = Scratch::new("viewer");
    let output = success(scratch.run(&["--smoke-test"]));
    assert_eq!(output.field("ok").and_then(Json::as_bool), Some(true));
    let project = scratch.0.join("Viewer Sample.splice");
    let path = project.to_str().unwrap();
    success(scratch.run(&["new", path]));
    let output = success(scratch.run(&["--smoke-test", path]));
    assert_eq!(output.field("ok").and_then(Json::as_bool), Some(true));
    let saved = scratch
        .0
        .join("Library/Application Support/Splice/last-project");
    assert_eq!(
        fs::read(saved).unwrap(),
        project.canonicalize().unwrap().as_os_str().as_bytes()
    );
    assert_eq!(
        success(scratch.run(&["check", path]))
            .field("revision")
            .and_then(Json::as_str),
        Some("r0")
    );
}

#[test]
fn cli_imports_media_and_persists_the_descriptor() {
    let scratch = Scratch::new("import");
    let project = scratch.0.join("import.splice");
    let package = project.to_str().unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media");
    let operations = fixtures.join("import.ops.json");
    let operations = operations.to_str().unwrap();
    success(scratch.run(&["new", package]));

    if cfg!(target_os = "macos") {
        fs::copy(fixtures.join("probe.mov"), project.join("probe.mov")).unwrap();
    } else {
        // Portable acceptance uses the same operation with a pre-probed fixture descriptor.
        fs::remove_file(project.join("journal.base.json")).unwrap();
        let snapshot = fs::read_to_string(project.join("project.json"))
            .unwrap()
            .replace(r#""serial":1"#, r#""serial":2"#);
        fs::write(project.join("project.json"), snapshot).unwrap();
        fs::write(project.join("media.json"), r#"{"format":"splice/1","media":[{"id":"med_1","kind":"video","path":"probe.mov","bookmark":"Zml4dHVyZQ==","size":3014,"mtime":1,"probe":{"fps":30,"frames":30,"width":96,"height":64,"audio":[],"codec":"h264"}}]}"#).unwrap();
    }

    let receipt = success(scratch.run(&["apply", operations, package, "--if-revision", "r0"]));
    assert_eq!(receipt.field("revision").and_then(Json::as_str), Some("r1"));
    let snapshot = success(scratch.run(&["snapshot", package]));
    let clip = &snapshot.field("clips").and_then(Json::as_array).unwrap()[0];
    assert_eq!(clip.field("media").and_then(Json::as_str), Some("med_1"));
    assert_eq!(clip.field("length").and_then(Json::as_u64), Some(30));
    let media = parse(&fs::read_to_string(project.join("media.json")).unwrap()).unwrap();
    let media = &media.field("media").and_then(Json::as_array).unwrap()[0];
    assert_eq!(
        media.field("path").and_then(Json::as_str),
        Some("probe.mov")
    );
    assert!(
        media
            .field("bookmark")
            .and_then(Json::as_str)
            .is_some_and(|value| !value.is_empty())
    );
    let probe = media.field("probe").unwrap();
    for (field, value) in [("frames", 30), ("width", 96), ("height", 64)] {
        assert_eq!(probe.field(field).and_then(Json::as_u64), Some(value));
    }
    if cfg!(target_os = "macos") {
        fs::remove_file(project.join("probe.mov")).unwrap();
    }
    assert_eq!(
        success(scratch.run(&["check", package]))
            .field("revision")
            .and_then(Json::as_str),
        Some("r1")
    );
}

#[cfg(target_os = "macos")]
fn export_sample(scratch: &Scratch) -> PathBuf {
    let project = scratch.0.join("native.splice");
    let package = project.to_str().unwrap();
    success(scratch.run(&["new", package]));
    // Keep this subprocess acceptance small. The complete sample has a separate media check.
    let snapshot = project.join("project.json");
    let json = fs::read_to_string(&snapshot)
        .unwrap()
        .replace("[1080,1920]", "[96,64]");
    fs::write(&snapshot, json).unwrap();
    fs::remove_file(project.join("journal.base.json")).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media");
    fs::copy(fixtures.join("probe.mov"), project.join("probe.mov")).unwrap();
    success(scratch.run(&[
        "apply",
        fixtures.join("import.ops.json").to_str().unwrap(),
        package,
    ]));
    project
}

#[cfg(target_os = "macos")]
#[test]
fn native_export_cli_uses_revision_names_and_exclusive_publication() {
    let scratch = Scratch::new("native-cli-export");
    let project = export_sample(&scratch);
    let package = project.to_str().unwrap();
    assert!(!project.join("exports/native-r1.mp4").exists());
    error(
        scratch.run(&["export", package, "--output", "project.json", "--overwrite"]),
        "BAD_ARGS",
    );
    let output = success(scratch.run(&["export", package, "--preset", "social-h264"]));
    let file = project.join("exports/native-r1.mp4");
    assert_eq!(
        output.field("output").and_then(Json::as_str),
        file.canonicalize().unwrap().to_str()
    );
    assert_eq!(output.field("revision").and_then(Json::as_str), Some("r1"));
    assert_eq!(output.field("state").and_then(Json::as_str), Some("done"));
    let bytes = fs::read(&file).unwrap();
    assert_eq!(&bytes[4..8], b"ftyp");
    error(scratch.run(&["export", package]), "EXISTS");
    assert_eq!(fs::read(&file).unwrap(), bytes);
    fs::write(&file, b"old movie").unwrap();
    success(scratch.run(&["export", package, "--preset", "social-h264", "--overwrite"]));
    assert_eq!(&fs::read(&file).unwrap()[4..8], b"ftyp");
    assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
    let project = success(scratch.run(&["check", package]));
    assert_eq!(project.field("revision").and_then(Json::as_str), Some("r1"));
}

#[cfg(target_os = "macos")]
#[test]
fn native_export_mcp_publishes_without_status_calls_and_cancels_safely() {
    use std::{
        io::{BufRead, BufReader},
        time::{Duration, Instant},
    };

    let scratch = Scratch::new("native-mcp-export");
    let project = export_sample(&scratch);
    let mut child = scratch
        .command()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut next_id = 0;
    let mut send = |input: &mut std::process::ChildStdin, name: &str, args: &str| {
        next_id += 1;
        writeln!(input, r#"{{"jsonrpc":"2.0","id":{next_id},"method":"tools/call","params":{{"name":"{name}","arguments":{args}}}}}"#).unwrap();
        input.flush().unwrap();
    };
    writeln!(input, r#"{{"jsonrpc":"2.0","id":"init","method":"initialize","params":{{"protocolVersion":"2025-11-25","capabilities":{{}},"clientInfo":{{"name":"export-test","version":"1"}}}}}}"#).unwrap();
    input.flush().unwrap();
    let mut line = String::new();
    assert!(output.read_line(&mut line).unwrap() > 0);
    let mut receive = || {
        let mut line = String::new();
        assert_ne!(output.read_line(&mut line).unwrap(), 0);
        let response = parse(&line).unwrap();
        response
            .field("result")
            .unwrap()
            .field("structuredContent")
            .unwrap()
            .clone()
    };
    send(&mut input, "project.info", r#"{"path":"native.splice"}"#);
    assert_eq!(
        receive().field("projectId").and_then(Json::as_str),
        Some("prj_1")
    );
    send(
        &mut input,
        "session.start",
        r#"{"projectId":"prj_1","agent":"export-test"}"#,
    );
    assert_eq!(
        receive().field("sessionId").and_then(Json::as_str),
        Some("ses_1")
    );
    send(
        &mut input,
        "export.start",
        r#"{"projectId":"prj_1","sessionId":"ses_1","ifRevision":"r0"}"#,
    );
    assert_eq!(
        receive()
            .field("error")
            .unwrap()
            .field("code")
            .and_then(Json::as_str),
        Some("STALE_REVISION")
    );
    send(
        &mut input,
        "export.start",
        r#"{"projectId":"prj_1","sessionId":"ses_1","ifRevision":"r1","path":"finished.mp4"}"#,
    );
    let started = receive();
    assert_eq!(
        started.field("jobId").and_then(Json::as_str),
        Some("job_1"),
        "{started:?}"
    );
    assert_eq!(
        started.field("state").and_then(Json::as_str),
        Some("running")
    );
    assert_eq!(
        started
            .field("settings")
            .unwrap()
            .field("codec")
            .and_then(Json::as_str),
        Some("HEVC")
    );

    let finished = project.join("finished.mp4");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !finished.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(15));
    }
    send(&mut input, "jobs.status", r#"{"jobId":"job_1"}"#);
    let status = receive();
    assert_eq!(
        status.field("progress").and_then(Json::as_u64),
        Some(1),
        "{status:?}"
    );
    assert_eq!(
        status.field("output").and_then(Json::as_str),
        finished.canonicalize().unwrap().to_str(),
        "{status:?}"
    );

    let preserved = project.join("preserved.mp4");
    fs::write(&preserved, b"previous movie").unwrap();
    send(
        &mut input,
        "export.start",
        r#"{"projectId":"prj_1","sessionId":"ses_1","ifRevision":"r1","path":"preserved.mp4","overwrite":true}"#,
    );
    send(&mut input, "jobs.cancel", r#"{"jobId":"job_2"}"#);
    let started = receive();
    assert_eq!(
        started.field("jobId").and_then(Json::as_str),
        Some("job_2"),
        "{started:?}"
    );
    let cancelled = receive();
    assert!(
        matches!(
            cancelled.field("state").and_then(Json::as_str),
            Some("cancelling" | "cancelled")
        ),
        "{cancelled:?}"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        send(&mut input, "jobs.status", r#"{"jobId":"job_2"}"#);
        let status = receive();
        match status.field("state").and_then(Json::as_str) {
            Some("cancelled") => break,
            Some("cancelling") if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(15))
            }
            _ => panic!("Cancellation did not finish: {status:?}"),
        }
    }
    assert_eq!(fs::read(preserved).unwrap(), b"previous movie");
    assert!(fs::read_dir(&project).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".splice-export-")
    }));
    drop(input);
    assert!(child.wait_with_output().unwrap().status.success());
}

#[cfg(target_os = "macos")]
#[test]
fn native_frames_cli_and_mcp_render_pngs_from_the_same_revision() {
    let scratch = Scratch::new("native-frames");
    let project = scratch.0.join("Frame Sample #1.splice");
    fs::rename(export_sample(&scratch), &project).unwrap();
    let package = project.to_str().unwrap();
    let signature = b"\x89PNG\r\n\x1a\n";
    let output = success(scratch.run(&["frame", "--at", "15", "grab.png", package]));
    assert_eq!(output.field("revision").and_then(Json::as_str), Some("r1"));
    assert_eq!(output.field("frame").and_then(Json::as_u64), Some(15));
    let png = fs::read(scratch.0.join("grab.png")).unwrap();
    assert_eq!(&png[..8], signature);
    error(
        scratch.run(&["frame", "--at", "30", "late.png", package]),
        "OUT_OF_RANGE",
    );
    assert!(!scratch.0.join("late.png").exists());
    error(
        scratch.run(&["frame", "--at", "15", "grab.png", package]),
        "EXISTS",
    );
    assert_eq!(fs::read(scratch.0.join("grab.png")).unwrap(), png);

    let mut server = scratch
        .command()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    server.stdin.take().unwrap().write_all(concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"frame-test","version":"1"}}}"#, "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"project.info","arguments":{"path":"Frame Sample #1.splice"}}}"#, "\n",
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"view.frames","arguments":{"projectId":"prj_1","count":3}}}"#, "\n"
    ).as_bytes()).unwrap();
    let output = server.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let response = parse(
        std::str::from_utf8(&output.stdout)
            .unwrap()
            .lines()
            .nth(2)
            .unwrap(),
    )
    .unwrap();
    let result = response.field("result").unwrap();
    assert_eq!(
        result.field("isError").and_then(Json::as_bool),
        Some(false),
        "{result:?}"
    );
    let data = result.field("structuredContent").unwrap();
    assert_eq!(data.field("revision").and_then(Json::as_str), Some("r1"));
    let frames = data.field("frames").and_then(Json::as_array).unwrap();
    let content = result.field("content").and_then(Json::as_array).unwrap();
    assert_eq!(content.len(), 7);
    assert_eq!(frames.len(), 3);
    for ((frame, attachments), at) in frames
        .iter()
        .zip(content[1..].chunks_exact(2))
        .zip([0, 15, 29])
    {
        assert_eq!(frame.field("frame").and_then(Json::as_u64), Some(at));
        assert_eq!(frame.field("width").and_then(Json::as_u64), Some(96));
        let [image, link] = attachments else {
            unreachable!()
        };
        assert_eq!(image.field("type").and_then(Json::as_str), Some("image"));
        assert!(
            image
                .field("data")
                .unwrap()
                .as_str()
                .unwrap()
                .starts_with("iVBORw0KGgo")
        );
        assert_eq!(
            link.field("type").and_then(Json::as_str),
            Some("resource_link")
        );
        let uri = link.field("uri").and_then(Json::as_str).unwrap();
        assert_eq!(frame.field("uri").and_then(Json::as_str), Some(uri));
        assert!(
            uri.starts_with("file://")
                && uri.contains("/Frame%20Sample%20%231.splice/cache/frames/")
        );
        let cached = project
            .join("cache/frames")
            .join(uri.rsplit('/').next().unwrap());
        assert_eq!(&fs::read(cached).unwrap()[..8], signature);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_viewer_smoke_presents_media() {
    let scratch = Scratch::new("viewer-media");
    let project = export_sample(&scratch);
    let output = success(scratch.run(&["--smoke-test", project.to_str().unwrap()]));
    assert_eq!(output.field("ok").and_then(Json::as_bool), Some(true));
    assert_eq!(output.field("programFrame").and_then(Json::as_u64), Some(0));
}

#[cfg(target_os = "macos")]
#[test]
fn native_viewer_snapshot_renders_requested_frames_and_preserves_existing_files() {
    let scratch = Scratch::new("viewer-snapshot");
    let project = export_sample(&scratch);
    let package = project.to_str().unwrap();
    fs::write(
        scratch.0.join("title.json"),
        r#"[{"cmd":"title.add","at":12,"length":12,"text":"Snapshot title"}]"#,
    )
    .unwrap();
    success(scratch.run(&["apply", "title.json", package]));
    let first = success(scratch.run(&["open", package, "--snapshot", "first.png"]));
    assert_eq!(first.field("frame").and_then(Json::as_u64), Some(0));
    let later = success(scratch.run(&["open", package, "--snapshot", "later.png", "--at", "15"]));
    assert_eq!(later.field("frame").and_then(Json::as_u64), Some(15));
    assert_eq!(later.field("revision").and_then(Json::as_str), Some("r2"));
    let first = fs::read(scratch.0.join("first.png")).unwrap();
    let later = fs::read(scratch.0.join("later.png")).unwrap();
    for png in [&first, &later] {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[16..20], &960u32.to_be_bytes());
        assert_eq!(&png[20..24], &640u32.to_be_bytes());
    }
    assert_ne!(first, later);
    error(
        scratch.run(&["open", package, "--snapshot", "later.png"]),
        "EXISTS",
    );
    assert_eq!(fs::read(scratch.0.join("later.png")).unwrap(), later);
    error(
        scratch.run(&["open", package, "--snapshot", "outside.png", "--at", "30"]),
        "OUT_OF_RANGE",
    );
    assert!(!scratch.0.join("outside.png").exists());
    assert!(
        !scratch
            .0
            .join("Library/Application Support/Splice/last-project")
            .exists()
    );
}

#[test]
fn viewer_snapshot_rejects_malformed_options() {
    let scratch = Scratch::new("viewer-snapshot-args");
    for args in [
        vec!["open", "project.splice", "--snapshot"],
        vec![
            "open",
            "project.splice",
            "--snapshot",
            "out.png",
            "--at",
            "-1",
        ],
        vec![
            "open",
            "project.splice",
            "--snapshot",
            "out.png",
            "--at",
            "2147483648",
        ],
        vec!["open", "project.splice", "--at", "0"],
    ] {
        error(scratch.run(&args), "BAD_ARGS");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_viewer_playbar_golden() {
    let scratch = Scratch::new("playbar-golden");
    let project = "Transport.splice";
    success(scratch.run(&["new", project]));
    fs::write(
        scratch.0.join("title.json"),
        r#"[{"cmd":"title.add","at":60,"length":60,"text":"End card"}]"#,
    )
    .unwrap();
    success(scratch.run(&["apply", "title.json", project]));
    success(scratch.run(&["open", project, "--snapshot", "bar.png", "--at", "46"]));
    let png = fs::read(scratch.0.join("bar.png")).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let file = if std::env::var("SPLICE_BLESS").as_deref() == Ok("1") {
        root.join("target/splice-playbar.png")
    } else {
        root.join("fixtures/render/playbar.png")
    };
    splice_testkit::golden(&file, &png).unwrap();
}
