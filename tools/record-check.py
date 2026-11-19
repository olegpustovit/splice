#!/usr/bin/env python3
"""Live recording checks on the Mac (PLAN.md T22.1, T22.6), through the installed `splice mcp`.

    tools/record-check.py            pause/resume: 1.5 s, pause 1.5 s, resume with a marker, 1.5 s
    tools/record-check.py --soak N   one N-second display take: dropped frames, drift, memory

Prints one JSON line of measurements and exits non-zero when an acceptance check fails.
"""
import json, os, subprocess, sys, tempfile, time

SPLICE = os.environ.get("SPLICE", os.path.expanduser("~/.local/bin/splice"))


class Server:
    def __init__(self):
        self.proc = subprocess.Popen([SPLICE, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.next = 0
        self.project = None
        self.send("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                 "clientInfo": {"name": "record-check", "version": "1"}})

    def send(self, method, params):
        self.next += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        while True:
            reply = json.loads(self.proc.stdout.readline())
            if reply.get("id") == self.next:
                return reply.get("result", reply)

    def call(self, name, **args):
        result = self.send("tools/call", {"name": name, "arguments": args})
        content = result.get("structuredContent", result)
        if "error" in content:
            raise SystemExit(f"{name}: {json.dumps(content['error'])}")
        return content

    def wait(self, job, until):
        while True:
            status = self.call("jobs.status", projectId=self.project, jobId=job)
            if until(status):
                return status
            time.sleep(0.2)


def rss(pid):
    out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return int(out or 0) * 1024


def pause_resume(server, path):
    start = server.call("record.start", path=path, create=True, source={"display": "main"}, events=False)
    server.project, job = start["projectId"], start["jobId"]
    server.wait(job, lambda s: s.get("state") == "recording")
    time.sleep(1.5)
    server.call("record.stop", projectId=server.project, jobId=job, pause=True)
    time.sleep(1.5)
    server.call("record.start", projectId=server.project, resume=job, marker="Second part")
    time.sleep(1.5)
    stop = server.call("record.stop", projectId=server.project, jobId=job)
    snapshot = server.call("timeline.snapshot", projectId=server.project)
    fps = server.call("project.info", projectId=server.project)["fps"]
    length = sum(c["length"] for c in snapshot["spine"]) / fps
    markers = [m["name"] for m in snapshot.get("markers", [])]
    result = {"check": "pause-resume", "seconds": round(length, 2), "markers": markers, "revision": stop.get("revision")}
    # 3 s recorded around a 1.5 s pause: the pause is not in the take.
    ok = 2.4 <= length <= 3.9 and "Second part" in markers
    return result, ok


def soak(server, path, seconds):
    start = server.call("record.start", path=path, create=True, source={"display": "main"}, events=True,
                        duration=seconds)
    server.project, job = start["projectId"], start["jobId"]
    server.wait(job, lambda s: s.get("state") == "recording")
    samples = []
    began = time.time()
    while time.time() - began < seconds - 5:
        samples.append(rss(server.proc.pid))
        time.sleep(30)
    status = server.wait(job, lambda s: s.get("state") in ("ready", "done"))
    stop = server.call("record.stop", projectId=server.project, jobId=job)
    snapshot = server.call("timeline.snapshot", projectId=server.project)
    fps = server.call("project.info", projectId=server.project)["fps"]
    frames = sum(c["length"] for c in snapshot["spine"])
    drift = abs(frames - seconds * fps)
    growth = (max(samples[1:] or [0]) - samples[1]) if len(samples) > 1 else 0
    result = {"check": "soak", "seconds": seconds, "frames": frames, "driftFrames": drift,
              "dropped": status.get("droppedFrames"), "rssStartMB": samples[1] // 2**20 if len(samples) > 1 else None,
              "rssGrowthMB": growth // 2**20, "revision": stop.get("revision")}
    ok = (status.get("droppedFrames") or 0) == 0 and drift <= 1 and growth < 64 * 2**20
    return result, ok


def main(argv):
    folder = tempfile.mkdtemp(prefix="splice-record-check-")
    server = Server()
    if argv[:1] == ["--soak"]:
        result, ok = soak(server, os.path.join(folder, "soak.splice"), int(argv[1]))
    else:
        result, ok = pause_resume(server, os.path.join(folder, "pause.splice"))
    server.proc.stdin.close()
    server.proc.wait(timeout=60)
    print(json.dumps(dict(result, passed=ok)))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
