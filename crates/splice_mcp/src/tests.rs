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
    assert!(!text.contains("record."));
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
    assert!(
        index.len() * 2 < full.len(),
        "index {} vs full {}",
        index.len(),
        full.len()
    );
    for row in splice_core::commands::COMMANDS {
        let listed = index.contains(&format!("   *   {} — ", row.id));
        assert_eq!(
            listed,
            row.tier != splice_core::commands::Tier::Planned,
            "{}",
            row.id
        );
    }
    for tool in super::tools::TOOLS
        .iter()
        .filter(|tool| !tool.name.starts_with("record."))
    {
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
    let filled = super::enrich::ops(&store, ops.as_array().unwrap())
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
