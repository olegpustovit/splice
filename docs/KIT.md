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
    pub fn new(capacity: usize) -> Self;
    pub fn responds(&mut self, node: u64, parent: u64, ids: &'static [CommandId]);
    /// focused node → ancestors → window → app; text fields first for their editing keys
    pub fn resolve(&self, focus: u64, key: Shortcut, table: &[Command]) -> Option<Invoked>;
    pub fn invoke(&self, focus: u64, id: CommandId, source: Source) -> Option<Invoked>;
    pub fn set_keymap(&mut self, keymap: Keymap);
}
pub fn validate(table: &[Command], states: &mut [CommandState],
                fill: &mut dyn FnMut(CommandId, &mut CommandState));
```

- Splice's command table is generated from the core registry plus view commands
  (`transport.play`, `transport.step`, `history.undo`, `history.redo`). A key, a menu item, the palette
  and MCP all produce the same `Invoked` → the same core op, so parity holds below the UI.
- Call `validate` with a reusable state buffer in table order. Check the command's current
  `enabled` state before handling its `Invoked` intent.
- Load the UTF-8 keymap with `Keymap::read`. Each `key = command` line overrides one chord.
  Other default chords stay available. Use `#` for full-line comments. Chords use `Cmd`, `Shift`,
  `Ctrl` or `Alt` and a key joined by `+`, as in `Cmd+Shift+Z = history.redo`.
  Keys include characters, `Space`, the arrows (`Left`, `Right`, `Up`, `Down`), `F1` through `F12`,
  and named navigation keys. Spell the delimiters `Plus`, `Equals` and `Hash` when binding them.
  The loader reports the line number for unknown commands and both line numbers for duplicate chords.

## Undo

```rust
pub enum Merge { Step, Gesture(u64), Burst { key: u64, window_ms: u32 }, Repair }
pub struct Entry { pub label: Cow<'static, str>, pub merge: Merge, pub time_ms: u64, pub payload: u64 }
pub struct UndoStack { /* preallocated ring, count and byte caps */ }
impl UndoStack {
    pub fn new(present: u64, capacity: usize, byte_capacity: usize) -> Self;
    pub fn push(&mut self, e: Entry);      // merges per policy
    pub fn seal(&mut self);                 // end of gesture / burst
    pub fn undo(&mut self) -> Option<u64>;  // payload to restore
    pub fn redo(&mut self) -> Option<u64>;
    pub fn titles(&self) -> (Option<&str>, Option<&str>);
}
```

- The merge policy follows rays' `editor_history`. Consecutive updates with the same gesture ID
  merge until sealed. Key bursts use the gap from the last update and start a new step at the
  window boundary. Repairs replace the current revision and preserve existing undo and redo steps.
- The byte budget counts retained ring slots and owned title buffers. Oldest steps are evicted
  first. The newest step stays available even when it alone exceeds the byte budget.
- In Splice the payload is a **core revision**. The core's history (journal, sessions, revert with
  replay) is the source of truth. Feed the kit the revision after each edit with the core's batch
  label. The stack accepts owned labels from the journal and borrowed labels for constant actions.
  Agent batches are `Step`s labelled `Agent: …`. The viewer uses `titles` for the actions on each
  side of the current revision.

## Gesture phases

`Phase::{Began, Changed, Ended, Cancelled}` on continuous controls (scrubber now, timeline drags
later). `Began` opens a `Gesture(id)` merge, `Ended` seals it, `Cancelled` restores the payload from `Began`.

## Later (only when a Splice task needs it)

External/YUV video textures (CVMetalTextureCache), native NSMenu bar from the command table, filled
icon paths, styled text spans for caption editing, drag-out with typed pasteboard. Each starts in
`splice_kit` (or in a `*_sys` crate for native glue) with the same rules.
