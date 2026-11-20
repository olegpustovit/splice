# Recorder parity audit (independent, read-only)

Scope: `docs/RECORDER-PARITY.md` (RP) against Recorder `docs/SPEC.md`, `docs/MCP.md`,
`docs/AUDIO-LAYERS.md`, and Splice `table.rs`, `effects.json`, `tools.rs`, `docs/MCP.md`, `SKILL.md`.
Every finding below was checked in the source. They are ranked by how likely each one is to make
an agent produce a wrong video. Line numbers are 1-based. RP:n is a line of RECORDER-PARITY.md.

Paths used below:
- table = crates/splice_core/src/commands/table.rs
- fx = crates/splice_core/src/commands/effects.json
- tools = crates/splice_mcp/src/tools.rs
- skill = .agents/skills/splice/SKILL.md

## Top findings (ranked)

1. **The redaction blur defaults to 60 %, and nothing proves that hides text.**
   - `mask.add` documents `amount` as just "number · 0…1", with no default (table:555). The effect's default is 0.6 (fx:744).
   - The skill recipe says `mask.add {item, mode:"blur", x, y, w, h}` "over anything private" and passes no amount (skill:103-104).
   - Recorder's new masks are full-opacity blur (Recorder SPEC "Recording/editor polish").
   - The only redaction evidence is `looks_ocr_reads_text_and_a_blur_mask_hides_it`, cited by RP:142. Despite its name it uses `mode:"pixelate"` with `amount:1.0` (crates/splice_av/tests/looks.rs:1250-1270).
   - So no test shows that a default blur makes text unreadable. An agent following the recipe can export readable secrets.

2. **Mask `x`/`y` is the top-left corner, but nothing says so where agents look.**
   - `mask.add` says only "x number · picture space" (table:551). docs/MCP.md:351 says `x, y, w, h` but not which corner.
   - Only the effect labels say it ("Left"/"Top", fx:701), which agents rarely read.
   - Nearby parameters with the same names are centres:
     - camera `x`/`y` are "Center x … of the canvas" (fx:277);
     - `mask.track` keys are "mask centers" (table:787) and are converted to left/top (crates/splice_core/src/edit/tier_c.rs:1236-1246);
     - zoom `focus` and the `clip.crop` focus are centre points.
   - An agent that passes a centre puts the blur half a box off and misses what it should hide.

3. **Analysis results come back in timeline frames, but the commands that take them want source frames.** Nothing names the conversion.
   - These return timeline frames:
     - `takes.events` says "in timeline frames" (tools:154);
     - `media.analyze` activity returns `{from,to}` timeline ranges (docs/MCP.md:355-360).
   - These take source frames:
     - `intent.autoZoom` `clicks.at` and `typing` (table:677-678);
     - `intent.speedTyping` `typing` (table:873);
     - `intent.steps` `activity` (table:861);
     - `cut.pauses`, `intent.pace` and `intent.fitDuration` `activity` (table:644, 653, 662).
   - The `sourceFrame` field that `takes.events` also returns (crates/splice_mcp/src/media.rs:579, 623) is not mentioned in the tool description. "Source frames" never says at which rate: the media's 60 fps (`at * media.probe.fps`, media.rs:561), not the project's.
   - Other units in the same area:
     - `take.shots[].at` is in take **seconds** (docs/MCP.md:605-606);
     - `keys.set hold` and `cursor.set arriveBefore` are seconds (table:313, 295);
     - camera layout keys are clip-relative (skill:131);
     - `crop.keys` uses owner frames (table:528);
     - mask `from`/`to` are timeline frames (docs/MCP.md:266).
   - Yet skill:82 says "Time is integer frames … `at`, `from`, `to`, `length` are frames". After any cut or speed change, copied numbers put zooms and speed-ups in the wrong place.

