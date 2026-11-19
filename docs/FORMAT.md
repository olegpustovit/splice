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
    events.json         {"t0","source":{kind,rect,scale},"events":[{t,k:move|down|up|drag|scroll|key|flags|cursor,…}],"dropped"?:count}
    cursors/<hash>.png  + <hash>.json {hotX, hotY, scale}
  exports/              rendered output; never overwritten unless asked
  friction.jsonl        MCP friction log for this project (docs/LOOP.md)
  cache/                disposable: thumbnails, peaks, proxies. Never in git, never required
  lock                  single-writer lock: flock + {"pid","host","since"}
```

For a recorded take, `t0` is the host-clock time of the first complete screen frame in seconds.
Event `t` values are seconds from that frame with pauses removed. Positions are normalized to
0…1 within the source rectangle. `dropped` counts input events lost if the native queue fills.
A cursor event references a SHA-256 digest of the PNG, hotspot and scale. The hotspot is in
PNG pixels, and `scale` is pixels per point.

## Durability

- **Commit** = append one line to `journal.ops` + `fsync`. That is when a revision exists. A batch is
  never half-applied: `apply` validates the whole batch in memory first.
- **Snapshot** = `project.json` written as `project.json.tmp`, `fsync`, `rename`, `fsync(dir)`. Every
  64 batches, on close, and on `splice snapshot`. The snapshot records the `rev` and `hash` it is at.
  When a checkpoint changes the media table, `media.previous.json` preserves the old table
  until the new snapshot is published.
- **Open** = read the snapshot, replay journal lines after its `rev`, verify each line's `hash`.
  A final record without its newline is truncated with a warning. A corrupt complete line opens
  read-only with `JOURNAL_CORRUPT` and the last verified revision. If corruption precedes the
  checkpoint, replay starts at `journal.base.json`.
- **Lock**: one writer per project. A viewer that has the project open is the writer. Its
  same-user Unix socket lives in the `live` subdirectory of the Splice state directory.
  `splice mcp` and project CLI commands forward to that writer. With no viewer, the first process takes
  the lock. The owner record stores the PID, hostname and Unix time in seconds. Clean close clears
  the record with the lock held. An owner record left behind after the kernel releases the lock
  is logged before takeover.

## Headless CLI

- On macOS, `splice record project.splice --display main --duration 3 --mic` records a take and
  appends it as one undoable edit. A new path creates a project. An existing path uses its current
  writer, including a viewer. Omit `--duration` in an interactive terminal to record until Return.
  Control-D discards an interactive recording. Use `--still` for a PNG, `--no-insert` to keep the
  media off the timeline, or `--no-events` to omit cursor and input capture. `splice record --dry-run`
  reports sources and grant status without opening or creating a project.
- Use `--window win_<id>` or `--window <app> [--title <title>]` for a window. The app selector
  needs a unique match. `--area <display> <x> <y> <width> <height>` takes display-local points.
  `--source '<json>'` accepts the same selectors as `record.start`. `--countdown` sets a whole
  number of seconds before capture. `--system-audio` and `--camera` enable those tracks.

- On macOS, `splice open Name.splice` opens the viewer. Run `splice` or `splice open`
  to choose a project in the native open panel. The panel starts in the last project's directory
  when that project is available.
- In the viewer, press Space to play or pause and Left or Right to step one frame. The scrubber
  previews while dragged and returns to its starting frame if capture is cancelled. The clock shows
  hours, minutes, seconds, and frames. Command-Z and Command-Shift-Z undo and redo the latest batch
  through the project journal, including batches committed by an agent.
- `splice open Name.splice --snapshot out.png [--at N]` saves the viewer content at frame `N`.
  The default is the live viewer's playhead, or frame zero when the project is closed. The PNG
  uses the viewer's default 960 × 640 size at 1×. Pass a new filename. The command renders
  offscreen and prints the frame, project revision, and output path.
- `splice new Name.splice` creates a package. It reports the canonical path and initial revision.
- `splice apply ops.json Name.splice` applies one batch as a local human edit. The file contains a
  JSON array of operations or an object with only an `ops` array. Use `-` as the operations
  filename to read stdin. `--dry-run` previews the batch, `--if-revision r12` guards a known
  revision, and `--idempotency-key` makes a retry return the original receipt.
- `splice snapshot Name.splice` checkpoints the journal tip and prints the canonical timeline
  snapshot. `splice check Name.splice` verifies the journal and checkpoint and reports the tip's
  content hash. A valid checkpoint may lag the journal tip.
- `splice describe item.trim` shows one command. Use `splice describe --query trim` to search.
- `splice frame --at 30 out.png Name.splice` writes a full-size PNG of an exact timeline frame.
  Choose an unused output filename.

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
- The timeline is as long as the spine: every item ends at or before the end of the last clip
  (0.4; edits trim or remove items the spine no longer covers). Every clip and item is at least
  three frames long.
- IDs are `<prefix>_<base36 serial>`; `serial` is the next value. Deterministic minting means a
  replay of the journal mints the same IDs.
- Arrays are in model order (spine order, lane stack order, creation order). Unknown keys are an error
  (`FORMAT_UNKNOWN_KEY`) within a major version: strict files keep agents and humans honest.
- Effects carry `{id, type, params}`; params are the effect type's schema, and unset params inherit
  through project › lane › item. Parameter keys are written in lexical order.
- Markers may carry an optional `color` swatch. `project.set` keeps the canvas's shorter edge
  when it changes the aspect.

### 0.3 additions (additive within `splice/1`)

- **Keys.** A numeric effect parameter marked `"keys": true` in `effects.json` may be keyed (0.4:
  the others refuse keys with `NOT_KEYABLE`, see below):
  `{"keys":[{"at":0,"value":1,"ease":"inOut"},{"at":60,"value":1.4}]}`. `at` is a frame relative
  to the owner's start (it may be negative), strictly increasing. `ease` is the curve toward the
  next key: `linear`, `hold`, `in`, `out`, `inOut` or a cubic Bezier `[x1, y1, x2, y2]`. Files
  always store it (`linear` when absent on read); commands default an omitted ease to `inOut`. Splitting, in-trimming or rolling an owner shifts its keys so they stay on
  the same content.
- **Effects** cover the look as well as the frame: `transform`, `crop` (keyed rect), `color`
  (with an optional `.cube` `lut` path), `mask` (several per owner), `key`, `cutout`,
  `transition` (on the incoming spine clip), `speed` (keyed `rate`; 0 is a freeze), `volume`, `eq`,
  `compressor`, `gate`, `voiceIsolation`, `text`, `animate`, `shape`, `template` (groups the items
  a template made) and `brand` (project style). `commands/effects.json` is the schema; every
  writer and the validator check params against it.
- **Shapes.** An item `{"kind":"shape","params":{"shape":"arrow","rect":{"x","y","w","h"}}}` on a
  graphics lane; `shape` is `solid`, `gradient`, `rect`, `ellipse`, `arrow`, `line` or
  `callout`. Its fill, stroke and corner radius are a `shape` effect.
- **Stills.** Image media (`kind:"image"`) have no duration of their own: `probe.frames` is a
  large sentinel and a clip of an image may have any length (5 s when inserted).
- **Joins.** A clip made by `clip.join` carries `"joins":[frame, …]`, the offsets of the former
  cuts, so `clip.ungroup` can split it back.
- **Compounds.** `"compounds":[{"media":"med_…","clips":[…],"items":[…],"lanes":[…]}]` holds the
  nested timelines of `clip.compound`; the compound's media entry has `kind:"compound"` and the
  outer clip plays it like any media. `clip.compound` refuses nesting past 4 levels (`TOO_DEEP`);
  an item that runs past the compounded clips stays outside, anchored to the compound clip.

### 0.4 changes (additive within `splice/1`, migrated on load)

- **Camera lane and zoom items (D12, D13).** Lane kind `camera` (0.3 `motion` loads as `camera`)
  holds items `{"kind":"zoom","params":{"amount":1.5,"focus":{"x":0.7,"y":0.3},"mode":"auto",
  "spring":"focused"}}`. `amount` is 1–5; `focus` is a content point (0–1) the viewport centers
  on, clamped so the viewport never leaves the content (omit to keep the current framing);
  `mode` is `auto` (content on portrait canvases, else frame), `content` (zoom inside a fixed
  frame) or `frame` (the card scales); `spring` is `focused`, `smooth`, `snappy` or `instant`.
  One camera per timeline: the evaluator solves an analytic spring per channel from these
  segments, holding the center outside them. 0.3 `motion` items migrate to zoom items
  (`amount = max(to, 1)`, same focus) and the 0.3 `clickZoom` effect is dropped
  (`intent.autoZoom` writes zoom items instead).
- **Keys only where they animate (T36.4).** A 0.3 file with keys on a parameter that is not
  keyable loads with the first key's value (0.3 rendered the default there).
- **Free-text caption cues (T41.5).** A caption item's `params` may carry `"text"` after
  `words` and `edits`: `{"words":[],"edits":[],"text":"Open the settings"}` is a cue that needs no
  transcript. It keeps the item's own anchor and length (settle never retimes it), and `words`
  must then be empty. Files without `text` read as before.
- **Transitions consume handles (D14).** A `transition` effect belongs to a spine clip right after
  another; its length is at most the shorter clip and needs `length/2` unused source frames before
  the incoming clip and the rest after the outgoing one. Writers refuse anything else
  (`WRONG_KIND`, `NO_NEIGHBOR`, `TOO_LONG`, `NO_HANDLES`). Files from 0.3 without handles still
  play: edge frames hold.

- **Declared pointer marks (T39.4).** A take's `components` may carry
  `"pointer":[{"t":2.0,"x":0.6,"y":0.4,"click":true}]` (source seconds, positions normalized to the
  captured source, time order), written by `take.pointer`. They merge with the recorded cursor
  samples, and the renderer synthesizes a human path to each one.

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
- Path imports record an `imports` array of `{request, descriptor}` entries alongside the original
  operations. The descriptor uses the `media.json` shape and lets the batch replay offline.
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

On macOS, `clip.insert {path}` accepts an absolute path or one relative to the project package.
Set `insert:false` with `path` to add only the media descriptor. A recorded take's primary
file is `takes/<med>/screen.mov`. The project saves the descriptor and optional clip in a single
journal batch. The take files remain available after undo for history restores.
New audio and video descriptors include a base64-encoded Foundation file bookmark. Paths
inside the package are stored relative to it. A successful checkpoint writes descriptors to
`media.json`. Portable clients can insert paths already present in that table.

A missing file is `MEDIA_OFFLINE` with the last known path; `project.info` lists offline media, and
relinking is a command (`media.relink`, 0.3). The core never touches media bytes; it only reads descriptors.

## Transcripts

`transcripts/<med>.json`: `{"hash","language","words":[{"id":"w:screen:12","text":"so","from":131,"to":139,"conf":0.93}]}`
in source frames. Caption cues and word anchors refer to word IDs plus the transcript hash, so a
re-transcription that changes words is detected rather than silently mis-anchored.

Word IDs are project-unique (0.4): `w:<media>:<n>` is the `n`th word (from 1) of the transcript of
`med_<media>`, so `w:screen:12` lives in `transcripts/med_screen.json`. A media item that plays in
several clips (a split or duplicated take) shows the same word in each; a caption cue resolves its
words in the clip it is anchored to first. Files written before 0.4 use `w<n>`: loading binds each
to its transcript's media (a caption follows its clip's media), and the next save writes the new
form. In ops, a legacy `w<n>` still works when one transcript has that number; a journal replays
it with its old meaning, and a live call that several transcripts could mean is `AMBIGUOUS_WORD`.
`captions.generate` starts a new cue at every clip boundary.

An item placed at a word (`anchor:{word, transcriptHash}`) is stored as the clip-source frame
under that word, so cutting the word or re-transcribing never strands it; files that saved a word
anchor are converted the same way on load. Edits measure a clip's source through its `speed`
effect, so a freeze (`rate` 0) can be trimmed, split and retimed anywhere in its media.

Session metadata is versioned independently of the timeline with `format:"splice/1"`. Starting or
closing a session writes that metadata atomically. It does not advance the revision.

The headless sample under `fixtures/projects/sample.splice` uses the prototype's timeline and
transcript with descriptor-only media. The native media acceptance tests in T4 need playable assets.

## Playback planning

The evaluator prepares one immutable project snapshot and reuses a frame buffer during playback.
It resolves effects per parameter from project to spine and clip, from overlay lane to overlay,
and from project to captions lane and cue. Source geometry uses the media probe dimensions.
The frame plan carries ordered visual layers, audio gains, caption word timing, the camera view
of each picture, and at most one *group*: the layers of a transition's incoming side or a fading
clip, composited as one flattened image with an opacity, scale, offset and clip (D14), plus an
optional rigid *shift* of the outgoing side. Cursor and keystroke layers carry the source clock
for the capture event renderer.

On macOS, `splice_av` divides that plan where the layer stack changes and maps each layer to a
composition track (lowering only maps sources). The GPU compositor draws every frame from the
evaluated plan itself: bounds, opacity, look, camera, window and cursor come from the evaluator at
that exact frame, never interpolated. A group renders into a pooled transparent canvas and is
composited once, so a window and its backdrop fade together and the backdrop is drawn once.
Frame reads and MP4 encoding share those tracks and instructions. Export and frame grabs use the same title and caption
bitmaps and opacity ramps. The viewer samples the same video composition through
`AVPlayerItemVideoOutput`, displays the image at the project's aspect, and paints the resolved
titles and captions with PXUI. The color conversion writes directly into three reusable shared
Metal textures. A texture becomes writable again when its last draw completes.

## Versioning

`format` is `splice/<major>`. Within a major, changes are additive and gated by a fixture. A new major
ships a migration `splice/N → N+1` with golden before/after fixtures; opening a newer major is
`FORMAT_TOO_NEW`. Interchange (FCPXML, OTIO) is export/import, never the native format.
