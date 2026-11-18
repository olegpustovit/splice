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
            vec!["sample.splice"],
            vec!["export", "sample.splice"],
            vec!["frame", "--at", "30", "out.png", "sample.splice"],
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
