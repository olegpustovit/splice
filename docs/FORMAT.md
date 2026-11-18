# The `.splice` project format · `splice/1`

A project is a **package directory** `Name.splice/`. Finder shows it as one file (UTI
`com.olegpustovit.splice.project`, conforms to `com.apple.package`). Every file in it is UTF-8 JSON
written by `splice_json`, which is canonical: fixed key order, no insignificant whitespace in
`journal.ops`, integers for every time value, shortest round-trip floats. The same project therefore
always serializes to the same bytes, and hashes (FNV-1a 64, PXUI's `hash_of`) are stable.

```
Name.splice/
  project.json          snapshot of the project at revision `rev`
  journal.ops           append-only command log, one batch per line (the source of truth)
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
  A torn last line (crash mid-append) is truncated with a warning. A hash mismatch anywhere else is
  `JOURNAL_CORRUPT`: open read-only and report which revision is the last good one.
- **Lock**: one writer per project. A viewer that has the project open is the writer and serves
  MCP over its socket; `splice mcp` and the CLI forward to it. With no viewer, the first process takes
  the lock. A stale lock (dead pid on the same host) is taken over and logged.

## `project.json`

```json
{
  "format": "splice/1",
  "rev": 41, "hash": "9f2c51e07a3b6d18", "serial": 212,
  "fps": 30, "size": [1080, 1920], "aspect": "9:16", "access": "live",
  "effects": [{"id": "fx_proj_win", "type": "window", "params": {"padding": 0.07, "corner": 0.03}}],
  "spine": {"id": "lan_spine", "name": "Video", "effects": []},
  "lanes": [{"id": "lan_cap", "name": "Captions", "kind": "captions", "effects": []}],
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
  main spec (`{item, edge, offset}`, `{marker, offset}`, `{word, transcriptHash}`); resolved frames are
  never stored, they are derived on load.
- IDs are `<prefix>_<base36 serial>`; `serial` is the next value. Deterministic minting means a
  replay of the journal mints the same IDs.
- Arrays are in model order (spine order, lane stack order, creation order). Unknown keys are an error
  (`FORMAT_UNKNOWN_KEY`) within a major version: strict files keep agents and humans honest.
- Effects carry `{id, type, params}`; params are the effect type's schema, and unset params inherit
  through project › lane › item.

## `journal.ops`

One JSON object per line:

```json
{"rev":42,"parent":"9f2c51e07a3b6d18","hash":"0c11d7e2aa904b35","time":1763459812345,"source":"agent","session":"ses_2x","label":"Agent: Split Screen take at 372","key":"k-81f","ops":[{"cmd":"item.split","item":"clp_take","at":372}],"mints":["clp_2y"]}
{"rev":43,"parent":"0c11d7e2aa904b35","hash":"…","source":"human","label":"Undo agent batch: Split…","revertOf":[42],"skipped":[]}
```

- `parent` is the previous line's `hash`: a hash chain, so corruption and reordering are detectable.
- `ops` are exactly what `edit.apply` received (after `$ref` resolution is recorded in `mints`), so a
  batch replays byte-identically.
- Undo is a line too (`revertOf`, or `restoreTo` for `history.undo {scope:"to"}`). History is never
  rewritten or truncated: an agent can always get back to any revision, including one before an undo.
- `key` is the idempotency key; a retried call with the same key returns the original result.
- Journal compaction (0.3): after a snapshot older than N days, earlier lines move to
  `journal.<rev>.ops.gz`-free archive files (plain, rotated); open never needs them.

## `media.json`

```json
{"format":"splice/1","media":[{"id":"med_take","kind":"take","path":"/Users/…/take.mov",
  "bookmark":"base64…","size":81234567,"mtime":1763450000,
  "probe":{"fps":30,"frames":5400,"width":2880,"height":1800,"audio":["mic","system"],"codec":"hevc"}}]}
```

A missing file is `MEDIA_OFFLINE` with the last known path; `project.info` lists offline media, and
relinking is a command (`media.relink`, 0.3). The core never touches media bytes; it only reads descriptors.

## Transcripts

`transcripts/<med>.json`: `{"hash","language","words":[{"id":"w12","text":"so","from":131,"to":139,"conf":0.93}]}`
in source frames. Caption cues and word anchors refer to word IDs plus the transcript hash, so a
re-transcription that changes words is detected rather than silently mis-anchored.

## Versioning

`format` is `splice/<major>`. Within a major, changes are additive and gated by a fixture. A new major
ships a migration `splice/N → N+1` with golden before/after fixtures; opening a newer major is
`FORMAT_TOO_NEW`. Interchange (FCPXML, OTIO) is export/import, never the native format.
