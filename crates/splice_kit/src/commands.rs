use crate::{Keymap, Shortcut};

/// App-defined command identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CommandId(pub u32);

/// A row in an app's command table.
pub struct Command {
    /// Identity used for dispatch.
    pub id: CommandId,
    /// Stable name used in the keymap file.
    pub name: &'static str,
    /// Default display title.
    pub title: &'static str,
    /// Default shortcuts, matched with exact modifiers.
    pub keys: &'static [Shortcut],
    /// Display group.
    pub group: &'static str,
}

/// The current presentation and availability of a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandState {
    /// Whether the command can be invoked.
    pub enabled: bool,
    /// Whether the command is selected or toggled on.
    pub checked: bool,
    /// A title override. `None` uses the command's default title.
    pub title: Option<&'static str>,
}

impl Default for CommandState {
    fn default() -> Self {
        Self {
            enabled: true,
            checked: false,
            title: None,
        }
    }
}

/// Where an invocation came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Keyboard shortcut.
    Key,
    /// Menu selection.
    Menu,
    /// Command palette selection.
    Palette,
    /// Accessibility action.
    A11y,
    /// Programmatic request.
    Automation,
}

/// A routed command intent. The caller checks its current state before dispatching it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Invoked {
    /// Command to dispatch.
    pub id: CommandId,
    /// Source of the intent.
    pub source: Source,
    /// Responder node that owns the command.
    pub target: u64,
}

struct Responder {
    node: u64,
    parent: u64,
    ids: &'static [CommandId],
}

/// Command routing along a focus chain. Resolution uses the retained registrations in place.
pub struct Responders {
    nodes: Vec<Responder>,
    keymap: Keymap,
}

impl Responders {
    /// Reserves room for the expected number of nodes.
    pub fn new(capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(capacity),
            keymap: Keymap::default(),
        }
    }

    /// Registers or updates a node. Use parent 0 for the root. Node 0 is reserved.
    pub fn responds(&mut self, node: u64, parent: u64, ids: &'static [CommandId]) {
        assert_ne!(node, 0);
        let responder = Responder { node, parent, ids };
        if let Some(existing) = self.nodes.iter_mut().find(|entry| entry.node == node) {
            *existing = responder;
        } else {
            self.nodes.push(responder);
        }
    }

    /// Installs shortcuts loaded for the current command table.
    pub fn set_keymap(&mut self, keymap: Keymap) {
        self.keymap = keymap;
    }

    /// Resolves a shortcut from the focused node through its parents, window and app.
    pub fn resolve(&self, focus: u64, key: Shortcut, table: &[Command]) -> Option<Invoked> {
        if let Some(id) = self.keymap.command(key) {
            return self.invoke(focus, id, Source::Key);
        }
        self.route(focus, Source::Key, &|id| {
            table
                .iter()
                .any(|command| command.id == id && command.keys.contains(&key))
        })
    }

    /// Routes a known command to the nearest responder that handles it.
    pub fn invoke(&self, focus: u64, id: CommandId, source: Source) -> Option<Invoked> {
        self.route(focus, source, &|candidate| candidate == id)
    }

    fn route(
        &self,
        mut node: u64,
        source: Source,
        accepts: &dyn Fn(CommandId) -> bool,
    ) -> Option<Invoked> {
        // Bound the walk even if a caller supplies a cyclic chain.
        for _ in 0..self.nodes.len() {
            let responder = self.nodes.iter().find(|entry| entry.node == node)?;
            if let Some(&id) = responder.ids.iter().find(|&&id| accepts(id)) {
                return Some(Invoked {
                    id,
                    source,
                    target: node,
                });
            }
            node = responder.parent;
        }
        None
    }
}

/// Recomputes command states in table order into caller-owned storage of the same length.
pub fn validate(
    table: &[Command],
    states: &mut [CommandState],
    fill: &mut dyn FnMut(CommandId, &mut CommandState),
) {
    assert_eq!(table.len(), states.len());
    for (command, state) in table.iter().zip(states) {
        *state = CommandState::default();
        fill(command.id, state);
    }
}
