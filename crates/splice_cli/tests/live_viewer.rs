#[cfg(target_os = "macos")]
#[rustfmt::skip]
#[path = "../../../pxui/tools/t/window_harness.rs"]
mod harness;

#[cfg(target_os = "macos")]
fn release_keepalive() {}

#[cfg(target_os = "macos")]
mod live {
    use splice_core::history::Store;
    use splice_json::{Value as Json, parse};
    use splice_mcp::live::{Server, ViewCommand, ViewTab, Viewer, ViewerZoom};
    use std::{
        fs,
        io::{BufRead, BufReader, Write},
        os::unix::fs::FileTypeExt,
        path::Path,
        process::{ChildStdin, ChildStdout, Command, Stdio},
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    struct Peer {
        input: ChildStdin,
        output: BufReader<ChildStdout>,
    }

    impl Peer {
        fn request(&mut self, line: &str) -> Json {
            writeln!(self.input, "{line}").unwrap();
            self.input.flush().unwrap();
            let mut wire = String::new();
            self.output.read_line(&mut wire).unwrap();
            parse(&wire).unwrap()
        }

        fn tool(&mut self, name: &str, args: &str) -> Json {
            let reply = self.request(&format!(r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"{name}","arguments":{args}}}}}"#));
            let result = reply.field("result").unwrap();
            assert_eq!(
                result.field("isError").and_then(Json::as_bool),
                Some(false),
                "{name} {args}: {reply:?}"
            );
            result.field("structuredContent").unwrap().clone()
        }
    }

    fn command(dir: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_splice"));
        command.current_dir(dir).env("HOME", dir);
        command
    }

    fn snapshot(dir: &Path, file: &str) -> Json {
        let output = command(dir)
            .args(["open", "Live.splice", "--snapshot", file])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap()
    }

    pub(super) fn native_viewer_live_mcp_updates_snapshots_and_undo_restores_them() {
        let dir = std::env::temp_dir().join(format!(
            "splice-live-viewer-with-a-long-state-path-for-darwin-sockets-{}",
            std::process::id()
        ));
        fs::create_dir(&dir).unwrap();
        let state = dir.join("Library/Application Support/Splice");
        let path = dir.join("Live.splice");
        let mut store = Store::create(&path, &Default::default()).unwrap();
        let baseline =
            parse(r#"[{"cmd":"title.add","at":0,"length":60,"text":"Before"}]"#).unwrap();
        store
            .apply(baseline.as_array().unwrap(), &Default::default())
            .unwrap();
        let items = &store.project().items;
        let title = items.id(items.order()[0]).unwrap().to_string();
        let mut viewer = Viewer::new(store, Some(state.clone()));
        let server = Server::start(viewer.store(), &state, || true).unwrap();
        let mut sockets = fs::read_dir(state.join("live")).unwrap();
        let socket = sockets.next().unwrap().unwrap();
        assert!(socket.file_type().unwrap().is_socket());
        assert!(sockets.next().is_none());
        let (undo, undo_request) = mpsc::channel();
        let (complete, undone) = mpsc::channel();
        let remote = dir.clone();
        let worker = thread::spawn(move || {
            let mut child = command(&remote)
                .arg("mcp")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut peer = Peer {
                input: child.stdin.take().unwrap(),
                output: BufReader::new(child.stdout.take().unwrap()),
            };
            assert!(peer.request(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"live-test","version":"1"}}}"#).field("result").is_some());
            let local = peer.tool(
                "project.info",
                r#"{"path":"Headless.splice","create":true}"#,
            );
            assert_eq!(
                local.field("projectId").and_then(Json::as_str),
                Some("prj_1")
            );
            let headless = peer.tool(
                "ui.show",
                r#"{"projectId":"prj_1","range":{"from":5,"to":25},"view":"mcp","viewer":{"zoom":1.25}}"#,
            );
            assert_eq!(headless.field("applied"), Some(&Json::Bool(false)));
            assert_eq!(
                headless.field("reason").and_then(Json::as_str),
                Some("headless")
            );
            let live = peer.tool("project.info", r#"{"path":"Live.splice"}"#);
            assert_eq!(
                live.field("projectId").and_then(Json::as_str),
                Some("prj_2")
            );
            let focused = peer.tool(
                "ui.show",
                &format!(r#"{{"projectId":"prj_2","at":{{"item":"{title}","offset":15}},"select":["{title}","{title}"],"range":{{"from":5,"to":25}},"view":"mcp","viewer":{{"zoom":1.25}}}}"#),
            );
            assert_eq!(focused.field("applied"), Some(&Json::Bool(true)));
            assert_eq!(focused.field("playhead").and_then(Json::as_u64), Some(15));
            let ids = Json::Array(vec![Json::String(title.to_string())]);
            assert_eq!(focused.field("selection"), Some(&ids));
            let query = peer.tool("ui.show", r#"{"projectId":"prj_2"}"#);
            assert_eq!(query.field("selection"), Some(&ids));
            let clear = peer.tool("ui.show", r#"{"projectId":"prj_2","select":[]}"#);
            assert_eq!(clear.field("selection"), Some(&Json::Array(Vec::new())));
            let baseline = snapshot(&remote, "before.png");
            assert_eq!(baseline.field("frame").and_then(Json::as_u64), Some(15));
            let session = peer.tool("session.start", r#"{"projectId":"prj_2"}"#);
            let session = session.field("sessionId").and_then(Json::as_str).unwrap();
            let edit = peer.tool("edit.apply", &format!(r#"{{"projectId":"prj_2","sessionId":"{session}","ifRevision":"r1","ops":[{{"cmd":"title.set","item":"{title}","text":"Agent edit"}}]}}"#));
            assert_eq!(edit.field("revision").and_then(Json::as_str), Some("r2"));
            let updated = snapshot(&remote, "after.png");
            assert_eq!(updated.field("revision").and_then(Json::as_str), Some("r2"));
            assert_eq!(updated.field("frame").and_then(Json::as_u64), Some(15));
            let mut before = without_revision_badge(&remote.join("before.png"));
            let after = without_revision_badge(&remote.join("after.png"));
            let stage = (64 * 960 * 4)..(241 * 960 * 4);
            assert!(
                before[stage.clone()] != after[stage],
                "The Program should show the title edit"
            );
            let rendered = peer.tool("view.frames", r#"{"projectId":"prj_2","at":[15,30]}"#);
            assert_eq!(
                rendered.field("revision").and_then(Json::as_str),
                Some("r2")
            );
            let frames = rendered.field("frames").and_then(Json::as_array).unwrap();
            assert_eq!(frames.len(), 2);
            assert!(
                frames
                    .iter()
                    .all(|image| image.field("uri").and_then(Json::as_str).is_some())
            );
            undo.send(()).unwrap();
            undone.recv().unwrap();
            let ended = peer.tool(
                "session.end",
                &format!(r#"{{"projectId":"prj_2","sessionId":"{session}"}}"#),
            );
            assert_eq!(ended.field("open"), Some(&Json::Bool(false)));
            let restored = snapshot(&remote, "undo.png");
            assert_eq!(
                restored.field("revision").and_then(Json::as_str),
                Some("r3")
            );
            let mut undone_pixels = without_revision_badge(&remote.join("undo.png"));
            assert_eq!(mask_redo_ink(&mut before), 0);
            assert!(
                mask_redo_ink(&mut undone_pixels) > 0,
                "Undo should enable Redo"
            );
            assert!(
                before == undone_pixels,
                "Undo should restore every pixel outside the revision badge and Redo control"
            );
            assert_eq!(
                peer.tool("timeline.snapshot", r#"{"projectId":"prj_1"}"#)
                    .field("revision")
                    .and_then(Json::as_str),
                Some("r0")
            );
            drop(peer);
            assert!(child.wait().unwrap().success());
        });
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut at = 0;
        let mut selection = Vec::new();
        while !worker.is_finished() {
            if let Some(request) = server.next_request() {
                let reply = viewer.handle(&request.body, at, &mut |store, intent| match intent {
                    ViewCommand::Show {
                        at: seek,
                        select,
                        expand,
                        range,
                        view,
                        zoom,
                    } => {
                        assert!(expand.is_none());
                        if let Some(seek) = seek {
                            assert_eq!(seek, 15);
                            assert_eq!(range, Some([5, 25]));
                            assert_eq!(view, Some(ViewTab::Agent));
                            assert_eq!(zoom, Some(ViewerZoom::Factor(1.25)));
                            at = seek;
                        }
                        if let Some(select) = select {
                            selection = select;
                        }
                        Ok(Json::Array(
                            selection
                                .iter()
                                .map(|id| Json::String(id.to_string()))
                                .collect(),
                        ))
                    }
                    ViewCommand::Snapshot { at, output } => {
                        splice_app::snapshot_output(store, at, &output)
                    }
                });
                request.reply(reply);
            }
            if undo_request.try_recv().is_ok() {
                assert_eq!(viewer.store_mut().human_undo(0).unwrap().restored, Some(1));
                complete.send(()).unwrap();
            }
            assert!(
                Instant::now() < deadline,
                "live MCP subprocess did not finish"
            );
            thread::sleep(Duration::from_millis(1));
        }
        worker.join().unwrap();
        drop(server);
        viewer.close().unwrap();
        assert!(fs::read_dir(state.join("live")).unwrap().next().is_none());
        let fallback = snapshot(&dir, "headless.png");
        assert_eq!(fallback.field("frame").and_then(Json::as_u64), Some(0));
        assert_eq!(
            fallback.field("revision").and_then(Json::as_str),
            Some("r3")
        );
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.project().rev, 3);
        reopened.close().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
    /// T10.1: an agent opened the project first; the viewer takes it over, and the agent's
    /// next edit on the same connection lands in the viewer.
    fn without_revision_badge(path: &Path) -> Vec<u8> {
        let (mut pixels, [width, height]) = apple_sys::image::decode_rgba(path).unwrap();
        assert_eq!([width, height], [960, 640]);
        let stride = width as usize * 4;
        for line in pixels.chunks_exact_mut(stride).take(34) {
            // A durable undo writes a new revision.
            line[845 * 4..881 * 4].fill(0);
        }
        pixels
    }

    fn mask_redo_ink(pixels: &mut [u8]) -> usize {
        let mut bright = 0;
        for row in pixels.chunks_exact_mut(960 * 4).skip(282).take(28) {
            let control = &mut row[325 * 4..351 * 4];
            bright += control
                .chunks_exact(4)
                .filter(|pixel| pixel[0] > 128)
                .count();
            control.fill(0);
        }
        bright
    }

    pub(super) fn native_viewer_takes_over_a_project_held_by_headless_mcp() {
        let dir = std::env::temp_dir().join(format!("splice-handoff-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let state = dir.join("Library/Application Support/Splice");
        let path = dir.join("Live.splice");
        let mut store = Store::create(&path, &Default::default()).unwrap();
        let baseline =
            parse(r#"[{"cmd":"title.add","at":0,"length":60,"text":"Before"}]"#).unwrap();
        store
            .apply(baseline.as_array().unwrap(), &Default::default())
            .unwrap();
        let items = &store.project().items;
        let title = items.id(items.order()[0]).unwrap().to_string();
        store.close().unwrap();
        let (held, holding) = mpsc::channel();
        let (go, edit) = mpsc::channel();
        let remote = dir.clone();
        let worker = thread::spawn(move || {
            let mut child = command(&remote)
                .arg("mcp")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut peer = Peer {
                input: child.stdin.take().unwrap(),
                output: BufReader::new(child.stdout.take().unwrap()),
            };
            assert!(peer.request(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"handoff-test","version":"1"}}}"#).field("result").is_some());
            let info = peer.tool("project.info", r#"{"path":"Live.splice"}"#);
            assert_eq!(
                info.field("projectId").and_then(Json::as_str),
                Some("prj_1")
            );
            let session = peer.tool("session.start", r#"{"projectId":"prj_1"}"#);
            let session = session.field("sessionId").and_then(Json::as_str).unwrap();
            held.send(()).unwrap();
            edit.recv().unwrap();
            let edit = peer.tool("edit.apply", &format!(r#"{{"projectId":"prj_1","sessionId":"{session}","ifRevision":"r1","ops":[{{"cmd":"title.set","item":"{title}","text":"Agent edit"}}]}}"#));
            assert_eq!(edit.field("revision").and_then(Json::as_str), Some("r2"));
            drop(peer);
            assert!(child.wait().unwrap().success());
        });
        holding.recv().unwrap();
        assert_eq!(
            Store::open(&path).err().map(|error| error.code),
            Some("LOCKED")
        );
        assert!(splice_mcp::live::handoff(&state, &path).unwrap());
        let mut viewer = Viewer::new(Store::open(&path).unwrap(), Some(state.clone()));
        let server = Server::start(viewer.store(), &state, || true).unwrap();
        go.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while !worker.is_finished() {
            if let Some(request) = server.next_request() {
                let reply = viewer.handle(&request.body, 0, &mut |_, _| Ok(Json::Null));
                request.reply(reply);
            }
            assert!(Instant::now() < deadline, "handed-off MCP did not finish");
            thread::sleep(Duration::from_millis(1));
        }
        worker.join().unwrap();
        assert_eq!(viewer.store().project().rev, 2);
        drop(server);
        viewer.close().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(target_os = "macos")]
fn main() {
    harness::run(&[
        (
            "native_viewer_live_mcp_updates_snapshots_and_undo_restores_them",
            live::native_viewer_live_mcp_updates_snapshots_and_undo_restores_them,
        ),
        (
            "native_viewer_takes_over_a_project_held_by_headless_mcp",
            live::native_viewer_takes_over_a_project_held_by_headless_mcp,
        ),
    ]);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
