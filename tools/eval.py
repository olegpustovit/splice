#!/usr/bin/env python3
"""Outcome evals (PLAN.md T3.6). Runs evals/tasks/*.json through a real agent with the splice skill
against `splice mcp`, checks the end state, and appends a row to evals/results.tsv.

    tools/eval.py [--agent claude|codex] [--runs N] [--dry] [task-id ...]

A task file:
    {"id": "...", "platform": "any"|"macos",
     "seed": {"copy": "fixtures/projects/sample.splice"} | {"new": true} | {"record": {...}},
     "assets": {"music.wav": {"tone": 12}},          # optional generated inputs
     "prompt": "... {project} ... {assets} ...",
     "checks": [{"name": "...", "expr": "python expression over the helpers below"}]}

Check helpers: project (`splice snapshot`, journal included), clips, items, lanes, media_kind(id), effect(kind, owner=None),
take_clips(), frame(at) -> Frame with .bands(), exports(), words_on_timeline(), seed_hash, state_hash,
batches (journal batches).
Task files are trusted repository content.
"""
import json, math, os, shutil, struct, subprocess, sys, tempfile, time, wave, zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPLICE = os.environ.get("SPLICE", "splice")


def sh(args, **kw):
    return subprocess.run(args, capture_output=True, text=True, **kw)


def splice(*args):
    out = sh([SPLICE, *args])
    if out.returncode != 0:
        raise RuntimeError(f"splice {' '.join(args)}: {out.stderr.strip()}")
    return json.loads(out.stdout) if out.stdout.strip().startswith("{") else out.stdout


# ── seeds and assets ──

def tone(path, seconds):
    """A cheerful 4-chord loop as 48 kHz stereo WAV, so evals need no downloaded music."""
    rate, chords = 48000, [(261.6, 329.6, 392.0), (220.0, 261.6, 329.6), (174.6, 220.0, 261.6), (196.0, 246.9, 293.7)]
    with wave.open(path, "wb") as w:
        w.setnchannels(2), w.setsampwidth(2), w.setframerate(rate)
        frames = bytearray()
        for i in range(int(seconds * rate)):
            t = i / rate
            chord = chords[int(t * 2) % 4]
            beat = 0.6 + 0.4 * (1 - (t * 4 % 1))
            v = sum(math.sin(2 * math.pi * f * t) for f in chord) / 3 * 0.25 * beat
            s = int(max(-1, min(1, v)) * 32767)
            frames += struct.pack("<hh", s, s)
        w.writeframes(bytes(frames))


def record_seed(project, spec):
    """Records a real window on macOS: opens a text in TextEdit and captures it."""
    # A unique name per run makes the title selector match exactly one window.
    name = f"notes-{os.path.basename(os.path.dirname(project))}.txt"
    text = os.path.join(os.path.dirname(project), name)
    with open(text, "w") as f:
        f.write(spec.get("text", "Splice eval\n\nThings to try:\n- crop\n- zoom\n- music\n"))
    app = spec.get("app", "TextEdit")
    # Start from a known state: earlier runs can leave windows or a hung app behind.
    sh(["killall", app])
    time.sleep(1)
    sh(["open", "-a", app, text])
    time.sleep(2)
    splice("record", project, "--window", app, "--title", name, "--duration", str(spec.get("duration", 6)),
           *(["--mic"] if spec.get("mic") else []))
    sh(["killall", app])


def seed(task, work):
    project = os.path.join(work, "project.splice")
    s = task["seed"]
    if "copy" in s:
        shutil.copytree(os.path.join(ROOT, s["copy"]), project)
    elif "record" in s:
        record_seed(project, s["record"])
    else:
        splice("new", project)
    assets = {}
    for name, spec in task.get("assets", {}).items():
        path = os.path.join(work, name)
        tone(path, spec.get("tone", 10))
        assets[name] = path
    return project, assets


# ── agent ──

