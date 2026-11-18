use crate::commands::{self, CommandError, Op, RefPart, Target};
use crate::package::{journal::Source, validate};
use splice_json::Value as Json;
use splice_model::*;
use std::fmt;

mod edits;
mod mints;
mod properties;
mod settle;
mod spine;
mod time;

pub use time::span;

#[cfg(test)]
mod tests;

/// One frame range and its lane, when it occupies one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// Inclusive start.
    pub start: Frame,
    /// Exclusive end. Markers have a zero-length range.
    pub end: Frame,
    /// Drawing lane.
    pub lane: Option<LaneId>,
}

/// A direct command result, in batch order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditChange {
    /// Zero-based index of the operation within the batch.
    pub op: usize,
    /// Wire command ID.
    pub cmd: &'static str,
    /// Human-readable edit description.
    pub summary: String,
    /// IDs created directly by the command.
    pub created: Vec<Target>,
}

/// An implicit change caused by maintaining timeline relationships.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideEffect {
    /// A dependent item was removed when its anchor disappeared.
    Removed {
        /// Removed item.
        id: ItemId,
        /// Deleted timeline object that triggered the cascade, if present.
        cascade_from: Option<TimelineId>,
    },
    /// An item followed a split or a trimmed edge.
    Reanchored {
        /// Moved item.
        id: ItemId,
        /// Previous anchor.
        from: Anchor,
        /// New anchor.
        to: Anchor,
    },
    /// An occupied lane caused an implicit lane move.
    Bumped {
        /// Moved item.
        id: ItemId,
        /// Previous lane.
        from: LaneId,
        /// New lane.
        to: LaneId,
    },
}

/// Timeline ranges and properties changed by an atomic batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diff {
    /// Changed clip, item, or marker.
    pub id: TimelineId,
    /// Previous resolved range, when the object existed.
    pub before: Option<Span>,
    /// Resulting resolved range, when the object still exists.
    pub after: Option<Span>,
}

/// The same result is returned for preview and for a committed in-memory batch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplyReport {
    /// Direct results.
    pub changes: Vec<EditChange>,
    /// Derived timeline changes.
    pub side_effects: Vec<SideEffect>,
    /// Timeline changes relative to the input.
    pub diff: Vec<Diff>,
    /// IDs minted in execution order, including during settle.
    pub mints: Vec<String>,
}

/// An atomic batch rejection, with the failing command index if known.
#[derive(Debug, PartialEq, Eq)]
pub struct ApplyError {
    /// Operation index. A batch-level failure has no index.
    pub op: Option<usize>,
    /// Typed error and repair hint.
    pub error: CommandError,
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(op) = self.op {
            write!(f, "op {op}: ")?;
        }
        self.error.fmt(f)
    }
}

impl std::error::Error for ApplyError {}

/// Applies an atomic in-memory batch. A dry run returns its report without publishing the candidate.
/// The journal assigns and persists the revision independently.
pub fn apply(
    project: &mut Project,
    ops: &[Json],
    source: Source,
    dry_run: bool,
) -> Result<ApplyReport, ApplyError> {
    let (next, report) = stage(project, ops, source, None)?;
    if !dry_run {
        *project = next;
    }
    Ok(report)
}

/// Rebuilds a batch using its exact recorded ID sequence.
pub fn replay(
    project: &mut Project,
    ops: &[Json],
    source: Source,
    mints: &[String],
) -> Result<ApplyReport, ApplyError> {
    let (next, report) = stage(project, ops, source, Some(mints))?;
    *project = next;
    Ok(report)
}

pub(crate) fn stage(
    base: &Project,
    ops: &[Json],
    source: Source,
    replay: Option<&[String]>,
) -> Result<(Project, ApplyReport), ApplyError> {
    stage_above(base, ops, source, replay, base.serial)
}

pub(crate) fn stage_above(
    base: &Project,
    ops: &[Json],
    source: Source,
    replay: Option<&[String]>,
    serial_floor: u64,
) -> Result<(Project, ApplyReport), ApplyError> {
    if source == Source::Agent && base.access != Access::Live {
        return Err(batch_error(error(
            "PERMISSION_DENIED",
            "This project does not allow agent edits.",
            "Ask the project owner to enable live agent access.",
        )));
    }
    // ponytail: stage in a clone until T2.6 provides reusable edit storage for warm apply.
    let mut edit = Editor {
        project: base.clone(),
        source,
        serial_floor,
        replay: replay.map(<[String]>::to_vec),
        report: ApplyReport::default(),
        bindings: Vec::new(),
        explicit_lanes: Vec::new(),
    };
    for (index, value) in ops.iter().enumerate() {
        edit.step(value, index).map_err(|error| ApplyError {
            op: Some(index),
            error,
        })?;
    }
    edit.settle(base).map_err(batch_error)?;
    if edit
        .replay
        .as_ref()
        .is_some_and(|mints| mints.len() != edit.report.mints.len())
    {
        return Err(batch_error(mints::bad_replay()));
    }
    validate::project(&edit.project).map_err(|error| {
        batch_error(CommandError::new(
            "INVARIANT",
            error.message,
            error.fix.into(),
        ))
    })?;
    edit.report.diff = time::diff(base, &edit.project).map_err(batch_error)?;
    Ok((edit.project, edit.report))
}

