#!/usr/bin/env python3
"""Transcription and analysis speed on the Mac (PLAN.md T24.3), through the installed `splice mcp`.

Synthesizes about a minute of speech with `say`, inserts it, then times `transcript.get
{transcribe:true}` (budget >= 10x realtime) and `media.analyze` over the audio kinds (>= 5x).
Prints one JSON line; exits non-zero when a budget fails.
"""
import json, os, subprocess, sys, tempfile, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from importlib import import_module

record_check = import_module("record-check")

TEXT = " ".join([
    "Today we look at how the editor handles a long recording from start to finish.",
    "First we open the project, then we trim the pauses and tighten the opening.",
    "Next we add captions, check the frames, and export a vertical version for social.",
    "Every edit lands as one undoable step, so trying an idea costs almost nothing.",
] * 4)


def main():
    folder = tempfile.mkdtemp(prefix="splice-speed-")
    speech = os.path.join(folder, "speech.m4a")
    subprocess.run(["say", "-o", speech, "--file-format=m4af", TEXT], check=True)
    # Transcription works on spine clips, so the speech goes under a looped video fixture.
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    movie = os.path.join(folder, "speech.mov")
    subprocess.run(["swift", os.path.join(root, "tools/speech-movie.swift"),
                    os.path.join(root, "fixtures/media/probe.mov"), speech, movie], check=True)
    server = record_check.Server()
    info = server.call("project.info", path=os.path.join(folder, "speed.splice"), create=True)
    server.project = info["projectId"]
    session = server.call("session.start", projectId=server.project)["sessionId"]
    edit = server.call("edit.apply", projectId=server.project, sessionId=session, ifRevision=info["revision"],
                       ops=[{"cmd": "clip.insert", "path": movie}])
    snapshot = server.call("timeline.snapshot", projectId=server.project)
    fps = server.call("project.info", projectId=server.project)["fps"]
    item = snapshot["spine"][0]
    seconds = (item["to"] - item["from"]) / fps
    target = {"clip": item["id"]}
    began = time.time()
    words = server.call("transcript.get", projectId=server.project, sessionId=session, transcribe=True, **target)
    transcribe = time.time() - began
    began = time.time()
    server.call("media.analyze", projectId=server.project, kinds=["loudness", "peaks", "beats", "sounds"], **target)
    analyze = time.time() - began
    count = len(words.get("words", []))
    server.proc.stdin.close()
    server.proc.wait(timeout=60)
    result = {"check": "speed", "seconds": round(seconds, 1), "words": count,
              "transcribeX": round(seconds / transcribe, 1), "analyzeX": round(seconds / analyze, 1),
              "revision": edit.get("revision")}
    ok = count > 50 and seconds / transcribe >= 10 and seconds / analyze >= 5
    print(json.dumps(dict(result, passed=ok)))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