def run_agent(agent, prompt, work, code=False):
    started = time.time()
    args = ["mcp", "--code"] if code else ["mcp"]
    MCP = {"mcpServers": {"splice": {"command": SPLICE, "args": args}}}
    if agent == "codex":
        cmd = ["codex", "exec", "--skip-git-repo-check", "-c", f'mcp_servers.splice.command="{SPLICE}"',
               "-c", f"mcp_servers.splice.args={json.dumps(args)}", "--json", prompt]
    else:
        cmd = ["claude", "-p", prompt, "--output-format", "stream-json", "--verbose",
               "--mcp-config", json.dumps(MCP), "--strict-mcp-config",
               "--allowedTools", "mcp__splice", "--permission-mode", "bypassPermissions"]
    out = sh(cmd, cwd=ROOT, timeout=1800)
    with open(os.path.join(work, "agent.jsonl"), "w") as f:
        f.write(out.stdout)
    calls = errors = recoveries = tokens = 0
    failed = {}
    names = {}
    for line in out.stdout.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get("type") == "result":
            usage = event.get("usage") or {}
            tokens = sum(usage.get(k) or 0 for k in ("input_tokens", "output_tokens",
                         "cache_creation_input_tokens", "cache_read_input_tokens"))
        for part in (event.get("message") or {}).get("content") or []:
            if not isinstance(part, dict):
                continue
            if part.get("type") == "tool_use" and "splice" in part.get("name", ""):
                calls += 1
                names[part["id"]] = part["name"]
            if part.get("type") == "tool_result" and part.get("tool_use_id") in names:
                name = names[part["tool_use_id"]]
                if part.get("is_error") or '"isError":true' in json.dumps(part.get("content")):
                    errors += 1
                    failed[name] = True
                elif failed.pop(name, False):
                    recoveries += 1
    return {"calls": calls, "errors": errors, "recoveries": recoveries, "tokens": tokens,
            "wall": round(time.time() - started, 1),
            "exit": out.returncode}


# ── checks ──

class Frame:
    def __init__(self, path):
        self.w, self.h, self.rgba = read_png(path)

    def luma_rows(self, y0, y1):
        total = n = 0
        for y in range(y0, y1):
            row = self.rgba[y * self.w * 4:(y + 1) * self.w * 4]
            for x in range(0, len(row), 16):
                total += 0.2126 * row[x] + 0.7152 * row[x + 1] + 0.0722 * row[x + 2]
                n += 1
        return total / max(n, 1)

    def bands(self):
        """True when the top or bottom 6% of the picture is near black (letterbox bands)."""
        edge = max(1, self.h * 6 // 100)
        return self.luma_rows(0, edge) < 8 or self.luma_rows(self.h - edge, self.h) < 8


def read_png(path):
    data = open(path, "rb").read()
    pos, idat, w = 8, b"", 0
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, color = struct.unpack(">IIBB", body[:10])
            assert depth == 8 and color == 6, "RGBA8 PNG"
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    raw, stride, out, prev = zlib.decompress(idat), w * 4, bytearray(), bytearray(w * 4)
    for y in range(h):
        f, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - 4] if i >= 4 else 0
            b, c = prev[i], prev[i - 4] if i >= 4 else 0
            if f == 1: line[i] = (line[i] + a) & 255
            elif f == 2: line[i] = (line[i] + b) & 255
            elif f == 3: line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        out += line
        prev = line
    return w, h, bytes(out)


def helpers(project_path, work, seed_hash):
    # The journal is the truth; project.json is only a periodic checkpoint.
    project = splice("snapshot", project_path)
    media = {m["id"]: m for m in project.get("media", [])}
    clips, items, lanes = project["clips"], project["items"], project["lanes"]

    def owner_effects(owner):
        if owner == "project":
            return project["effects"]
        if owner == "lan_spine":
            return project["spine"]["effects"]
        for row in lanes + clips + items:
            if row["id"] == owner:
                return row.get("effects", [])
        return []

    def effect(kind, owner=None):
        """Merged params of an enabled effect at any scope (or one owner), else None."""
        owners = [owner] if owner else ["project", "lan_spine"] + [r["id"] for r in lanes + clips + items]
        found = None
        for o in owners:
            for e in owner_effects(o):
                if e["type"] == kind:
                    if e.get("enabled") is False:
                        continue
                    found = dict(found or {}, **e.get("params", {}))
        return found

    def frame(at):
        out = os.path.join(work, f"frame-{at}.png")
        splice("frame", "--at", str(at), out, project_path)
        return Frame(out)

    def words_on_timeline():
        """Transcript word texts whose start is inside a clip of their media."""
        out = []
        folder = os.path.join(project_path, "transcripts")
        for name in sorted(os.listdir(folder)) if os.path.isdir(folder) else []:
            mid = name[:-5]
            for w in json.load(open(os.path.join(folder, name)))["words"]:
                if any(c.get("media") == mid and c["source"] <= w["from"] < c["source"] + c["length"] * c["speed"]
                       for c in clips):
                    out.append(w["text"].lower())
        return out

    state = splice("check", project_path)
    return {
        "project": project, "clips": clips, "items": items, "lanes": lanes, "media": media,
        "media_kind": lambda mid: media.get(mid, {}).get("kind"),
        "take_clips": lambda: [c for c in clips if media.get(c.get("media"), {}).get("kind") == "take"],
        "effect": effect, "frame": frame, "words_on_timeline": words_on_timeline,
        "exports": lambda: sorted(f for f in os.listdir(os.path.join(project_path, "exports"))
                                  if f.endswith(".mp4")) if os.path.isdir(os.path.join(project_path, "exports")) else [],
        "seed_hash": seed_hash, "state_hash": state.get("stateHash"), "batches": state.get("batches", 0),
    }