4. **Sound past the last clip is cut, and RP still calls it verified (RP:168, AC-EXP-4).**
   - In Recorder, the export runs to the last audio clip or layer. In Splice an item past the spine end is trimmed or removed (`cause:"spineEnd"`, docs/MCP.md:319-321).
   - Recorder's voice-over or music tail plays out. In Splice it is cut, or removed when a later edit shortens the spine.
   - The row admits the difference but labels it verified. It should be GAP.

5. **Pause cuts have no fade (RP:151 claims "keep tail, fade").**
   - `cut.pauses` takes `min`, `keep`, `basis` and `activity` only (table:641-644). The apply path makes hard cuts (crates/splice_core/src/edit/tier_c.rs:181-197).
   - Recorder's pause eraser fades each seam by 80 ms (Recorder SPEC "Voice workflow").
   - Every Splice pause cut is a hard audio seam that can click. The cited test (silent_tests.rs:28) only covers the activity basis.

6. **Two ways to set keystroke chips behave differently, and one can silently do nothing.**
   - `keys.set` turns the keys component on (crates/splice_core/src/edit/tier_b.rs:597-602). New takes start with keys off.
   - `effect.set {type:"keystrokes", params:{show:"shortcuts"}}` only stores parameters. That component check (tier_b.rs:819-833) only confirms the take has keys; it does not switch them on. So no chips render and no error is returned.
   - The reverse case leaks text. `component.set {component:"keys", enabled:true}` with no keystrokes effect uses the default `show:"all"` (fx:426-428), which includes typed text. The `KEYS_SHOW_TYPING` lint warns but does not refuse.

7. **Voice-over cannot re-record a range, and RP has no row for it.**
   - Recorder can re-record a selected audio clip: 1.5 s pre-roll, auto-stop, the original piece's gain set to 0, and any earlier take in the same range removed, all as one undo step (Recorder AUDIO-LAYERS §4.4, AC-VO-2).
   - Splice only has `record.start {voiceover:true, at}` (tools:534). The replaced original keeps playing unless the agent mutes it by hand, which is undocumented. The result is two voices over each other.

8. **Layer backdrop (Dim / Blur) and Free placement are missing, though RP:141 claims them.**
   - There is no backdrop parameter anywhere (grep of table and fx). The `overlay.add` placement values are `full|card|corner-*` (table:136). Free placement is not in the model (package/timeline.rs:460-480).
   - `behindPresenter` is not "in front of camera". Splice draws overlays above the camera bubble by default and moves them behind it with this flag (crates/splice_eval/src/eval.rs:1127-1160, 825-833). That is Recorder's opposite default.

9. **The `transform` `x`/`y` units are undocumented, yet the skill tells agents to place windows with it.**
   - skill:48 says "move one with `effect.set … transform params:{x, y, scale}`".
   - fx labels them only "X"/"Y" (range −2…2, default 0). Neither docs/MCP.md (:592, :596) nor FORMAT.md says whether this is a canvas-fraction offset or a centre.

10. **Auto-zoom does not follow Recorder's rules, though RP:108 claims AC-AZ-1.**
    - Splice defaults are lead 0.3 s, tail 2.5 s, merge 2.5 s and amount 1.5, with no rule dropping zooms under 1 s (crates/splice_core/src/edit/tier_c.rs:1565-1571).
    - Recorder uses −0.4 s / +1.5 s, a 3 s and 0.25-distance cluster, merging gaps under 1 s, dropping zooms under 1 s, and scale 2.0 (Recorder SPEC §6.4).
    - The cited tests use two clicks 210 frames apart, so they never exercise clustering, merging or dropping.

11. **The "Highlight this zoom" row (RP:143) needs a coordinate conversion nobody documents.**
    - The zoom is stored as a crop-space centre and an amount. The mask is a picture-space top-left rect.
    - The agent has to compute `x = fx − 0.5/amount`, and so on, itself. Splice has no helper.
    - The spotlight mode is never rendered to pixels in a test (only plan values are checked, compositing_tests.rs:266-288).

