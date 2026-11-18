#!/bin/sh
# Run from the Splice tree on macOS.
set -eu
check_dir=$(mktemp -d "${TMPDIR:-/tmp}/splice-export-check.XXXXXX")
trap 'rm -rf "$check_dir"' EXIT
make build PROFILE=debug
swiftc -suppress-warnings fixtures/media/export-sample.swift -o "$check_dir/sample"
project="$check_dir/sample.splice"
"$check_dir/sample" make "$project"
target/debug/splice export "$project" --preset social-h264
"$check_dir/sample" verify "$project/exports/sample-r0.mp4" avc1
target/debug/splice export "$project" --preset social-hevc --output hevc.mp4
"$check_dir/sample" verify "$project/hevc.mp4" hvc1
for frame in 4 180 448 649; do
    png="$check_dir/frame-$frame.png"
    target/debug/splice frame --at "$frame" "$png" "$project"
    "$check_dir/sample" compare "$project/exports/sample-r0.mp4" "$png" "$frame"
    "$check_dir/sample" compare "$project/hevc.mp4" "$png" "$frame"
done
# Cancel only after the encoder has produced an intermediate progress update.
python3 - "$project" <<'PY'
import json
import subprocess
import sys
import time
from pathlib import Path

project = Path(sys.argv[1])
movie = project / "hevc.mp4"
previous = movie.read_bytes()
rpc_id = 0

with subprocess.Popen(
    ["target/debug/splice", "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True
) as server:
    def rpc(method, params):
        global rpc_id
        rpc_id += 1
        server.stdin.write(json.dumps({"jsonrpc": "2.0", "id": rpc_id, "method": method, "params": params}) + "\n")
        server.stdin.flush()
        response = json.loads(server.stdout.readline())
        assert response["id"] == rpc_id and "result" in response, response
        return response["result"]

    def call(name, **arguments):
        result = rpc("tools/call", {"name": name, "arguments": arguments})
        assert not result["isError"], result
        return result["structuredContent"]

    rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                       "clientInfo": {"name": "export-acceptance", "version": "1"}})
    project_id = call("project.info", path=str(project))["projectId"]
    session_id = call("session.start", projectId=project_id, agent="export-acceptance")["sessionId"]
    job = call("export.start", projectId=project_id, sessionId=session_id, ifRevision="r0",
               preset="social-hevc", path=movie.name, overwrite=True)["jobId"]
    deadline = time.monotonic() + 15
    while True:
        status = call("jobs.status", jobId=job)
        assert status["state"] == "running" and time.monotonic() < deadline, status
        if 0 < status["progress"] < 1:
            progress = status["progress"]
            break
        time.sleep(0.05)
    call("jobs.cancel", jobId=job)
    deadline = time.monotonic() + 5
    while True:
        status = call("jobs.status", jobId=job)
        if status["state"] == "cancelled":
            break
        assert status["state"] == "cancelling" and time.monotonic() < deadline, status
        time.sleep(0.025)
    assert movie.read_bytes() == previous
    assert not list(project.glob(".splice-export-*"))
    server.stdin.close()
    assert server.wait(timeout=5) == 0
    print(f"Cancelled at {progress:.1%}; kept the previous HEVC movie")
PY