struct Editor {
    project: Project,
    source: Source,
    serial_floor: u64,
    replay: Option<Vec<String>>,
    report: ApplyReport,
    bindings: Vec<Binding>,
    explicit_lanes: Vec<ItemId>,
}

struct Binding {
    name: String,
    primary: Option<Target>,
    first: Option<Target>,
}

struct Outcome {
    summary: String,
    primary: Option<Target>,
    created: Vec<Target>,
}

impl Outcome {
    fn edit(summary: String, primary: Target) -> Self {
        Self {
            summary,
            primary: Some(primary),
            created: Vec::new(),
        }
    }

    fn create(summary: String, target: Target) -> Self {
        Self {
            summary,
            primary: Some(target.clone()),
            created: vec![target],
        }
    }
}

impl Editor {
    fn step(&mut self, value: &Json, op: usize) -> Result<(), CommandError> {
        let commands::Decoded { op: decoded, bind } = commands::decode(value)?;
        let id = value
            .field("cmd")
            .and_then(Json::as_str)
            .and_then(commands::lookup)
            .expect("decoded command")
            .id;
        if bind
            .as_ref()
            .is_some_and(|name| self.bindings.iter().any(|binding| &binding.name == name))
        {
            return Err(CommandError::bad("ref", "a unique name within the batch"));
        }
        let outcome = self.run(decoded, id)?;
        if let Some(name) = bind {
            self.bindings.push(Binding {
                name,
                primary: outcome.primary,
                first: outcome.created.first().cloned(),
            });
        }
        self.report.changes.push(EditChange {
            op,
            cmd: id,
            summary: outcome.summary,
            created: outcome.created,
        });
        Ok(())
    }

    fn resolve(&self, target: &Target) -> Result<Target, CommandError> {
        let Target::Ref(reference) = target else {
            return Ok(target.clone());
        };
        let value = self
            .bindings
            .iter()
            .find(|binding| binding.name == reference.name)
            .and_then(|binding| {
                match reference.part {
                    RefPart::Primary => &binding.primary,
                    RefPart::FirstCreated => &binding.first,
                }
                .as_ref()
            });
        value.cloned().ok_or_else(|| {
            error(
                "BAD_ARGS",
                &format!("Unresolved batch reference {}.", reference.name),
                "Bind the name on an earlier operation and select a result it actually created.",
            )
        })
    }

    fn timeline(&self, target: &Target) -> Result<TimelineId, CommandError> {
        match self.resolve(target)? {
            Target::Timeline(id) => Ok(id),
            _ => Err(wrong_kind(
                "Use a clip, overlay, or marker ID from timeline.snapshot.",
            )),
        }
    }

    fn selection(&self, selection: &[Target]) -> Result<Vec<TimelineId>, CommandError> {
        let mut ids = Vec::with_capacity(selection.len());
        for target in selection {
            let id = self.timeline(target)?;
            time::span(&self.project, id)?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(ids)
    }
}

fn batch_error(error: CommandError) -> ApplyError {
    ApplyError { op: None, error }
}

fn error(code: &'static str, message: &str, fix: &str) -> CommandError {
    CommandError::new(code, message.into(), fix.into())
}

fn not_found(label: &dyn fmt::Display, fix: &str) -> CommandError {
    error("NOT_FOUND", &format!("No object with ID {label}."), fix)
}

fn wrong_kind(fix: &str) -> CommandError {
    error(
        "WRONG_KIND",
        "This command requires a different kind of object.",
        fix,
    )
}

fn frame_add(frame: Frame, delta: Frame) -> Result<Frame, CommandError> {
    frame.checked_add(delta).ok_or_else(frame_range)
}

fn frame_range() -> CommandError {
    error(
        "OUT_OF_RANGE",
        "The resulting frame is outside the timeline range.",
        "Use a smaller frame or offset.",
    )
}