12. **The `cursor.set` docs promise animation that the core refuses.**
    - table:287 says size is "(keyable)". table:290 says hidden can be done with "split first for a range, or key it".
    - fx marks neither as `keys:true`, so keys are refused with `NOT_KEYABLE` (crates/splice_core/src/commands/mod.rs:749-760). This is a typed error, so it costs retries rather than wrong output.

13. **AC-CUR-1 is marked verified while Splice breaks half of it (RP:117).**
    - Recorder requires "output never leads the raw path". The row says Splice "may lead the hand mid-move by design".
    - The test checks only the error at rest (crates/splice_eval/src/cursor.rs:783).

14. **Wrong evidence for audio cleanup and keystroke layout.**
    - RP:136 cites `looks_cleanup_chain_lowers_the_noise_floor_and_keeps_speech`, which builds the chain with `isolation: None` (looks.rs:448). No test runs AUSoundIsolation.
    - RP:145 cites `native_take_draws_the_recorded_cursor_and_zooms_on_click` (splice_av/tests/native.rs:284), which never touches keys. No test renders a chip at a grid position or for its `hold` time.

15. **Exported square variants come out a different size than the same aspect set on the project.**
    - The export `variants` keep the project's **longer** edge (crates/splice_mcp/src/presets.rs:215-233): `1:1` from 1920×1080 is 1920×1920.
    - `project.set {aspect}` keeps the **shorter** edge (table:353): 1080×1080.
    - Adding `resolution:1080` changes the variant to 1080×1080. These are three results for "1:1", and only the docs explain them.

## 1. Coverage: Recorder capabilities with no row, or a row whose call doesn't provide them

7 missing or overclaimed. Checked by grepping table, fx, tools and docs/MCP.md.

| Recorder capability (source) | Finding |
| --- | --- |
| Pause eraser Fade 80 ms (SPEC Voice workflow) | Claimed at RP:151. `cut.pauses` has no fade (top 5) |
| Voice-over re-record of a range (AUDIO-LAYERS §4.4) | No row, no call (top 7) |
| Layer backdrop Dim/Blur, Free placement (AUDIO-LAYERS §4.6) | Claimed at RP:141. No parameter (top 8) |
| Recording frame rate 30/60 (SPEC §8 Settings ▸ Recording) | No row. `record.start` has no `fps` input (tools:556-601); the receipt is always 60 |
| `project.list` / `project.assets`, and a library recording as a layer (Recorder MCP tools table, "Assets and layers") | RP:175 claims "`project list`" via `project.info`, which only opens by path or ID (tools:54). Splice has no listing tool |
| Export tail = last audio or layer endpoint (AC-EXP-4) | Marked verified while Splice behaves differently (top 4) |
| Multi-window hard cuts (SPEC Windows as layers) | RP:53 offers only eased 0.25 s switches. A hard cut means rewriting the opacity keys, which is undocumented |

