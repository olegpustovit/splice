---
name: splice
description: Edit, export and record video projects with Splice over its MCP server (`splice mcp`). Use whenever a task involves a .splice project, cutting or arranging video, titles, captions, exports or screen recordings through Splice. Covers the call pattern, self-correction, owning undo history, and reporting friction.
---

# Using Splice

Splice is an agent-first video editor. You have the whole editor: every edit a human can make is a
command you can send. There are no drafts and no approvals. Your edits land live, and **you own the
undo history**, so being bold is safe as long as you check your work.

Start the server with `splice mcp` (already configured in `.mcp.json`). It never opens UI. If a human
has the project open in the viewer, they see your edits as you make them.

## The pattern

1. `project.info {path}`: open the project (`create:true` makes one). Keep `projectId` and `revision`.
2. `session.start`: get `sessionId` and a checkpoint revision.
3. Read before writing: `timeline.snapshot`, then `items.inspect`, `transcript.get` and `timeline.find` as needed.
4. `commands.describe` when you are unsure about a command: every command has params and a working example.
5. `edit.apply {sessionId, ifRevision, ops:[…]}`: one batch per intent. Use `$ref` to point at items
   created earlier in the same batch. Send `dryRun:true` first when a batch deletes, ripples or touches
   more than a few items.
6. Check the result: `changes` and `sideEffects` in the response, then `view.frames {at:[…]}` to see it.
7. Wrong? `history.undo {scope:"last"}`, then try again. `{scope:"session"}` reverts all your work;
   `{scope:"to", revision}` restores any earlier state exactly. Undo is itself a revision, so you can
   undo an undo.
8. `export.start` when done. It writes `<project>.splice/exports/…`, refuses to overwrite unless
   `overwrite:true`, and reports through `jobs.status`.

## Recording the screen

- `record.start {path:"…/New.splice", create:true, dryRun:true}` first: it lists displays, windows,
  apps, microphones and cameras (stable `id`s), and says which permissions are granted. Then record
  with a `source` (`{display:"main"}`, `{window:{app:"com.apple.Safari"}}` or `{area:{display,
  rect}}`), plus `mic` / `camera` (`true` or a device `id`) and `systemAudio` (`true`, `false` or
  `{apps:["Music"]}`); cursor and input events are recorded unless `events:false`. `size:[w,h]`
  resizes a window first; `exclude:["desktop"]` hides desktop icons. Read the receipt's
  `settings`: it is what will be recorded.
- Never sleep to time a take. `duration` (seconds of recorded time) ends it; `then:"pause"` pauses
  there; `record.start {resume:jobId, duration:3}` records exactly 3 s more. Then
  `record.stop {jobId, wait:30}` returns once it ends. `countdown` is seconds too.
- A new project (`create:true`) gets the canvas nearest the source's shape; pass `aspect` to pick.
- `record.stop {jobId}`: the take lands on the spine as one undoable batch, with its cursor, clicks
  and keys as components you can query (`takes.events`) and edit. Check its `take` summary
  (`seconds`, `size`, `clicks`, `micPeakDb`/`silent`) and `lints` (`TAKE_ASPECT`, `MIC_SILENT`).
  Keystroke chips start off (they can show typed text): `keys.set {clip, show:"shortcuts"}` shows
  only ⌘/⌃/⌥ shortcuts, `show:"all"` every key. Your renders draw chips as dots; the export draws
  the real keys.
- Pause with `record.stop {jobId, pause:true}`, resume with `record.start {resume:jobId}`, and
  drop a marker with `record.start {resume:jobId, marker:"Name"}`. `voiceover:true, at` records
  the mic onto an audio lane; `retakeOf` places a retake after the earlier take.
- `TCC_DENIED` with `needsHuman:true`: stop and tell the human the exact fix. Don't retry in a loop.

## When a call fails

Errors come back as results: `{code, message, fix, current?, valid?}`.

- Do what `fix` says, literally. It names the call and arguments that work.
- `STALE_REVISION`: a human (or another agent) edited. Re-read with `timeline.snapshot`, re-check your
  intent against the new state, then retry with the new `ifRevision`. Never retry blindly.
- `BAD_ARGS` / `OUT_OF_RANGE`: the `valid` field lists accepted values or ranges.
- `NEEDS_TEXT` (captions.translate), `NO_HANDLES` (roll, slip, slide), `FIT_IMPOSSIBLE`
  (intent.fitDuration): the fix says what to pass or change.
- Pass the same `idempotencyKey` when you retry after a transport error.

## Report friction (this is how Splice improves)

Splice logs every error and every undo you make, and its developers fix the causes. Help them: if
something was confusing, surprising or needed several attempts, add `friction:"<one sentence: what you
expected vs what happened>"` to your next call (any tool accepts it). Be specific: name the command
and the argument.

