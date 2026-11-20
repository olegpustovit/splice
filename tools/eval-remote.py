#!/usr/bin/env python3
"""Runs an eval task's agent here against `splice mcp` on another Mac over ssh, for tasks that
need hardware the eval Mac lacks (voiceover: a microphone).

    tools/eval-remote.py user@host [--runs N] [--code] task-id ...

The remote Mac needs an installed, permission-granted Splice at ~/.local/bin/splice and this
repository's fixtures under ~/projects/editor-verify (make mac-sync MAC=… MAC_DIR=projects/editor-verify).
Supports tasks seeded by {"copy": …} or {"new": true} whose checks read the project snapshot
(project, clips, items, lanes, media, take_clips, seconds). Prints one line per run.
"""
import json, subprocess, sys, time, uuid

REPO = "~/projects/editor-verify"
SPLICE = "~/.local/bin/splice"


def ssh(host, command, **kw):
    return subprocess.run(["ssh", host, f"bash -lc '{command}'"], capture_output=True, text=True, **kw)


def run(host, task, code):
    work = f"/tmp/splice-remote-eval-{task['id']}-{uuid.uuid4().hex[:8]}"
    project = f"{work}/project.splice"
    seed = task["seed"]
    if "copy" in seed:
        ssh(host, f"mkdir -p {work} && cp -R {REPO}/{seed['copy']} {project}", check=True)
    else:
        ssh(host, f"mkdir -p {work} && {SPLICE} new {project}", check=True)
    prompt = task["prompt"].format(project=project)
    args = ["-o", "BatchMode=yes", host, f"{SPLICE} mcp" + (" --code" if code else "")]
    mcp = {"mcpServers": {"splice": {"command": "ssh", "args": args}}}
    started = time.time()
    out = subprocess.run(["claude", "-p", prompt, "--output-format", "stream-json", "--verbose",
                          "--mcp-config", json.dumps(mcp), "--strict-mcp-config",
                          "--allowedTools", "mcp__splice", "--permission-mode", "bypassPermissions"],
                         capture_output=True, text=True, timeout=1800, stdin=subprocess.DEVNULL)
    calls = errors = 0
    for line in out.stdout.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        for part in (event.get("message") or {}).get("content") or []:
            if isinstance(part, dict) and part.get("type") == "tool_use" and "splice" in part.get("name", ""):
                calls += 1
            if isinstance(part, dict) and part.get("type") == "tool_result" and (
                    part.get("is_error") or '"isError":true' in json.dumps(part.get("content"))):
                errors += 1
    snapshot = json.loads(ssh(host, f"{SPLICE} snapshot {project}", check=True).stdout)
    media = {m["id"]: m for m in snapshot.get("media", [])}
    env = {
        "project": snapshot, "clips": snapshot["clips"], "items": snapshot["items"],
        "lanes": snapshot["lanes"], "media": media,
        "take_clips": lambda: [c for c in snapshot["clips"] if media.get(c.get("media"), {}).get("kind") == "take"],
        "seconds": lambda frames: frames / snapshot["fps"],
    }
    results = []
    for check in task["checks"]:
        try:
            ok = bool(eval(check["expr"], {"__builtins__": __builtins__, **env}))
        except Exception as error:  # a failing probe is a failing check
            ok = False
            check = dict(check, error=str(error))
        results.append((check["name"], ok))
    ssh(host, f"rm -rf {work}")
    return results, calls, errors, round(time.time() - started, 1)


def main(argv):
    host, runs, code, ids = argv[0], 1, False, []
    it = iter(argv[1:])
    for arg in it:
        if arg == "--runs": runs = int(next(it))
        elif arg == "--code": code = True
        else: ids.append(arg)
    failed = 0
    for task_id in ids:
        task = json.load(open(f"evals/tasks/{task_id}.json"))
        for n in range(runs):
            results, calls, errors, wall = run(host, task, code)
            passed = all(ok for _, ok in results)
            failed += not passed
            for name, ok in results:
                if not ok:
                    print(f"  ✗ {task_id}: {name}")
            print(f"{'PASS' if passed else 'FAIL'} {task_id}{' (code)' if code else ''} run {n + 1} on {host}: "
                  f"{sum(ok for _, ok in results)}/{len(results)} checks, {calls} calls, {errors} errors, {wall} s")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
