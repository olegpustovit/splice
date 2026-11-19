use crate::commands::{self, CommandError, Op, RefPart, Target};
use crate::package::{
    journal::{MediaImport, Source},
    validate,
};
use splice_json::Value as Json;
use splice_model::*;
use std::fmt::{self, Write};

mod edits;
mod mints;
mod properties;
mod settle;
mod silent;
mod spine;
mod tier_b;
mod tier_c;
mod time;

pub use tier_c::{consolidated_path, is_filler, parse_cues, retakes};
pub(crate) use time::resolve_word_anchors;
pub use time::{resolve_view_frame, span};

#[cfg(test)]
mod fuzz_tests;
#[cfg(test)]
mod silent_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tier_b_tests;
#[cfg(test)]
mod tier_c_tests;

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
        /// Why it went.
        cause: Cause,
    },
    /// An item followed a split or a trimmed edge.
    Reanchored {
        /// Moved item.
        id: ItemId,
        /// Previous anchor.
        from: Anchor,
        /// New anchor.
        to: Anchor,
        /// Why it moved.
        cause: Cause,
    },
    /// An occupied lane caused an implicit lane move.
    Bumped {
        /// Moved item.
        id: ItemId,
        /// Previous lane.
        from: LaneId,
        /// New lane.
        to: LaneId,
        /// Why it moved.
        cause: Cause,
    },
}

/// Why a side effect happened (D21: results say what moved and why).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// The object it was anchored to was deleted, or a cut removed its whole parent clip.
    Cascade,
    /// It follows its content through a split, trim, join or edit of its parent.
    Follow,
    /// A cut removed the piece of its clip it was anchored to; it moved to the nearest surviving
    /// clip (`connected:"reparent"`).
    Reparent,
    /// A cut removed the frames it covered (`connected:"delete"` or `"trim"`).
    Cut,
    /// It ran past the end of the spine, which is the end of the timeline.
    SpineEnd,
    /// Its caption words are no longer on the timeline.
    Words,
    /// Its lane was busy where it landed: this item was there first.
    Overlap(ItemId),
    /// Its lane was deleted.
    LaneDeleted,
}

impl Cause {
    /// Wire name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Cascade => "cascade",
            Self::Follow => "follow",
            Self::Reparent => "reparent",
            Self::Cut => "cut",
            Self::SpineEnd => "spineEnd",
            Self::Words => "words",
            Self::Overlap(_) => "overlap",
            Self::LaneDeleted => "laneDeleted",
        }
    }
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
#[derive(Debug, PartialEq)]
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

/// Computes the timeline difference between two retained project states.
pub fn diff(before: &Project, after: &Project) -> Result<Vec<Diff>, CommandError> {
    let mut changes = Vec::new();
    time::diff(before, after, &mut changes)?;
    Ok(changes)
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
    let (project, report, _) =
        stage_above(base, ops, source, replay, base.serial, &mut missing_media)?;
    Ok((project, report))
}

pub(crate) fn stage_recorded(
    base: &Project,
    ops: &[Json],
    source: Source,
    mints: &[String],
    imports: &[MediaImport],
) -> Result<(Project, ApplyReport), ApplyError> {
    let mut load = |path: &str| {
        imports
            .iter()
            .find(|import| import.request == path)
            .map(|import| import.descriptor.clone())
            .map_or_else(|| missing_media(path), Ok)
    };
    let (project, report, resolved) =
        stage_above(base, ops, source, Some(mints), base.serial, &mut load)?;
    if resolved != imports {
        return Err(batch_error(error(
            "REPLAY_MEDIA",
            "The batch does not match its recorded media descriptors.",
            "Recover from the verified journal prefix.",
        )));
    }
    Ok((project, report))
}

pub(crate) fn stage_above(
    base: &Project,
    ops: &[Json],
    source: Source,
    replay: Option<&[String]>,
    serial_floor: u64,
    load: &mut dyn FnMut(&str) -> Result<Media, CommandError>,
) -> Result<(Project, ApplyReport, Vec<MediaImport>), ApplyError> {
    let mut work = ApplyWorkspace::default();
    work.stage(base, ops, source, replay, serial_floor, load)?;
    Ok((work.edit.project, work.edit.report, work.edit.imports))
}

