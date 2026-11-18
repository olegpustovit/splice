# Signal Editor

## Product model

A social editor starts from a blank sequence, imported camera footage, audio, a graphic or a capture take. In the prototype, it opens on a sample vertical sequence that mixes motion, a screen take and an end card. Capture adds media through the same intake boundary as import. It can run in a separate process or be omitted entirely.

| Reference | Adopt | Tradeoff |
| --- | --- | --- |
| [Final Cut Pro Magnetic Timeline](https://support.apple.com/my-mm/guide/final-cut-pro/verb8fcfc133/12.4/mac/26.6) | Anchor overlays to a primary clip. Move the anchor and attached material follows. | Use visible tracks and an explicit Ripple switch so deliberate gaps and compositing order are legible. |
| [Final Cut Pro audio components](https://support.apple.com/en-lb/guide/final-cut-pro/ver549f212d/mac) | Expand components in place. Reserve detaching for deliberate independent timing. | Generalize audio disclosure to picture, pointer and key cues. Captured components share one source clock. |
| [Resolve Edit](https://www.blackmagicdesign.com/products/davinciresolve/edit) | Keep spatial preview, timeline and selection inspector together. Mark animation keys in the timeline. | Use clip-local properties for routine work. Keep a larger compositing workspace behind an explicit action when its graph earns the space. |
| [Resolve Fusion](https://www.blackmagicdesign.com/products/davinciresolve/fusion) | Use typed graphics with independently animated parameters and a future curve editor. | Prototype two scale keys and one text primitive. Define animation independently of UI framework and playback history. |
| Recorder [UI kit](../recorder/docs/UI-KIT.md), [editor study](../recorder/docs/EDITOR-UX.md) and [MCP implementation](../recorder/docs/MCP.md) | Carry over Signal UI, explicit scope, non-destructive source mapping, one preview/export evaluator and inspect → propose → render → submit → review. | Move project ownership out of the open window. The Rust core can serve a headless client or any eventual native UI. |

## Rust domain boundary

- `editor_core` owns the project, revision, normalized command batches, validation, undo and change events. The UI and stdio adapter both use this boundary. A repository port handles atomic project and draft persistence, with a single project writer and immutable snapshots for rendering.
- `media` provides immutable asset descriptors and decode access. `compositor` evaluates a revision and sequence time to a frame and audio interval. Preview, proxy and delivery use that evaluation path with different output quality. Capture extensions publish descriptors and bytes through media intake.
- A sequence has ordered tracks. A clip instance points into an asset through a source range and a sequence-to-source map. Use rational asset timestamps and integral sequence edit units. The prototype fixes sequence time to integer frames at 30 fps. The production engine needs sample-accurate audio and a defined quantization rule per track.
- `TakeAsset` contains a manifest with streams, per-stream clock offsets, discontinuities, and nullable component descriptors. Picture/audio streams only reference present media. Pointer samples and approved key cues are timed data that can produce ordinary compositing elements. Raw characters stay local and out of the tool view and agent preview rendering.
- Split makes two instances with distinct IDs, source ranges and copied overrides. Ripple moves later primary instances, along with attached graphics. Clipping an attachment samples its property curves at the cut boundaries. A production detach operation would lift a component to an ordinary track, preserve its sampled playback map, and disable the parent's copy in one undoable batch. This avoids duplicate audio and double compositing.
- Reusable motion compositions are typed layers with stable property IDs, interpolation and a duration. Bind templates through validated parameters. Production scope includes camera transforms, shape/text primitives, masks, curves and compositing order. The prototype uses linear scale and anchored title text.

## MCP contract

Use MCP stdio for the first native adapter. Discover tools with JSON Schema through `tools/list`; return typed object results with `structuredContent`. Pick and test protocol negotiation as part of implementation. [MCP tool specification](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)

| Tool | Main arguments | Result in editor |
| --- | --- | --- |
| `timeline.inspect` | `projectId`, `ifRevision` | Read current sequence, timing, clip IDs and safe component states. |
| `clip.trim` | `draftId?`, `clipId`, `removeTailFrames`, `ripple` | Shorten the take, shift later clips with ripple, and outline changed clips in the preview. |
| `motion.add` | `draftId?`, `clipId`, `offsetFrames`, `durationFrames`, `scale` | Add a motion block and preview the interpolated transform. |
| `title.add` | `draftId?`, `clipId`, `offsetFrames`, `durationFrames`, `text` | Add a visible title and timeline block. |
| `component.set` | `draftId?`, `clipId`, `component`, `enabled` | Hide cue renderings while retaining their source component. |
| `render.preview` | `frame`, `draftId?` | Preview the requested frame. Native response should return scoped frame or proxy handles. |
| `export.prepare` | `projectId`, `ifRevision`, `profile`, `aspect` | Show dimensions, duration and codec. A confirmed native delivery job owns destination selection, encoding, progress and cancellation. |
| `draft.submit`, `draft.accept`, `draft.discard` | `draftId`, `ifRevision` for submit and accept, and `changeIds` for accept | Review a frozen draft, commit selected operations atomically or discard them. |

Proposals also carry `projectId` and `ifRevision`. A production mutation envelope adds a client action ID for retry deduplication. Stage typed operations against an immutable base. Validate every replay, including review subset changes. Compute affected IDs, cascade details and invalidated sequence intervals from the resulting state. At acceptance, compare the base revision atomically. A production rebase must surface semantic conflicts and produce a fresh review. The prototype lets reviewers discard a stale draft and propose again.

The prototype's tool names, request examples and structured results describe an interaction contract. The HTML simulates both sides in the browser. Revision numbers cover committed model changes, including undo and redo. Imported videos stay in the running browser session. Capture samples are generated, and export prepares a profile in memory.

## Cold review

An independent reviewer exercised the seeded sequence at desktop and phone widths. The pass found that preview canvases overflowed their portrait frames, contiguous primary clips could not be reordered, event markers were hardcoded and inert, and MCP comparison omitted resolved anchor movement and component state.

The revised prototype constrains canvases to their frames, supports ripple insertion when a drag crosses an occupied primary position, maps sample cue events through the visible source range and seeks on selection, and compares on a shared time scale. The review view now lists time and component deltas and can be reopened from the editor. Browser checks covered these interactions, selective acceptance, undo, stale-draft rejection, and delivery dimensions.