def run_task(task, agent, dry, code=False):
    work = tempfile.mkdtemp(prefix=f"splice-eval-{task['id']}-")
    project, assets = seed(task, work)
    seed_hash = splice("check", project).get("stateHash")
    prompt = task["prompt"].format(project=project, assets=json.dumps(assets), **assets_by_stem(assets))
    stats = {"calls": 0, "errors": 0, "recoveries": 0, "tokens": 0, "wall": 0, "exit": 0} if dry else run_agent(agent, prompt, work, code)
    env = helpers(project, work, seed_hash)
    results = []
    for check in task["checks"]:
        try:
            # Helpers are globals: generator expressions in a check cannot see eval locals.
            ok = bool(eval(check["expr"], {"__builtins__": __builtins__, **env}))
        except Exception as error:  # a failing probe is a failing check
            ok, check = False, dict(check, error=str(error))
        results.append((check["name"], ok, check.get("error")))
    return work, stats, results


def assets_by_stem(assets):
    return {os.path.splitext(name)[0]: path for name, path in assets.items()}


def main(argv):
    # --code runs the agent against `splice mcp --code` (T14.5 A/B); rows record agent "<agent>+code".
    agent, runs, dry, code, ids = "claude", 1, False, False, []
    it = iter(argv)
    for arg in it:
        if arg == "--agent": agent = next(it)
        elif arg == "--runs": runs = int(next(it))
        elif arg == "--dry": dry = True
        elif arg == "--code": code = True
        else: ids.append(arg)
    folder = os.path.join(ROOT, "evals", "tasks")
    tasks = [json.load(open(os.path.join(folder, f))) for f in sorted(os.listdir(folder)) if f.endswith(".json")]
    macos = sys.platform == "darwin"
    tasks = [t for t in tasks if (not ids or t["id"] in ids) and (t.get("platform", "any") == "any" or macos)]
    # The Mac tree is rsynced without .git; `make mac-run` callers pass SPLICE_COMMIT.
    commit = os.environ.get("SPLICE_COMMIT") or sh(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT).stdout.strip()
    failed = 0
    for task in tasks:
        for run in range(runs):
            work, stats, results = run_task(task, agent, dry, code)
            passed = all(ok for _, ok, _ in results)
            failed += not passed
            with open(os.path.join(work, "result.json"), "w") as f:
                json.dump({"task": task["id"], "agent": agent, "code": code, "commit": commit, "passed": passed,
                           "checks": [{"name": n, "ok": ok, "error": e} for n, ok, e in results], **stats}, f, indent=1)
            for name, ok, error in results:
                print(f"  {'✓' if ok else '✗'} {task['id']}: {name}" + (f" ({error})" if error else ""))
            print(f"{'PASS' if passed else 'FAIL'} {task['id']} run {run + 1}: {sum(ok for _, ok, _ in results)}/{len(results)} checks, "
                  f"{stats['calls']} calls, {stats['errors']} errors, {stats['recoveries']} recoveries, {stats['wall']} s ({work})")
            if not dry:
                row = [time.strftime("%Y-%m-%d"), commit, agent + ("+code" if code else ""), task["id"], "pass" if passed else "fail",
                       f"{sum(ok for _, ok, _ in results)}/{len(results)}", str(stats["calls"]), str(stats["errors"]),
                       str(stats["recoveries"]), str(stats["wall"]), str(stats["tokens"])]
                path = os.path.join(ROOT, "evals", "results.tsv")
                new = not os.path.exists(path)
                with open(path, "a") as f:
                    if new:
                        f.write("date\tcommit\tagent\ttask\tresult\tchecks\tcalls\terrors\trecoveries\twall_s\ttokens\n")
                    f.write("\t".join(row) + "\n")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
