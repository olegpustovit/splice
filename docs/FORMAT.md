# The `.splice` project format · `splice/1`

A project is a **package directory** `Name.splice/`. Finder shows it as one file (UTI
`com.olegpustovit.splice.project`, conforms to `com.apple.package`). Every file in it is UTF-8 JSON
written by `splice_json`, which is canonical: fixed key order, no insignificant whitespace in
`journal.ops`, integers for every time value, shortest round-trip floats. The same project therefore
always serializes to the same bytes, and hashes (FNV-1a 64, PXUI's `hash_of`) are stable.
Schema encoders supply keys in the order shown below. Integer fields keep their exact decimal
spelling. The decoder reports invalid JSON with a byte offset and path and rejects duplicate keys.

```
Name.splice/
  project.json          snapshot of the project at revision `rev`
  journal.ops           append-only command log, one batch per line (the source of truth)
  journal.base.json     original snapshot, media table and transcripts for retained history
  sessions.json         session IDs, agent names, checkpoints, start times and open status
  media.json            media table: med_ id → location and probe
  transcripts/<med>.json  words for a media item, with its hash
  takes/<med>/          a recorded take, in Recorder's layout (recordings live in the package; imports stay where they are):
    screen.mov          HEVC, cursor-less, variable frame rate
    mic.m4a  system.m4a camera.mov   optional, one file per source, AAC 48 kHz / HEVC
    events.json         {"t0","source":{kind,rect,scale},"events":[{t,k:move|down|up|drag|scroll|key|flags|cursor,…}]}, t in seconds since t0, positions normalized 0…1
    cursors/<hash>.png  + <hash>.json {hotX, hotY, scale}
  exports/              rendered output; never overwritten unless asked
  friction.jsonl        MCP friction log for this project (docs/LOOP.md)
  cache/                disposable: thumbnails, peaks, proxies. Never in git, never required
  lock                  single-writer lock: flock + {"pid","host","since"}
```

## Durability

- **Commit** = append one line to `journal.ops` + `fsync`. That is when a revision exists. A batch is
  never half-applied: `apply` validates the whole batch in memory first.
- **Snapshot** = `project.json` written as `project.json.tmp`, `fsync`, `rename`, `fsync(dir)`. Every
  64 batches, on close, and on `splice snapshot`. The snapshot records the `rev` and `hash` it is at.
- **Open** = read the snapshot, replay journal lines after its `rev`, verify each line's `hash`.
  A final record without its newline is truncated with a warning. A corrupt complete line opens
  read-only with `JOURNAL_CORRUPT` and the last verified revision. If corruption precedes the
  checkpoint, replay starts at `journal.base.json`.
- **Lock**: one writer per project. A viewer that has the project open is the writer and serves
  MCP over its socket; `splice mcp` and the CLI forward to it. With no viewer, the first process takes
  the lock. The owner record stores the PID, hostname and Unix time in seconds. Clean close clears
  the record with the lock held. An owner record left behind after the kernel releases the lock
  is logged before takeover.

## Headless CLI

- `splice new Name.splice` creates a package. It reports the canonical path and initial revision.
- `splice apply ops.json Name.splice` applies one batch as a local human edit. The file contains a
  JSON array of operations or an object with only an `ops` array. Use `-` as the operations
  filename to read stdin. `--dry-run` previews the batch, `--if-revision r12` guards a known
  revision, and `--idempotency-key` makes a retry return the original receipt.
- `splice snapshot Name.splice` checkpoints the journal tip and prints the canonical timeline
  snapshot. `splice check Name.splice` verifies the journal and checkpoint and reports the tip's
  content hash. A valid checkpoint may lag the journal tip.
- `splice describe item.trim` shows one command. Use `splice describe --query trim` to search.

`apply`, `snapshot`, and `check` can infer the project from the enclosing `.splice` directory,
or take a path with `--project`. Structured successes go to stdout. Errors carry a code, message,
and fix on stderr and exit nonzero. On Linux, `open`, `export`, and `frame` report
`UNSUPPORTED_PLATFORM`.

## `project.json`

```json
{
  "format": "splice/1",
  "rev": 41, "hash": "9f2c51e07a3b6d18", "serial": 212,
  "fps": 30, "size": [1080, 1920], "aspect": "9:16", "access": "live",
  "effects": [{"id": "fx_projwin", "type": "window", "params": {"padding": 0.07, "corner": 0.03}}],
  "spine": {"id": "lan_spine", "name": "Video", "effects": []},
  "lanes": [{"id": "lan_gfx", "name": "Graphics", "kind": "graphics", "effects": []}],
  "clips": [{"id": "clp_take", "media": "med_take", "start": 0, "length": 420, "source": 120,
             "speed": 1, "fadeIn": 0, "fadeOut": 0, "volume": {"mic": 1, "system": 0.8},
             "enabled": {"mic": true, "cursor": true}, "crop": null, "fit": "fit", "effects": []}],
  "items": [{"id": "ovl_title", "kind": "title", "lane": "lan_gfx",
             "anchor": {"item": "clp_take", "edge": "start", "offset": 12}, "length": 60,
             "params": {"text": "Field notes"}, "effects": []}],
  "markers": [{"id": "mrk_2", "at": 372, "name": "Hook"}]
}
```

Rules:
- Times are integer frames at `fps`. Clip `source` is in source frames. Anchors are the forms of the
  main spec (`{item, edge, offset}`, `{marker, offset}`, `{word, transcriptHash}`), plus fixed frames
  (`{pinned:true, frame}`) and clip-source frames (`{clip, sourceFrame}`). Resolved frames are derived
  on load. A gap has `media:null`.
- IDs are `<prefix>_<base36 serial>`; `serial` is the next value. Deterministic minting means a
  replay of the journal mints the same IDs.
- Arrays are in model order (spine order, lane stack order, creation order). Unknown keys are an error
  (`FORMAT_UNKNOWN_KEY`) within a major version: strict files keep agents and humans honest.
- Effects carry `{id, type, params}`; params are the effect type's schema, and unset params inherit
  through project › lane › item. Parameter keys are written in lexical order.
- Markers may carry an optional `color` swatch. `project.set` keeps the canvas's shorter edge
  when it changes the aspect.

## `journal.ops`

One JSON object per line:

```json
{"rev":42,"parent":"9f2c51e07a3b6d18","hash":"0c11d7e2aa904b35","time":1763459812345,"source":"agent","session":"ses_2x","label":"Agent: Split Screen take at 372","key":"k-81f","ops":[{"cmd":"item.split","item":"clp_take","at":372}],"mints":["clp_2y"]}
{"rev":43,"parent":"0c11d7e2aa904b35","hash":"…","source":"human","label":"Undo agent batch: Split…","revertOf":[42],"skipped":[]}
```

- `parent` is the previous line's `hash`, or the origin hash on the first line. `hash` is FNV-1a 64 of
  the canonical record with its `hash` field and trailing newline omitted. Readers verify the
  revision sequence, links, and record bytes.
- `ops` are exactly what `edit.apply` received (after `$ref` resolution is recorded in `mints`), so a
  batch replays byte-identically.
- Undo appends `revertOf` and `skipped`, or records a `restoreTo` revision. Human undo and redo
  also store `history:"undo"` or `history:"redo"` so the stacks survive reopening. Prior revisions
  stay addressable, including the state before any undo.
- `key` is scoped to the source and session. Repeating the same operations with the same key
  returns their original result. Different operations with a used key return `CONFLICT`.
- Restoring a revision reproduces its project content and gives the restore its own journal stamp.
  The allocator reserves IDs issued anywhere in retained history, including revisions after the
  restore target.
- Journal compaction (0.3): after a snapshot older than N days, earlier lines move to
  `journal.<rev>.ops.gz`-free archive files (plain, rotated); open never needs them.

## `media.json`

```json
{"format":"splice/1","media":[{"id":"med_take","kind":"take","path":"/Users/…/take.mov",
  "bookmark":"base64…","size":81234567,"mtime":1763450000,
  "probe":{"fps":30,"frames":5400,"width":2880,"height":1800,"audio":["mic","system"],"codec":"hevc"}}]}
```

Audio entries are track names or `{name, channels}` when the probe has a channel count. Take
descriptors can include a `components` object with paths relative to the take directory.

A missing file is `MEDIA_OFFLINE` with the last known path; `project.info` lists offline media, and
relinking is a command (`media.relink`, 0.3). The core never touches media bytes; it only reads descriptors.

## Transcripts

`transcripts/<med>.json`: `{"hash","language","words":[{"id":"w12","text":"so","from":131,"to":139,"conf":0.93}]}`
in source frames. Caption cues and word anchors refer to word IDs plus the transcript hash, so a
re-transcription that changes words is detected rather than silently mis-anchored.

Session metadata is versioned independently of the timeline with `format:"splice/1"`. Starting or
closing a session writes that metadata atomically. It does not advance the revision.

The headless sample under `fixtures/projects/sample.splice` uses the prototype's timeline and
transcript with descriptor-only media. The native media acceptance tests in T4 need playable assets.

## Playback planning

The evaluator prepares one immutable project snapshot and reuses a frame buffer during playback.
It resolves effects per parameter from project to spine and clip, from overlay lane to overlay,
and from project to captions lane and cue. Source geometry uses the media probe dimensions.
The frame plan carries ordered visual layers, audio gains, and caption word timing. Cursor,
keystroke and click-zoom layers also carry the source clock for the capture event renderer.

## Versioning

`format` is `splice/<major>`. Within a major, changes are additive and gated by a fixture. A new major
ships a migration `splice/N → N+1` with golden before/after fixtures; opening a newer major is
`FORMAT_TOO_NEW`. Interchange (FCPXML, OTIO) is export/import, never the native format.
