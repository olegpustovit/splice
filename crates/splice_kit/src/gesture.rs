/// A continuous control's interaction with its caller-owned value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The control took pointer capture and began previewing.
    Began,
    /// Capture continues with a preview value.
    Changed,
    /// The final value was committed.
    Ended,
    /// Capture was lost without a commit.
    Cancelled,
}

impl Phase {
    /// Resolves capture changes and commits. A keyboard or accessibility commit can end on its own.
    pub fn from_interaction(was_held: bool, held: bool, committed: bool) -> Option<Self> {
        if committed {
            return Some(Self::Ended);
        }
        match (was_held, held) {
            (false, false) => None,
            (false, true) => Some(Self::Began),
            (true, true) => Some(Self::Changed),
            (true, false) => Some(Self::Cancelled),
        }
    }
}
