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
    use splice_mcp::live::{Server, ViewCommand, Viewer};
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
                "{reply:?}"
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
            let live = peer.tool("project.info", r#"{"path":"Live.splice"}"#);
            assert_eq!(
                live.field("projectId").and_then(Json::as_str),
                Some("prj_2")
            );
            let focused = peer.tool(
                "ui.show",
                &format!(r#"{{"projectId":"prj_2","at":{{"item":"{title}","offset":15}}}}"#),
            );
            assert_eq!(focused.field("playhead").and_then(Json::as_u64), Some(15));
            let baseline = snapshot(&remote, "before.png");
            assert_eq!(baseline.field("frame").and_then(Json::as_u64), Some(15));
            let session = peer.tool("session.start", r#"{"projectId":"prj_2"}"#);
            let session = session.field("sessionId").and_then(Json::as_str).unwrap();
            let edit = peer.tool("edit.apply", &format!(r#"{{"projectId":"prj_2","sessionId":"{session}","ifRevision":"r1","ops":[{{"cmd":"title.set","item":"{title}","text":"Agent edit"}}]}}"#));
            assert_eq!(edit.field("revision").and_then(Json::as_str), Some("r2"));
            let updated = snapshot(&remote, "after.png");
            assert_eq!(updated.field("revision").and_then(Json::as_str), Some("r2"));
            assert_eq!(updated.field("frame").and_then(Json::as_u64), Some(15));
            let before = fs::read(remote.join("before.png")).unwrap();
            assert_ne!(before, fs::read(remote.join("after.png")).unwrap());
            undo.send(()).unwrap();
            undone.recv().unwrap();
            let restored = snapshot(&remote, "undo.png");
            assert_eq!(
                restored.field("revision").and_then(Json::as_str),
                Some("r3")
            );
            assert_eq!(before, fs::read(remote.join("undo.png")).unwrap());
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
        while !worker.is_finished() {
            if let Some(request) = server.next_request() {
                let reply = viewer.handle(&request.body, at, &mut |store, intent| match intent {
                    ViewCommand::Seek(frame) => {
                        at = frame;
                        Ok(Json::Null)
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
}

#[cfg(target_os = "macos")]
fn main() {
    harness::run(&[(
        "native_viewer_live_mcp_updates_snapshots_and_undo_restores_them",
        live::native_viewer_live_mcp_updates_snapshots_and_undo_restores_them,
    )]);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
