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
    assert_eq!(tools.len(), 19);
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
        copy_fixture(&fixtures.join("../projects/sample.splice"), &sample).unwrap();
        let take = sample.join("takes/med_screen");
        fs::create_dir_all(&take).unwrap();
        fs::write(take.join("events.json"), r#"{"events":[{"t":0.2,"k":"move","x":0.25,"y":0.5},{"t":0.4,"k":"down","b":0},{"t":0.5,"k":"key","ch":"private","keyCode":4}]}"#).unwrap();

        let seed = case.with_extension("splice");
        let project = directory.join("repro.splice");
        let mut paths = vec![("$sample", sample.as_path()), ("$new", new.as_path())];
        if seed.is_dir() {
            copy_fixture(&seed, &project).unwrap();
            paths.push(("$project", &project));
        }
        transcript::run(&case, LEGACY_PRELUDE, &paths, &mut |input| {
            let output = replay_headless(input)?;
            if name == "takes.events" {
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

#[test]
fn m10_enrich_resolves_refs_and_refuses_overlays() {
    let root = TestDirectory::new("enrich-refs");
    let sample = root.0.join("sample.splice");
    copy_fixture(&mcp_fixtures().join("../projects/sample.splice"), &sample).unwrap();
    let take = sample.join("takes/med_screen");
    fs::create_dir_all(&take).unwrap();
    fs::write(
        take.join("events.json"),
        r#"{"events":[{"t":0.4,"k":"down","b":0,"x":0.9,"y":0.1},{"t":4.0,"k":"move","x":0.1,"y":0.9},{"t":5.0,"k":"key","ch":"a","keyCode":4}]}"#,
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
    let store = splice_core::history::Store::open(&sample).unwrap();
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
    let events = super::media::events(
        store.path(),
        store.project(),
        &splice_json::parse(r#"{"clip":"clp_take","from":120,"to":240}"#).unwrap(),
    )
    .unwrap();
    assert_eq!(events.field("clicks").unwrap().as_array().unwrap().len(), 1);
    assert!(events.field("cursor").unwrap().as_array().unwrap().len() > 300);
}

#[test]
fn code_run_reports_revisions_for_every_project_it_touched() {
    let args = splice_json::parse(
        r#"{"code":"splice.edit.apply({projectId:\"prj_1\", ifRevision:\"r3\", ops:[]}); splice.edit.apply({projectId:\"prj_2\", ifRevision:\"r7\", ops:[]}); splice.edit.apply({projectId:\"prj_1\", ifRevision:\"r4\", ops:[]}); return 1;"}"#,
    )
    .unwrap();
    let mut revision = 3;
    let reply = code::run(&args, &mut |_, args| {
        let project = args.field("projectId").and_then(Json::as_str).unwrap_or("");
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
    let setup = |id: &str| -> &'static str {
        match id {
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
            "caption.set" | "caption.split" | "caption.merge" => "[]",
            _ => "[]",
        }
    };
    let mut rows = String::from("[");
    for (index, row) in splice_core::commands::COMMANDS.iter().enumerate() {
        if index > 0 {
            rows.push(',');
        }
        rows.push_str(&format!(
            "[\"{}\",{},{}]",
            row.id,
            row.example,
            setup(row.id)
        ));
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
    let failures = result.field("failures").unwrap().as_array().unwrap();
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

/// T38.6: a zoom item reports the largest amount whose card stays on the canvas. On the
/// portrait sample `auto` zooms the content (never overflows, 5×); in `frame` mode the
/// 972×1057 card on a 1080×1920 canvas fits up to 1080/972.
#[test]
fn zoom_items_report_their_fit_safe_scale() {
    let mut project = sample_project();
    let safe = |project: &splice_model::Project| {
        let args = splice_json::parse(r#"{"ids":["ovl_detail"]}"#).unwrap();
        let reply = super::timeline::inspect(project, &args).unwrap();
        reply.field("items").unwrap().as_array().unwrap()[0]
            .field("fitSafeScale")
            .and_then(Json::as_f64)
            .unwrap()
    };
    assert_eq!(safe(&project), 5.0);
    let ops =
        splice_json::parse(r#"[{"cmd":"zoom.set","zoom":"ovl_detail","mode":"frame"}]"#).unwrap();
    splice_core::edit::apply(
        &mut project,
        ops.as_array().unwrap(),
        splice_core::package::journal::Source::Agent,
        false,
    )
    .unwrap();
    assert_eq!(
        safe(&project),
        (1080.0_f64 / 972.0 * 1000.0).floor() / 1000.0
    );
}