## Time, IDs, anchors

- Time is integer frames at the project `fps` (`project.info`). `at`, `from`, `to`, `length` are frames.
- IDs are stable and prefixed: `clp_` clips, `ovl_` overlays, `lan_` lanes, `mrk_` markers, `med_` media, `w:screen:12` words (`w:<media>:<n>`, from `transcript.get`).
- Anchor overlays to *what is said* (`{word, transcriptHash}`) or to items (`{item, edge, offset}`)
  rather than to absolute frames, so they follow later edits.

## Pitfalls

<!-- Maintained by the build loop (docs/LOOP.md). One line each, citing a friction ID; deleted once a
core change makes the line unnecessary. -->
- (none yet)

## Recipes

Each recipe is a few batches; `commands.describe {query}` gives every command's full types.
The capability sweep runs these exact ops, so they stay correct.

- **Tutorial (screen take):** `cursor.set {size:2}`; `intent.autoZoom {item}` (clicks and typing
  come from the take; `lead`/`tail` in frames, `from`/`to` for a window, `focus:"cursor"` to
  follow the cursor, `motionBlur:0.5` for blurred camera moves); a long demo with one zoom that
  tracks the pointer is `zoom.add {at, length, amount:2, focus:"cursor"}`; `effect.add
  {target:clip, type:"cursor", params:{clickSound:true, alwaysArrow:true, loop:30}}` for audible
  clicks, a plain arrow and a seamless loop; `media.analyze {kinds:["text"]}` to find on-screen labels; `mask.add
  {item, mode:"blur", x, y, w, h}` over anything private; `template.add {name:"lower-third"}`;
  `marker.add` per chapter; `captions.generate`; export `variants:[{aspect:"16:9"},{aspect:"9:16"}]`.
- **Podcast:** `audio.sync {item, to}` for a second mic; `transcript.get` lists `fillers` and
  `retakes`, cut them with `cut.words`; `cut.pauses {min:21}`; `effect.add {type:"voiceIsolation"}`
  and `{type:"compressor"}` on the voice; `audio.set {items, normalize:-16}` on music; export
  `audio-m4a` plus the video.
- **Talking head:** `effect.add {type:"cutout", params:{background:"blur"}}` after
  `media.analyze {kinds:["matte"]}`; `crop.keys {item, follow:"face", aspect:"9:16"}`;
  `effect.add {type:"color", params:{lut:"look.cube"}}`; `audio.unlink` then trim the sound for a
  J-cut.
- **B-roll story:** `clip.insert {path:"photo.heic"}` for stills; `param.keys {path:"transform.scale",
  keys:[…]}` for Ken Burns moves; `transition.add {item, kind:"dissolve"}` between clips;
  `media.analyze {kinds:["beats"]}` on the music and cut on the beats; `style.set` then
  `template.add {name:"title-card"}`.
- **Custom motion (HTML):** when no command draws it (a chart, a code reveal, a branded card),
  `html.add {html, at, length}` with a self-contained page: transparent body, motion from CSS
  animations, `requestAnimationFrame` time or `window.seek(seconds)`, no network, no
  `Math.random`. It becomes a full-canvas overlay; check it with `view.frames`.
- **Music bed:** `commands.describe {music:{query:"calm", maxLength:120}}` lists the user's library
  (license, length, BPM, loudness); `music.add {track}` lays the pick under the whole timeline,
  looped or cut with a fade-out, ducked under speech and leveled to -14 LUFS. A track with
  `license:"unknown"` is refused: ask the human, then `allowUnlicensed:true`. The export writes a
  `.credits.txt` beside the movie; pass it on with the video.
- **Screen-demo look:** `effect.set {target:"project", type:"window", params:{background:"image",
  image:"/path/wall.jpg", blur:0.4}}` (or `background:"wallpaper", wallpaper:"ocean"`, or a
  gradient with `angle`); `effect.set {target:"clp_take", type:"camera", params:{aspect:"circle",
  x:0.85, y:0.8, size:0.3, mirror:true, shrinkWhenZoomed:true}}`; a fullscreen camera stretch is
  `layout:{keys:[{at:0, value:"bubble"}, {at:300, value:"full"}, {at:450, value:"bubble"}]}`
  (clip-relative frames, each key eases over 0.3 s, no split); a timed blur is `mask.add {item,
  mode:"blur", x, y, w, h, from, to}` (timeline frames). `look.save {target:"project",
  file:"looks/brand.json"}` keeps the look; `look.apply {target, file}` reuses it in any project.
- **Hand-off:** `export.start {format:"fcpxml"}` for Final Cut Pro, `{preset:"prores-422"}` for a
  master, `{captions:"srt"}` for a sidecar.

