use crate::Phase;
use std::{borrow::Cow, collections::VecDeque};

#[cfg(test)]
mod tests;

/// How a new revision joins the current undo history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Merge {
    /// An independent undo step.
    Step,
    /// Consecutive updates with the same gesture ID form one step until sealed.
    Gesture(u64),
    /// Consecutive updates sharing a key within a rolling time window form one step.
    Burst {
        /// Identity of the action being repeated.
        key: u64,
        /// Exclusive upper bound on the time between updates, in milliseconds.
        window_ms: u32,
    },
    /// Replace the present revision while preserving undo and redo entries.
    Repair,
}

/// A committed revision and the action that produced it.
pub struct Entry {
    /// The action's label, borrowed from a constant or owned by the stack.
    pub label: Cow<'static, str>,
    /// How this action participates in undo.
    pub merge: Merge,
    /// The time of the action in milliseconds.
    pub time_ms: u64,
    /// Revision after the action.
    pub payload: u64,
}

struct Slot {
    label: Cow<'static, str>,
    payload: u64,
}

impl Slot {
    fn bytes(&self) -> usize {
        size_of::<Self>()
            + match &self.label {
                Cow::Owned(label) => label.capacity(),
                Cow::Borrowed(_) => 0,
            }
    }
}

/// A preallocated ring of revision IDs for interactive undo and redo.
pub struct UndoStack {
    slots: VecDeque<Slot>,
    // Undo slots precede the cursor. Redo slots start at the cursor.
    cursor: usize,
    present: u64,
    merge: Option<(Merge, u64)>,
    capacity: usize,
    byte_capacity: usize,
    bytes: usize,
}

impl UndoStack {
    /// Starts at `present`. Retains at most `capacity` steps across undo and redo.
    /// The byte budget counts slots and owned labels, with the newest step retained
    /// even when that step alone exceeds the budget. A count of zero disables retention.
    pub fn new(present: u64, capacity: usize, byte_capacity: usize) -> Self {
        let capacity = capacity.min((byte_capacity / size_of::<Slot>()).max(1));
        Self {
            slots: VecDeque::with_capacity(capacity),
            cursor: 0,
            present,
            merge: None,
            capacity,
            byte_capacity,
            bytes: 0,
        }
    }

    /// Records the revision. New steps abandon redo. Repeated revisions are no-ops.
    pub fn push(&mut self, entry: Entry) {
        if entry.payload == self.present {
            return;
        }
        let continuation = match (entry.merge, self.merge) {
            (Merge::Repair, _) => true,
            (Merge::Gesture(id), Some((Merge::Gesture(previous), _))) => id == previous,
            (Merge::Burst { key, window_ms }, Some((Merge::Burst { key: previous, .. }, at))) => {
                key == previous
                    && entry
                        .time_ms
                        .checked_sub(at)
                        .is_some_and(|elapsed| elapsed < u64::from(window_ms))
            }
            _ => false,
        };
        if !continuation && self.capacity != 0 {
            while self.slots.len() > self.cursor {
                self.bytes -= self.slots.pop_back().expect("redo slot").bytes();
            }
            let slot = Slot {
                label: entry.label,
                payload: self.present,
            };
            let bytes = slot.bytes();
            while !self.slots.is_empty()
                && (self.slots.len() == self.capacity
                    || bytes > self.byte_capacity.saturating_sub(self.bytes))
            {
                self.bytes -= self.slots.pop_front().expect("oldest undo slot").bytes();
            }
            self.slots.push_back(slot);
            self.bytes += bytes;
            self.cursor = self.slots.len();
        }
        self.present = entry.payload;
        if entry.merge != Merge::Repair {
            self.merge = Some((entry.merge, entry.time_ms));
        }
    }

    /// Records one phase of continuous control `id`, so a whole drag is one undo step.
    /// `Began`/`Changed` merge `entry` into the gesture, `Ended` merges it and seals. `Cancelled`
    /// drops the gesture's step and returns the revision the caller restores (no redo entry).
    pub fn gesture(&mut self, id: u64, phase: Phase, entry: Option<Entry>) -> Option<u64> {
        if let Some(entry) = entry.filter(|_| phase != Phase::Cancelled) {
            self.push(Entry {
                merge: Merge::Gesture(id),
                ..entry
            });
        }
        let open = self
            .merge
            .is_some_and(|(merge, _)| merge == Merge::Gesture(id));
        match phase {
            Phase::Began | Phase::Changed => None,
            Phase::Ended => {
                self.seal();
                None
            }
            Phase::Cancelled if open => {
                let restored = self.undo();
                while self.slots.len() > self.cursor {
                    self.bytes -= self.slots.pop_back().expect("redo slot").bytes();
                }
                restored
            }
            Phase::Cancelled => None,
        }
    }

    /// Ends the current gesture or burst so the next update starts a step.
    pub fn seal(&mut self) {
        self.merge = None;
    }

    /// Moves to the previous revision and returns its ID.
    pub fn undo(&mut self) -> Option<u64> {
        let previous = self.cursor.checked_sub(1)?;
        std::mem::swap(&mut self.slots[previous].payload, &mut self.present);
        self.cursor = previous;
        self.seal();
        Some(self.present)
    }

    /// Moves to the next revision and returns its ID.
    pub fn redo(&mut self) -> Option<u64> {
        let slot = self.slots.get_mut(self.cursor)?;
        std::mem::swap(&mut slot.payload, &mut self.present);
        self.cursor += 1;
        self.seal();
        Some(self.present)
    }

    /// Labels of the next actions to undo and redo, respectively.
    pub fn titles(&self) -> (Option<&str>, Option<&str>) {
        (
            self.cursor
                .checked_sub(1)
                .map(|at| self.slots[at].label.as_ref()),
            self.slots.get(self.cursor).map(|slot| slot.label.as_ref()),
        )
    }
}