/// `missing_media`'s message: no media backend probed the path (see `Editor::relink`).
pub(crate) const UNPROBED: &str = "The path has no registered media descriptor.";

/// The media loader without a native backend: every path is unknown.
pub fn missing_media(_: &str) -> Result<Media, CommandError> {
    Err(error(
        "MEDIA_OFFLINE",
        UNPROBED,
        "Import the file through the native media backend or pass a media ID from project.info.",
    ))
}

/// Stages `ops` without publishing them and resolves each batch reference in `refs` (`$cut`,
/// `$cut.right`) the way an op appended to the batch would see it. `splice mcp` reads the inputs
/// of objects a batch creates from this staged project (M10).
pub(crate) fn preview(
    base: &Project,
    ops: &[Json],
    serial_floor: u64,
    refs: &[&str],
    load: &mut dyn FnMut(&str) -> Result<Media, CommandError>,
) -> Result<(Project, Vec<Target>), ApplyError> {
    let mut work = ApplyWorkspace::default();
    work.stage(base, ops, Source::Human, None, serial_floor, load)?;
    let targets = refs
        .iter()
        .map(|name| {
            let reference = commands::reference(name, "ref")?;
            work.edit.resolve(&Target::Ref(reference))
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(batch_error)?;
    Ok((work.edit.project, targets))
}

/// Reusable in-memory storage for repeated batches on a project.
///
/// Reuse the same workspace across previews and commits to retain capacity for steady-state edits.
/// Larger batches and new project content can grow these buffers.
#[derive(Default)]
pub struct ApplyWorkspace {
    edit: Editor,
}

impl ApplyWorkspace {
    /// Applies or previews a batch. The report is available until the next use of this workspace.
    pub fn apply(
        &mut self,
        project: &mut Project,
        ops: &[Json],
        source: Source,
        dry_run: bool,
    ) -> Result<&ApplyReport, ApplyError> {
        self.stage(
            project,
            ops,
            source,
            None,
            project.serial,
            &mut missing_media,
        )?;
        if !dry_run {
            std::mem::swap(project, &mut self.edit.project);
        }
        Ok(&self.edit.report)
    }

    fn stage(
        &mut self,
        base: &Project,
        ops: &[Json],
        source: Source,
        replay: Option<&[String]>,
        serial_floor: u64,
        load: &mut dyn FnMut(&str) -> Result<Media, CommandError>,
    ) -> Result<(), ApplyError> {
        if source == Source::Agent && base.access != Access::Live {
            return Err(batch_error(error(
                "PERMISSION_DENIED",
                "This project does not allow agent edits.",
                "Ask the project owner to enable live agent access.",
            )));
        }
        let edit = &mut self.edit;
        edit.project.clone_from(base);
        edit.source = source;
        edit.serial_floor = serial_floor;
        edit.replay = replay.map(<[String]>::to_vec);
        edit.spare_changes.append(&mut edit.report.changes);
        edit.report.side_effects.clear();
        edit.report.mints.clear();
        edit.imports.clear();
        edit.bindings.clear();
        edit.explicit_lanes.clear();
        edit.splits.clear();
        for (index, value) in ops.iter().enumerate() {
            edit.step(value, index, load).map_err(|mut error| {
                // The refusing code knows the rule; the op knows what was passed.
                if let Some(rule) = &mut error.rule
                    && rule.got.is_none()
                {
                    let key = rule.param.split(['.', '[']).next().unwrap_or("");
                    rule.got = value.field(key).cloned();
                }
                ApplyError {
                    op: Some(index),
                    error,
                }
            })?;
        }
        edit.settle(base).map_err(batch_error)?;
        if replay.is_some_and(|mints| mints.len() != edit.report.mints.len()) {
            return Err(batch_error(mints::bad_replay()));
        }
        validate::with_scratch(&edit.project, &mut edit.validate_scratch).map_err(|error| {
            batch_error(CommandError::new(
                "INVARIANT",
                error.message,
                error.fix.into(),
            ))
        })?;
        time::diff(base, &edit.project, &mut edit.report.diff).map_err(batch_error)
    }
}

struct Editor {
    project: Project,
    source: Source,
    serial_floor: u64,
    replay: Option<Vec<String>>,
    report: ApplyReport,
    imports: Vec<MediaImport>,
    spare_changes: Vec<EditChange>,
    bindings: Vec<Binding>,
    explicit_lanes: Vec<ItemId>,
    /// Clips split in this batch, as (original, new right half), so later ops can follow a take.
    splits: Vec<(ClipId, ClipId)>,
    settle_scratch: settle::Scratch,
    validate_scratch: validate::Scratch,
    decode_scratch: commands::DecodeScratch,
    selection_scratch: Vec<TimelineId>,
    move_scratch: spine::MoveScratch,
    /// The current op's summary, written in place (`Summary::Note`) so warm edits allocate nothing.
    note: String,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            project: Project::default(),
            source: Source::Human,
            serial_floor: 0,
            replay: None,
            report: ApplyReport::default(),
            imports: Vec::new(),
            spare_changes: Vec::new(),
            bindings: Vec::new(),
            explicit_lanes: Vec::new(),
            splits: Vec::new(),
            settle_scratch: settle::Scratch::default(),
            validate_scratch: validate::Scratch::default(),
            decode_scratch: commands::DecodeScratch::default(),
            selection_scratch: Vec::new(),
            move_scratch: spine::MoveScratch::default(),
            note: String::new(),
        }
    }
}