Covered but different (worth noting in RP):
- auto-zoom constants (top 10);
- click pulse goes to 0.8, Recorder 0.85 (crates/splice_eval/src/cursor.rs:835);
- GIF `width` ≤ 960 rather than Recorder's long edge ≤ 960 (presets.rs:472), so a 9:16 GIF can be 960×1707;
- `layout:"fullscreen"` (Recorder's name) is refused; Splice wants `"full"`, with a typed error;
- the AC-REC-1 test machine differs; RP:49 is open about it.

## 2. Evidence: 86 verified rows judged (all recording, cursor, zoom, camera, crop, audio, captions and export rows)

Every cited test, eval and tool exists. The `splice_av::*` names resolve to custom-harness tests in `crates/splice_av/tests/*.rs`.

- **Strong: 45.**
- **Wrong: 4.**
  - RP:117 (top 13).
  - RP:136 (top 14).
  - RP:145 (top 14).
  - RP:142. Its blur claim rests on a pixelate test at amount 1 (top 1).
- **Weak: 37** (35 listed below, plus 53 and 125). Each line is a row number and what its evidence leaves unproven.

Recording:
- 26: no test checks that `record.start {dryRun:true}` reports the grant fields (only `required()`/`permission_reply()`, capture_agent/permissions.rs:197, 260).
- 30: native size and 60 fps are never measured. `capture_agent_echoes_resolved_settings` echoes a hand-built geometry and a constant `"fps":60` (capture_agent/jobs.rs:694).
- 31 (AC-WIN-3): the test is about ambiguous titles (sources.rs:256). Nothing covers a covered window.
- 32: only option parsing is tested (record_options.rs:656). The resize rests on the live note.
- 33 (AC-AREA-2): the publish test asserts a fixture's 96×64 size (record.rs:832). `rect × scale`, even-rounded, is untested (apple_sys/native/capture.m:283).
- 36: the cited chips test is render-only. The `takes.events` redaction test that matters (splice_mcp/src/tests.rs:990) is not cited. `eval:keys-private` checks only `show=="shortcuts"`.
- 39: the cited echo test uses `systemAudio:{apps:[…]}`, not "all".
- 42: countdown parsing only. Nothing checks the delay.
- 46: `eval:record-pause-marker` checks a 5–8 s total, not the pause length.
- 47: only a pending take is cancelled (jobs.rs:776, 830). The CLI test is mocked.
- 50 (AC-REC-3): only the `recoverable()` predicate is tested (record_publish.rs:347). No playable-fragment or `recover:true` publish test.

Zoom and cursor:
- 108: see top 10. `native_take_draws…zooms_on_click` builds its zoom by hand and never runs `intent.autoZoom` (native.rs:345).
- 110: `zoom_commands_take_the_names_agents_use` is a dry run with no assertion (edit/tests.rs:989). Nothing checks that `instant` jumps.
- 113: `smooth` is only parsed. `h11` covers `focused` only (camera.rs:235).
- 118: no 4× sharpness measurement; the row admits it.
- 120: a dry run that asserts only that the call succeeds (edit/tests.rs:955). The `component.set pointer` route is untested.

Canvas and crop:
- 73 (AC-ED-2): one flat still at frame 0 via `read_frame` (looks.rs:1333). Not `view.frames`, no zoom, cursor or camera. H.264 delta 3 is outside Recorder's ≤ 1.
- 75: every render sets `corner 0, shadow 0` (looks.rs:896, 969). Corners and shadow are never asserted.
- 78: "one undo step" is not asserted.
- 80: `follow:"face"` is untested (enrich.rs:296). Only cursor following is tested.

Camera and audio:
- 132: roundness and shadow are 0 in the render test.
- 135: refusals and stored values only. Mute, a disabled component or the `volume` effect never changes a gain in a plan or export.
- 137: no unlink-then-normalize of take audio. Both tests use music items.
- 139: neither test covers fades on audio clips. The fades do work through `Interval::alpha` (crates/splice_eval/src/eval.rs:847), but that is untested here.
- 141: only `Placement::Full` is rendered. `behindPresenter` is tested only in an uncited test (splice_eval/src/tests.rs:269).
- 143: spotlight is never rendered (top 11).
- 144: `size:2` is passed but never asserted.
- 151: no fade (top 5). Transcript-basis pauses are untested.
- 152: only the duck parameters are stored. The real ducking test, `splice_eval::tests::music_ducks_under_transcript_speech_and_ramps_back` (tests.rs:318), is not cited.

Export:
- 159: nothing resolves `social-h264` or probes the output codec.
- 163: the row admits `eval:gif-teaser` was not re-run, which contradicts the header's "3/3 at 63fe58a". `fps:10` is never asserted.
- 165: the test allows 2 s against Recorder's 1 s. No test asserts that progress notifications are sent.
- 166 (AC-EXP-1): 8 s of stills, HEVC rather than H.264, extrapolated. No QuickTime, Safari or Chrome playback check.
- 167: the tool drives the `splice export` CLI, not MCP `export.start` (tools/av-drift.sh:24).
- 168: timeline arithmetic only; no exported duration is probed. It also has the behaviour difference (top 4).

Also weak: 53, where the behaviour differs (eased vs hard cut), and 125, where the value differs (0.8 vs 0.85).

## 3. UI-only rows (14 judged; 4 have a headless meaning)

| Row | Verdict |
| --- | --- |
| RP:27 toolbar/overlays (AC-TB-1) | **Not purely UI.** AC-TB-1 means Splice's own windows never appear in a capture. That matters headless whenever the viewer is open. The row admits the bundle-ID exclusion (`capture.m`) is untested |
| RP:51 editor in < 2 s (AC-REC-4) | **Has a headless analogue.** How fast `record.stop` publishes a 5-minute take. Agents block on it with `wait`, and it is unmeasured |
| RP:64 library (AC-LIB-1…4) | **Partly headless.** Listing projects and recordings is a Recorder MCP tool (`project.list`); see coverage |
| RP:86 timeline gestures (AC-TL-3…8) | Mostly UI. AC-TL-3 (smooth with 200 zooms on a 30-min project) has a headless analogue in `edit.apply` latency at that size, which is unmeasured |
| RP:28, 34, 44, 48, 111, 164, 180 and the playback half of RP:72 | Genuinely UI-only, or a deliberate contract (no drafts). RP:111 is acceptable: re-adding a zoom is exact |

## 4. Agent clarity (19 items)

High (wrong output without an error):
- table:551, docs/MCP.md:351, skill:104. The mask rect corner is unstated; centre-vs-corner trap (top 2).
- table:555, fx:744. The mask `amount` default (0.6) is unstated, with a privacy impact (top 1).
- tools:154, table:644/653/662/677/678/861/873, skill:82. Timeline vs source frames, source fps unstated (top 3).
- docs/MCP.md:605-606. `take.shots.at` is in seconds, against skill:82 "all frames".
- tier_b.rs:597-602 against `effect.set keystrokes`. A silent no-op. And with `component.set keys` the `show:"all"` default (fx:426-428) leaks typed text (top 6).
- skill:48, fx `transform.x/y`. Units unstated (top 9).
- docs/MCP.md:319-321. Items past the spine end are trimmed or removed. An agent adding a voice-over or music tail past the last clip loses it without a refusal (only `PAST_END` when placed at or past the end).

Medium (two ways that differ, or confusing names):
- table:182 vs table:133 vs table:367. `focus` means three spaces: crop (zoom), canvas (motion overlay), source (crop aspect).
- `corner` is a radius number on `window` (0–0.12) but an enum `br|bl|tr|tl` on `camera` (fx). RP:75 says "`window` effect `corner`".
- table:275 vs fx `volume.gain`. Volume is 0–2 in `audio.set` but 0–4 in the `volume` effect. Two ways with different ceilings, and the docs don't say whether they multiply.
- presets.rs:215-233 vs table:353. Aspect size rules differ (top 15).
- table:641-642. `cut.pauses keep` defaults to "frames = 4" but to "0.25 s" on the activity basis. Defaults are given in seconds for frame parameters (also `lead`/`tail`, table:680). An agent passing `lead:0.3` is refused, or gets 0, rather than 0.3 s.
- fx cursor `smoothing` defaults to 1, softer than every Recorder preset (table:292 lists them up to 0.75). A fresh take does not look like Recorder's default unless set.

Low (a typed error or a cosmetic difference):
- table:287, 290. "keyable" / "or key it" claims; `NOT_KEYABLE` (top 12).
- tools:445-470, docs/MCP.md:528. The `bitrate` unit (Mbps) is not in the schema or the docs, only in the refusal (presets.rs:270).
- docs/MCP.md:524-526. Export `to` is not said to be exclusive. Mask `to` is (docs/MCP.md:266).
- tools:463. The `fps` enum includes 10 and 15 for every preset; only GIF accepts them.
- `layout:"fullscreen"` (Recorder's word) is refused for `"full"` (fx camera `layout`).
