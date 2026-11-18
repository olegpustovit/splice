# `splice_kit`: the command system and undo, PXUI-shaped

PXUI (rust-port) has no command system, responder chain, document undo manager or gesture phases yet
(main spec › UI framework: PXUI). Splice needs them now, and LocalWave's OS will need them later. They
are built **here first**, in the shape PXUI would ship, and move into PXUI once PXUI is mature (after
the Unification Plan's 2.0 freeze, as an additive minor). Until then PXUI stays unchanged.

## Rules that keep the move cheap

- `splice_kit` depends on std (and `pxui_types` for `Key`/`Shortcut` if needed). Never on `splice_*`
  or private PXUI. The gate enforces it.
- No generics, no boxing, no macros, const tables, zero warm allocations. The same rules as PXUI public.
- Names and signatures are the proposed PXUI ones below. Migration = move the files into `pxui`,
  re-export from `splice_kit` for one commit, then delete `splice_kit`.
- Every app-specific thing stays out: payloads, labels' content, the core's Store and agent sessions.

## Commands

```rust
pub struct CommandId(pub u32);
pub struct Command { pub id: CommandId, pub name: &'static str, pub title: &'static str,
                     pub keys: &'static [Shortcut], pub group: &'static str }
pub struct CommandState { pub enabled: bool, pub checked: bool, pub title: Option<&'static str> }
pub enum Source { Key, Menu, Palette, A11y, Automation }
pub struct Invoked { pub id: CommandId, pub source: Source, pub target: u64 }

pub struct Responders { /* preallocated: node → &'static [CommandId] */ }
impl Responders {
    pub fn responds(&mut self, node: u64, parent: u64, ids: &'static [CommandId]);
    /// focused node → ancestors → window → app; text fields first for their editing keys
    pub fn resolve(&self, focus: u64, key: Shortcut, table: &[Command]) -> Option<Invoked>;
}
pub fn validate(table: &[Command], fill: &mut dyn FnMut(CommandId, &mut CommandState));
```

- Splice's command table is generated from the core registry plus view commands
  (`transport.play`, `transport.step`, `history.undo`, `history.redo`). A key, a menu item, the palette
  and MCP all produce the same `Invoked` → the same core op, so parity holds below the UI.
- Keymap: a flat `key = command` file with user overrides and a conflict test (from rays' `Router`).

## Undo

```rust
pub enum Merge { Step, Gesture(u64), Burst { key: u64, window_ms: u32 }, Repair }
pub struct Entry { pub label: &'static str, pub merge: Merge, pub time_ms: u64, pub payload: u64 }
pub struct UndoStack { /* preallocated ring, count and byte caps */ }
impl UndoStack {
    pub fn push(&mut self, e: Entry);      // merges per policy
    pub fn seal(&mut self);                 // end of gesture / burst
    pub fn undo(&mut self) -> Option<u64>;  // payload to restore
    pub fn redo(&mut self) -> Option<u64>;
    pub fn titles(&self) -> (Option<&'static str>, Option<&'static str>); // "Undo Move Clip"
}
```

- The merge policy is rays' proven `editor_history` (one gesture = one step, keystroke bursts
  coalesce, repairs fold into the step they repair).
- In Splice the payload is a **core revision**. The core's history (journal, sessions, revert with
  replay) is the truth; `UndoStack` only decides what ⌘Z means for the human and labels it. Agent
  batches arrive as `Step`s labelled `Agent: …`, so ⌘Z reverts an agent edit exactly like a human one.

## Gesture phases

`Phase::{Began, Changed, Ended, Cancelled}` on continuous controls (scrubber now, timeline drags
later). `Began` opens a `Gesture(id)` merge, `Ended` seals it, `Cancelled` restores the payload from `Began`.

## Later (only when a Splice task needs it)

External/YUV video textures (CVMetalTextureCache), native NSMenu bar from the command table, filled
icon paths, styled text spans for caption editing, drag-out with typed pasteboard. Each starts in
`splice_kit` (or in a `*_sys` crate for native glue) with the same rules.