struct Binding {
    name: String,
    primary: Option<Target>,
    first: Option<Target>,
}

enum Summary {
    Text(String),
    /// The text is in `Editor::note`.
    Note,
    Trim(TimelineId, Frame),
    Move(usize, Frame),
    Update(usize),
}

struct Outcome {
    summary: Summary,
    primary: Option<Target>,
    created: Vec<Target>,
}

impl Outcome {
    fn edit(summary: String, primary: Target) -> Self {
        Self::with_summary(Summary::Text(summary), primary)
    }

    fn noted(primary: Target) -> Self {
        Self::with_summary(Summary::Note, primary)
    }

    fn with_summary(summary: Summary, primary: Target) -> Self {
        Self {
            summary,
            primary: Some(primary),
            created: Vec::new(),
        }
    }

    fn create(summary: String, target: Target) -> Self {
        Self {
            summary: Summary::Text(summary),
            primary: Some(target.clone()),
            created: vec![target],
        }
    }
}

impl Editor {
    /// Writes the current op's summary into the reused note buffer (see `Outcome::noted`).
    fn say(&mut self, args: std::fmt::Arguments) {
        self.note.clear();
        self.note.write_fmt(args).expect("writing to a String");
    }

    fn step(
        &mut self,
        value: &Json,
        op: usize,
        load: &mut dyn FnMut(&str) -> Result<Media, CommandError>,
    ) -> Result<(), CommandError> {
        let commands::Decoded { op: decoded, bind } =
            commands::decode_reusing(value, &mut self.decode_scratch)?;
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
        let outcome = self.run(decoded, id, load)?;
        if let Some(name) = bind {
            self.bindings.push(Binding {
                name,
                primary: outcome.primary,
                first: outcome.created.first().cloned(),
            });
        }
        let (mut summary, mut created) = self.spare_changes.pop().map_or_else(
            || (String::new(), Vec::new()),
            |change| (change.summary, change.created),
        );
        summary.clear();
        match outcome.summary {
            Summary::Text(text) => summary = text,
            Summary::Note => summary.push_str(&self.note),
            Summary::Trim(id, to) => write!(summary, "Trimmed {id} to frame {to}").unwrap(),
            Summary::Move(count, delta) => {
                write!(summary, "Moved {count} item(s) by {delta} frames").unwrap()
            }
            Summary::Update(count) => write!(summary, "Updated {count} item(s)").unwrap(),
        }
        created.clear();
        created.extend(outcome.created);
        self.report.changes.push(EditChange {
            op,
            cmd: id,
            summary,
            created,
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
        self.selection_into(selection, &mut ids)?;
        Ok(ids)
    }

    fn selection_into(
        &self,
        selection: &[Target],
        ids: &mut Vec<TimelineId>,
    ) -> Result<(), CommandError> {
        ids.clear();
        for target in selection {
            let id = self.timeline(target)?;
            time::span(&self.project, id)?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(())
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

/// The shortest clip or item any edit may leave (M1).
const MIN_LENGTH: Frame = 3;

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
