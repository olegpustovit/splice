use super::{
    commands::{REDO, UNDO},
    viewer_drawers,
};
use input::{Event, Key, ScrollPhase};
use pxui::{
    Button, Fill, Flags, Handle, Icon, IconButton, Image, ImageStyle, KeyCode, Label, Node, Paint,
    Point, Pressable, Rect, Role, Scrubber, Signal, Stroke, Ui, ValueCallbacks, ValueField,
    draw::flags::rect::{BOTTOM, TOP},
    geom::{Edges, Rgba},
    theme::ColorToken,
    ui::{Align, Along, Bar, Cursor, Look, Menu, MenuItem, Size, State as UiState, States},
};
use splice_av::{Evaluator, SourceSpan};
use splice_core::{
    commands::{Component as TakeComponent, Target},
    edit::{EditChange, effect_target_allowed},
    history::StoreError,
};
use splice_kit::{CommandId, Phase};
use splice_model::{
    Anchor, ClipData, ClipId, ClipTiming, Effect, EffectValue, ItemData, ItemId, ItemKind, LaneId,
    LaneKind, MarkerId, Media, MediaId, MediaKind, Placement, Project, ScaleMotion, Slot,
    TimelineId, WordId,
    timing::{anchor_start, consumed},
};

use std::{collections::HashSet, rc::Rc};

const LABEL_WIDTH: f32 = 156.0;
const COMPACT_LABEL_WIDTH: f32 = 98.0;
const TOOLBAR_HEIGHT: f32 = 32.0;
const RULER_HEIGHT: f32 = 24.0;
const LANE_HEIGHT: f32 = 26.0;
const LANE_CONTROL_SIZE: f32 = 18.0;
const SPINE_HEIGHT: f32 = 46.0;
const TAKE_HEAD_HEIGHT: f32 = 18.0;
const COMPONENT_HEIGHT: f32 = 24.0;
const AUDIO_COMPONENT_HEIGHT: f32 = 30.0;
const TRIM_GRIP: f32 = 6.0;
const FX_BADGE_HEIGHT: f32 = 12.0;
const FADE_HANDLE: f32 = 7.0;
const MIN_TRIM_LENGTH: i32 = 3;
const FOOTER_HEIGHT: f32 = 26.0;
const OVERVIEW_HEIGHT: f32 = 18.0;
const TRACK_OVERSCAN: f32 = 52.0;
const ANCHOR_DIAMOND_RADIUS: f32 = 2.5 * std::f32::consts::SQRT_2;
const TICK_STEPS: [f32; 12] = [
    1., 3., 15., 30., 150., 300., 900., 1800., 3600., 9000., 18000., 54000.,
];

const PANEL: Rgba = Rgba::rgb(15, 15, 16);
const SURFACE: Rgba = Rgba::rgb(11, 11, 12);
const CONTROL: Rgba = Rgba::rgb(20, 20, 21);
const LINE: Rgba = Rgba::rgb(37, 37, 39);
const QUIET: Rgba = Rgba::rgb(130, 130, 125);
const INK: Rgba = Rgba::rgb(244, 243, 238);
const ACCENT: Rgba = Rgba::rgb(255, 138, 92);
const WARN: Rgba = Rgba::rgb(216, 200, 106);
const VIDEO: Rgba = Rgba::rgb(236, 230, 206);
const TAKE_TRACK: Rgba = Rgba::rgb(13, 13, 14);
const TAKE_LABEL: Rgba = Rgba::rgb(17, 17, 18);
const TAKE_MUTED: Rgba = Rgba::rgb(177, 177, 171);
const TAKE_SAGE: Rgba = Rgba::rgb(211, 228, 205);
const TAKE_CONTROL: States = States::of(Look::NONE.text(QUIET))
    .hover(Look::NONE.text(INK))
    .checked(Look::NONE.text(TAKE_MUTED));
const TAKE_OFF: States =
    States::of(Look::NONE.text(Rgba::rgb(227, 130, 118))).hover(Look::NONE.text(INK));
const CAPTION: Label = Label::MONO.size(10.0);
const SMALL: Label = Label::MONO.size(11.0);

const TOOL_IDLE: Look = Look::NONE.text(Rgba::rgb(177, 177, 171));
const TOOL_HOT: Look = TOOL_IDLE.fill(Rgba::rgb(34, 34, 36)).text(INK);
const TOOLS: States = States::of(TOOL_IDLE)
    .hover(TOOL_HOT)
    .pressed(TOOL_HOT)
    .checked(TOOL_HOT)
    .disabled(Look::NONE.text(Rgba::rgb(74, 74, 77)));

#[derive(Clone, PartialEq)]
pub(super) enum ToolbarCommand {
    AddTitle(i32),
    AddMotion(i32),
    AddMarker(i32),
    MoveMarker(MarkerId, i32),
    Nudge(Rc<[TimelineId]>, i32),
    SetFade(TimelineId, bool, i32),
    SetFades(Rc<[TimelineId]>, [Option<i32>; 2]),
    SetSpeed(Rc<[TimelineId]>, f64, bool),
    SetColor(Rc<[TimelineId]>, Option<&'static str>),
    LiftGroup(Rc<[TimelineId]>),
    Lift(ClipId),
    ReplaceSpine(ItemId),
    ReverseMotion(ItemId, ScaleMotion),
    SetPinned(ItemId, bool),
    SetPlacement(ItemId, Placement),
    SetBehind(ItemId, bool),
    SplitCaption(ItemId, WordId),
    MergeCaptions(Rc<[TimelineId]>),
    SetCaptionStyle(ItemId, &'static str),
    SetCaptionEmphasis(ItemId, WordId, bool),
    AddEffect(Target, &'static str),
    RemoveEffects(TimelineId),
    DeleteKeepOverlays(ClipId),
    Delete(TimelineId, bool),
    DeleteGroup(Rc<[TimelineId]>, bool),
    SetComponent(ClipId, TakeComponent, bool),
    SetTakeAudio(ClipId, [bool; 2], bool),
    SetKeystrokeHidden(ClipId, u32, bool),
    Duplicate(Rc<[TimelineId]>),
    LaneStep(Rc<[TimelineId]>, &'static str),
    NewLane(LaneKind),
    RenameLane(LaneId, String),
    DeleteLane(LaneId),
    AddTitleInLane(i32, LaneId),
    AddMotionInLane(i32, LaneId),
}

impl ToolbarCommand {
    pub(super) fn operations(self) -> Vec<splice_json::Value> {
        match self.operation() {
            splice_json::Value::Array(batch) => batch,
            op => vec![op],
        }
    }

    pub(super) fn operation(self) -> splice_json::Value {
        use splice_json::Value as Json;
        let (command, args) = match self {
            Self::AddTitle(at) | Self::AddMotion(at) | Self::AddMarker(at) => {
                let command = match self {
                    Self::AddTitle(_) => "title.add",
                    Self::AddMotion(_) => "motion.add",
                    _ => "marker.add",
                };
                (
                    command,
                    vec![("at".into(), Json::Unsigned(at.max(0) as u64))],
                )
            }
            Self::MoveMarker(id, at) => (
                "marker.set",
                vec![
                    ("marker".into(), Json::String(id.to_string())),
                    ("at".into(), Json::Unsigned(at.max(0) as u64)),
                ],
            ),
            Self::Nudge(ids, delta) => (
                "item.move",
                vec![
                    ("items".into(), timeline_ids(&ids)),
                    ("delta".into(), Json::Signed(i64::from(delta))),
                    ("ripple".into(), Json::Bool(false)),
                ],
            ),
            Self::SetFade(id, incoming, frames) => (
                "item.set",
                vec![
                    ("items".into(), timeline_ids(&[id])),
                    (
                        if incoming { "fadeIn" } else { "fadeOut" }.into(),
                        Json::Unsigned(frames as u64),
                    ),
                ],
            ),
            Self::SetFades(ids, fades) => {
                let mut args = vec![("items".into(), timeline_ids(&ids))];
                for (field, frames) in ["fadeIn", "fadeOut"].into_iter().zip(fades) {
                    if let Some(frames) = frames {
                        args.push((field.into(), Json::Unsigned(frames as u64)));
                    }
                }
                ("item.set", args)
            }
            Self::SetSpeed(ids, speed, ripple) => (
                "clip.speed",
                vec![
                    ("items".into(), timeline_ids(&ids)),
                    ("speed".into(), Json::Float(speed)),
                    ("ripple".into(), Json::Bool(ripple)),
                ],
            ),
            Self::SetColor(ids, color) => (
                "item.set",
                vec![
                    ("items".into(), timeline_ids(&ids)),
                    (
                        "color".into(),
                        color.map_or(Json::Null, |name| Json::String(name.into())),
                    ),
                ],
            ),
            Self::LiftGroup(ids) => ("clip.lift", vec![("items".into(), timeline_ids(&ids))]),
            Self::Lift(id) => (
                "clip.lift",
                vec![(
                    "items".into(),
                    Json::Array(vec![Json::String(id.to_string())]),
                )],
            ),
            Self::ReplaceSpine(id) => (
                "overlay.replaceSpine",
                vec![("item".into(), Json::String(id.to_string()))],
            ),
            Self::ReverseMotion(id, motion) => (
                "motion.set",
                vec![
                    ("item".into(), Json::String(id.to_string())),
                    ("from".into(), Json::Float(motion.to)),
                    ("to".into(), Json::Float(motion.from)),
                ],
            ),
            Self::SetPinned(id, pinned) => (
                "item.anchor",
                vec![
                    ("item".into(), Json::String(id.to_string())),
                    ("pinned".into(), Json::Bool(pinned)),
                ],
            ),
            Self::SetPlacement(id, placement) => (
                "overlay.set",
                vec![
                    ("item".into(), Json::String(id.to_string())),
                    (
                        "placement".into(),
                        Json::String(
                            PLACEMENTS
                                .iter()
                                .find(|(setting, _)| *setting == placement)
                                .expect("all placements")
                                .1
                                .into(),
                        ),
                    ),
                ],
            ),
            Self::SetBehind(id, behind) => (
                "overlay.set",
                vec![
                    ("item".into(), Json::String(id.to_string())),
                    ("behindPresenter".into(), Json::Bool(behind)),
                ],
            ),
            Self::SplitCaption(id, word) => (
                "caption.split",
                vec![
                    ("item".into(), Json::String(id.to_string())),
                    ("word".into(), Json::String(word.to_string())),
                ],
            ),
            Self::MergeCaptions(ids) => {
                ("caption.merge", vec![("items".into(), timeline_ids(&ids))])
            }
            Self::SetCaptionStyle(id, preset) => (
                "effect.set",
                vec![
                    ("target".into(), Json::String(id.to_string())),
                    ("type".into(), Json::String("captions".into())),
                    (
                        "params".into(),
                        Json::Object(vec![("preset".into(), Json::String(preset.into()))]),
                    ),
                ],
            ),
            Self::SetCaptionEmphasis(id, word, emph) => (
                "caption.set",
                vec![
                    ("item".into(), Json::String(id.to_string())),
                    (
                        "words".into(),
                        Json::Object(vec![(
                            word.to_string(),
                            Json::Object(vec![("emph".into(), Json::Bool(emph))]),
                        )]),
                    ),
                ],
            ),
            Self::AddEffect(target, kind) => (
                "effect.add",
                vec![
                    (
                        "target".into(),
                        Json::String(match target {
                            Target::Lane(id) => id.to_string(),
                            Target::Timeline(id) => id.to_string(),
                            _ => unreachable!("timeline effect menu target"),
                        }),
                    ),
                    ("type".into(), Json::String(kind.into())),
                ],
            ),
            Self::RemoveEffects(id) => (
                "effect.remove",
                vec![("target".into(), Json::String(id.to_string()))],
            ),
            Self::DeleteKeepOverlays(id) => (
                "item.delete",
                vec![
                    ("items".into(), timeline_ids(&[TimelineId::Clip(id)])),
                    ("overlays".into(), Json::String("keep".into())),
                ],
            ),
            Self::Delete(id, ripple) => (
                "item.delete",
                vec![
                    (
                        "items".into(),
                        Json::Array(vec![Json::String(id.to_string())]),
                    ),
                    ("ripple".into(), Json::Bool(ripple)),
                ],
            ),
            Self::DeleteGroup(ids, ripple) => (
                "item.delete",
                vec![
                    (
                        "items".into(),
                        Json::Array(ids.iter().map(|id| Json::String(id.to_string())).collect()),
                    ),
                    ("ripple".into(), Json::Bool(ripple)),
                ],
            ),
            Self::SetComponent(id, part, enabled) => (
                "component.set",
                vec![
                    ("clip".into(), Json::String(id.to_string())),
                    ("component".into(), Json::String(component_key(part).into())),
                    ("enabled".into(), Json::Bool(enabled)),
                ],
            ),
            Self::SetTakeAudio(id, present, enabled) => {
                return Json::Array(
                    [TakeComponent::Mic, TakeComponent::System]
                        .into_iter()
                        .zip(present)
                        .filter(|(_, present)| *present)
                        .map(|(part, _)| Self::SetComponent(id, part, enabled).operation())
                        .collect(),
                );
            }
            Self::SetKeystrokeHidden(id, index, hidden) => (
                "keys.hide",
                vec![
                    ("clip".into(), Json::String(id.to_string())),
                    (
                        "events".into(),
                        Json::Array(vec![Json::Unsigned(u64::from(index))]),
                    ),
                    ("hidden".into(), Json::Bool(hidden)),
                ],
            ),
            Self::Duplicate(ids) => ("item.duplicate", vec![("items".into(), timeline_ids(&ids))]),
            Self::LaneStep(ids, dir) => (
                "item.lane",
                vec![
                    ("items".into(), timeline_ids(&ids)),
                    ("dir".into(), Json::String(dir.into())),
                ],
            ),
            Self::NewLane(kind) => (
                "lane.create",
                vec![(
                    "kind".into(),
                    Json::String(
                        match kind {
                            LaneKind::Graphics => "graphics",
                            LaneKind::Overlay => "overlay",
                            LaneKind::Camera => "camera",
                            LaneKind::Captions => "captions",
                            LaneKind::Audio => "audio",
                        }
                        .into(),
                    ),
                )],
            ),
            Self::RenameLane(id, name) => (
                "lane.rename",
                vec![
                    ("lane".into(), Json::String(id.to_string())),
                    ("name".into(), Json::String(name)),
                ],
            ),
            Self::DeleteLane(id) => (
                "lane.delete",
                vec![("lane".into(), Json::String(id.to_string()))],
            ),
            Self::AddTitleInLane(at, id) | Self::AddMotionInLane(at, id) => (
                if matches!(self, Self::AddMotionInLane(..)) {
                    "motion.add"
                } else {
                    "title.add"
                },
                vec![
                    ("at".into(), Json::Unsigned(at.max(0) as u64)),
                    ("lane".into(), Json::String(id.to_string())),
                ],
            ),
        };
        let mut operation = Vec::with_capacity(args.len() + 1);
        operation.push(("cmd".into(), Json::String(command.into())));
        operation.extend(args);
        Json::Object(operation)
    }
}

fn timeline_ids(ids: &[TimelineId]) -> splice_json::Value {
    splice_json::Value::Array(
        ids.iter()
            .map(|id| splice_json::Value::String(id.to_string()))
            .collect(),
    )
}

#[derive(Clone, Copy, PartialEq)]
struct ContextMenu {
    anchor: Point,
    target: ContextTarget,
}

#[derive(Clone, Copy, PartialEq)]
enum ContextTarget {
    Block,
    Component,
    Marker(i32),
    Lane(LaneId),
    Background(i32, Option<LaneId>),
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ClipboardAction {
    Cut,
    Copy,
    Paste,
    PasteHere(i32),
    PasteEffects,
}

impl ClipboardAction {
    pub(super) fn command(id: CommandId) -> Option<Self> {
        use super::commands::{COPY, CUT, PASTE, PASTE_EFFECTS};
        match id {
            CUT => Some(Self::Cut),
            COPY => Some(Self::Copy),
            PASTE => Some(Self::Paste),
            PASTE_EFFECTS => Some(Self::PasteEffects),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq)]
struct Clipboard {
    ids: Rc<[TimelineId]>,
    cut: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct LaneRename {
    id: LaneId,
    begin: bool,
}

#[derive(Clone, Default, PartialEq)]
pub(super) struct State {
    scale: f32,
    origin: f32,
    framed_range: Option<[i32; 2]>,
    manual_pan: bool,
    selection: Option<TimelineId>,
    component: Option<(TakeComponent, Option<u32>)>,
    group: Option<Rc<SelectionGroup>>,
    clipboard: Option<Clipboard>,
    clipboard_note: Option<Rc<str>>,
    marquee: Option<Marquee>,
    overview_drag: Option<OverviewDrag>,
    context: Option<ContextMenu>,
    lane: Option<LaneId>,
    lane_rename: Option<LaneRename>,
    scrubbing: bool,
    blade: bool,
    expanded: Option<ClipId>,
    all_components: bool,
    free_move: bool,
    free_snap: bool,
    drag: Option<ItemDrag>,
    fade_drag: Option<FadeDrag>,
}

impl State {
    pub(super) fn command_selection(&self) -> &[TimelineId] {
        if self.component.is_some() {
            return &[];
        }
        self.selected()
    }

    pub(super) fn clipboard_enabled(&self, action: ClipboardAction) -> bool {
        match action {
            ClipboardAction::Cut | ClipboardAction::Copy => !self.command_selection().is_empty(),
            ClipboardAction::Paste | ClipboardAction::PasteHere(_) => self.clipboard.is_some(),
            ClipboardAction::PasteEffects => {
                self.clipboard.is_some() && !self.command_selection().is_empty()
            }
        }
    }

    fn clipboard_has_effects(&self, project: &Project) -> bool {
        let Some(clipboard) = &self.clipboard else {
            return false;
        };
        match clipboard.ids[0] {
            TimelineId::Clip(id) => project
                .clips
                .find(id)
                .and_then(|slot| project.clips.data(slot))
                .is_some_and(|clip| !clip.appearance.effects.is_empty()),
            id @ TimelineId::Item(_) => item_data(project, id)
                .is_some_and(|(_, _, item)| !item.appearance.effects.is_empty()),
            TimelineId::Marker(_) => false,
        }
    }

    pub(super) fn clipboard_operation(
        &mut self,
        action: ClipboardAction,
        project: &Project,
        at: i32,
    ) -> Option<splice_json::Value> {
        use splice_json::Value as Json;

        if matches!(action, ClipboardAction::Cut | ClipboardAction::Copy) {
            let selected = self.command_selection();
            let cut = action == ClipboardAction::Cut;
            if selected.is_empty() {
                self.clipboard_note = Some(if cut {
                    "Select items to cut.".into()
                } else {
                    "Select items to copy.".into()
                });
            } else {
                let ids: Rc<[TimelineId]> = Rc::from(selected);
                let marked = if cut { "Cut" } else { "Copied" };
                let paste = if cut { "moves them" } else { "pastes a copy" };
                self.clipboard_note = Some(
                    format!(
                        "{marked} {} item(s). ⌘V {paste} at the playhead.",
                        ids.len()
                    )
                    .into(),
                );
                self.clipboard = Some(Clipboard { ids, cut });
            }
            return None;
        }

        if action == ClipboardAction::PasteEffects {
            if let Some(marked) = &self.clipboard
                && !self.command_selection().is_empty()
            {
                return Some(Json::Object(vec![
                    ("cmd".into(), Json::String("effect.paste".into())),
                    ("from".into(), Json::String(marked.ids[0].to_string())),
                    ("to".into(), timeline_ids(self.command_selection())),
                ]));
            }
            self.clipboard_note =
                Some("Copy an item with effects (⌘C), select targets, then ⌥⌘V.".into());
            return None;
        }

        let Some(marked) = &self.clipboard else {
            self.clipboard_note = Some("Nothing to paste.".into());
            return None;
        };
        let live: Vec<_> = marked
            .ids
            .iter()
            .copied()
            .filter(|&id| timeline_exists(project, id))
            .collect();
        if live.is_empty() {
            self.clipboard_note = Some("The copied items no longer exist.".into());
            return None;
        }
        let position = Json::Unsigned(at.max(0) as u64);
        let items = timeline_ids(&live);
        Some(Json::Object(if marked.cut {
            vec![
                ("cmd".into(), Json::String("item.move".into())),
                ("items".into(), items),
                ("to".into(), position),
                ("ripple".into(), Json::Bool(self.ripple())),
            ]
        } else {
            vec![
                ("cmd".into(), Json::String("item.duplicate".into())),
                ("items".into(), items),
                ("at".into(), position),
            ]
        }))
    }

    pub(super) fn clipboard_committed(
        &mut self,
        action: ClipboardAction,
        changes: Vec<EditChange>,
    ) -> bool {
        let paste = matches!(
            action,
            ClipboardAction::Paste | ClipboardAction::PasteHere(_)
        );
        if paste && self.clipboard.as_ref().is_some_and(|marked| marked.cut) {
            self.clipboard = None;
        }
        let mut created = Vec::new();
        for change in changes {
            self.clipboard_note = Some(format!("{}.", change.summary).into());
            if paste {
                created.extend(
                    change
                        .created
                        .into_iter()
                        .filter_map(|target| match target {
                            Target::Timeline(id) => Some(id),
                            _ => None,
                        }),
                );
            }
        }
        if created.is_empty() {
            return false;
        }
        self.set_selected(created);
        true
    }

    pub(super) fn clipboard_failed(&mut self, error: &StoreError) {
        self.clipboard_note =
            Some(format!("{} · {} {}", error.code, error.message, error.fix).into());
    }
}

fn timeline_exists(project: &Project, id: TimelineId) -> bool {
    match id {
        TimelineId::Clip(id) => project.clips.find(id).is_some(),
        TimelineId::Item(id) => project.items.find(id).is_some(),
        TimelineId::Marker(id) => project.markers.iter().any(|marker| marker.id == id),
    }
}

#[derive(PartialEq, Eq)]
struct SelectionGroup {
    ids: Vec<TimelineId>,
    members: HashSet<TimelineId>,
}

#[derive(Clone, Copy, PartialEq)]
struct Marquee {
    start: [f32; 2],
    pointer: [f32; 2],
    additive: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct OverviewDrag {
    pointer: f32,
    origin: f32,
    restore: bool,
}

impl Marquee {
    fn moved(self) -> bool {
        let dx = self.pointer[0] - self.start[0];
        let dy = self.pointer[1] - self.start[1];
        dx * dx + dy * dy >= 9.0
    }

    fn rect(self) -> Rect {
        Rect::new(
            self.start[0].min(self.pointer[0]),
            self.start[1].min(self.pointer[1]),
            (self.pointer[0] - self.start[0]).abs(),
            (self.pointer[1] - self.start[1]).abs(),
        )
    }
}

#[derive(Clone, Copy, PartialEq)]
struct ItemDrag {
    item: TimelineId,
    revision: u64,
    edge: DragEdge,
    start: i32,
    origin: (f32, f32),
    moved: bool,
    to: i32,
    lane: Option<LaneId>,
    snap: Option<i32>,
}

#[derive(Clone, Copy, PartialEq)]
struct FadeDrag {
    item: TimelineId,
    revision: u64,
    edge: usize,
    origin: f32,
    value: i32,
    moved: bool,
    cancelled: bool,
}

#[derive(Clone, Copy)]
struct DropViewport {
    rect: Rect,
    scroll: f32,
}

#[derive(Clone, Copy, Default)]
struct DragOptions {
    lane: Option<LaneId>,
    bypass_snap: bool,
    leave_overlays: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MoveIntent {
    pub item: TimelineId,
    pub to: Option<i32>,
    pub ripple: bool,
    pub lane: Option<LaneId>,
    pub carry_overlays: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum DragEdge {
    Move,
    In,
    Out,
}

impl DragEdge {
    fn point(self, start: i32, length: i32) -> i32 {
        if self == Self::Out {
            start.saturating_add(length)
        } else {
            start
        }
    }

    fn bounds(self, start: i32, length: i32) -> (i32, i32) {
        match self {
            Self::Move => (0, i32::MAX.saturating_sub(length)),
            Self::In => (0, start.saturating_add(length - MIN_TRIM_LENGTH).max(0)),
            Self::Out => (start.saturating_add(MIN_TRIM_LENGTH), i32::MAX),
        }
    }

    fn preview(self, start: i32, length: i32, to: i32) -> (i32, i32) {
        match self {
            Self::Move => (to, length),
            Self::In => (to, start.saturating_add(length).saturating_sub(to)),
            Self::Out => (start, to.saturating_sub(start)),
        }
    }
}

impl State {
    pub(super) fn selection(&self) -> Option<TimelineId> {
        self.selection
    }

    pub(super) fn component(&self) -> Option<(ClipId, TakeComponent, Option<u32>)> {
        let Some(TimelineId::Clip(clip)) = self.selection else {
            return None;
        };
        let (component, key) = self.component?;
        Some((clip, component, key))
    }

    fn select_component(&mut self, clip: ClipId, part: TakeComponent, key: Option<u32>) {
        self.set_selection(Some(TimelineId::Clip(clip)), None);
        self.component = Some((part, key));
    }

    pub(super) fn selected(&self) -> &[TimelineId] {
        self.group
            .as_ref()
            .map_or(self.selection.as_slice(), |group| group.ids.as_slice())
    }

    pub(super) fn set_selected(&mut self, mut items: Vec<TimelineId>) {
        self.lane = None;
        self.component = None;
        if items.len() > 1 {
            let mut members = HashSet::with_capacity(items.len());
            items.retain(|&id| members.insert(id));
            if items.len() > 1 {
                self.selection = None;
                self.group = Some(Rc::new(SelectionGroup {
                    ids: items,
                    members,
                }));
                return;
            }
        }
        self.set_selection(items.first().copied(), None);
    }

    pub(super) fn retain_selection(&mut self, project: &Project) {
        let present = |id| timeline_exists(project, id);
        if self.selection.is_some_and(|id| !present(id)) {
            self.selection = None;
            self.component = None;
        }
        if let Some((clip, part, _)) = self.component()
            && !project
                .clips
                .find(clip)
                .and_then(|slot| project.clips.data(slot))
                .is_some_and(|data| {
                    components(data)
                        .iter()
                        .any(|row| row.kind == part && row.enabled.is_some())
                })
        {
            self.component = None;
        }
        if let Some(group) = &self.group
            && group.ids.iter().any(|&id| !present(id))
        {
            self.set_selected(
                group
                    .ids
                    .iter()
                    .copied()
                    .filter(|&id| present(id))
                    .collect(),
            );
        }
        if self.lane.is_some_and(|id| {
            id != project.spine.id && !project.lanes.iter().any(|lane| lane.id == id)
        }) {
            self.lane = None;
        }
    }

    fn contains(&self, id: TimelineId) -> bool {
        self.selection == Some(id)
            || self
                .group
                .as_ref()
                .is_some_and(|group| group.members.contains(&id))
    }

    fn toggle(&mut self, id: TimelineId) {
        if self.selected().is_empty() {
            self.set_selection(Some(id), None);
            return;
        }
        if self.selection == Some(id) {
            self.set_selection(None, None);
            return;
        }
        let mut items = self.selected().to_vec();
        if let Some(index) = items.iter().position(|&item| item == id) {
            items.remove(index);
        } else {
            items.push(id);
        }
        self.set_selected(items);
    }

    pub(super) fn set_selection(&mut self, item: Option<TimelineId>, lane: Option<LaneId>) {
        self.selection = item;
        self.component = None;
        self.group = None;
        self.lane = lane;
    }

    pub(super) fn select_tool(&mut self) {
        self.blade = false;
    }

    pub(super) fn toggle_blade(&mut self) {
        self.blade = !self.blade;
    }

    pub(super) fn toggle_snap(&mut self) {
        self.free_snap = !self.free_snap;
    }

    pub(super) fn ripple(&self) -> bool {
        !self.free_move
    }

    pub(super) fn zoom_about(&mut self, factor: f32, width: f32, at: i32, end: i32, fps: u32) {
        let geometry = Geometry::new(width, fps.max(1), end, self);
        geometry.zoom(self, geometry.scale * factor, at);
    }

    pub(super) fn zoom_range(&self, project: &Project) -> Option<[i32; 2]> {
        let mut spans = self.command_selection().iter().filter_map(|&id| {
            let (start, length) = match id {
                TimelineId::Clip(id) => {
                    let timing = project.clips.timing(project.clips.find(id)?)?;
                    (timing.start, timing.length)
                }
                TimelineId::Item(id) => {
                    let timing = project.items.timing(project.items.find(id)?)?;
                    (anchor_start(project, timing.anchor).ok()?, timing.length)
                }
                TimelineId::Marker(_) => return None,
            };
            Some([start, start.saturating_add(length)])
        });
        if let Some(first) = spans.next() {
            return Some(spans.fold(first, |[from, to], [start, end]| {
                [from.min(start), to.max(end)]
            }));
        }
        let TimelineId::Marker(id) = self.selection? else {
            return None;
        };
        let at = project.markers.iter().find(|marker| marker.id == id)?.at;
        Some([at.saturating_sub(30), at.saturating_add(30)])
    }

    pub(super) fn zoom_selection(&mut self, project: &Project, width: f32, end: i32) {
        let Some([from, to]) = self.zoom_range(project) else {
            return;
        };
        let geo = Geometry::new(width, project.fps.max(1), end, self);
        let span = (to as f32 - from as f32).max(2.0);
        self.framed_range = None;
        self.scale = (geo.width / (span * 1.2)).clamp(geo.fit, geo.max_scale);
        self.origin =
            (from as f32 - geo.width / (12.0 * self.scale)).clamp(0.0, geo.max_origin(self.scale));
    }

    pub(super) fn select_current_lane(&mut self, project: &Project) {
        let lane = self
            .selection
            .and_then(|id| item_data(project, id))
            .map_or(project.spine.id, |(_, _, item)| item.lane);
        select_lane(project, self, lane);
    }

    pub(super) fn nudge(&self, project: &Project, delta: i32) -> Option<ToolbarCommand> {
        if let Some(TimelineId::Marker(id)) = self.selection {
            let marker = project.markers.iter().find(|marker| marker.id == id)?;
            return Some(ToolbarCommand::MoveMarker(
                id,
                marker.at.saturating_add(delta),
            ));
        }
        let ids = self.command_selection();
        (!ids.is_empty()).then(|| ToolbarCommand::Nudge(Rc::from(ids), delta))
    }

    pub(super) fn can_escape(&self) -> bool {
        self.marquee.is_some()
            || self.overview_drag.is_some()
            || self.drag.is_some()
            || self.fade_drag.is_some()
            || self.scrubbing
            || self.blade
            || self.lane.is_some()
            || !self.selected().is_empty()
    }

    pub(super) fn fit(&mut self) {
        self.framed_range = None;
        self.scale = 0.0;
        self.origin = 0.0;
    }

    pub(super) fn resume_follow(&mut self) {
        self.manual_pan = false;
    }

    pub(super) fn follow(&mut self, width: f32, at: i32, end: i32, fps: u32) {
        if self.manual_pan || end <= 0 {
            return;
        }
        let geo = Geometry::new(width, fps.max(1), end, self);
        let at = at as f32;
        let x = geo.x(at);
        let origin = if x < 0.0 {
            at - 0.9 * geo.width / geo.scale
        } else if x > geo.width - 1.0 {
            at
        } else {
            return;
        };
        self.framed_range = None;
        self.scale = geo.scale;
        self.origin = origin;
    }

    fn pan_wheel(&mut self, geo: Geometry, points: f32) {
        self.framed_range = None;
        self.scale = geo.scale;
        self.origin = (geo.origin + points / geo.scale).clamp(0.0, geo.max_origin(geo.scale));
        self.manual_pan = true;
        self.context = None;
    }

    pub(super) fn menu_open(&self) -> bool {
        self.context.is_some()
    }

    fn open_menu(&mut self, target: ContextTarget, (x, y): (f32, f32)) {
        self.context = Some(ContextMenu {
            target,
            anchor: Point::new(x, y),
        });
    }

    fn begin_lane_rename(&mut self, id: LaneId) {
        self.lane_rename = Some(LaneRename { id, begin: true });
    }

    pub(super) fn frame_range(&mut self, range: [i32; 2]) {
        self.framed_range = Some(range);
    }

    pub(super) fn expand(&mut self, id: ClipId) {
        if !self.all_components {
            self.expanded = Some(id);
        }
    }

    fn toggle_take(&mut self, id: ClipId) {
        self.expanded = if self.all_components || self.expanded == Some(id) {
            None
        } else {
            Some(id)
        };
        self.all_components = false;
    }
    pub(super) fn lane(&self) -> Option<LaneId> {
        self.lane
    }
}

#[derive(Clone)]
pub(super) struct View {
    pub width: f32,
    pub height: f32,
    pub at: i32,
    pub end: i32,
    pub dragging: bool,
    pub history: (bool, bool),
    pub state: State,
}

#[derive(Clone, Default, PartialEq)]
pub(super) struct Intents {
    pub bounds: Rect,
    pub scrub: Option<(Phase, i32)>,
    pub split: Option<(TimelineId, i32)>,
    pub crop: Option<TimelineId>,
    pub clipboard: Option<ClipboardAction>,
    pub toolbar: Option<ToolbarCommand>,
    pub history: Option<CommandId>,
    pub moved: Option<MoveIntent>,
    pub trimmed: Option<(TimelineId, &'static str, i32, bool)>,
    marquee_seen: bool,
    overview_seen: Option<OverviewDrag>,
    drag_seen: bool,
    fade_seen: bool,
    rename_painted: bool,
    blade_hover: Option<(i32, f32, f32)>,
    pub component: Option<(ClipId, &'static str, bool)>,
    pub inspect_selection: bool,
    pub inspector_focus: Option<super::inspector::Focus>,
    pub effect: Option<viewer_drawers::Effect>,
    pub state: State,
}

impl Intents {
    // PXUI settles layout by rebuilding with the same frame's events removed.
    pub(super) fn merge(&mut self, next: Self) {
        self.scrub = next.scrub.or(self.scrub);
        self.split = next.split.or(self.split);
        self.crop = next.crop.or(self.crop);
        self.clipboard = next.clipboard.or(self.clipboard);
        self.toolbar = next.toolbar.or_else(|| self.toolbar.take());
        self.history = next.history.or(self.history);
        self.moved = next.moved.or(self.moved);
        self.trimmed = next.trimmed.or(self.trimmed);
        self.marquee_seen |= next.marquee_seen;
        self.overview_seen = next.overview_seen.or(self.overview_seen);
        self.component = next.component.or(self.component);
        self.inspect_selection |= next.inspect_selection;
        self.inspector_focus = next.inspector_focus.or(self.inspector_focus);
        self.effect = next.effect.or_else(|| self.effect.take());
        self.bounds = next.bounds;
        let requested_rename = self.state.lane_rename.filter(|rename| rename.begin);
        self.state = next.state;
        // A menu choice or double-click can request the next frame's editor
        // before a settling pass rebuilds the unedited header with no input.
        if !next.rename_painted {
            self.state.lane_rename = self.state.lane_rename.or(requested_rename);
        }
    }

    pub(super) fn escape(&mut self, prior: &State, at: i32) -> bool {
        let state = &mut self.state;
        if self.marquee_seen || state.marquee.take().is_some() || prior.marquee.is_some() {
            state.marquee = None;
            state.selection = prior.selection;
            state.group.clone_from(&prior.group);
            state.component = prior.component;
            state.lane = prior.lane;
        } else if let Some(drag) = state
            .overview_drag
            .take()
            .or(prior.overview_drag)
            .or(self.overview_seen)
        {
            if drag.restore {
                state.origin = drag.origin;
            }
        } else if state.drag.is_some()
            || prior.drag.is_some()
            || state.fade_drag.is_some()
            || prior.fade_drag.is_some()
            || self.moved.is_some()
            || self.trimmed.is_some()
            || matches!(self.toolbar, Some(ToolbarCommand::SetFade(..)))
        {
            state.drag = None;
            state.fade_drag = None;
            self.moved = None;
            self.trimmed = None;
            if matches!(self.toolbar, Some(ToolbarCommand::SetFade(..))) {
                self.toolbar = None;
            }
        } else if state.scrubbing || prior.scrubbing || self.scrub.is_some() {
            state.scrubbing = false;
            self.scrub = Some((Phase::Cancelled, at));
        } else if state.blade {
            state.select_tool();
        } else {
            state.set_selection(None, None);
            return true;
        }
        false
    }

    fn inspect_component(&mut self, clip: ClipId, part: TakeComponent, key: Option<u32>) {
        self.state.select_component(clip, part, key);
        self.inspect_selection = true;
    }
}

pub(super) struct MediaPreview {
    pub id: MediaId,
    pub thumbnail: Option<Image>,
    pub waveform: Vec<f32>,
    pub camera: Option<Image>,
    pub camera_only: bool,
    pub mic: Vec<f32>,
    pub system: Vec<f32>,
    pub events: splice_av::TimelineEvents,
}

impl MediaPreview {
    fn filmstrip(&self) -> Option<Image> {
        self.thumbnail.or(self.camera.filter(|_| self.camera_only))
    }
}

#[derive(Default)]
struct BlockMedia {
    thumbnail: Option<Image>,
    waveform: Option<PeakColumns>,
}

#[derive(Default)]
struct BlockDecor {
    fades: [i32; 2],
    fade_handles: bool,
    pinned: bool,
    behind: bool,
    speed: Option<f64>,
    duration: Option<(i32, u32)>,
    placement: Option<&'static str>,
    motion: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FxBadgeKind {
    Local,
    Inherited,
    Override,
}

struct FxBadge {
    kind: FxBadgeKind,
    tip: String,
}

impl FxBadge {
    fn width(&self) -> f32 {
        if self.kind == FxBadgeKind::Override {
            26.0
        } else {
            20.0
        }
    }

    fn lane(effects: &[Effect]) -> Option<Self> {
        (!effects.is_empty()).then(|| Self {
            kind: FxBadgeKind::Local,
            tip: format!("{} lane effect(s)", effects.len()),
        })
    }

    fn resolve(
        scopes: [(&str, &[Effect]); 3],
        clip: bool,
        applies: &dyn Fn(&str) -> bool,
    ) -> Option<Self> {
        if scopes.iter().all(|(_, effects)| effects.is_empty()) {
            return None;
        }
        let local = scopes[2].1;
        let mut tip = String::new();
        let mut override_here = false;
        let schemas = splice_core::commands::effect_schemas();
        for (kind, schema) in schemas.as_object().into_iter().flatten() {
            let Some(class) = schema.field("applies").and_then(splice_json::Value::as_str) else {
                continue;
            };
            if !applies(class) {
                continue;
            }
            // Window on a media overlay takes its lane as the outer scope. The editor's
            // other inherited effects follow the evaluator's project ? lane ? owner chain.
            let included = match kind.as_str() {
                "window" => [clip, true, true],
                "cursor" | "camera" | "captions" | "keystrokes" | "color" | "volume" | "eq"
                | "compressor" | "gate" | "voiceIsolation" | "text" | "animate" | "layout"
                | "transform" => [true, true, true],
                _ => [false, false, true],
            };
            let composed = kind == "transform";
            let multiple = schema
                .field("multiple")
                .and_then(splice_json::Value::as_bool)
                == Some(true);
            let mut instances: [Option<&Effect>; 3] = [None; 3];
            for (index, ((_, effects), instance)) in scopes.iter().zip(&mut instances).enumerate() {
                if included[index] {
                    *instance = effects.iter().find(|effect| {
                        effect.kind == *kind
                            && (!(composed || multiple) || effect.enabled != Some(false))
                    });
                }
            }
            if instances.iter().all(Option::is_none)
                || instances
                    .iter()
                    .rev()
                    .flatten()
                    .find_map(|effect| effect.enabled)
                    == Some(false)
            {
                continue;
            }
            override_here |= !composed
                && instances[2].is_some()
                && (instances[0].is_some() || instances[1].is_some());
            tip.push_str(if tip.is_empty() { "Effects: " } else { " · " });
            tip.push_str(
                schema
                    .field("title")
                    .and_then(splice_json::Value::as_str)
                    .unwrap_or(kind),
            );
            let mut separator = " (";
            for ((name, _), instance) in scopes.iter().zip(instances) {
                if instance.is_some() {
                    tip.push_str(separator);
                    tip.push_str(name);
                    separator = " › ";
                }
            }
            tip.push(')');
        }
        if local.iter().any(|effect| effect.enabled == Some(false)) {
            tip.push_str(if tip.is_empty() { "Effects: " } else { " · " });
            tip.push_str("some turned off here");
        }
        let kind = if override_here {
            FxBadgeKind::Override
        } else if !local.is_empty() {
            if tip.is_empty() {
                tip.push_str("Effects set here");
            }
            FxBadgeKind::Local
        } else if !tip.is_empty() {
            FxBadgeKind::Inherited
        } else {
            return None;
        };
        Some(Self { kind, tip })
    }
}

fn clip_fx_applies(class: &str, media: Option<&Media>) -> bool {
    let Some(media) = media else { return false };
    match class {
        "take" => media.kind == MediaKind::Take,
        "visual" | "layer" | "spine" => media.kind != MediaKind::Audio,
        "audio" => media.kind == MediaKind::Take || !media.probe.audio.is_empty(),
        _ => false,
    }
}

fn item_fx_applies(class: &str, kind: &ItemKind) -> bool {
    match class {
        "caption" => matches!(kind, ItemKind::Caption { .. }),
        "text" => matches!(kind, ItemKind::Caption { .. } | ItemKind::Title { .. }),
        "visual" => matches!(kind, ItemKind::Media(_)),
        "layer" => !matches!(
            kind,
            ItemKind::Sound(_) | ItemKind::Zoom(_) | ItemKind::Motion(_)
        ),
        "audio" => matches!(kind, ItemKind::Sound(_) | ItemKind::Media(_)),
        "shape" => matches!(kind, ItemKind::Shape { .. }),
        "title" => matches!(kind, ItemKind::Title { .. }),
        "template" => true,
        _ => false,
    }
}

const WAVE_COLUMNS: usize = 256;

struct PeakColumns {
    heights: [u8; WAVE_COLUMNS],
    count: usize,
}

pub(super) fn preview_for(previews: &[MediaPreview], id: MediaId) -> Option<&MediaPreview> {
    previews
        .binary_search_by_key(&id.serial(), |preview| preview.id.serial())
        .ok()
        .map(|index| &previews[index])
}

fn audio_peaks(
    previews: &[MediaPreview],
    project: &Project,
    source: (MediaId, i32, f64, f64),
    (geo, start, x, width): (Geometry, i32, f32, f32),
) -> Option<PeakColumns> {
    let (id, source_in, speed, gain) = source;
    let peak = preview_for(previews, id)?;
    let media = project.media.iter().find(|media| media.id == id)?;
    if media.probe.frames <= 0 || peak.waveform.is_empty() || width < 3.0 {
        return None;
    }
    let rate = media.probe.fps * speed / f64::from(project.fps.max(1));
    let source_at = |pixel: f32| {
        let time = f64::from((pixel - geo.labels) / geo.scale + geo.origin - start as f32);
        (f64::from(source_in) + time * rate) / f64::from(media.probe.frames)
    };
    Some(peak_columns(
        &peak.waveform,
        [source_at(x), source_at(x + width)],
        width,
        gain,
    ))
}

fn peak_columns(wave: &[f32], [from, to]: [f64; 2], width: f32, gain: f64) -> PeakColumns {
    let count = column_count(width);
    columns_for(wave, count, gain, &|column| {
        from + (to - from) * column / count as f64
    })
}

fn column_count(width: f32) -> usize {
    ((width / 3.0).ceil() as usize).clamp(1, WAVE_COLUMNS)
}

fn columns_for(
    wave: &[f32],
    count: usize,
    gain: f64,
    source_at: &dyn Fn(f64) -> f64,
) -> PeakColumns {
    let mut heights = [0; WAVE_COLUMNS];
    let len = wave.len() as f64;
    let mut left = source_at(0.0) * len;
    for (column, height) in heights[..count].iter_mut().enumerate() {
        let right = source_at((column + 1) as f64) * len;
        let first = left.floor().max(0.0) as usize;
        let last = right.ceil().max(first as f64 + 1.0).min(len) as usize;
        let maximum = wave
            .get(first..last)
            .unwrap_or_default()
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .fold(0.0_f32, f32::max);
        *height = (f64::from(maximum) * gain * 255.0)
            .clamp(0.0, 255.0)
            .round() as u8;
        left = right;
    }
    PeakColumns { heights, count }
}

fn take_component_preview(
    cache: &mut Cache,
    previews: &[MediaPreview],
    project: &Project,
    slot: Slot,
    timing: ClipTiming,
    component: Component,
    enabled: bool,
    (geo, x, width): (Geometry, f32, f32),
) -> BlockMedia {
    let mut visual = BlockMedia::default();
    if !enabled {
        return visual;
    }
    let Some((data, id)) = project
        .clips
        .data(slot)
        .and_then(|data| data.media.map(|id| (data, id)))
    else {
        return visual;
    };
    let Some(preview) = preview_for(previews, id) else {
        return visual;
    };
    match component.kind {
        TakeComponent::Screen => visual.thumbnail = preview.thumbnail,
        TakeComponent::Camera => visual.thumbnail = preview.camera,
        TakeComponent::Mic | TakeComponent::System => {
            let wave = if component.kind == TakeComponent::Mic {
                &preview.mic
            } else {
                &preview.system
            };
            let Some(media) = project.media.iter().find(|media| media.id == id) else {
                return visual;
            };
            if wave.is_empty() || media.probe.frames <= 0 || width < 3.0 {
                return visual;
            }
            let first = f64::from((x - geo.labels) / geo.scale + geo.origin - timing.start as f32);
            let last = first + f64::from(width / geo.scale);
            let first = first.clamp(0.0, f64::from(timing.length));
            let last = last.clamp(first, f64::from(timing.length));
            let gain = component.gain.unwrap_or(1.0);
            let media_frames = f64::from(media.probe.frames);
            let source_in = f64::from(timing.source);
            let keyed = data
                .appearance
                .effects
                .iter()
                .find(|effect| effect.kind == "speed" && effect.enabled != Some(false))
                .is_some_and(|effect| {
                    effect.params.iter().any(|parameter| {
                        parameter.name == "rate" && matches!(parameter.value, EffectValue::Keys(_))
                    })
                });
            visual.waveform = if keyed {
                let count = column_count(width);
                cache
                    .keyed_sources(project, slot, [first, last], count, source_in, media_frames)
                    .map(|sources| {
                        columns_for(wave, count, gain, &|column| sources[column as usize])
                    })
            } else {
                consumed(project, slot, 1.0).ok().map(|rate| {
                    peak_columns(
                        wave,
                        [
                            (source_in + rate * first) / media_frames,
                            (source_in + rate * last) / media_frames,
                        ],
                        width,
                        gain,
                    )
                })
            };
        }
        _ => {}
    }
    visual
}

struct KeyedWaveMap {
    slot: Slot,
    local: [f64; 2],
    count: usize,
    source: [f64; WAVE_COLUMNS + 1],
}

#[derive(Default)]
pub(super) struct Cache {
    pub(super) wheel: WheelInput,
    project: Option<(*const Project, u64, u64)>,
    items: Vec<IndexedItem>,
    clip_fx: Vec<(Slot, FxBadge)>,
    lane_fx: Vec<(LaneId, FxBadge)>,
    guides: Rc<TrackGuides>,
    has_components: bool,
    snap_points: Vec<SnapPoint>,
    anchored_clips: Vec<u64>,
    overview_size: Option<(f32, f32)>,
    overview: Vec<OverviewClip>,
    keyed_waves: Vec<KeyedWaveMap>,
    take_events: Vec<TakeEventTrack>,
    rows: Vec<TrackRow>,
    rows_for: Option<(Option<ClipId>, bool)>,
    rows_height: f32,
}

#[derive(Clone, Copy)]
enum Track {
    Lane(usize),
    Spine,
    Take(Slot),
    Component(Slot, Component),
}

#[derive(Clone, Copy)]
struct TrackRow {
    track: Track,
    top: f32,
    height: f32,
}

struct TakeEventTrack {
    slot: Slot,
    rings: bool,
    categories: u8,
    hidden: Vec<u32>,
}

struct SnapPoint {
    frame: i32,
    item: TimelineId,
}

struct IndexedItem {
    lane: u64,
    order: usize,
    slot: Slot,
    start: Option<i32>,
    length: i32,
    label: Option<String>,
    caption: Option<CaptionChrome>,
    fx: Option<FxBadge>,
}

struct CaptionChrome {
    ticks: Rc<[CaptionTick]>,
    words: Vec<CaptionMenuWord>,
}

struct CaptionMenuWord {
    id: WordId,
    text: String,
    at: i32,
    emph: bool,
    hidden: bool,
}

struct CaptionTick {
    at: i32,
    emph: bool,
}

#[derive(Clone, Default)]
struct TrackGuides {
    markers: Vec<i32>,
    anchors: Vec<AnchorGuide>,
    spine_end: f32,
}

#[derive(Clone, Copy)]
struct AnchorGuide {
    item: TimelineId,
    frame: i32,
    top: f32,
    tone: Rgba,
}

struct OverviewClip {
    slot: Slot,
    x: f32,
    width: f32,
}

impl Cache {
    fn refresh(&mut self, project: Option<&Project>, geo: Geometry, fps: u32) -> bool {
        let key = project.map(|project| (std::ptr::from_ref(project), project.rev, project.hash));
        let changed = self.project != key;
        if changed {
            self.project = key;
            self.overview_size = None;
            self.rows_for = None;
            self.items.clear();
            self.snap_points.clear();
            self.anchored_clips.clear();
            self.keyed_waves.clear();
            self.take_events.clear();
            self.has_components = project.is_some_and(|project| {
                project
                    .clips
                    .order()
                    .iter()
                    .any(|&slot| project.clips.data(slot).is_some_and(has_components))
            });
            if let Some(project) = project {
                self.items.reserve(project.items.order().len());
                for (order, &slot) in project.items.order().iter().enumerate() {
                    let Some(data) = project.items.data(slot) else {
                        continue;
                    };
                    let Some(timing) = project.items.timing(slot) else {
                        continue;
                    };
                    if let Some(clip) = clip_anchor(timing.anchor) {
                        self.anchored_clips.push(clip.serial());
                    }
                    let start = anchor_start(project, timing.anchor).ok();
                    let (label, caption) = resolved_item_chrome(project, data, timing, start);
                    self.items.push(IndexedItem {
                        lane: data.lane.serial(),
                        order,
                        slot,
                        start,
                        length: timing.length,
                        label,
                        caption,
                        fx: None,
                    });
                    if let Some((start, id)) = start.zip(project.items.id(slot)) {
                        self.add_snap_span(TimelineId::Item(id), start, timing.length);
                    }
                }
                self.items
                    .sort_unstable_by_key(|item| (item.lane, item.order));
                self.anchored_clips.sort_unstable();
                for &slot in project.clips.order() {
                    if let Some((id, timing)) =
                        project.clips.id(slot).zip(project.clips.timing(slot))
                    {
                        self.add_snap_span(TimelineId::Clip(id), timing.start, timing.length);
                    }
                    if let Some(data) = project.clips.data(slot)
                        && (data.enabled.pointer == Some(true) || data.enabled.keys == Some(true))
                    {
                        let (rings, categories) = splice_av::timeline_event_visibility(&[
                            &project.effects,
                            &project.spine.effects,
                            &data.appearance.effects,
                        ]);
                        let mut hidden = data.hidden_keys.clone();
                        hidden.sort_unstable();
                        self.take_events.push(TakeEventTrack {
                            slot,
                            rings,
                            categories,
                            hidden,
                        });
                    }
                }
                self.take_events.sort_unstable_by_key(|entry| entry.slot);
                self.snap_points
                    .extend(project.markers.iter().map(|marker| SnapPoint {
                        frame: marker.at,
                        item: TimelineId::Marker(marker.id),
                    }));
                self.snap_points.sort_unstable_by_key(|point| point.frame);
            }
            self.refresh_fx(project);
            self.refresh_guides(project);
        }
        let size = (geo.overview_width(), geo.end.max(fps as f32));
        if self.overview_size != Some(size) {
            self.overview_size = Some(size);
            self.overview.clear();
            if let Some(project) = project {
                let (width, duration) = size;
                for &slot in project.clips.order() {
                    if let Some(timing) = project.clips.timing(slot) {
                        let clip_width = width * timing.length as f32 / duration - 1.0;
                        if clip_width > 0.0 {
                            self.overview.push(OverviewClip {
                                slot,
                                x: width * timing.start as f32 / duration,
                                width: clip_width,
                            });
                        }
                    }
                }
            }
        }
        changed
    }

    fn refresh_fx(&mut self, project: Option<&Project>) {
        self.clip_fx.clear();
        self.lane_fx.clear();
        let Some(project) = project else { return };
        let global = ("Project", project.effects.as_slice());
        let spine = ("Video lane", project.spine.effects.as_slice());
        if let Some(badge) = FxBadge::lane(spine.1) {
            self.lane_fx.push((project.spine.id, badge));
        }
        for &slot in project.clips.order() {
            let Some(data) = project.clips.data(slot) else {
                continue;
            };
            let local = data.appearance.effects.as_slice();
            if global.1.is_empty() && spine.1.is_empty() && local.is_empty() {
                continue;
            }
            let media = data
                .media
                .and_then(|id| project.media.iter().find(|media| media.id == id));
            if let Some(badge) =
                FxBadge::resolve([global, spine, ("this item", local)], true, &|class| {
                    clip_fx_applies(class, media)
                })
            {
                self.clip_fx.push((slot, badge));
            }
        }
        self.clip_fx.sort_unstable_by_key(|(slot, _)| *slot);

        for lane in &project.lanes {
            if let Some(badge) = FxBadge::lane(&lane.effects) {
                self.lane_fx.push((lane.id, badge));
            }
            let name = if lane.name.is_empty() {
                "lane"
            } else {
                &lane.name
            };
            let scope = (name, lane.effects.as_slice());
            let range = indexed_lane_range(&self.items, lane.id);
            for item in &mut self.items[range] {
                let Some(data) = project.items.data(item.slot) else {
                    continue;
                };
                let local = data.appearance.effects.as_slice();
                // V13 puts inherited caption style on the lane header. A cue shows FX
                // when it carries a definition of its own.
                if local.is_empty() && matches!(data.kind, ItemKind::Caption { .. }) {
                    continue;
                }
                item.fx =
                    FxBadge::resolve([global, scope, ("this item", local)], false, &|class| {
                        item_fx_applies(class, &data.kind)
                    });
            }
        }
        self.lane_fx.sort_unstable_by_key(|(id, _)| id.serial());
    }

    fn clip_fx(&self, slot: Slot) -> Option<&FxBadge> {
        let index = self
            .clip_fx
            .binary_search_by_key(&slot, |(slot, _)| *slot)
            .ok()?;
        Some(&self.clip_fx[index].1)
    }

    fn lane_fx(&self, id: LaneId) -> Option<&FxBadge> {
        let index = self
            .lane_fx
            .binary_search_by_key(&id.serial(), |(id, _)| id.serial())
            .ok()?;
        Some(&self.lane_fx[index].1)
    }

    fn refresh_guides(&mut self, project: Option<&Project>) {
        let guides = Rc::make_mut(&mut self.guides);
        guides.markers.clear();
        guides.anchors.clear();
        let Some(project) = project else {
            return;
        };
        guides.spine_end = project.lanes.len() as f32 * LANE_HEIGHT + 5.0;
        for (index, lane) in project.lanes.iter().enumerate() {
            let top = (index + 1) as f32 * LANE_HEIGHT - 3.0;
            let tone = lane_tone(lane.kind);
            for item in indexed_lane(&self.items, lane.id) {
                if let Some(frame) = item.start
                    && let Some(timing) = project.items.timing(item.slot)
                    && !matches!(timing.anchor, Anchor::Pinned(_))
                    && let Some(id) = project.items.id(item.slot)
                {
                    guides.anchors.push(AnchorGuide {
                        item: TimelineId::Item(id),
                        frame,
                        top,
                        tone,
                    });
                }
            }
        }
        guides
            .anchors
            .sort_unstable_by(|a, b| a.frame.cmp(&b.frame).then_with(|| a.top.total_cmp(&b.top)));
        guides
            .markers
            .extend(project.markers.iter().map(|marker| marker.at));
        guides.markers.sort_unstable();
    }

    fn keyed_sources(
        &mut self,
        project: &Project,
        slot: Slot,
        local: [f64; 2],
        count: usize,
        source_in: f64,
        media_frames: f64,
    ) -> Option<&[f64]> {
        let index = self
            .keyed_waves
            .iter()
            .position(|map| map.slot == slot)
            .unwrap_or_else(|| {
                self.keyed_waves.push(KeyedWaveMap {
                    slot,
                    local,
                    count: 0,
                    source: [0.0; WAVE_COLUMNS + 1],
                });
                self.keyed_waves.len() - 1
            });
        let map = &mut self.keyed_waves[index];
        if map.local != local || map.count != count {
            let [first, last] = local;
            for (column, source) in map.source[..=count].iter_mut().enumerate() {
                let time = first + (last - first) * column as f64 / count as f64;
                *source = (source_in + consumed(project, slot, time).ok()?) / media_frames;
            }
            map.local = local;
            map.count = count;
        }
        Some(&map.source[..=count])
    }

    fn refresh_rows(&mut self, project: &Project, state: &State) {
        let key = (state.expanded, state.all_components);
        if self.rows_for == Some(key) {
            return;
        }
        self.rows_for = Some(key);
        self.rows.clear();
        self.rows_height = 0.0;
        for i in 0..project.lanes.len() {
            self.push_row(Track::Lane(i), LANE_HEIGHT);
        }
        self.push_row(Track::Spine, SPINE_HEIGHT);
        if state.all_components {
            for &slot in project.clips.order() {
                self.push_take(project, slot);
            }
        } else if let Some(slot) = state.expanded.and_then(|id| project.clips.find(id)) {
            self.push_take(project, slot);
        }
    }

    fn push_take(&mut self, project: &Project, slot: Slot) {
        let Some(data) = project.clips.data(slot).filter(|data| has_components(data)) else {
            return;
        };
        self.push_row(Track::Take(slot), TAKE_HEAD_HEIGHT);
        for component in components(data)
            .into_iter()
            .filter(|component| component.enabled.is_some())
        {
            self.push_row(Track::Component(slot, component), component.height());
        }
    }

    fn push_row(&mut self, track: Track, height: f32) {
        self.rows.push(TrackRow {
            track,
            top: self.rows_height,
            height,
        });
        self.rows_height += height;
    }

    fn first_visible_row(&self, top: f32) -> usize {
        self.rows
            .partition_point(|row| row.top + row.height <= top - TRACK_OVERSCAN)
    }

    fn add_snap_span(&mut self, item: TimelineId, start: i32, length: i32) {
        self.snap_points.push(SnapPoint { frame: start, item });
        self.snap_points.push(SnapPoint {
            frame: start.saturating_add(length),
            item,
        });
    }

    pub(super) fn edit_point(
        &self,
        project: &Project,
        at: i32,
        end: i32,
        forward: bool,
    ) -> Option<i32> {
        if end <= 0 || (forward && at >= end - 1) || (!forward && at <= 0) {
            return None;
        }
        let boundary = if forward { end } else { 0 };
        let key = (std::ptr::from_ref(project), project.rev, project.hash);
        let target = if self.project == Some(key) {
            let points = &self.snap_points;
            let edge = if forward {
                points.get(points.partition_point(|point| point.frame <= at))
            } else {
                points[..points.partition_point(|point| point.frame < at)].last()
            };
            edge.map_or(boundary, |point| point.frame)
        } else {
            // An edit can commit before the next paint refreshes the timeline index.
            let mut target = boundary;
            let mut include = |frame| {
                let nearer = if forward {
                    at < frame && frame < target
                } else {
                    target < frame && frame < at
                };
                if nearer {
                    target = frame;
                }
            };
            for &slot in project.clips.order() {
                if let Some(clip) = project.clips.timing(slot) {
                    include(clip.start);
                    if let Some(end) = clip.end() {
                        include(end);
                    }
                }
            }
            for &slot in project.items.order() {
                if let Some(item) = project.items.timing(slot)
                    && let Ok(start) = anchor_start(project, item.anchor)
                {
                    include(start);
                    include(start.saturating_add(item.length));
                }
            }
            for marker in &project.markers {
                include(marker.at);
            }
            target
        };
        Some(target.clamp(0, end - 1))
    }

    fn snap_drag(
        &self,
        item: TimelineId,
        to: i32,
        swing: Option<i32>,
        bounds: (i32, i32),
        at: i32,
        scale: f32,
    ) -> (i32, Option<i32>) {
        let threshold = (6.0 / scale).floor() as i32;
        let mut best = threshold.saturating_add(1);
        let mut adjust = 0;
        let mut snapped = None;
        for edge in std::iter::once(to).chain(swing.map(|length| to.saturating_add(length))) {
            let from = self
                .snap_points
                .partition_point(|point| point.frame < edge.saturating_sub(threshold));
            for target in [at, 0].into_iter().chain(
                self.snap_points[from..]
                    .iter()
                    .take_while(|point| point.frame <= edge.saturating_add(threshold))
                    .filter(|point| point.item != item)
                    .map(|point| point.frame),
            ) {
                let delta = target.saturating_sub(edge);
                if delta.saturating_abs() < best
                    && (bounds.0..=bounds.1).contains(&to.saturating_add(delta))
                {
                    best = delta.saturating_abs();
                    adjust = delta;
                    snapped = Some(target);
                }
            }
        }
        (to.saturating_add(adjust), snapped)
    }

    fn lane(&self, lane: LaneId) -> &[IndexedItem] {
        indexed_lane(&self.items, lane)
    }

    fn anchored_count(&self, clip: ClipId) -> usize {
        let id = clip.serial();
        let first = self.anchored_clips.partition_point(|&key| key < id);
        self.anchored_clips[first..].partition_point(|&key| key == id)
    }
}

fn indexed_lane(items: &[IndexedItem], lane: LaneId) -> &[IndexedItem] {
    &items[indexed_lane_range(items, lane)]
}

fn indexed_lane_range(items: &[IndexedItem], lane: LaneId) -> std::ops::Range<usize> {
    let id = lane.serial();
    let from = items.partition_point(|item| item.lane < id);
    let count = items[from..].partition_point(|item| item.lane == id);
    from..from + count
}

#[derive(Clone, Copy)]
struct Geometry {
    labels: f32,
    width: f32,
    end: f32,
    origin: f32,
    scale: f32,
    fit: f32,
    max_scale: f32,
}

impl Geometry {
    fn new(width: f32, fps: u32, end: i32, state: &State) -> Self {
        let labels = if width <= 480.0 {
            COMPACT_LABEL_WIDTH
        } else {
            LABEL_WIDTH
        };
        let labels = labels.min(width * 0.5);
        let width = (width - labels).max(1.0);
        let end = end.max(0) as f32;
        let fit = width / (end.max(fps as f32) * 1.04);
        let max_scale = (480.0 / fps as f32).max(fit);
        let mut scale = state.scale.clamp(fit, max_scale);
        let mut requested_origin = state.origin;
        if let Some([from, to]) = state.framed_range {
            let from = (from as f32).clamp(0.0, end);
            let to = (to as f32).clamp(from, end);
            if to > from {
                scale = (width / (to - from)).clamp(fit, max_scale);
                requested_origin = from + ((to - from) - width / scale) * 0.5;
            }
        }
        let origin = requested_origin.clamp(0.0, timeline_max_origin(end, width, scale));
        Self {
            labels,
            width,
            end,
            origin,
            scale,
            fit,
            max_scale,
        }
    }

    fn x(self, frame: f32) -> f32 {
        (frame - self.origin) * self.scale
    }

    fn overview_width(self) -> f32 {
        let width = self.width + self.labels;
        let fraction = if width <= 480.0 {
            1.0
        } else if width <= 820.0 {
            0.38
        } else {
            0.33
        };
        (width - 16.0).max(1.0) * fraction
    }

    fn visible(self, start: i32, length: i32) -> Option<(f32, f32)> {
        let lo = self.x(start as f32).max(0.0);
        let hi = self.x(start.saturating_add(length) as f32).min(self.width);
        (hi > lo).then_some((self.labels + lo, hi - lo))
    }

    fn ticks(self, fps: u32) -> i32 {
        let steps = TICK_STEPS.map(|step| ((step * fps as f32 / 30.0).round() as i32).max(1));
        steps
            .into_iter()
            .find(|&step| step as f32 * self.scale >= 70.0)
            .unwrap_or_else(|| {
                let last = steps[11] as f32;
                let multiple = (70.0 / (last * self.scale)).ceil();
                (last * multiple) as i32
            })
    }

    fn max_origin(self, scale: f32) -> f32 {
        timeline_max_origin(self.end, self.width, scale)
    }

    fn zoom(self, state: &mut State, scale: f32, at: i32) {
        self.zoom_at_x(state, scale, self.x(at as f32));
    }

    fn zoom_at_x(self, state: &mut State, scale: f32, x: f32) {
        let x = x.clamp(0.0, self.width);
        state.framed_range = None;
        state.scale = scale.clamp(self.fit, self.max_scale);
        state.origin = self.origin + x / self.scale - x / state.scale;
    }
}

fn timeline_max_origin(end: f32, width: f32, scale: f32) -> f32 {
    (end - width * 0.88 / scale).max(0.0)
}

#[derive(Clone, Copy)]
struct WheelArea {
    ruler: Handle,
    tracks: Handle,
    width: f32,
    fps: u32,
    end: i32,
    overflow: bool,
}

#[derive(Default)]
pub(super) struct WheelInput {
    area: Option<WheelArea>,
    pointer: Option<Point>,
    modifiers: u8,
    precise: bool,
    track_travel: f32,
    ruler_scroll: f32,
}

impl WheelInput {
    pub(super) fn begin(&mut self, events: &[Event]) {
        // A native gesture can arrive with SDL wheel steps in the same batch.
        self.precise = events.iter().any(
            |event| matches!(event, Event::TrackpadScrolled { delta, .. } if *delta != (0.0, 0.0)),
        );
        self.track_travel = 0.0;
    }

    pub(super) fn route(
        &mut self,
        ui: &Ui,
        state: &mut State,
        event: &Event,
        active: bool,
    ) -> bool {
        const SHIFT: u8 = 1;
        const CTRL: u8 = 2;
        const META: u8 = 4;
        let (delta, lines, momentum) = match *event {
            Event::PointerMoved((x, y))
            | Event::PointerPressed(_, (x, y))
            | Event::PointerReleased(_, (x, y)) => {
                let (x, y) = (x as f32, y as f32);
                self.pointer = (x.is_finite() && y.is_finite()).then_some(Point::new(x, y));
                return false;
            }
            Event::KeyPressed(key) | Event::KeyReleased(key) => {
                let flag = match key {
                    Key::Shift => SHIFT,
                    Key::Ctrl => CTRL,
                    Key::Meta => META,
                    _ => 0,
                };
                if matches!(event, Event::KeyPressed(_)) {
                    self.modifiers |= flag;
                } else {
                    self.modifiers &= !flag;
                }
                return false;
            }
            Event::FocusLost => {
                self.pointer = None;
                self.modifiers = 0;
                return false;
            }
            Event::Scrolled(delta) => (delta, true, false),
            Event::TrackpadScrolled { delta, phase, .. } if phase != ScrollPhase::Lifted => {
                (delta, false, phase == ScrollPhase::Momentum)
            }
            _ => return false,
        };
        let (Some(area), Some(pointer)) = (self.area.filter(|_| active), self.pointer) else {
            return false;
        };
        let ruler = pxui::Key::Resolved(area.ruler.box_key);
        let tracks = pxui::Key::Resolved(area.tracks.box_key);
        let Some(target) = ui.root_at(pointer, &[ruler, tracks]) else {
            return false;
        };
        if lines && self.precise {
            return true;
        }

        // The macOS host reports SDL deltas: positive right and up.
        let (dx, dy) = (delta.0 as f32, -delta.1 as f32);
        if !dx.is_finite() || !dy.is_finite() || (dx == 0.0 && dy == 0.0) {
            return false;
        }
        if self.modifiers & (CTRL | META) != 0 {
            let geo = Geometry::new(area.width, area.fps, area.end, state);
            let x = pointer.x - ui.rect(&area.ruler).x - geo.labels;
            let sensitivity = if lines {
                0.1
            } else if dy.abs() >= 40.0 {
                0.0025
            } else {
                0.012
            };
            geo.zoom_at_x(state, geo.scale * (-dy * sensitivity).exp(), x);
            state.origin = state.origin.clamp(0.0, geo.max_origin(state.scale));
            return true;
        }

        let vertical = dy.abs() > dx.abs();
        if vertical && self.modifiers & SHIFT == 0 && area.overflow {
            if target == ruler {
                // The ruler sits above PXUI's scroll box, but shares its vertical scroll.
                self.ruler_scroll += dy * if lines { LANE_HEIGHT } else { 1.0 };
                return true;
            }
            if !lines && !momentum {
                self.track_travel += dy;
            }
            return false;
        }

        let points = if vertical { dy } else { dx } * if lines { 16.0 } else { 1.0 };
        state.pan_wheel(Geometry::new(area.width, area.fps, area.end, state), points);
        true
    }
}

pub(super) fn paint(
    ui: &mut Ui,
    project: Option<&Project>,
    view: View,
    cache: &mut Cache,
    (previews, evaluator): (&[MediaPreview], Option<&Evaluator>),
) -> Intents {
    let fps = project.map_or(30, |project| project.fps.max(1));
    let geometry = Geometry::new(view.width, fps, view.end, &view.state);
    let project_changed = cache.refresh(project, geometry, fps);
    let mut intents = Intents {
        state: view.state.clone(),
        ..Intents::default()
    };
    if project_changed && let Some(project) = project {
        intents.state.retain_selection(project);
    }
    let pane = ui.column(
        Node {
            w: Size::Grow,
            h: Size::Px(
                view.height
                    .max(TOOLBAR_HEIGHT + RULER_HEIGHT + FOOTER_HEIGHT),
            ),
            ..plate(PANEL)
        },
        |ui| {
            toolbar(ui, project, &view, geometry, fps, &mut intents);
            let ruler_handle = ruler(ui, project, &view, geometry, fps, &mut intents);
            let viewport_height =
                (view.height - TOOLBAR_HEIGHT - RULER_HEIGHT - FOOTER_HEIGHT).max(0.0);
            let viewport = ui.node(
                "timeline-tracks",
                Node::column()
                    .w(Size::Grow)
                    .h_px(viewport_height)
                    .flags(
                        Flags::SCROLL
                            | Flags::CLIP
                            | Flags::BLOCKING
                            | Flags::OVER_THUMB
                            | Flags::CAPTURE_UNCLAIMED,
                    )
                    .role(Role::ScrollArea)
                    .scroll_step(LANE_HEIGHT)
                    .bar(Bar::new(Rgba::rgb(56, 56, 59), 6.0).margin(2.0))
                    .look(Look::NONE.fill(SURFACE)),
            );
            cache.wheel.area = None;
            let ruler_scroll = std::mem::take(&mut cache.wheel.ruler_scroll);
            let track_travel = std::mem::take(&mut cache.wheel.track_travel);
            if let Some(project) = project {
                cache.refresh_rows(project, &intents.state);
                let max_scroll = (cache.rows_height - viewport_height).max(0.0);
                if ruler_scroll != 0.0 {
                    let offset = ui.scroll(&viewport).settled + ruler_scroll;
                    ui.set_scroll_offset(&viewport, offset.clamp(0.0, max_scroll));
                }
                let scroll = ui.scroll(&viewport);
                let signal = ui.signal(&viewport);
                // PXUI applies native track travel during layout, after these rows are built.
                let travel = if cache.wheel.precise {
                    track_travel
                } else {
                    -signal.scroll.1 * LANE_HEIGHT
                };
                let top = (scroll.painted + travel).clamp(0.0, max_scroll);
                if travel != 0.0 {
                    ui.request_frame();
                }
                if signal.pressed && signal.button == Some(Signal::BUTTON_SECONDARY) {
                    let rect = ui.rect(&viewport);
                    let x = signal.press_point.0 - rect.x;
                    if x >= geometry.labels {
                        let y = signal.press_point.1 - rect.y + scroll.painted;
                        let index = cache.rows.partition_point(|row| row.top + row.height <= y);
                        let lane = cache.rows.get(index).and_then(|row| match row.track {
                            Track::Lane(index) => Some(project.lanes[index].id),
                            Track::Spine => Some(project.spine.id),
                            _ => None,
                        });
                        let at = blade_frame(geometry, x).clamp(0, view.end.max(0));
                        intents
                            .state
                            .open_menu(ContextTarget::Background(at, lane), signal.press_point);
                    }
                }
                if signal.pressed || intents.state.marquee.is_some() {
                    let rect = ui.rect(&viewport);
                    let shift = ui.key_down(KeyCode::Shift);
                    intents.marquee_seen = update_marquee(
                        &signal,
                        rect,
                        scroll.painted,
                        shift,
                        project,
                        cache,
                        geometry,
                        &mut intents.state,
                    );
                }
                let drop_viewport = intents.state.drag.is_some().then(|| DropViewport {
                    rect: ui.rect(&viewport),
                    scroll: scroll.painted,
                });
                ui.within(&viewport, &mut |ui| {
                    tracks(
                        ui,
                        project,
                        geometry,
                        fps,
                        view.at,
                        &mut intents,
                        cache,
                        (previews, evaluator),
                        (top, viewport_height),
                        drop_viewport,
                    );
                    track_overlay(
                        ui,
                        (&viewport, &ruler_handle),
                        project,
                        cache,
                        geometry,
                        (viewport_height, view.at, fps, view.dragging),
                        scroll,
                        &intents,
                    );
                });
                cache.wheel.area = Some(WheelArea {
                    ruler: ruler_handle,
                    tracks: viewport,
                    width: view.width,
                    fps,
                    end: view.end,
                    overflow: cache.rows_height > viewport_height + 1.0,
                });
            }
            overview(ui, cache, view.at, geometry, fps, &mut intents);
        },
    );
    if intents.state.menu_open()
        && let Some(project) = project
    {
        paint_context(ui, project, &view, cache, &mut intents);
    }
    if !intents.drag_seen {
        intents.state.drag = None;
    }
    if !intents.fade_seen {
        intents.state.fade_drag = None;
    }
    if !intents.rename_painted && intents.state.lane_rename == view.state.lane_rename {
        intents.state.lane_rename = None;
    }
    intents.bounds = ui.rect(&pane);
    intents
}

fn paint_context(
    ui: &mut Ui,
    project: &Project,
    view: &View,
    cache: &Cache,
    intents: &mut Intents,
) {
    let state = &mut intents.state;
    let Some(context) = state.context else { return };
    match context.target {
        ContextTarget::Block => {
            paint_block_context(ui, project, view, cache, intents, context.anchor)
        }
        ContextTarget::Component => {
            let (Some((clip, _, _)), Some((label, command))) = (
                state.component(),
                component_visibility(project, state, true),
            ) else {
                state.context = None;
                return;
            };
            let entries = [
                MenuItem::new(label).id(0),
                MenuItem::new("Select parent clip").id(1),
            ];
            match context_choice(
                ui,
                state,
                context.anchor,
                &entries,
                "Take component actions",
            ) {
                Some(0) => intents.toolbar = Some(command),
                Some(1) => state.set_selection(Some(TimelineId::Clip(clip)), None),
                _ => {}
            }
        }
        ContextTarget::Marker(at) => {
            let entries = [
                MenuItem::new("Jump to Marker").id(0),
                MenuItem::new("Delete Marker").id(1).shortcut("⌫").danger(),
            ];
            match context_choice(
                ui,
                state,
                context.anchor,
                &entries,
                "Timeline marker actions",
            ) {
                Some(0) => {
                    intents.scrub =
                        Some((Phase::Ended, at.clamp(0, view.end.saturating_sub(1).max(0))))
                }
                Some(1) => intents.toolbar = selection_delete(state, false),
                _ => {}
            }
        }
        ContextTarget::Lane(id) => {
            paint_lane_context(ui, project, cache, intents, id, context.anchor);
        }
        ContextTarget::Background(at, lane) => {
            let graphics = lane.is_some_and(|id| {
                project
                    .lanes
                    .iter()
                    .any(|lane| lane.id == id && lane.kind == LaneKind::Graphics)
            });
            let camera = lane.is_some_and(|id| {
                project
                    .lanes
                    .iter()
                    .any(|lane| lane.id == id && lane.kind == LaneKind::Camera)
            });
            let entries = [
                MenuItem::new("Paste Here")
                    .id(8)
                    .shortcut("⌘V")
                    .disabled(!state.clipboard_enabled(ClipboardAction::Paste)),
                MenuItem::new("Add Marker Here").id(0).shortcut("M"),
                MenuItem::new("Move Playhead Here")
                    .id(1)
                    .disabled(view.end == 0),
                MenuItem::new(if graphics {
                    "Add Title Here"
                } else if camera {
                    "Add Scale Animation Here"
                } else {
                    ""
                })
                .id(2)
                .disabled(!graphics && !camera),
                MenuItem::new("Select All in Lane")
                    .id(3)
                    .disabled(lane.is_none()),
                MenuItem::new(""),
                MenuItem::new("Fit").id(4).shortcut("⇧Z"),
                MenuItem::new("Zoom to Selection")
                    .id(5)
                    .shortcut("F")
                    .disabled(state.selected().is_empty()),
                MenuItem::new("Snapping")
                    .id(6)
                    .shortcut("N")
                    .checked(!state.free_snap),
                MenuItem::new("Magnetic Timeline")
                    .id(7)
                    .checked(!state.free_move),
            ];
            let from = usize::from(lane != Some(project.spine.id));
            match context_choice(
                ui,
                state,
                context.anchor,
                &entries[from..],
                "Timeline position actions",
            ) {
                Some(0) => intents.toolbar = Some(ToolbarCommand::AddMarker(at)),
                Some(1) => {
                    intents.scrub =
                        Some((Phase::Ended, at.clamp(0, view.end.saturating_sub(1).max(0))))
                }
                Some(2) => {
                    intents.toolbar = if graphics {
                        lane.map(|id| ToolbarCommand::AddTitleInLane(at, id))
                    } else {
                        lane.map(|id| ToolbarCommand::AddMotionInLane(at, id))
                    };
                }
                Some(3) => {
                    if let Some(id) = lane {
                        select_lane(project, state, id);
                    }
                }
                Some(4) => state.fit(),
                Some(5) => state.zoom_selection(project, view.width, view.end),
                Some(6) => state.toggle_snap(),
                Some(7) => state.free_move = !state.free_move,
                Some(8) => intents.clipboard = Some(ClipboardAction::PasteHere(at)),
                _ => {}
            }
        }
    }
}

fn paint_lane_context(
    ui: &mut Ui,
    project: &Project,
    cache: &Cache,
    intents: &mut Intents,
    id: LaneId,
    anchor: Point,
) {
    let state = &mut intents.state;
    let name = "Timeline lane actions";
    if id == project.spine.id {
        // Public v13 EFFECT_TYPES order. Core decides which of those effects the spine accepts.
        const EFFECTS: [(&str, &str); 6] = [
            ("window", "Add Window"),
            ("cursor", "Add Cursor"),
            ("clickZoom", "Add Click Zoom"),
            ("camera", "Add Camera Layout"),
            ("captions", "Add Caption Style"),
            ("keystrokes", "Add Keystroke Chips"),
        ];
        let target = Target::Lane(id);
        let mut effects = [MenuItem::new(""); EFFECTS.len()];
        let mut count = 0;
        for (index, (kind, label)) in EFFECTS.iter().copied().enumerate() {
            if effect_target_allowed(project, &target, kind) {
                effects[count] = MenuItem::new(label).id(5 + index as i64).checked(
                    project
                        .spine
                        .effects
                        .iter()
                        .any(|effect| effect.kind == kind),
                );
                count += 1;
            }
        }
        let entries = [
            MenuItem::new(if state.all_components || state.expanded.is_some() {
                "Hide All Components"
            } else {
                "Show All Components"
            })
            .id(2)
            .disabled(!cache.has_components),
            MenuItem::new("Select All Clips").id(0).shortcut("⌘A"),
            MenuItem::new("Lane Effects").submenu(&effects[..count]),
            MenuItem::new("Edit Lane Effects…").id(4),
            MenuItem::new("New Overlay Lane").id(1),
        ];
        match context_choice(ui, state, anchor, &entries, name) {
            Some(0) => select_lane(project, state, id),
            Some(1) => intents.toolbar = Some(ToolbarCommand::NewLane(LaneKind::Overlay)),
            Some(2) => {
                state.all_components = !state.all_components && state.expanded.is_none();
                state.expanded = None;
            }
            Some(4) => {
                state.set_selection(None, Some(id));
                intents.inspect_selection = true;
            }
            Some(row) if row >= 5 => {
                intents.toolbar = EFFECTS
                    .get((row - 5) as usize)
                    .map(|&(kind, _)| ToolbarCommand::AddEffect(target, kind));
            }
            _ => {}
        }
        return;
    }

    let Some(lane) = project.lanes.iter().find(|lane| lane.id == id) else {
        state.context = None;
        return;
    };
    let target = Target::Lane(id);
    let effect = BLOCK_EFFECTS.iter().find(|&&(kind, _, _)| {
        matches!(
            (lane.kind, kind),
            (LaneKind::Overlay, "window") | (LaneKind::Captions, "captions")
        ) && effect_target_allowed(project, &target, kind)
    });
    let effects = [MenuItem::new(effect.map_or("", |entry| entry.1)).id(5)];
    let new_lane = match lane.kind {
        LaneKind::Overlay => "New Overlay Lane",
        LaneKind::Graphics => "New Graphics Lane",
        LaneKind::Captions => "New Captions Lane",
        LaneKind::Camera => "New Motion Lane",
        LaneKind::Audio => "New Audio Lane",
    };
    let can_delete = cache.lane(id).is_empty()
        || project
            .lanes
            .iter()
            .any(|other| other.kind == lane.kind && other.id != id);
    let entries = [
        MenuItem::new("Lane Effects").submenu(&effects),
        MenuItem::new("Edit Lane Effects…").id(4),
        MenuItem::new(""),
        MenuItem::new(new_lane).id(1),
        MenuItem::new("Rename…").id(2),
        MenuItem::new("Select All in Lane").id(0).shortcut("⌘A"),
        MenuItem::new(""),
        MenuItem::new("Delete Lane")
            .id(3)
            .disabled(!can_delete)
            .danger(),
    ];
    let first = if effect.is_some() { 0 } else { 3 };
    match context_choice(ui, state, anchor, &entries[first..], name) {
        Some(0) => select_lane(project, state, id),
        Some(1) => intents.toolbar = Some(ToolbarCommand::NewLane(lane.kind)),
        Some(2) => state.begin_lane_rename(id),
        Some(3) => intents.toolbar = Some(ToolbarCommand::DeleteLane(id)),
        Some(4) => {
            state.set_selection(None, Some(id));
            intents.inspect_selection = true;
        }
        Some(5) => intents.toolbar = effect.map(|entry| ToolbarCommand::AddEffect(target, entry.0)),
        _ => {}
    }
}

const SPEEDS: [(f64, &str); 6] = [
    (0.5, "0.5×"),
    (1.0, "1×"),
    (1.5, "1.5×"),
    (2.0, "2×"),
    (4.0, "4×"),
    (8.0, "8×"),
];
const PLACEMENTS: [(Placement, &str); 6] = [
    (Placement::Full, "full"),
    (Placement::Card, "card"),
    (Placement::TopLeft, "corner-tl"),
    (Placement::TopRight, "corner-tr"),
    (Placement::BottomLeft, "corner-bl"),
    (Placement::BottomRight, "corner-br"),
];
const COLORS: [(&str, &str, Rgba); 7] = [
    ("red", "Red", Rgba::rgb(224, 121, 108)),
    ("orange", "Orange", Rgba::rgb(229, 154, 91)),
    ("yellow", "Yellow", Rgba::rgb(217, 194, 90)),
    ("green", "Green", Rgba::rgb(134, 191, 126)),
    ("blue", "Blue", Rgba::rgb(120, 169, 219)),
    ("purple", "Purple", Rgba::rgb(174, 146, 216)),
    ("gray", "Gray", Rgba::rgb(154, 154, 148)),
];
const CAPTION_STYLES: [(&str, &str); 6] = [
    ("bold", "Bold"),
    ("karaoke", "Karaoke"),
    ("pop", "Pop"),
    ("box", "Box"),
    ("clean", "Clean"),
    ("typewriter", "Typewriter"),
];
const BLOCK_EFFECTS: [(&str, &str, &str); 5] = [
    ("window", "Add Window", "visual"),
    ("cursor", "Add Cursor", "take"),
    ("camera", "Add Camera Layout", "take"),
    ("captions", "Add Caption Style", "caption"),
    ("keystrokes", "Add Keystroke Chips", "take"),
];

fn clip_anchor(anchor: Anchor) -> Option<ClipId> {
    match anchor {
        Anchor::ClipSource { clip, .. }
        | Anchor::Item {
            item: TimelineId::Clip(clip),
            ..
        } => Some(clip),
        _ => None,
    }
}

fn item_data(project: &Project, id: TimelineId) -> Option<(ItemId, Slot, &ItemData)> {
    let TimelineId::Item(id) = id else {
        return None;
    };
    let slot = project.items.find(id)?;
    Some((id, slot, project.items.data(slot)?))
}

fn menu_color(project: &Project, id: TimelineId) -> Option<&str> {
    match id {
        TimelineId::Clip(id) => project
            .clips
            .data(project.clips.find(id)?)?
            .appearance
            .color
            .as_deref(),
        TimelineId::Item(id) => project
            .items
            .data(project.items.find(id)?)?
            .appearance
            .color
            .as_deref(),
        TimelineId::Marker(id) => project
            .markers
            .iter()
            .find(|marker| marker.id == id)?
            .color
            .as_deref(),
    }
}

fn menu_caption_style(project: &Project, cue: &ItemData) -> Option<&'static str> {
    let lane = project.lanes.iter().find(|lane| lane.id == cue.lane);
    let mut enabled = true;
    let mut preset = "bold";
    for scope in [
        project.effects.as_slice(),
        lane.map_or(&[][..], |lane| &lane.effects),
        cue.appearance.effects.as_slice(),
    ] {
        if let Some(fx) = scope.iter().find(|fx| fx.kind == "captions") {
            enabled = fx.enabled.unwrap_or(enabled);
            if let Some(EffectValue::Text(value)) = fx
                .params
                .iter()
                .find(|param| param.name == "preset")
                .map(|param| &param.value)
            {
                preset = value;
            }
        }
    }
    CAPTION_STYLES
        .iter()
        .find_map(|&(name, _)| (enabled && preset == name).then_some(name))
}

fn paint_block_context(
    ui: &mut Ui,
    project: &Project,
    view: &View,
    cache: &Cache,
    intents: &mut Intents,
    anchor: Point,
) {
    const SPLIT: i64 = 1;
    const TRIM_IN: i64 = 2;
    const TRIM_OUT: i64 = 3;
    const COMPONENTS: i64 = 4;
    const MUTE_AUDIO: i64 = 5;
    const LIFT: i64 = 6;
    const CROP: i64 = 7;
    const REVERSE: i64 = 8;
    const BEHIND: i64 = 9;
    const REPLACE: i64 = 10;
    const PIN: i64 = 11;
    const LANE_UP: i64 = 12;
    const LANE_DOWN: i64 = 13;
    const SELECT_ANCHOR: i64 = 14;
    const DUPLICATE: i64 = 15;
    const ZOOM: i64 = 16;
    const DELETE: i64 = 17;
    const LEAVE_GAP: i64 = 18;
    const KEEP_OVERLAYS: i64 = 19;
    const MERGE_CAPTIONS: i64 = 20;
    const REMOVE_EFFECTS: i64 = 21;
    const CUT: i64 = 22;
    const COPY: i64 = 23;
    const PASTE: i64 = 24;
    const PASTE_EFFECTS: i64 = 25;
    const EDIT_TEXT: i64 = 26;
    const CUSTOM_SPEED: i64 = 27;
    const SPEED: i64 = 32;
    const FADE: i64 = 40;
    const CLEAR_COLOR: i64 = 48;
    const COLOR: i64 = 49;
    const PLACEMENT: i64 = 64;
    const STYLE: i64 = 72;
    const EFFECT: i64 = 80;
    const WORD: i64 = 96;

    let state = &mut intents.state;
    let selected = state.selected();
    let Some(&first) = selected.first() else {
        state.context = None;
        return;
    };
    let group = selected.len() > 1;
    let single = (!group).then_some(first);
    let clip = single.and_then(|id| {
        let TimelineId::Clip(id) = id else {
            return None;
        };
        let slot = project.clips.find(id)?;
        Some((id, project.clips.timing(slot)?, project.clips.data(slot)?))
    });
    let item = single.and_then(|id| item_data(project, id));
    let text = single.and_then(|id| text_focus(project, id));
    if !group && clip.is_none() && item.is_none() {
        state.context = None;
        return;
    }
    let timing = item.and_then(|(_, slot, _)| project.items.timing(slot));
    let caption = item.filter(|(_, _, data)| matches!(data.kind, ItemKind::Caption { .. }));
    let motion = item.and_then(|(id, _, data)| match data.kind {
        ItemKind::Motion(motion) => Some((id, motion)),
        _ => None,
    });
    let overlay = item.and_then(|(id, _, data)| match data.kind {
        ItemKind::Media(overlay) => Some((id, overlay)),
        _ => None,
    });
    let pinned = timing.is_some_and(|timing| matches!(timing.anchor, Anchor::Pinned(_)));
    let target_clip = timing.and_then(|timing| clip_anchor(timing.anchor));
    let split_span = single
        .and_then(|id| item_span(project, id))
        .filter(|&(start, length)| {
            view.at >= start.saturating_add(MIN_TRIM_LENGTH)
                && view.at <= start.saturating_add(length).saturating_sub(MIN_TRIM_LENGTH)
        });
    let all_clips = selected.iter().all(|id| matches!(id, TimelineId::Clip(_)));
    let all_items = selected.iter().all(|id| matches!(id, TimelineId::Item(_)));
    let all_captions = group
        && selected.iter().all(|&id| {
            item_data(project, id)
                .is_some_and(|(_, _, data)| matches!(data.kind, ItemKind::Caption { .. }))
        });
    let caps_share_lane = all_captions
        && item_data(project, first).is_some_and(|(_, _, first)| {
            selected.iter().all(|&id| {
                item_data(project, id).is_some_and(|(_, _, data)| {
                    data.lane == first.lane
                        && matches!(data.kind, ItemKind::Caption { text: None, .. })
                })
            })
        });
    let clip_media = |id| {
        let TimelineId::Clip(id) = id else {
            return None;
        };
        let slot = project.clips.find(id)?;
        project.clips.data(slot)?.media?;
        project.clips.timing(slot)
    };
    let can_speed = all_clips && selected.iter().all(|&id| clip_media(id).is_some());
    let can_lift = can_speed
        && selected
            .iter()
            .all(|&id| clip_media(id).is_some_and(|t| t.speed == 1.0));
    let show_clip_actions = all_clips && (group || can_speed);
    let take = clip.filter(|(_, _, data)| has_components(data));
    let audio = take
        .map(|(_, _, data)| [data.enabled.mic, data.enabled.system])
        .unwrap_or_default();
    let muted = audio.iter().flatten().all(|&on| !on);
    let has_audio = audio.iter().any(Option::is_some);
    let expanded =
        take.is_some_and(|(id, _, _)| state.all_components || state.expanded == Some(id));
    let can_crop = single.is_some_and(|id| super::viewer_crop::Draft::new(project, id).is_some());
    let kids = clip.map_or(0, |(id, _, _)| cache.anchored_count(id));
    let current_speed = clip.map(|(_, timing, _)| timing.speed);
    let speed = std::array::from_fn::<_, { SPEEDS.len() + 2 }, _>(|i| {
        if let Some(&(value, label)) = SPEEDS.get(i) {
            MenuItem::new(label)
                .id(SPEED + i as i64)
                .checked(current_speed == Some(value))
        } else if i == SPEEDS.len() {
            MenuItem::new("")
        } else {
            MenuItem::new("Custom…").id(CUSTOM_SPEED)
        }
    });
    let fades = [
        MenuItem::new("Fade In 0.5 s").id(FADE),
        MenuItem::new("Fade Out 0.5 s").id(FADE + 1),
        MenuItem::new("Fade Both 0.5 s").id(FADE + 2),
        MenuItem::new("Fade Both 1 s").id(FADE + 3),
        MenuItem::new(""),
        MenuItem::new("Remove Fades").id(FADE + 4),
    ];
    let show_fades = group
        || !item.is_some_and(|(_, _, data)| {
            matches!(data.kind, ItemKind::Motion(_) | ItemKind::Zoom(_))
        });
    let color = menu_color(project, first);
    let same_color = selected.iter().all(|&id| menu_color(project, id) == color);
    let mut colors = [MenuItem::new(""); COLORS.len() + 2];
    colors[0] = MenuItem::new("None")
        .id(CLEAR_COLOR)
        .checked(same_color && color.is_none());
    for (i, &(name, label, color_chip)) in COLORS.iter().enumerate() {
        colors[i + 2] = MenuItem::new(label)
            .id(COLOR + i as i64)
            .swatch(color_chip)
            .checked(same_color && color == Some(name));
    }
    let placements = std::array::from_fn::<_, { PLACEMENTS.len() }, _>(|i| {
        let (place, label) = PLACEMENTS[i];
        MenuItem::new(label)
            .id(PLACEMENT + i as i64)
            .checked(overlay.is_some_and(|(_, overlay)| overlay.placement == place))
    });
    let styled = caption.and_then(|(_, _, data)| menu_caption_style(project, data));
    let styles = std::array::from_fn::<_, { CAPTION_STYLES.len() }, _>(|i| {
        let (preset, label) = CAPTION_STYLES[i];
        MenuItem::new(label)
            .id(STYLE + i as i64)
            .checked(styled == Some(preset))
    });
    let caption_items = caption
        .map(|(_, _, data)| cache.lane(data.lane))
        .unwrap_or_default();
    let indexed = caption.and_then(|(_, slot, _)| caption_items.iter().find(|i| i.slot == slot));
    let words = indexed
        .and_then(|item| item.caption.as_ref())
        .map_or(&[][..], |c| c.words.as_slice());
    let split_word = words
        .iter()
        .skip(1)
        .find(|word| word.at >= view.at.saturating_sub(1))
        .map(|word| word.id);
    let next = indexed.and_then(|current| {
        let start = current.start?;
        let next = caption_items
            .iter()
            .filter(|item| item.start.is_some_and(|at| at > start))
            .min_by_key(|item| (item.start, item.order))?;
        let current_data = project.items.data(current.slot)?;
        let next_data = project.items.data(next.slot)?;
        if matches!(current_data.kind, ItemKind::Caption { text: None, .. })
            && matches!(next_data.kind, ItemKind::Caption { text: None, .. })
        {
            project.items.id(next.slot).map(TimelineId::Item)
        } else {
            None
        }
    });
    // Large merged cues keep their full word controls in the Inspector.
    let mut emphasis = [MenuItem::new(""); 65];
    let mut word_count = 0;
    for (i, word) in words
        .iter()
        .enumerate()
        .filter(|(_, word)| !word.hidden && !word.text.is_empty())
    {
        if word_count == 64 {
            emphasis[word_count] = MenuItem::new("More words in Inspector").disabled(true);
            word_count += 1;
            break;
        }
        emphasis[word_count] = MenuItem::new(&word.text)
            .id(WORD + i as i64)
            .checked(word.emph);
        word_count += 1;
    }
    let mut effects = [MenuItem::new(""); BLOCK_EFFECTS.len() + 3];
    let mut effect_count = 0;
    let local = clip
        .map(|(_, _, data)| data.appearance.effects.as_slice())
        .or_else(|| item.map(|(_, _, data)| data.appearance.effects.as_slice()))
        .unwrap_or_default();
    let media = clip
        .and_then(|(_, _, data)| data.media)
        .and_then(|id| project.media.iter().find(|media| media.id == id));
    for (i, &(kind, label, class)) in BLOCK_EFFECTS.iter().enumerate() {
        if clip_fx_applies(class, media)
            || item.is_some_and(|(_, _, data)| item_fx_applies(class, &data.kind))
        {
            effects[effect_count] = MenuItem::new(label)
                .id(EFFECT + i as i64)
                .checked(local.iter().any(|fx| fx.kind == kind));
            effect_count += 1;
        }
    }
    if effect_count > 0 {
        effects[effect_count + 1] = MenuItem::new("Paste Effects")
            .id(PASTE_EFFECTS)
            .shortcut("⌥⌘V")
            .disabled(!state.clipboard_has_effects(project));
        effects[effect_count + 2] = MenuItem::new("Remove Effects")
            .id(REMOVE_EFFECTS)
            .disabled(local.is_empty())
            .danger();
        effect_count += 3;
    }
    let mut delete_text = [0_u8; 48];
    let mut keep_text = [0_u8; 48];
    let delete_label = if group {
        let mut tail = delete_text.as_mut_slice();
        std::io::Write::write_fmt(&mut tail, format_args!("Delete {} Items", selected.len()))
            .unwrap();
        let len = 48 - tail.len();
        std::str::from_utf8(&delete_text[..len]).unwrap()
    } else if caption.is_some() {
        "Delete Caption"
    } else if clip.is_some() && state.ripple() {
        "Ripple Delete"
    } else {
        "Delete"
    };
    let keep_label = if kids != 0 {
        let mut tail = keep_text.as_mut_slice();
        std::io::Write::write_fmt(&mut tail, format_args!("Delete, Keep Overlays ({kids})"))
            .unwrap();
        let len = 48 - tail.len();
        std::str::from_utf8(&keep_text[..len]).unwrap()
    } else {
        "Delete, Keep Overlays"
    };
    let sep = MenuItem::new("");
    let rows = [
        (
            text.is_some(),
            MenuItem::new("Edit Text…")
                .id(EDIT_TEXT)
                .shortcut("dbl-click"),
        ),
        (
            !group,
            MenuItem::new("Split at Playhead")
                .id(SPLIT)
                .shortcut("C")
                .disabled(if caption.is_some() {
                    split_word.is_none()
                } else {
                    split_span.is_none()
                }),
        ),
        (
            clip.is_some(),
            MenuItem::new("Trim Start to Playhead")
                .id(TRIM_IN)
                .shortcut("Q")
                .disabled(split_span.is_none()),
        ),
        (
            clip.is_some(),
            MenuItem::new("Trim End to Playhead")
                .id(TRIM_OUT)
                .shortcut("W")
                .disabled(split_span.is_none()),
        ),
        (clip.is_some(), sep),
        (
            caption.is_some() || all_captions,
            MenuItem::new(if group {
                "Merge Captions"
            } else {
                "Merge with Next"
            })
            .id(MERGE_CAPTIONS)
            .disabled(if group {
                !caps_share_lane
            } else {
                next.is_none()
            }),
        ),
        (
            caption.is_some(),
            MenuItem::new("Emphasize Word")
                .submenu(&emphasis[..word_count])
                .disabled(word_count == 0),
        ),
        (caption.is_some(), MenuItem::new("Style").submenu(&styles)),
        (
            show_clip_actions,
            MenuItem::new("Speed").submenu(&speed).disabled(!can_speed),
        ),
        (show_fades, MenuItem::new("Fades").submenu(&fades)),
        (
            take.is_some(),
            MenuItem::new(if expanded {
                "Hide Components"
            } else {
                "Show Components"
            })
            .id(COMPONENTS)
            .shortcut("dbl-click"),
        ),
        (
            take.is_some(),
            MenuItem::new("Mute Audio")
                .id(MUTE_AUDIO)
                .checked(has_audio && muted)
                .disabled(!has_audio),
        ),
        (
            show_clip_actions,
            MenuItem::new("Lift to Overlay")
                .id(LIFT)
                .disabled(!can_lift),
        ),
        (motion.is_some(), MenuItem::new("Reverse Scale").id(REVERSE)),
        (
            overlay.is_some(),
            MenuItem::new("Placement").submenu(&placements),
        ),
        (
            overlay.is_some(),
            MenuItem::new("Behind Presenter")
                .id(BEHIND)
                .checked(overlay.is_some_and(|(_, media)| media.behind_presenter)),
        ),
        (
            overlay.is_some(),
            MenuItem::new("Replace Spine Clip")
                .id(REPLACE)
                .disabled(pinned),
        ),
        (can_crop, MenuItem::new("Crop…").id(CROP).shortcut("⇧C")),
        (item.is_some(), sep),
        (
            item.is_some() && caption.is_none(),
            MenuItem::new(if pinned {
                "Anchor to Clip Below"
            } else {
                "Pin to Timeline"
            })
            .id(PIN),
        ),
        (
            all_items,
            MenuItem::new("Lane Up").id(LANE_UP).shortcut("⌥↑"),
        ),
        (
            all_items,
            MenuItem::new("Lane Down").id(LANE_DOWN).shortcut("⌥↓"),
        ),
        (
            caption.is_none() && target_clip.is_some(),
            MenuItem::new("Select Anchor Clip").id(SELECT_ANCHOR),
        ),
        (true, sep),
        (
            effect_count != 0,
            MenuItem::new("Effects").submenu(&effects[..effect_count]),
        ),
        (
            caption.is_none(),
            MenuItem::new("Cut").id(CUT).shortcut("⌘X"),
        ),
        (
            caption.is_none(),
            MenuItem::new("Copy").id(COPY).shortcut("⌘C"),
        ),
        (
            caption.is_none(),
            MenuItem::new("Paste")
                .id(PASTE)
                .shortcut("⌘V")
                .disabled(!state.clipboard_enabled(ClipboardAction::Paste)),
        ),
        (
            caption.is_none(),
            MenuItem::new("Duplicate").id(DUPLICATE).shortcut("⌘D"),
        ),
        (caption.is_none(), sep),
        (
            true,
            MenuItem::new(if group {
                "Zoom to Selection"
            } else {
                "Zoom to Item"
            })
            .id(ZOOM)
            .shortcut("F"),
        ),
        (true, MenuItem::new("Color").submenu(&colors)),
        (
            true,
            MenuItem::new(delete_label)
                .id(DELETE)
                .shortcut("⌫")
                .danger(),
        ),
        (
            group || clip.is_some(),
            MenuItem::new("Delete Leaving Gap")
                .id(LEAVE_GAP)
                .shortcut("⇧⌫"),
        ),
        (
            clip.is_some(),
            MenuItem::new(keep_label)
                .id(KEEP_OVERLAYS)
                .disabled(kids == 0),
        ),
    ];
    let mut entries = [sep; 37];
    let mut count = 0;
    for (show, row) in rows {
        if show {
            entries[count] = row;
            count += 1;
        }
    }
    let Some(choice) = context_choice(
        ui,
        state,
        anchor,
        &entries[..count],
        "Timeline item actions",
    ) else {
        return;
    };
    let ripple = state.ripple();
    let ids = || Rc::from(state.selected());
    match choice {
        SPLIT if caption.is_none() => intents.split = single.map(|id| (id, view.at)),
        TRIM_IN | TRIM_OUT => {
            if let Some(((id, _, _), (start, length))) = clip.zip(split_span) {
                let incoming = choice == TRIM_IN;
                let boundary = if incoming {
                    start
                } else {
                    start.saturating_add(length)
                };
                intents.trimmed = Some((
                    TimelineId::Clip(id),
                    if incoming { "in" } else { "out" },
                    view.at.saturating_sub(boundary),
                    ripple,
                ));
            }
        }
        COMPONENTS => {
            if let Some((id, _, _)) = take {
                state.toggle_take(id);
            }
        }
        CROP => intents.crop = single,
        EDIT_TEXT => intents.inspector_focus = text,
        CUSTOM_SPEED if can_speed => {
            if let TimelineId::Clip(clip) = first {
                state.set_selection(Some(first), None);
                intents.inspector_focus = Some(super::inspector::Focus::Speed(clip));
            }
        }
        CUT => intents.clipboard = Some(ClipboardAction::Cut),
        COPY => intents.clipboard = Some(ClipboardAction::Copy),
        PASTE => intents.clipboard = Some(ClipboardAction::Paste),
        PASTE_EFFECTS => intents.clipboard = Some(ClipboardAction::PasteEffects),
        SELECT_ANCHOR => {
            if let Some(clip) = target_clip {
                state.set_selection(Some(TimelineId::Clip(clip)), None);
            }
        }
        ZOOM => state.zoom_selection(project, view.width, view.end),
        _ => {
            intents.toolbar = match choice {
                SPLIT => caption
                    .zip(split_word)
                    .map(|((id, _, _), word)| ToolbarCommand::SplitCaption(id, word)),
                MERGE_CAPTIONS if group => Some(ToolbarCommand::MergeCaptions(ids())),
                MERGE_CAPTIONS => {
                    next.map(|next| ToolbarCommand::MergeCaptions(Rc::from([first, next])))
                }
                MUTE_AUDIO => take.map(|(id, _, _)| {
                    ToolbarCommand::SetTakeAudio(id, audio.map(|on| on.is_some()), muted)
                }),
                LIFT if group => Some(ToolbarCommand::LiftGroup(ids())),
                LIFT => clip.map(|(id, _, _)| ToolbarCommand::Lift(id)),
                REVERSE => motion.map(|(id, motion)| ToolbarCommand::ReverseMotion(id, motion)),
                BEHIND => overlay
                    .map(|(id, media)| ToolbarCommand::SetBehind(id, !media.behind_presenter)),
                REPLACE => overlay.map(|(id, _)| ToolbarCommand::ReplaceSpine(id)),
                PIN => item.map(|(id, _, _)| ToolbarCommand::SetPinned(id, !pinned)),
                LANE_UP | LANE_DOWN => Some(ToolbarCommand::LaneStep(
                    ids(),
                    if choice == LANE_UP { "up" } else { "down" },
                )),
                DUPLICATE => Some(ToolbarCommand::Duplicate(ids())),
                DELETE | LEAVE_GAP => selection_delete(state, choice == DELETE && ripple),
                KEEP_OVERLAYS => clip.map(|(id, _, _)| ToolbarCommand::DeleteKeepOverlays(id)),
                REMOVE_EFFECTS => single.map(ToolbarCommand::RemoveEffects),
                CLEAR_COLOR => Some(ToolbarCommand::SetColor(ids(), None)),
                row if (SPEED..SPEED + SPEEDS.len() as i64).contains(&row) => Some(
                    ToolbarCommand::SetSpeed(ids(), SPEEDS[(row - SPEED) as usize].0, ripple),
                ),
                row if (FADE..FADE + 5).contains(&row) => {
                    let fps = project.fps.max(1).min(i32::MAX as u32) as i32;
                    let half = fps / 2 + fps % 2;
                    let values = [
                        [Some(half), None],
                        [None, Some(half)],
                        [Some(half); 2],
                        [Some(fps); 2],
                        [Some(0); 2],
                    ];
                    Some(ToolbarCommand::SetFades(
                        ids(),
                        values[(row - FADE) as usize],
                    ))
                }
                row if (COLOR..COLOR + COLORS.len() as i64).contains(&row) => Some(
                    ToolbarCommand::SetColor(ids(), Some(COLORS[(row - COLOR) as usize].0)),
                ),
                row if (PLACEMENT..PLACEMENT + PLACEMENTS.len() as i64).contains(&row) => overlay
                    .map(|(id, _)| {
                        ToolbarCommand::SetPlacement(id, PLACEMENTS[(row - PLACEMENT) as usize].0)
                    }),
                row if (STYLE..STYLE + CAPTION_STYLES.len() as i64).contains(&row) => {
                    caption.map(|(id, _, _)| {
                        ToolbarCommand::SetCaptionStyle(
                            id,
                            CAPTION_STYLES[(row - STYLE) as usize].0,
                        )
                    })
                }
                row if (EFFECT..EFFECT + BLOCK_EFFECTS.len() as i64).contains(&row) => {
                    single.map(|id| {
                        ToolbarCommand::AddEffect(
                            Target::Timeline(id),
                            BLOCK_EFFECTS[(row - EFFECT) as usize].0,
                        )
                    })
                }
                row if row >= WORD => {
                    caption
                        .zip(words.get((row - WORD) as usize))
                        .map(|((id, _, _), word)| {
                            ToolbarCommand::SetCaptionEmphasis(id, word.id, !word.emph)
                        })
                }
                _ => None,
            };
        }
    }
}

fn text_focus(project: &Project, item: TimelineId) -> Option<super::inspector::Focus> {
    let (id, _, data) = item_data(project, item)?;
    matches!(data.kind, ItemKind::Title { .. } | ItemKind::Caption { .. })
        .then_some(super::inspector::Focus::Text(id))
}

fn item_span(project: &Project, item: TimelineId) -> Option<(i32, i32)> {
    match item {
        TimelineId::Clip(id) => {
            let timing = project.clips.timing(project.clips.find(id)?)?;
            Some((timing.start, timing.length))
        }
        TimelineId::Item(id) => {
            let timing = project.items.timing(project.items.find(id)?)?;
            Some((anchor_start(project, timing.anchor).ok()?, timing.length))
        }
        TimelineId::Marker(_) => None,
    }
}

fn context_choice(
    ui: &mut Ui,
    state: &mut State,
    at: Point,
    entries: &[MenuItem<'_>],
    name: &str,
) -> Option<i64> {
    let menu = Menu::DEFAULT
        .w_px(220.0)
        .row_h(22.0)
        .padding(4.0)
        .focus_on_click(true)
        .max_h((ui.content_rect().h - 8.0).max(1.0))
        .name(name);
    let picked = ui.menu("timeline-context-menu", at, entries, &menu);
    if picked.dismissed || picked.chosen_id.is_some() {
        state.context = None;
    }
    picked.chosen_id
}

pub(super) fn selection_delete(state: &State, ripple: bool) -> Option<ToolbarCommand> {
    match state.selected() {
        [] => None,
        [id] => Some(ToolbarCommand::Delete(*id, ripple)),
        ids => Some(ToolbarCommand::DeleteGroup(Rc::from(ids), ripple)),
    }
}

pub(super) fn component_visibility(
    project: &Project,
    state: &State,
    toggle: bool,
) -> Option<(&'static str, ToolbarCommand)> {
    let (clip, part, key) = state.component()?;
    let data = project.clips.data(project.clips.find(clip)?)?;
    if let Some(index) = key {
        let hidden = !data.hidden_keys.contains(&index);
        return Some((
            if hidden {
                "Hide This Keystroke"
            } else {
                "Show This Keystroke"
            },
            ToolbarCommand::SetKeystrokeHidden(clip, index, hidden),
        ));
    }
    let row = components(data).into_iter().find(|row| row.kind == part)?;
    let enabled = row.enabled?;
    let change = if toggle { !enabled } else { false };
    (enabled != change).then_some((
        if change { row.show } else { row.hide },
        ToolbarCommand::SetComponent(clip, part, change),
    ))
}

pub(super) fn selected_media_overlay(project: &Project, ids: &[TimelineId]) -> Option<ItemId> {
    ids.iter().find_map(|id| {
        let TimelineId::Item(item) = *id else {
            return None;
        };
        let data = project.items.data(project.items.find(item)?)?;
        matches!(data.kind, ItemKind::Media(_)).then_some(item)
    })
}

fn select_lane(project: &Project, state: &mut State, id: LaneId) {
    let items = if id == project.spine.id {
        project
            .clips
            .order()
            .iter()
            .filter_map(|&slot| project.clips.id(slot).map(TimelineId::Clip))
            .collect()
    } else {
        project
            .items
            .order()
            .iter()
            .filter_map(|&slot| {
                if project.items.data(slot)?.lane == id {
                    project.items.id(slot).map(TimelineId::Item)
                } else {
                    None
                }
            })
            .collect()
    };
    state.set_selected(items);
}

fn plate(color: Rgba) -> Node {
    Node::DEFAULT.look(Look::NONE.fill(color))
}

fn tile(ui: &mut Ui, key: u64, x: f32, y: f32, w: f32, h: f32, color: Rgba) {
    if w > 0.0 && h > 0.0 {
        ui.node(key, plate(color).at(x, y).w_px(w).h_px(h));
    }
}

fn toolbar(
    ui: &mut Ui,
    project: Option<&Project>,
    view: &View,
    geo: Geometry,
    fps: u32,
    intents: &mut Intents,
) {
    let state = &mut intents.state;
    let compact = view.width <= 480.0;
    let (clips, items) = project.map_or((0, 0), |p| (p.clips.order().len(), p.items.order().len()));
    let seconds = i64::from(view.end.max(0)) * 100 / i64::from(fps);
    let columns =
        29 + (seconds / 100).max(1).ilog10() + clips.max(1).ilog10() + items.max(1).ilog10();
    let duration_width = columns as f32 * 6.6 + 4.0;
    // The compact controls require 441 points. Wide mode adds the zoom controls and duration.
    let min_width = if compact {
        441.0
    } else {
        558.0 + duration_width
    };
    let scroll = ui.node(
        "timeline-toolbar",
        Node::column()
            .w(Size::Grow)
            .h_px(TOOLBAR_HEIGHT)
            .flags(Flags::SCROLL_X | Flags::CLIP | Flags::BLOCKING)
            .look(Look::NONE.fill(PANEL).border(LINE, 1.0).sides(BOTTOM)),
    );
    ui.within(&scroll, &mut |ui| {
        ui.row(
            Node {
                w: Size::Px(view.width.max(min_width)),
                h: Size::Px(TOOLBAR_HEIGHT - 1.0),
                align: Align::Center,
                padding: Edges::xy(8.0, 0.0),
                gap: 1.0,
                ..Node::DEFAULT
            },
            |ui| {
                if tool(ui, 0, "Select tool", !state.blade, true) {
                    state.select_tool();
                }
                if tool(ui, 1, "Blade tool", state.blade, true) {
                    if !state.blade {
                        state.toggle_blade();
                    }
                }
                if tool(ui, 2, "Snapping", !state.free_snap, true) {
                    state.toggle_snap();
                }
                if tool(ui, 3, "Magnetic ripple", !state.free_move, true) {
                    state.free_move = !state.free_move;
                }
                let split = project
                    .and_then(|project| split_at_playhead(project, state.selection, view.at));
                tool_separator(ui, "edit-tools-separator");
                if tool(ui, 4, "Split at playhead", false, split.is_some()) {
                    intents.split = split.map(|item| (item, view.at));
                }
                let lift = match state.selection {
                    Some(TimelineId::Clip(id))
                        if project.is_some_and(|project| {
                            project
                                .clips
                                .find(id)
                                .and_then(|slot| project.clips.data(slot))
                                .is_some_and(|data| data.media.is_some())
                        }) =>
                    {
                        Some(ToolbarCommand::Lift(id))
                    }
                    _ => None,
                };
                let commands = [
                    (
                        5,
                        "Add title",
                        project.map(|_| ToolbarCommand::AddTitle(view.at)),
                    ),
                    (
                        6,
                        "Add scale animation",
                        project.map(|_| ToolbarCommand::AddMotion(view.at)),
                    ),
                    (
                        7,
                        "Add marker",
                        project.map(|_| ToolbarCommand::AddMarker(view.at)),
                    ),
                    (8, "Lift to overlay", lift),
                ];
                for (icon, name, command) in commands {
                    if tool(ui, icon, name, false, command.is_some()) {
                        intents.toolbar = command;
                    }
                }
                if tool(
                    ui,
                    9,
                    "Delete selection",
                    false,
                    !state.selected().is_empty(),
                ) {
                    intents.toolbar = if state.component().is_some() {
                        project
                            .and_then(|p| component_visibility(p, state, false))
                            .map(|(_, command)| command)
                    } else {
                        selection_delete(state, !state.free_move)
                    };
                }
                tool_separator(ui, "history-tools-separator");
                if tool(ui, 16, "Undo", false, view.history.0) {
                    intents.history = Some(UNDO);
                }
                if tool(ui, 17, "Redo", false, view.history.1) {
                    intents.history = Some(REDO);
                }
                ui.node("timeline-tools-space", Node::DEFAULT.w(Size::Grow));
                if !compact {
                    let label = ui.label_fmt(
                        format_args!(
                            "{}.{:02} s · {clips} clips · {items} overlays",
                            seconds / 100,
                            seconds % 100
                        ),
                        SMALL.w_px(duration_width),
                    );
                    ui.style(
                        &label,
                        &States::of(Look::NONE.text(QUIET)),
                        UiState::default(),
                    );
                }
                if !compact
                    && tool(
                        ui,
                        10,
                        "Zoom to selection",
                        false,
                        !state.selected().is_empty(),
                    )
                    && let Some(project) = project
                {
                    state.zoom_selection(project, view.width, view.end);
                }
                if tool(
                    ui,
                    11,
                    "Fit timeline",
                    false,
                    geo.scale > geo.fit || geo.origin > 0.0,
                ) {
                    state.fit();
                }
                if tool(ui, 12, "Zoom timeline out", false, geo.scale > geo.fit) {
                    state.zoom_about(1.0 / 1.4, view.width, view.at, view.end, fps);
                }
                if !compact {
                    let zoom = (geo.scale / geo.fit).ln()
                        / (geo.max_scale / geo.fit).ln().max(f32::EPSILON);
                    ui.stack(
                        Node::DEFAULT
                            .w_px(80.0)
                            .h_px(24.0)
                            .margin(Edges::xy(4.0, 0.0)),
                        |ui| {
                            tile(ui, 1, 0.0, 11.0, 80.0, 2.0, Rgba::rgb(56, 56, 59));
                            tile(ui, 2, 73.0 * zoom, 6.0, 7.0, 12.0, INK);
                            let slider = ui.scrub(
                                "timeline-zoom",
                                zoom,
                                0.0..=1.0,
                                &Scrubber::DEFAULT.step(0.001).name("Timeline zoom"),
                            );
                            ui.set_node_layout(
                                &slider.response.handle,
                                &Node::DEFAULT.w_px(80.0).h_px(24.0),
                            );
                            if let Some(zoom) = slider.preview.or(slider.commit) {
                                geo.zoom(
                                    state,
                                    geo.fit * (geo.max_scale / geo.fit).powf(zoom),
                                    view.at,
                                );
                            }
                        },
                    );
                }
                if tool(ui, 13, "Zoom timeline in", false, geo.scale < geo.max_scale) {
                    state.zoom_about(1.4, view.width, view.at, view.end, fps);
                }
            },
        );
    });
}

fn tool(ui: &mut Ui, icon: u32, name: &str, selected: bool, enabled: bool) -> bool {
    let button = IconButton::sized(26.0, 26.0)
        .look(&TOOLS)
        .disabled(!enabled);
    let button = if icon <= 3 {
        button.selected(selected)
    } else {
        button
    };
    ui.icon_button(u64::from(icon), Icon::draw(tool_icon, icon), name, &button)
        .clicked
}

fn tool_separator(ui: &mut Ui, key: &'static str) {
    ui.node(
        key,
        plate(Rgba::rgb(56, 56, 59))
            .w_px(1.0)
            .h_px(16.0)
            .margin(Edges::xy(4.0, 0.0)),
    );
}

fn tool_icon(paint: &mut Paint<'_>, rect: Rect, ink: Rgba, icon: u32) {
    let side = rect.w.min(rect.h).min(15.0);
    let scale = side / 24.0;
    let x = rect.x + (rect.w - side) * 0.5;
    let y = rect.y + (rect.h - side) * 0.5;
    let point = |x1: f32, y1: f32| {
        (
            x + if icon == 17 { 24.0 - x1 } else { x1 } * scale,
            y + y1 * scale,
        )
    };
    let stroke = Stroke::new(ink);
    let paths: &[&[(f32, f32)]] = match icon {
        0 => &[&[(6., 4.), (18., 11.5), (12.4, 12.6), (9.6, 18.), (6., 4.)]],
        1 => &[
            &[(8., 8.5), (11., 11.5)],
            &[(14., 14.5), (20., 20.)],
            &[(8., 15.5), (20., 4.)],
        ],
        2 => &[
            &[
                (6., 4.),
                (6., 12.),
                (6.8, 15.),
                (9., 17.2),
                (12., 18.),
                (15., 17.2),
                (17.2, 15.),
                (18., 12.),
                (18., 4.),
                (14., 4.),
                (14., 12.),
                (13.4, 13.4),
                (12., 14.),
                (10.6, 13.4),
                (10., 12.),
                (10., 4.),
                (6., 4.),
            ],
            &[(6., 8.), (10., 8.)],
            &[(14., 8.), (18., 8.)],
        ],
        3 => &[
            &[(4., 5.), (4., 19.)],
            &[(19., 12.), (8., 12.)],
            &[(12., 8.), (8., 12.), (12., 16.)],
        ],
        4 => &[
            &[(12., 3.), (12., 21.)],
            &[(7., 8.), (4., 12.), (7., 16.)],
            &[(17., 8.), (20., 12.), (17., 16.)],
        ],
        5 => &[
            &[(5., 7.), (5., 5.), (19., 5.), (19., 7.)],
            &[(12., 5.), (12., 19.)],
            &[(9., 19.), (15., 19.)],
        ],
        6 => &[&[(12., 4.), (20., 12.), (12., 20.), (4., 12.), (12., 4.)]],
        7 => &[&[
            (6., 4.),
            (18., 4.),
            (18., 14.),
            (12., 20.),
            (6., 14.),
            (6., 4.),
        ]],
        8 => &[
            &[(12., 15.), (12., 4.)],
            &[(8., 8.), (12., 4.), (16., 8.)],
            &[(4., 15.), (4., 20.), (20., 20.), (20., 15.), (4., 15.)],
        ],
        9 => &[
            &[(4., 7.), (20., 7.)],
            &[(9., 7.), (9., 4.), (15., 4.), (15., 7.)],
            &[(6., 7.), (7., 20.), (17., 20.), (18., 7.)],
            &[(10., 11.), (10., 16.)],
            &[(14., 11.), (14., 16.)],
        ],
        10 => &[
            &[(4., 8.), (4., 4.), (8., 4.)],
            &[(16., 4.), (20., 4.), (20., 8.)],
            &[(20., 16.), (20., 20.), (16., 20.)],
            &[(8., 20.), (4., 20.), (4., 16.)],
            &[(9., 9.), (15., 9.), (15., 15.), (9., 15.), (9., 9.)],
        ],
        11 => &[
            &[(4., 6.), (4., 18.)],
            &[(20., 6.), (20., 18.)],
            &[(8., 12.), (16., 12.)],
            &[(10., 10.), (8., 12.), (10., 14.)],
            &[(14., 10.), (16., 12.), (14., 14.)],
        ],
        12 => &[&[(6., 12.), (18., 12.)]],
        13 => &[&[(6., 12.), (18., 12.)], &[(12., 6.), (12., 18.)]],
        14 => &[&[(9., 6.), (15., 12.), (9., 18.)]],
        16 | 17 => &[
            &[(9., 5.), (4., 10.), (9., 15.)],
            &[
                (4., 10.),
                (14., 10.),
                (16.5, 10.7),
                (18.3, 12.5),
                (19., 15.),
                (18.3, 17.5),
                (16.5, 19.3),
                (14., 20.),
                (12., 20.),
            ],
        ],
        _ => &[&[(6., 9.), (12., 15.), (18., 9.)]],
    };
    for path in paths {
        for segment in path.windows(2) {
            paint.line(
                point(segment[0].0, segment[0].1),
                point(segment[1].0, segment[1].1),
                stroke,
            );
        }
    }
    if icon == 1 {
        for y in [6.5, 17.5] {
            paint.circle(point(6.0, y), 2.5 * scale, None, Some(stroke));
        }
    }
}

fn clock(ui: &mut Ui, at: i32, fps: u32) {
    let at = at.max(0) as u32;
    let seconds = at / fps;
    ui.label_digits(
        format_args!("{:02}:{:02}:{:02}", seconds / 60, seconds % 60, at % fps),
        SMALL.color(ColorToken::ACCENT),
    );
}

fn ruler(
    ui: &mut Ui,
    project: Option<&Project>,
    view: &View,
    geo: Geometry,
    fps: u32,
    intents: &mut Intents,
) -> Handle {
    ui.stack(
        Node {
            w: Size::Grow,
            h: Size::Px(RULER_HEIGHT),
            flags: Flags::BLOCKING,
            ..plate(SURFACE)
        },
        |ui| {
            ui.row(
                plate(CONTROL)
                    .w_px(geo.labels)
                    .h_px(RULER_HEIGHT)
                    .pad_x(10.0)
                    .align(Align::Center),
                |ui| {
                    clock(ui, view.at, fps);
                },
            );
            let step = geo.ticks(fps);
            let end = geo.origin + geo.width / geo.scale;
            let first = (geo.origin / step as f32).floor() as i32;
            let last = (end / step as f32).floor() as i32;
            for index in first..=last {
                let frame = index.saturating_mul(step);
                let x = geo.x(frame as f32);
                if x >= 0.0 {
                    tile(
                        ui,
                        100 + index as u64 * 8,
                        geo.labels + x,
                        18.0,
                        1.0,
                        6.0,
                        QUIET,
                    );
                    if x + 48.0 < geo.width {
                        ui.stack(Node::DEFAULT.at(geo.labels + x + 4.0, 2.0), |ui| {
                            let frame = frame as u32;
                            let seconds = frame / fps;
                            if step >= fps as i32 {
                                ui.label_digits(
                                    format_args!("{}:{:02}", seconds / 60, seconds % 60),
                                    CAPTION,
                                );
                            } else {
                                ui.label_digits(
                                    format_args!(
                                        "{:02}:{:02}:{:02}",
                                        seconds / 60,
                                        seconds % 60,
                                        frame % fps
                                    ),
                                    CAPTION,
                                );
                            }
                        });
                    }
                }
                let spacing = step as f32 * geo.scale / 5.0;
                if spacing >= 10.0 {
                    for minor in 1..5 {
                        let x = x + minor as f32 * spacing;
                        if (0.0..geo.width).contains(&x) {
                            tile(
                                ui,
                                100 + index as u64 * 8 + minor,
                                geo.labels + x,
                                21.0,
                                1.0,
                                3.0,
                                LINE,
                            );
                        }
                    }
                }
            }
            if view.end > 0 {
                ui.stack(
                    Node::DEFAULT
                        .at(geo.labels, 0.0)
                        .w_px(geo.width)
                        .h_px(RULER_HEIGHT),
                    |ui| {
                        let scrub = ui.scrub(
                            "timeline-ruler",
                            (view.at as f32).clamp(geo.origin, end),
                            geo.origin..=end,
                            &Scrubber::DEFAULT.name("Timeline playhead"),
                        );
                        ui.set_node_layout(
                            &scrub.response.handle,
                            &Node::DEFAULT.w_px(geo.width).h_px(RULER_HEIGHT),
                        );
                        if scrub.response.secondary && project.is_some() {
                            let rect = ui.rect(&scrub.response.handle);
                            let point = ui.signal(&scrub.response.handle).press_point;
                            let at = blade_frame(geo, geo.labels + point.0 - rect.x)
                                .clamp(0, view.end.max(0));
                            intents
                                .state
                                .open_menu(ContextTarget::Background(at, None), point);
                        }
                        let at = scrub
                            .preview
                            .or(scrub.commit)
                            .map_or(view.at, |value| value.round() as i32);
                        if let Some(phase) = Phase::from_interaction(
                            view.dragging && view.state.scrubbing,
                            scrub.response.held,
                            scrub.commit.is_some(),
                        ) {
                            intents.scrub = Some((phase, at.clamp(0, view.end - 1)));
                            intents.state.scrubbing = scrub.response.held;
                        }
                    },
                );
            }
            if let Some(project) = project {
                for marker in &project.markers {
                    let x = geo.x(marker.at as f32);
                    if (0.0..geo.width).contains(&x) {
                        let width = 9.0_f32.min(geo.width - x);
                        ui.stack(
                            Node::DEFAULT.at(geo.labels + (x - 4.0).max(0.0), 12.0),
                            |ui| {
                                let selected =
                                    intents.state.contains(TimelineId::Marker(marker.id));
                                let look =
                                    States::of(Look::NONE.fill(WARN)).checked(Look::NONE.fill(INK));
                                let response = ui.button(
                                    marker.id.serial(),
                                    "",
                                    &Button::sized(width, 11.0)
                                        .look(&look)
                                        .selected(selected)
                                        .name(&marker.name),
                                );
                                if response.clicked || response.secondary {
                                    intents
                                        .state
                                        .set_selection(Some(TimelineId::Marker(marker.id)), None);
                                }
                                if response.secondary {
                                    let point = ui.signal(&response.handle).press_point;
                                    intents
                                        .state
                                        .open_menu(ContextTarget::Marker(marker.at), point);
                                }
                            },
                        );
                    }
                }
            }
            tile(ui, 4, 0.0, 23.0, view.width, 1.0, LINE);
            let x = geo.x(view.at as f32);
            if (0.0..geo.width).contains(&x) {
                tile(ui, 5, geo.labels + x, 2.0, 1.0, 22.0, ACCENT);
                tile(
                    ui,
                    6,
                    geo.labels + (x - 4.0).max(0.0),
                    2.0,
                    9.0_f32.min(geo.width - x),
                    7.0,
                    ACCENT,
                );
            }
        },
    )
}

fn tracks(
    ui: &mut Ui,
    project: &Project,
    geo: Geometry,
    fps: u32,
    at: i32,
    intents: &mut Intents,
    cache: &mut Cache,
    (previews, evaluator): (&[MediaPreview], Option<&Evaluator>),
    (top, viewport_height): (f32, f32),
    drop_viewport: Option<DropViewport>,
) {
    let mut index = cache.first_visible_row(top);
    let bottom = top + viewport_height + TRACK_OVERSCAN;
    let leading = cache
        .rows
        .get(index)
        .map_or(cache.rows_height, |row| row.top);
    let mut drawn_until = leading;
    if leading > 0.0 {
        ui.node(
            "timeline-before-window",
            Node::DEFAULT.h_px(leading).shrink(0.0),
        );
    }
    while let Some(row) = cache
        .rows
        .get(index)
        .copied()
        .filter(|row| row.top < bottom)
    {
        drawn_until = row.top + row.height;
        match row.track {
            Track::Lane(i) => {
                let lane = &project.lanes[i];
                ui.id("timeline-lanes", |ui| {
                    ui.id(lane.id.serial(), |ui| {
                        overlay_lane(
                            ui,
                            project,
                            geo,
                            at,
                            row.top,
                            intents,
                            cache,
                            previews,
                            lane,
                            drop_viewport,
                        );
                    })
                });
            }
            Track::Spine => {
                ui.id("timeline-spine", |ui| {
                    spine_lane(ui, project, geo, fps, at, row.top, intents, cache, previews)
                });
                // A spine or clip disclosure can change the rows below it on this build.
                cache.refresh_rows(project, &intents.state);
            }
            Track::Take(slot) => {
                if let Some((id, timing)) = project.clips.id(slot).zip(project.clips.timing(slot)) {
                    ui.id("timeline-takes", |ui| {
                        ui.id(id.serial(), |ui| {
                            take_head(ui, project, slot, id, timing, geo, intents);
                        })
                    });
                }
            }
            Track::Component(slot, component) => {
                if let Some((id, timing)) = project.clips.id(slot).zip(project.clips.timing(slot))
                    && let Some(enabled) = component.enabled
                {
                    ui.id("timeline-components", |ui| {
                        ui.id(id.serial(), |ui| {
                            ui.id(component_key(component.kind), |ui| {
                                let range = geo.visible(timing.start, timing.length);
                                let visual =
                                    range.map_or_else(BlockMedia::default, |(x, width)| {
                                        take_component_preview(
                                            cache,
                                            previews,
                                            project,
                                            slot,
                                            timing,
                                            component,
                                            enabled,
                                            (geo, x, width),
                                        )
                                    });
                                let row = component_row(
                                    ui, id, component, enabled, range, visual, geo, intents,
                                );
                                if enabled
                                    && matches!(
                                        component.kind,
                                        TakeComponent::Pointer | TakeComponent::Keys
                                    )
                                    && let Some((x, width)) = range
                                    && let Ok(style) = cache
                                        .take_events
                                        .binary_search_by_key(&slot, |entry| entry.slot)
                                    && let Some(data) = project.clips.data(slot)
                                    && let Some(preview) =
                                        data.media.and_then(|media| preview_for(previews, media))
                                    && let Some(clock) = evaluator
                                        .and_then(|evaluator| {
                                            evaluator.clip_clock(id, timing.start)
                                        })
                                        .or_else(|| {
                                            affine_event_span(timing, data, fps, preview.events.fps)
                                                .map(|span| (span, &[][..]))
                                        })
                                {
                                    ui.within(&row, &mut |ui| {
                                        take_event_marks(
                                            ui,
                                            &preview.events,
                                            (id, timing, component),
                                            (geo, x, width),
                                            clock,
                                            (&cache.take_events[style], intents),
                                        )
                                    });
                                }
                            })
                        })
                    });
                }
            }
        }
        index += 1;
    }
    let trailing = cache.rows_height - drawn_until;
    if trailing > 0.0 {
        ui.node(
            "timeline-after-window",
            Node::DEFAULT.h_px(trailing).shrink(0.0),
        );
    }
}

fn update_marquee(
    signal: &Signal,
    viewport: Rect,
    scroll: f32,
    shift: bool,
    project: &Project,
    cache: &Cache,
    geo: Geometry,
    state: &mut State,
) -> bool {
    let local = |point: (f32, f32)| {
        [
            (point.0 - viewport.x).clamp(geo.labels, (geo.labels + geo.width).max(geo.labels)),
            (point.1 - viewport.y + scroll).clamp(0.0, cache.rows_height),
        ]
    };
    if signal.pressed
        && signal.button == Some(Signal::BUTTON_PRIMARY)
        && signal.press_point.0 - viewport.x >= geo.labels
    {
        let start = local(signal.press_point);
        state.marquee = Some(Marquee {
            start,
            pointer: start,
            additive: shift,
        });
    }
    let Some(mut marquee) = state.marquee else {
        return false;
    };
    if !signal.held && !signal.released {
        state.marquee = None;
        return true;
    }
    marquee.pointer = local(if signal.released {
        signal.release_point
    } else {
        signal.pointer
    });
    if !signal.released {
        state.marquee = Some(marquee);
        return true;
    }
    state.marquee = None;
    if !marquee.moved() {
        if !marquee.additive {
            state.set_selection(None, None);
        }
        return true;
    }
    let rect = marquee.rect();
    let mut items = if marquee.additive {
        state.selected().to_vec()
    } else {
        Vec::new()
    };
    let rows = cache
        .rows
        .partition_point(|row| row.top + row.height <= rect.y);
    for row in cache.rows[rows..]
        .iter()
        .take_while(|row| row.top < rect.y + rect.h)
    {
        match row.track {
            Track::Lane(lane) => {
                for item in cache.lane(project.lanes[lane].id) {
                    if let Some((start, id)) = item.start.zip(project.items.id(item.slot))
                        && marquee_hits(rect, geo, row.top, row.height, start, item.length)
                    {
                        items.push(TimelineId::Item(id));
                    }
                }
            }
            Track::Spine => {
                for &slot in project.clips.order() {
                    if let Some((timing, id)) =
                        project.clips.timing(slot).zip(project.clips.id(slot))
                        && marquee_hits(rect, geo, row.top, row.height, timing.start, timing.length)
                    {
                        items.push(TimelineId::Clip(id));
                    }
                }
            }
            Track::Take(_) | Track::Component(_, _) => {}
        }
    }
    state.set_selected(items);
    true
}

fn marquee_hits(rect: Rect, geo: Geometry, row: f32, height: f32, start: i32, length: i32) -> bool {
    row + 2.0 < rect.y + rect.h
        && row + height - 2.0 > rect.y
        && geo
            .visible(start, length)
            .is_some_and(|(x, width)| x < rect.x + rect.w && x + width > rect.x)
}

fn compatible_drop(
    project: &Project,
    cache: &Cache,
    geo: Geometry,
    viewport: DropViewport,
    pointer: (f32, f32),
    source: LaneId,
    kind: LaneKind,
) -> Option<LaneId> {
    let x = pointer.0 - viewport.rect.x;
    if !viewport.rect.contains(pointer) || !(geo.labels..geo.labels + geo.width).contains(&x) {
        return None;
    }
    let y = pointer.1 - viewport.rect.y + viewport.scroll;
    let index = cache.rows.partition_point(|row| row.top + row.height <= y);
    let row = cache.rows.get(index)?;
    let Track::Lane(index) = row.track else {
        return None;
    };
    let lane = &project.lanes[index];
    (y >= row.top && lane.id != source && lane.kind == kind).then_some(lane.id)
}

fn track_overlay(
    ui: &mut Ui,
    (viewport, ruler): (&Handle, &Handle),
    project: &Project,
    cache: &Cache,
    geo: Geometry,
    (height, at, fps, dragging): (f32, i32, u32, bool),
    scroll: pxui::Scroll,
    intents: &Intents,
) {
    if height <= 0.0 {
        return;
    }
    let state = &intents.state;
    let guide = (!dragging
        && !state.scrubbing
        && !state.menu_open()
        && state.drag.is_none()
        && state.marquee.is_none())
    .then(|| {
        let pointer = ui.signal(viewport).pointer;
        let area = if ui.hovered(viewport, true) {
            ui.rect(viewport)
        } else if ui.hovered(ruler, true) {
            ui.rect(ruler)
        } else {
            return None;
        };
        let x = pointer.0 - area.x;
        (area.contains(pointer) && (geo.labels..geo.labels + geo.width).contains(&x))
            .then(|| blade_frame(geo, x).max(0))
    })
    .flatten();
    let blade = guide.and(intents.blade_hover);
    let marquee = state
        .marquee
        .filter(|marquee| marquee.moved())
        .map(Marquee::rect);
    let drag = state
        .drag
        .filter(|drag| intents.drag_seen && drag.moved)
        .and_then(|drag| drag_preview(project, cache, geo, drag));
    let viewport_slot = viewport.box_slot;
    let stretch = scroll.painted - scroll.settled;
    let clip = ui.node(
        "timeline-overlay-clip",
        Node::stack()
            .at(geo.labels, 0.0)
            .w_px(geo.width)
            .h_px(height)
            .flags(Flags::CLIP)
            .z(8),
    );
    ui.within(&clip, &mut |ui| {
        let art = ui.node(
            "timeline-overlay",
            Node::DEFAULT.w_px(geo.width).h_px(height),
        );
        let guides = Rc::clone(&cache.guides);
        let state = state.clone();
        ui.canvas(&art, move |paint, rect| {
            let scroll = paint.scroll_y(viewport_slot) + stretch;
            paint_track_guides(paint, rect, geo, scroll, &guides, &state);
            if let Some(frame) = guide {
                let x = geo.x(frame as f32).clamp(0.0, (rect.w - 1.0).max(0.0));
                paint.fill(
                    Rect::new(rect.x + x, rect.y, 1.0, rect.h),
                    Fill::new(Rgba::rgba(INK.r, INK.g, INK.b, 77)),
                );
            }
            let x = geo.x(at as f32);
            if (0.0..rect.w).contains(&x) {
                paint.fill(
                    Rect::new(rect.x + x, rect.y, 1.0, rect.h),
                    Fill::new(ACCENT),
                );
            }
            if let Some((frame, top, height)) = blade {
                let x = rect.x + geo.x(frame as f32);
                let y = rect.y + top - scroll;
                paint.dashed(
                    (x + 0.75, y),
                    (x + 0.75, y + height),
                    Stroke::new(INK).width(1.5),
                );
            }
            if let Some(drag) = &drag {
                paint_drag_preview(paint, rect, scroll, drag);
            }
            if let Some(local) = marquee {
                let bounds = Rect::new(
                    rect.x + local.x - geo.labels,
                    rect.y + local.y - scroll,
                    local.w,
                    local.h,
                )
                .intersect(rect);
                if bounds.w > 0.0 && bounds.h > 0.0 {
                    paint.fill(bounds, Fill::new(Rgba::rgba(244, 243, 238, 26)));
                    paint.stroke(bounds, Stroke::new(INK));
                }
            }
        });
        if let Some(frame) = guide {
            let (frame, y, fill, ink) = blade.map_or(
                (frame, 4.0, CONTROL, Rgba::rgb(177, 177, 171)),
                |(frame, top, _)| (frame, top - scroll.painted - 14.0, INK, SURFACE),
            );
            let frame = frame as u32;
            let seconds = frame / fps;
            let minutes = seconds / 60;
            let digits = minutes.checked_ilog10().map_or(2, |n| (n + 1).max(2));
            let frames = frame % fps;
            let frame_digits = frames.checked_ilog10().map_or(2, |n| (n + 1).max(2));
            let width = ((digits + frame_digits + 4) as f32 * 6.5 + 8.0).min(geo.width);
            let label_x = (geo.x(frame as f32) + 4.0).clamp(0.0, (geo.width - width).max(0.0));
            ui.row(
                plate(fill)
                    .at(label_x, y.clamp(0.0, (height - 14.0).max(0.0)))
                    .w_px(width)
                    .h_px(14.0)
                    .pad_x(4.0)
                    .align(Align::Center),
                |ui| {
                    let label = ui.label_digits(
                        format_args!("{minutes:02}:{:02}:{frames:02}", seconds % 60),
                        CAPTION.pad(0.0).role(Role::Unknown),
                    );
                    ui.style(
                        &label,
                        &States::of(Look::NONE.text(ink)),
                        pxui::ui::State::default(),
                    );
                },
            );
        }
    });
}

fn paint_track_guides(
    paint: &mut Paint,
    rect: Rect,
    geo: Geometry,
    scroll: f32,
    guides: &TrackGuides,
    state: &State,
) {
    let stem_end = rect.y + guides.spine_end - scroll;
    let anchors = if stem_end + ANCHOR_DIAMOND_RADIUS > rect.y {
        let from = guides
            .anchors
            .partition_point(|guide| geo.x(guide.frame as f32) < -1.0);
        let to = guides
            .anchors
            .partition_point(|guide| geo.x(guide.frame as f32) < rect.w + 1.0);
        &guides.anchors[from..to]
    } else {
        &[]
    };
    for guide in anchors.iter().filter(|guide| !state.contains(guide.item)) {
        paint_anchor_guide(paint, rect, geo, scroll, stem_end, guide, false);
    }
    let marker_fill = Fill::new(Rgba::rgba(WARN.r, WARN.g, WARN.b, 89));
    let from = guides
        .markers
        .partition_point(|&frame| geo.x(frame as f32) < 0.0);
    for &frame in &guides.markers[from..] {
        let x = geo.x(frame as f32);
        if x >= rect.w {
            break;
        }
        let x = rect.x + x;
        paint.grid(
            Rect::new(x, rect.y, 1.0, rect.h),
            (x, rect.y),
            6.0,
            3.0,
            marker_fill,
        );
    }
    for guide in anchors.iter().filter(|guide| state.contains(guide.item)) {
        paint_anchor_guide(paint, rect, geo, scroll, stem_end, guide, true);
    }
}

fn paint_anchor_guide(
    paint: &mut Paint,
    rect: Rect,
    geo: Geometry,
    scroll: f32,
    end: f32,
    guide: &AnchorGuide,
    selected: bool,
) {
    let x = rect.x + geo.x(guide.frame as f32);
    let y = rect.y + guide.top - scroll;
    let stem = Rect::new(x, y, 1.0, end - y).intersect(rect);
    let tone = guide.tone;
    if stem.w > 0.0 && stem.h > 0.0 {
        if selected {
            paint.fill(stem, Fill::new(tone));
        } else {
            let fill = Fill::new(Rgba::rgba(tone.r, tone.g, tone.b, 140));
            paint.grid(stem, (x, y), 2.0, 1.0, fill);
        }
    }
    let y = end - 1.5;
    let r = ANCHOR_DIAMOND_RADIUS;
    if y + r > rect.y && y - r < rect.y + rect.h {
        let x = x + 0.5;
        let fill = Fill::new(Rgba::rgba(
            tone.r,
            tone.g,
            tone.b,
            if selected { 255 } else { 179 },
        ));
        paint.triangle((x - r, y), (x, y - r), (x + r, y), fill);
        paint.triangle((x - r, y), (x + r, y), (x, y + r), fill);
    }
}

#[derive(Clone, Copy)]
struct DragPreview {
    row: TrackRow,
    tone: Rgba,
    highlight: bool,
    ghost: Option<(f32, f32)>,
    snap: Option<f32>,
}

fn drag_preview(
    project: &Project,
    cache: &Cache,
    geo: Geometry,
    drag: ItemDrag,
) -> Option<DragPreview> {
    let (index, tone, length) = match drag.item {
        TimelineId::Item(id) => {
            let slot = project.items.find(id)?;
            let source = project.items.data(slot)?.lane;
            let target = drag.lane.unwrap_or(source);
            let index = project.lanes.iter().position(|lane| lane.id == target)?;
            (
                index,
                lane_tone(project.lanes[index].kind),
                project.items.timing(slot)?.length,
            )
        }
        TimelineId::Clip(id) => {
            let slot = project.clips.find(id)?;
            let tone = if project.clips.data(slot)?.media.is_some() {
                VIDEO
            } else {
                QUIET
            };
            (
                project.lanes.len(),
                tone,
                project.clips.timing(slot)?.length,
            )
        }
        TimelineId::Marker(_) => return None,
    };
    let (start, length) = drag.edge.preview(drag.start, length, drag.to);
    Some(DragPreview {
        row: *cache.rows.get(index)?,
        tone,
        highlight: drag.lane.is_some(),
        ghost: geo.visible(start, length).map(|(x, w)| (x - geo.labels, w)),
        snap: drag.snap.map(|at| geo.x(at as f32)),
    })
}

fn paint_drag_preview(paint: &mut Paint, rect: Rect, scroll: f32, drag: &DragPreview) {
    let y = rect.y + drag.row.top - scroll;
    let track = Rect::new(rect.x, y, rect.w, drag.row.height).intersect(rect);
    if track.h <= 0.0 {
        return;
    }
    let tone = drag.tone;
    if drag.highlight {
        paint.fill(track, Fill::new(Rgba::rgba(tone.r, tone.g, tone.b, 24)));
    }
    if let Some((x, width)) = drag.ghost {
        let bounds = Rect::new(rect.x + x, y + 2.0, width, drag.row.height - 4.0).intersect(track);
        if bounds.w > 0.0 && bounds.h > 0.0 {
            paint.fill(bounds, Fill::new(blend(tone, 26)));
            paint.stroke(bounds, Stroke::new(tone));
        }
    }
    if let Some(x) = drag.snap.filter(|x| (0.0..track.w).contains(x)) {
        paint.fill(
            Rect::new(track.x + x, track.y, 1.0, track.h),
            Fill::new(INK),
        );
    }
}

fn track_grid(ui: &mut Ui, row: &Handle, geo: Geometry, fps: u32) {
    let step = geo.ticks(fps);
    let first = (geo.origin / step as f32).ceil() as i32;
    let last = ((geo.origin + geo.width / geo.scale) / step as f32).floor() as i32;
    ui.canvas(row, move |paint, rect| {
        let left = rect.x + geo.labels;
        let height = (rect.h - 1.0).max(0.0);
        let ink = Fill::new(Rgba::rgba(255, 255, 255, 10));
        for index in first..=last {
            let x = geo.x(index.saturating_mul(step) as f32);
            let width = (geo.width - x).min(1.0);
            if x >= 0.0 && width > 0.0 {
                paint.fill(Rect::new(left + x, rect.y, width, height), ink);
            }
        }
        paint.fill(
            Rect::new(left, rect.y + height, geo.width, 1.0),
            Fill::new(SURFACE),
        );
    });
}

fn overlay_lane(
    ui: &mut Ui,
    project: &Project,
    geo: Geometry,
    at: i32,
    top: f32,
    intents: &mut Intents,
    cache: &Cache,
    previews: &[MediaPreview],
    lane: &splice_model::Lane,
    drop_viewport: Option<DropViewport>,
) {
    let items = cache.lane(lane.id);
    let tone = lane_tone(lane.kind);
    let row = ui.stack(
        plate(Rgba::rgb(16, 16, 18))
            .w_px(geo.labels + geo.width)
            .h_px(LANE_HEIGHT),
        |ui| {
            for item in items {
                let Some(start) = item.start else { continue };
                let Some((x, width)) = geo.visible(start, item.length) else {
                    continue;
                };
                let Some((data, id)) = project
                    .items
                    .data(item.slot)
                    .zip(project.items.id(item.slot))
                else {
                    continue;
                };
                let Some(timing) = project.items.timing(item.slot) else {
                    continue;
                };
                let name = data.appearance.name.as_deref().or(item.label.as_deref()).unwrap_or(match &data.kind {
                    ItemKind::Title { text } => text,
                    ItemKind::Media(_) => "Media",
                    ItemKind::Motion(_) => "Scale animation",
                    ItemKind::Zoom(_) => "Zoom",
                    ItemKind::Sound(_) => "Sound",
                    ItemKind::Shape { .. } => "Shape",
                    ItemKind::Caption { text, .. } => text.as_deref().unwrap_or("Caption"),
                });
                let visual = match data.kind {
                    ItemKind::Media(overlay) => BlockMedia {
                        thumbnail: preview_for(previews, overlay.media)
                            .and_then(MediaPreview::filmstrip),
                        ..BlockMedia::default()
                    },
                    ItemKind::Sound(sound) => BlockMedia {
                        waveform: audio_peaks(
                            previews,
                            project,
                            (sound.media, sound.source, 1.0, sound.volume),
                            (geo, start, x, width),
                        ),
                        ..BlockMedia::default()
                    },
                    _ => BlockMedia::default(),
                };
                block(
                    ui,
                    TimelineId::Item(id),
                    name,
                    (x, width),
                    LANE_HEIGHT,
                    tone,
                    (geo, at, start, item.length, top),
                    false,
                    (cache, project),
                    visual,
                    BlockDecor {
                        fades: if matches!(data.kind, ItemKind::Motion(_) | ItemKind::Zoom(_)) {
                            [0; 2]
                        } else {
                            [data.appearance.fade_in, data.appearance.fade_out]
                        },
                        fade_handles: matches!(
                            data.kind,
                            ItemKind::Title { .. } | ItemKind::Media(_) | ItemKind::Caption { .. }
                        ),
                        pinned: matches!(timing.anchor, Anchor::Pinned(_)),
                        behind: matches!(data.kind, ItemKind::Media(overlay) if overlay.behind_presenter),
                        speed: None,
                        duration: None,
                        placement: match data.kind {
                            ItemKind::Media(overlay) => PLACEMENTS
                                .iter()
                                .find_map(|&(place, name)| (place == overlay.placement).then_some(name)),
                            _ => None,
                        },
                        motion: matches!(data.kind, ItemKind::Motion(_) | ItemKind::Zoom(_)),
                    },
                    item.fx.as_ref(),
                    item.caption.as_ref().map(|caption| &caption.ticks),
                    drop_viewport
                        .map(|viewport| (project, lane.id, data.kind.lane_kind(), viewport)),
                    intents,
                );
            }
            track_head(
                ui,
                project,
                lane.id,
                &lane.name,
                items.len(),
                geo.labels,
                LANE_HEIGHT,
                Some(lane.kind),
                0.0,
                cache.lane_fx(lane.id),
                intents,
            );
        },
    );
    track_grid(ui, &row, geo, project.fps.max(1));
}

fn spine_lane(
    ui: &mut Ui,
    project: &Project,
    geo: Geometry,
    fps: u32,
    at: i32,
    top: f32,
    intents: &mut Intents,
    cache: &Cache,
    previews: &[MediaPreview],
) {
    let row = ui.stack(
        plate(Rgba::rgb(19, 19, 21))
            .w_px(geo.labels + geo.width)
            .h_px(SPINE_HEIGHT),
        |ui| {
            track_head(
                ui,
                project,
                project.spine.id,
                "Video · spine",
                project.clips.order().len(),
                geo.labels,
                SPINE_HEIGHT,
                None,
                if cache.has_components { 24.0 } else { 0.0 },
                cache.lane_fx(project.spine.id),
                intents,
            );
            if cache.has_components {
                let expanded = intents.state.all_components || intents.state.expanded.is_some();
                let name = if expanded {
                    "Hide take components"
                } else {
                    "Show take components"
                };
                ui.stack(Node::DEFAULT.at(10.0, (SPINE_HEIGHT - 18.0) * 0.5), |ui| {
                    if take_disclosure(ui, "timeline-all-components", name, expanded) {
                        intents.state.all_components = !expanded;
                        intents.state.expanded = None;
                    }
                });
            }
            let clips = project.clips.order();
            let first = clips.partition_point(|&slot| {
                project
                    .clips
                    .timing(slot)
                    .and_then(ClipTiming::end)
                    .is_some_and(|end| geo.x(end as f32) <= 0.0)
            });
            for &slot in &clips[first..] {
                let Some((id, timing)) = project.clips.id(slot).zip(project.clips.timing(slot))
                else {
                    continue;
                };
                if geo.x(timing.start as f32) >= geo.width {
                    break;
                }
                let Some((x, width)) = geo.visible(timing.start, timing.length) else {
                    continue;
                };
                let Some(data) = project.clips.data(slot) else {
                    continue;
                };
                let name = clip_name(project, slot);
                let tone = if data.media.is_some() { VIDEO } else { QUIET };
                block(
                    ui,
                    TimelineId::Clip(id),
                    name,
                    (x, width),
                    SPINE_HEIGHT,
                    tone,
                    (geo, at, timing.start, timing.length, top),
                    has_components(data),
                    (cache, project),
                    BlockMedia {
                        thumbnail: data
                            .media
                            .and_then(|media| preview_for(previews, media))
                            .and_then(MediaPreview::filmstrip),
                        ..BlockMedia::default()
                    },
                    BlockDecor {
                        fades: if data.media.is_some() {
                            [data.appearance.fade_in, data.appearance.fade_out]
                        } else {
                            [0; 2]
                        },
                        fade_handles: data.media.is_some(),
                        duration: Some((timing.length, fps)),
                        speed: (timing.speed.is_finite()
                            && timing.speed > 0.0
                            && timing.speed != 1.0)
                            .then_some(timing.speed),
                        ..BlockDecor::default()
                    },
                    cache.clip_fx(slot),
                    None,
                    None,
                    intents,
                );
            }
        },
    );
    track_grid(ui, &row, geo, fps);
}

fn clip_name(project: &Project, slot: Slot) -> &str {
    let data = project.clips.data(slot).expect("live clip has data");
    data.appearance.name.as_deref().unwrap_or_else(|| {
        data.media
            .and_then(|id| media_stem(project, id))
            .unwrap_or(if data.media.is_some() { "Video" } else { "Gap" })
    })
}

fn media_stem(project: &Project, id: MediaId) -> Option<&str> {
    let media = project.media.iter().find(|media| media.id == id)?;
    viewer_drawers::media_title(media)
}

fn resolved_item_chrome(
    project: &Project,
    data: &ItemData,
    timing: splice_model::ItemTiming,
    start: Option<i32>,
) -> (Option<String>, Option<CaptionChrome>) {
    if let ItemKind::Caption { words, edits, text } = &data.kind {
        let free = text
            .as_deref()
            .zip(start)
            .map(|(text, at)| (text, at, at.saturating_add(timing.length)));
        let tokens = splice_av::caption_tokens(project, timing.anchor, words, edits, free, false)
            .unwrap_or_default();
        let mut label = String::new();
        let mut ticks = Vec::with_capacity(tokens.len());
        let mut menu_words = Vec::new();
        for token in tokens {
            if data.appearance.name.is_none() && free.is_none() && !token.hidden {
                if !label.is_empty() {
                    label.push(' ');
                }
                label.push_str(&token.text);
                if let Some(emoji) = token.emoji.as_deref() {
                    label.push(' ');
                    label.push_str(emoji);
                }
            }
            if start.is_some_and(|at| (at..at.saturating_add(timing.length)).contains(&token.start))
            {
                ticks.push(CaptionTick {
                    at: token.start,
                    emph: token.emph,
                });
            }
            if free.is_none() {
                menu_words.push(CaptionMenuWord {
                    id: token.id,
                    text: token.text,
                    at: token.start,
                    emph: token.emph,
                    hidden: token.hidden,
                });
            }
        }
        ticks.sort_unstable_by_key(|tick| (tick.at, !tick.emph));
        ticks.dedup_by_key(|tick| tick.at);
        return (
            (!label.is_empty()).then_some(label),
            Some(CaptionChrome {
                ticks: ticks.into(),
                words: menu_words,
            }),
        );
    }
    if data.appearance.name.is_some() {
        return (None, None);
    }
    let label = match &data.kind {
        ItemKind::Media(overlay) => media_stem(project, overlay.media).map(str::to_owned),
        ItemKind::Motion(motion) => Some(format!(
            "Scale animation {}→{}×",
            (motion.from * 100.0).round() / 100.0,
            (motion.to * 100.0).round() / 100.0,
        )),
        ItemKind::Zoom(zoom) => Some(format!("Scale animation 1→{}×", zoom.amount)),
        _ => None,
    };
    (label, None)
}

fn has_components(data: &ClipData) -> bool {
    let enabled = &data.enabled;
    [
        enabled.screen,
        enabled.camera,
        enabled.mic,
        enabled.system,
        enabled.pointer,
        enabled.keys,
    ]
    .into_iter()
    .any(|value| value.is_some())
}

#[derive(Clone, Copy)]
struct Component {
    kind: TakeComponent,
    name: &'static str,
    show: &'static str,
    hide: &'static str,
    tone: Rgba,
    enabled: Option<bool>,
    gain: Option<f64>,
}

pub(super) fn component_key(part: TakeComponent) -> &'static str {
    match part {
        TakeComponent::Screen => "screen",
        TakeComponent::Camera => "camera",
        TakeComponent::Mic => "mic",
        TakeComponent::System => "system",
        TakeComponent::Pointer => "pointer",
        TakeComponent::Keys => "keys",
    }
}

impl Component {
    fn height(self) -> f32 {
        if self.gain.is_some() {
            AUDIO_COMPONENT_HEIGHT
        } else {
            COMPONENT_HEIGHT
        }
    }
}

fn components(data: &ClipData) -> [Component; 6] {
    let sound = Rgba::rgb(198, 214, 230);
    [
        Component {
            kind: TakeComponent::Screen,
            name: "Screen video",
            show: "Show screen video",
            hide: "Hide screen video",
            tone: VIDEO,
            enabled: data.enabled.screen,
            gain: None,
        },
        Component {
            kind: TakeComponent::Camera,
            name: "Camera video",
            show: "Show camera video",
            hide: "Hide camera video",
            tone: TAKE_SAGE,
            enabled: data.enabled.camera,
            gain: None,
        },
        Component {
            kind: TakeComponent::Mic,
            name: "Microphone",
            show: "Show microphone",
            hide: "Hide microphone",
            tone: sound,
            enabled: data.enabled.mic,
            gain: Some(data.volume.mic),
        },
        Component {
            kind: TakeComponent::System,
            name: "System audio",
            show: "Show system audio",
            hide: "Hide system audio",
            tone: sound,
            enabled: data.enabled.system,
            gain: Some(data.volume.system),
        },
        Component {
            kind: TakeComponent::Pointer,
            name: "Cursor",
            show: "Show cursor",
            hide: "Hide cursor",
            tone: Rgba::rgb(191, 224, 234),
            enabled: data.enabled.pointer,
            gain: None,
        },
        Component {
            kind: TakeComponent::Keys,
            name: "Keystrokes",
            show: "Show keystrokes",
            hide: "Hide keystrokes",
            tone: Rgba::rgb(225, 212, 236),
            enabled: data.enabled.keys,
            gain: None,
        },
    ]
}

fn take_disclosure(ui: &mut Ui, key: &'static str, name: &'static str, expanded: bool) -> bool {
    ui.icon_button(
        key,
        Icon::draw(tool_icon, if expanded { 15 } else { 14 }),
        name,
        &IconButton::sized(18.0, 18.0).look(&TAKE_CONTROL),
    )
    .clicked
}

fn take_eye(paint: &mut Paint<'_>, rect: Rect, ink: Rgba, enabled: u32) {
    let x = rect.x + rect.w * 0.5;
    let y = rect.y + rect.h * 0.5;
    let stroke = Stroke::new(ink);
    let (left, right) = ((x - 5.0, y), (x + 5.0, y));
    paint.curve(left, (x, y - 6.0), right, stroke);
    paint.curve(left, (x, y + 6.0), right, stroke);
    if enabled != 0 {
        paint.circle((x, y), 1.5, None, Some(stroke));
    } else {
        paint.line((x - 4.0, y + 4.0), (x + 4.0, y - 4.0), stroke);
    }
}

fn take_head(
    ui: &mut Ui,
    project: &Project,
    slot: Slot,
    id: ClipId,
    timing: ClipTiming,
    geo: Geometry,
    intents: &mut Intents,
) {
    let Some(data) = project.clips.data(slot) else {
        return;
    };
    let range = geo.visible(timing.start, timing.length);
    let count = components(data)
        .iter()
        .filter(|component| component.enabled.is_some())
        .count();

    ui.stack(
        plate(TAKE_TRACK)
            .w_px(geo.labels + geo.width)
            .h_px(TAKE_HEAD_HEIGHT),
        |ui| {
            ui.row(
                plate(TAKE_LABEL)
                    .w_px(geo.labels)
                    .h_px(TAKE_HEAD_HEIGHT)
                    .pad_x(6.0)
                    .flags(Flags::CLIP)
                    .align(Align::Center),
                |ui| {
                    if take_disclosure(ui, "collapse-components", "Collapse take components", true)
                    {
                        intents.state.toggle_take(id);
                    }
                    let label = ui.label(
                        clip_name(project, slot),
                        CAPTION
                            .pad(0.0)
                            .ellipsis()
                            .w_px((geo.labels - 30.0).max(0.0)),
                    );
                    ui.style(
                        &label,
                        &States::of(Look::NONE.text(QUIET)),
                        pxui::ui::State::default(),
                    );
                },
            );
            if let Some((x, width)) = range {
                let bracket = Rgba::rgba(TAKE_SAGE.r, TAKE_SAGE.g, TAKE_SAGE.b, 102);
                tile(ui, 10, x, 9.0, width, 1.0, bracket);
                if geo.x(timing.start as f32) >= 0.0 {
                    tile(ui, 11, x, 10.0, width.min(1.0), 5.0, bracket);
                }
                if geo.x(timing.start.saturating_add(timing.length) as f32) <= geo.width {
                    let cap = width.min(1.0);
                    tile(ui, 12, x + width - cap, 10.0, cap, 5.0, bracket);
                }
                if width >= 94.0 {
                    let label = CAPTION.pad(0.0);
                    let legend_width = ui
                        .text_service()
                        .measure("6 components · one source clock", label, None)
                        .map_or(208.0, |metrics| metrics.width.ceil() + 8.0);
                    let width = (width - 12.0).min(legend_width);
                    ui.row(
                        plate(TAKE_TRACK)
                            .at(x + 6.0, 1.0)
                            .w_px(width)
                            .h_px(12.0)
                            .pad_x(4.0)
                            .align(Align::Center),
                        |ui| {
                            let label = ui.label_fmt(
                                format_args!("{count} components · one source clock"),
                                label.ellipsis().w_px(width - 8.0),
                            );
                            let ink = Rgba::rgba(TAKE_SAGE.r, TAKE_SAGE.g, TAKE_SAGE.b, 179);
                            ui.style(
                                &label,
                                &States::of(Look::NONE.text(ink)),
                                pxui::ui::State::default(),
                            );
                        },
                    );
                }
            }
        },
    );
}

fn component_row(
    ui: &mut Ui,
    id: ClipId,
    component: Component,
    enabled: bool,
    range: Option<(f32, f32)>,
    visual: BlockMedia,
    geo: Geometry,
    intents: &mut Intents,
) -> Handle {
    let height = component.height();
    let label_inset = if geo.labels < LABEL_WIDTH { 8.0 } else { 14.0 };
    let tone = if enabled { component.tone } else { QUIET };
    ui.stack(
        plate(TAKE_TRACK).w_px(geo.labels + geo.width).h_px(height),
        |ui| {
            ui.row(
                plate(TAKE_LABEL)
                    .w_px(geo.labels)
                    .h_px(height)
                    .padding(Edges {
                        l: label_inset,
                        r: 6.0,
                        t: 0.0,
                        b: 0.0,
                    })
                    .flags(Flags::CLIP)
                    .align(Align::Center)
                    .gap(6.0),
                |ui| {
                    let stripe = component.tone;
                    let opacity = if enabled { 255 } else { 102 };
                    let stripe = Rgba::rgba(stripe.r, stripe.g, stripe.b, opacity);
                    ui.node("component-stripe", plate(stripe).w_px(3.0).h_px(10.0));
                    let label = ui.label(
                        component.name,
                        SMALL
                            .pad(0.0)
                            .ellipsis()
                            .w_px((geo.labels - 39.0 - label_inset).max(0.0)),
                    );
                    let ink = Rgba::rgba(TAKE_MUTED.r, TAKE_MUTED.g, TAKE_MUTED.b, opacity);
                    ui.style(
                        &label,
                        &States::of(Look::NONE.text(ink)),
                        pxui::ui::State::default(),
                    );
                    let name = if enabled {
                        component.hide
                    } else {
                        component.show
                    };
                    if ui
                        .icon_button(
                            "component-visible",
                            Icon::draw(take_eye, u32::from(enabled)),
                            name,
                            &IconButton::sized(18.0, 18.0)
                                .look(if enabled { &TAKE_CONTROL } else { &TAKE_OFF })
                                .selected(enabled),
                        )
                        .clicked
                    {
                        intents.component = Some((id, component_key(component.kind), !enabled));
                    }
                },
            );
            if let Some((x, width)) = range {
                if !enabled {
                    inactive_component(ui, x, width, height - 4.0);
                }
                let block = if enabled {
                    Look::NONE
                        .fill(blend(tone, 15))
                        .border(blend(tone, 34), 1.0)
                } else {
                    Look::NONE
                };
                let chosen = intents.state.component() == Some((id, component.kind, None));
                let looks = States::of(block)
                    .hover(block.border(tone, 1.0))
                    .checked(block.fill(blend(tone, 25)).border(tone, 1.0));
                let label_over_picture = enabled && width >= 72.0 && visual.thumbnail.is_some();
                let press = ui.pressable(
                    "component-block",
                    Node::row()
                        .at(x, 2.0)
                        .w_px(width)
                        .h_px(height - 4.0)
                        .pad_x(6.0)
                        .gap(6.0)
                        .align(Align::Center)
                        .role(Role::Button),
                    &Pressable::DEFAULT.look(&looks).selected(chosen),
                    |ui, _| {
                        if width >= 72.0
                            && !label_over_picture
                            && (!enabled
                                || !matches!(
                                    component.kind,
                                    TakeComponent::Pointer | TakeComponent::Keys
                                ))
                        {
                            if enabled {
                                ui.label(
                                    component.name,
                                    SMALL.ellipsis().w_px((width - 54.0).max(0.0)),
                                );
                            } else {
                                let label = ui.label_fmt(
                                    format_args!("{} · off", component.name),
                                    SMALL.ellipsis().w_px(width - 12.0),
                                );
                                ui.style(
                                    &label,
                                    &States::of(Look::NONE.text(QUIET)),
                                    pxui::ui::State::default(),
                                );
                            }
                        }
                        if enabled
                            && width >= 72.0
                            && let Some(gain) = component.gain.filter(|gain| *gain != 1.0)
                        {
                            ui.label_digits(
                                format_args!("{}%", (gain * 100.0).round() as u32),
                                CAPTION,
                            );
                        }
                    },
                );
                ui.a11y_name(&press.handle, component.name);
                if press.clicked || press.secondary {
                    intents.inspect_component(id, component.kind, None);
                }
                if press.secondary {
                    let point = ui.signal(&press.handle).press_point;
                    intents.state.open_menu(ContextTarget::Component, point);
                }
                if let Some(thumbnail) = visual.thumbnail {
                    film_strip(
                        ui,
                        "component-thumbnail",
                        (x + 1.0, 3.0, (width - 2.0).max(0.0), height - 6.0),
                        thumbnail,
                        120,
                    );
                    if label_over_picture {
                        let style = SMALL.pad(0.0).ellipsis().role(Role::Unknown);
                        let available = width - 18.0;
                        let ink_width = ui
                            .text_service()
                            .measure(component.name, style, None)
                            .map_or(available, |m| m.width.ceil().min(available));
                        ui.row(
                            plate(PANEL.with_alpha(191))
                                .at(x + 6.0, (height - 14.0) * 0.5)
                                .w_px(ink_width + 6.0)
                                .h_px(14.0)
                                .pad_x(3.0)
                                .align(Align::Center),
                            |ui| {
                                ui.label(component.name, style.w_px(ink_width));
                            },
                        );
                    }
                }
                if let Some(ref waveform) = visual.waveform {
                    waveform_view(
                        ui,
                        "component-waveform",
                        (x + 1.0, 3.0, (width - 2.0).max(0.0), height - 6.0),
                        waveform,
                        tone,
                    );
                }
            }
        },
    )
}

fn inactive_component(ui: &mut Ui, x: f32, width: f32, height: f32) {
    let clip = ui.node(
        "disabled-component-clip",
        Node::stack()
            .look(Look::NONE.fill(PANEL))
            .at(x, 2.0)
            .w_px(width)
            .h_px(height)
            .flags(Flags::CLIP),
    );
    ui.within(&clip, &mut |ui| {
        let art = ui.node("disabled-component", Node::DEFAULT.w_px(width).h_px(height));
        ui.canvas(&art, |paint, rect| {
            let color = Rgba::rgba(QUIET.r, QUIET.g, QUIET.b, 26);
            let hatch = Stroke::new(color).width(3.5);
            let step = 10.0 * std::f32::consts::SQRT_2;
            for index in 0..=((rect.w + rect.h) / step).ceil() as u32 {
                let x = rect.x + index as f32 * step;
                paint.line((x, rect.y), (x - rect.h, rect.y + rect.h), hatch);
            }
            if rect.w > 1.0 {
                paint.dashed_rect(
                    Rect::new(rect.x + 0.5, rect.y + 0.5, rect.w - 1.0, rect.h - 1.0),
                    Stroke::new(QUIET),
                );
            }
        });
    });
}

pub(super) fn affine_event_span(
    timing: ClipTiming,
    data: &ClipData,
    fps: u32,
    source_fps: f64,
) -> Option<SourceSpan> {
    if fps == 0 || !source_fps.is_finite() || source_fps <= 0.0 {
        return None;
    }
    // Without a prepared program, affine clips still have a clock. A varying ramp uses the
    // renderer's table once preparation succeeds.
    let rate = data
        .appearance
        .effects
        .iter()
        .find(|effect| effect.kind == "speed" && effect.enabled != Some(false))
        .and_then(|effect| effect.params.iter().find(|param| param.name == "rate"));
    let multiplier = match rate.map(|param| &param.value) {
        Some(EffectValue::Number(rate)) => rate.max(0.0),
        Some(EffectValue::Keys(keys)) if !keys.is_empty() => {
            let first = keys[0].value;
            if keys.iter().any(|key| key.value != first) {
                return None;
            }
            first.max(0.0)
        }
        _ => 1.0,
    };
    Some(SourceSpan {
        start: timing.start,
        end: timing.end()?,
        source: f64::from(timing.source),
        step: timing.speed * source_fps / f64::from(fps) * multiplier,
    })
}

fn take_event_marks(
    ui: &mut Ui,
    events: &splice_av::TimelineEvents,
    (clip, timing, component): (ClipId, ClipTiming, Component),
    (geo, x, width): (Geometry, f32, f32),
    (span, map): (SourceSpan, &[f64]),
    (track, intents): (&TakeEventTrack, &mut Intents),
) {
    let TakeEventTrack {
        rings,
        categories,
        hidden,
        ..
    } = track;
    let (rings, categories) = (*rings, *categories);
    let pointer = component.kind == TakeComponent::Pointer;
    if !events.fps.is_finite()
        || events.fps <= 0.0
        || (pointer && events.clicks.is_empty())
        || (!pointer && (events.keys.is_empty() || categories == 0))
    {
        return;
    }
    let scale = f64::from(geo.scale);
    let origin = f64::from(geo.labels) + f64::from(geo.x(timing.start as f32));
    let length = f64::from(timing.length);
    let start = f64::from(timing.start);
    let source_at = |pixel: f64| {
        let frame = start + ((pixel - origin) / scale).clamp(0.0, length);
        span.source_at(frame, map)
    };
    let mark_width = if pointer { 9.0 } else { 18.0 };
    let from = source_at(f64::from(x) - f64::from(mark_width));
    let to = source_at(f64::from(x) + f64::from(width) + if pointer { 4.5 } else { 0.0 });
    if !from.is_finite()
        || !to.is_finite()
        || from > to
        || from == to && (pointer || events.key_lookback == 0.0)
    {
        return;
    }
    let position = |frame| (origin + (frame - start) * scale - f64::from(x)) as f32;
    let pixel = |seconds| span.frame_at(seconds * events.fps, map).map(position);
    let height = component.height() - 4.0;
    let layer = ui.node(
        "recorded-events",
        Node::stack()
            .at(x, 2.0)
            .w_px(width)
            .h_px(height)
            .flags(Flags::CLIP),
    );
    ui.within(&layer, &mut |ui| {
        if pointer {
            let color = if rings { component.tone } else { QUIET };
            let from = events.clicks.partition_point(|&t| t * events.fps < from);
            let mut next = -mark_width;
            for (index, &t) in events
                .clicks
                .iter()
                .enumerate()
                .skip(from)
                .take_while(|(_, t)| **t * events.fps < to)
            {
                let Some(x) = pixel(t) else { continue };
                let x = x - mark_width * 0.5;
                if x + mark_width <= 0.0 || x < next {
                    continue;
                }
                next = x + mark_width;
                let look = Look::NONE
                    .fill(SURFACE)
                    .border(color, 2.0)
                    .radius(mark_width * 0.5);
                let press = ui.pressable(
                    index as u64,
                    Node::DEFAULT
                        .at(x, (height - mark_width) * 0.5)
                        .w_px(mark_width)
                        .h_px(mark_width)
                        .role(Role::Button),
                    &Pressable::DEFAULT.look(&States::of(look).hover(look.border(INK, 2.0))),
                    |_, _| {},
                );
                ui.a11y_name(&press.handle, "Jump to recorded click");
                if press.clicked
                    && let Some(at) = span.seek_at(t * events.fps, map)
                {
                    intents.scrub = Some((Phase::Ended, at));
                }
            }
        } else {
            let first = events
                .keys
                .partition_point(|key| (key.t + events.key_lookback) * events.fps < from);
            let mut next = -mark_width;
            for (index, key) in events
                .keys
                .iter()
                .enumerate()
                .skip(first)
                .take_while(|(_, key)| key.t * events.fps <= to)
            {
                if categories & key.category == 0 {
                    continue;
                }
                let (x, key_width, authored_start) =
                    if let Some([at, end]) = key.authored_frames(span, map, events.fps) {
                        if at >= end {
                            continue;
                        }
                        (
                            position(f64::from(at)),
                            ((end - at) as f32 * geo.scale).max(mark_width),
                            Some(at),
                        )
                    } else {
                        if !(from..to).contains(&(key.t * events.fps)) {
                            continue;
                        }
                        let Some(x) = pixel(key.t) else { continue };
                        if x < next {
                            continue;
                        }
                        next = x + mark_width;
                        (x, mark_width, None)
                    };
                if x + key_width <= 0.0 || x >= width {
                    continue;
                }
                let is_hidden = key
                    .index
                    .is_some_and(|id| hidden.binary_search(&id).is_ok());
                let (look, ink) = if is_hidden {
                    (Look::NONE.border(QUIET, 1.0), QUIET)
                } else {
                    (Look::NONE.fill(component.tone), SURFACE)
                };
                let chosen = key.index.is_some()
                    && intents.state.component() == Some((clip, TakeComponent::Keys, key.index));
                let looks = States::of(look)
                    .hover(look.border(INK, 1.0))
                    .checked(look.border(ACCENT, 1.0));
                let press = ui.pressable(
                    index as u64,
                    Node::row()
                        .at(x, 2.0)
                        .w_px(key_width)
                        .h_px(height - 4.0)
                        .align(Align::Center)
                        .role(Role::Button),
                    &Pressable::DEFAULT.look(&looks).selected(chosen),
                    |ui, _| {
                        let label =
                            ui.label(&key.label, CAPTION.pad(2.0).ellipsis().w_px(key_width));
                        ui.style(
                            &label,
                            &States::of(Look::NONE.text(ink)),
                            pxui::ui::State::default(),
                        );
                    },
                );
                ui.a11y_name(&press.handle, &key.label);
                if press.clicked || press.secondary {
                    intents.inspect_component(clip, TakeComponent::Keys, key.index);
                    if press.clicked {
                        if let Some(at) =
                            authored_start.or_else(|| span.seek_at(key.t * events.fps, map))
                        {
                            intents.scrub = Some((Phase::Ended, at));
                        }
                    } else {
                        let point = ui.signal(&press.handle).press_point;
                        intents.state.open_menu(ContextTarget::Component, point);
                    }
                }
            }
        }
    });
}

fn track_head(
    ui: &mut Ui,
    project: &Project,
    id: LaneId,
    name: &str,
    count: usize,
    width: f32,
    height: f32,
    kind: Option<LaneKind>,
    leading: f32,
    fx: Option<&FxBadge>,
    intents: &mut Intents,
) {
    let state = &mut intents.state;
    let head = ui.stack(
        Node::DEFAULT.w_px(width).h_px(height).flags(Flags::CLIP),
        |ui| {
            let background = if kind.is_some() {
                PANEL
            } else {
                Rgba::rgb(21, 21, 23)
            };
            let normal = Look::NONE.fill(background);
            let looks = States::of(normal)
                .hover(normal.fill(CONTROL))
                .checked(normal.fill(Rgba::rgb(42, 42, 45)));
            let response = ui.pressable(
                id.serial(),
                Node::DEFAULT.w_px(width).h_px(height).role(Role::Button),
                &Pressable::DEFAULT
                    .look(&looks)
                    .selected(state.lane == Some(id)),
                |_, _| {},
            );
            ui.a11y_name(&response.handle, name);
            let rename = state.lane_rename.filter(|rename| rename.id == id);
            if kind.is_some() && rename.is_none() && (response.held || response.clicked) {
                let signal = ui.signal(&response.handle);
                // The double-click count arrives on the press. Rename only on its matching click.
                if signal.pressed {
                    ui.set_state(&response.handle, i64::from(signal.double_clicked));
                }
                if response.clicked {
                    let double = signal.released && ui.state(&response.handle, 0) != 0;
                    ui.set_state(&response.handle, 0);
                    if double {
                        state.begin_lane_rename(id);
                    }
                }
            }
            if response.clicked || response.secondary {
                state.set_selection(None, Some(id));
            }
            if response.secondary {
                let point = ui.signal(&response.handle).press_point;
                state.open_menu(ContextTarget::Lane(id), point);
            }
            tile(ui, 12, width - 1.0, 0.0, 1.0, height, LINE);
            tile(ui, 13, 0.0, height - 1.0, width, 1.0, SURFACE);
            if state.lane == Some(id) {
                tile(ui, 10, 0.0, 0.0, 2.0, height, ACCENT);
            }
            let labels_start = if let Some(kind) = kind {
                let icon = ui.node(
                    "lane-type",
                    Node::DEFAULT
                        .at(10.0, (height - 12.0) * 0.5)
                        .w_px(12.0)
                        .h_px(12.0),
                );
                ui.canvas(&icon, move |paint, rect| lane_icon(paint, rect, kind));
                28.0
            } else {
                tile(
                    ui,
                    11,
                    10.0 + leading,
                    (height - 10.0) * 0.5,
                    3.0,
                    10.0,
                    VIDEO,
                );
                19.0 + leading
            };
            let show_count = width >= LABEL_WIDTH;
            let control_y = (height - 1.0 - LANE_CONTROL_SIZE) * 0.5;
            let mut right = width - 6.0;
            if kind.is_some() {
                let menu_x = width - 7.0 - LANE_CONTROL_SIZE;
                let container = Node::DEFAULT
                    .at(menu_x, control_y)
                    .w_px(LANE_CONTROL_SIZE)
                    .h_px(LANE_CONTROL_SIZE);
                ui.stack(container, |ui| {
                    let idle = if response.hovered {
                        QUIET
                    } else {
                        QUIET.alpha(128)
                    };
                    let look = States::of(Look::NONE.text(idle))
                        .hover(Look::NONE.text(INK))
                        .focus(Look::NONE.text(INK).border(ACCENT, 1.0));
                    let button =
                        IconButton::sized(LANE_CONTROL_SIZE, LANE_CONTROL_SIZE).look(&look);
                    let menu =
                        ui.icon_button("lane-menu", Icon::draw(lane_more, 0), "Lane menu", &button);
                    if menu.secondary {
                        let point = ui.signal(&menu.handle).press_point;
                        state.set_selection(None, Some(id));
                        state.open_menu(ContextTarget::Lane(id), point);
                    } else if menu.clicked {
                        let rect = ui.rect(&menu.handle);
                        state.open_menu(ContextTarget::Lane(id), (rect.x, rect.y + rect.h));
                    }
                });
                right = menu_x - 6.0;
            }
            if let Some(fx) = fx.filter(|fx| right >= labels_start + fx.width()) {
                right -= fx.width();
                paint_fx_badge(
                    ui,
                    &response.handle,
                    fx,
                    (right, (height - FX_BADGE_HEIGHT) * 0.5),
                    INK,
                );
                right -= 6.0;
            }
            if show_count {
                let digits = count.checked_ilog10().unwrap_or(0) + 1;
                let w = digits as f32 * if kind.is_some() { 6.0 } else { 6.5 };
                right -= w;
                ui.stack(Node::DEFAULT.at(right, (height - 12.0) * 0.5), |ui| {
                    let count = ui.label_digits(format_args!("{count}"), CAPTION.pad(0.0).w_px(w));
                    ui.style(
                        &count,
                        &States::of(Look::NONE.text(QUIET)),
                        pxui::ui::State::default(),
                    );
                });
                right -= 6.0;
            }
            if let Some(rename) = rename {
                let rect = Rect::new(
                    labels_start,
                    control_y,
                    (right - labels_start).max(1.0),
                    LANE_CONTROL_SIZE,
                );
                let field = ValueField::DEFAULT.size(11).left(true).name("Lane name");
                let response = pxui::ext::value_field(
                    ui,
                    "lane-rename",
                    name,
                    rect,
                    &field.edit(rename.begin),
                    &ValueCallbacks::DEFAULT,
                );
                intents.rename_painted = true;
                state.lane_rename = response.editing.then_some(LaneRename { id, begin: false });
                if let Some(name) = response.changed.map(str::trim).filter(|value| {
                    !value.is_empty() && *value != name && !value.chars().any(char::is_control)
                }) {
                    intents.toolbar = Some(ToolbarCommand::RenameLane(id, name.to_owned()));
                }
                let handle = response.response.handle;
                ui.set_role(&handle, Role::TextField, pxui::A11yStates::EDITABLE);
            } else if right > labels_start {
                ui.row(
                    Node::DEFAULT
                        .at(labels_start, 0.0)
                        .w_px(right - labels_start)
                        .h_px(height)
                        .align(Align::Center),
                    |ui| {
                        ui.label(
                            name,
                            SMALL
                                .pad(0.0)
                                .ellipsis()
                                .w_px(right - labels_start)
                                .role(Role::Unknown),
                        );
                    },
                );
            }
        },
    );
    if let Some(effect) = viewer_drawers::effect_drop(ui, &head, project, Target::Lane(id)) {
        intents.effect = Some(effect);
    }
}

fn lane_more(paint: &mut Paint<'_>, rect: Rect, ink: Rgba, _: u32) {
    let (x, y) = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
    let fill = Fill::new(ink);
    for dx in [-3.0, 0.0, 3.0] {
        paint.circle((x + dx, y), 0.425, Some(fill), None);
    }
}

fn lane_icon(paint: &mut Paint, rect: Rect, kind: LaneKind) {
    let tone = lane_tone(kind);
    let ink = Rgba::rgba(tone.r, tone.g, tone.b, 230);
    let stroke = Stroke::new(ink).width(if kind == LaneKind::Graphics {
        1.0
    } else {
        0.85
    });
    let point = |(x, y): (f32, f32)| (rect.x + x * 0.5, rect.y + y * 0.5);
    let paths: &[&[(f32, f32)]] = match kind {
        LaneKind::Graphics => &[
            &[(5., 7.), (5., 5.), (19., 5.), (19., 7.)],
            &[(12., 5.), (12., 19.)],
            &[(9., 19.), (15., 19.)],
        ],
        LaneKind::Camera => &[&[(12., 4.), (20., 12.), (12., 20.), (4., 12.), (12., 4.)]],
        LaneKind::Captions => {
            paint.stroke(Rect::new(rect.x + 1.5, rect.y + 2.5, 9.0, 7.0), stroke);
            for x in [10.6, 17.0] {
                paint.curve(
                    point((x, 10.2)),
                    point((x - 6.4, 12.0)),
                    point((x, 13.8)),
                    stroke,
                );
            }
            &[]
        }
        LaneKind::Overlay | LaneKind::Audio => &[
            &[(12., 4.), (21., 8.5), (12., 13.), (3., 8.5), (12., 4.)],
            &[(3., 13.), (12., 17.5), (21., 13.)],
        ],
    };
    for path in paths {
        for line in path.windows(2) {
            paint.line(point(line[0]), point(line[1]), stroke);
        }
    }
}
fn lane_tone(kind: LaneKind) -> Rgba {
    match kind {
        LaneKind::Graphics => Rgba::rgb(240, 214, 196),
        LaneKind::Overlay => Rgba::rgb(211, 228, 205),
        LaneKind::Camera => Rgba::rgb(191, 224, 234),
        LaneKind::Captions => Rgba::rgb(242, 230, 166),
        LaneKind::Audio => Rgba::rgb(198, 214, 230),
    }
}

fn blend(tone: Rgba, percent: u16) -> Rgba {
    let mix =
        |a: u8, b: u8| ((u16::from(a) * percent + u16::from(b) * (100 - percent)) / 100) as u8;
    Rgba::rgb(
        mix(tone.r, PANEL.r),
        mix(tone.g, PANEL.g),
        mix(tone.b, PANEL.b),
    )
}

fn block(
    ui: &mut Ui,
    id: TimelineId,
    name: &str,
    (x, width): (f32, f32),
    height: f32,
    tone: Rgba,
    (geo, playhead, start, length, top): (Geometry, i32, i32, i32, f32),
    expandable: bool,
    (cache, project): (&Cache, &Project),
    visual: BlockMedia,
    decor: BlockDecor,
    fx: Option<&FxBadge>,
    caption: Option<&Rc<[CaptionTick]>>,
    drop: Option<(&Project, LaneId, LaneKind, DropViewport)>,
    intents: &mut Intents,
) {
    let revision = project.rev;
    let (key, video) = match id {
        TimelineId::Clip(id) => (id.serial(), true),
        TimelineId::Item(id) => (id.serial(), false),
        TimelineId::Marker(_) => return,
    };
    let normal = Look::NONE
        .fill(blend(tone, 15))
        .border(blend(tone, 34), 1.0)
        .text(tone);
    let hover = normal.fill(blend(tone, 22)).border(blend(tone, 60), 1.0);
    let selected = hover.fill(blend(tone, 26)).border(tone, 1.0).text(INK);
    let looks = States::of(normal).hover(hover).checked(selected);
    let narrow_caption = caption.is_some() && width < 32.0;
    let insets = (
        if width >= 34.0 || video || decor.motion {
            16.0
        } else {
            6.0
        },
        if decor.motion || decor.fade_handles && width >= 34.0 {
            14.0
        } else {
            6.0
        },
    );
    let separate_header = narrow_caption
        || video
        || decor.motion
        || fx.is_some_and(|badge| width >= insets.0 + badge.width() + insets.1)
        || decor.pinned
        || decor.behind
        || decor.placement.is_some()
        || decor.fades != [0; 2];
    ui.stack(
        Node::DEFAULT
            .at(x, 2.0)
            .w_px(width)
            .h_px(height - 4.0)
            .flags(Flags::CLIP),
        |ui| {
            let selected =
                if let Some(marquee) = intents.state.marquee.filter(|marquee| marquee.moved()) {
                    (marquee.additive && intents.state.contains(id))
                        || marquee_hits(marquee.rect(), geo, top, height, start, length)
                } else {
                    intents.state.contains(id)
                };
            let button = Button::DEFAULT
                .size(Size::Px(width), Size::Px(height - 4.0))
                .centered()
                .content_align(Along::Start)
                .look(&looks)
                .name(name)
                .label(SMALL.ellipsis())
                .selected(selected)
                .pad(Edges::new(insets.0, 0.0, insets.1, 0.0));
            let response = ui.button(
                key,
                if separate_header || width < 20.0 {
                    ""
                } else {
                    name
                },
                &button,
            );
            if let Some(effect) =
                viewer_drawers::effect_drop(ui, &response.handle, project, Target::Timeline(id))
            {
                intents.effect = Some(effect);
            }
            if response.secondary {
                if !intents.state.contains(id) {
                    intents.state.set_selection(Some(id), None);
                }
                let point = ui.signal(&response.handle).press_point;
                intents.state.open_menu(ContextTarget::Block, point);
            }
            let fading = intents.state.fade_drag.is_some_and(|drag| drag.item == id);
            let signal = (response.held
                || response.clicked
                || fading
                || intents.state.drag.is_some_and(|drag| drag.item == id))
            .then(|| ui.signal(&response.handle));
            let shift = signal.is_some() && ui.key_down(KeyCode::Shift);
            let capturing = intents.state.drag.is_some_and(|drag| drag.item == id);
            let blade = intents.state.blade
                || (signal.is_some() || response.hovered) && ui.key_down(KeyCode::Alt);
            if blade && response.hovered {
                let rect = ui.rect(&response.handle);
                let pointer = ui.signal(&response.handle).pointer;
                if rect.contains(pointer) {
                    let frame = blade_frame(geo, x + pointer.0 - rect.x);
                    if inside(start, length, frame) {
                        intents.blade_hover = Some((frame, top, height));
                    }
                }
            }
            // PXUI reports double-click on the press. Open text only after a matching
            // release that did not turn into a drag.
            let double_text = signal.as_ref().and_then(|signal| {
                if signal.pressed {
                    ui.set_state(
                        &response.handle,
                        i64::from(signal.double_clicked && !shift && !blade),
                    );
                }
                if !signal.released {
                    return None;
                }
                let double = ui.state(&response.handle, 0) != 0;
                ui.set_state(&response.handle, 0);
                double.then(|| text_focus(project, id)).flatten()
            });
            let mut fades = decor.fades;
            let dragged = signal.as_ref().is_some_and(|signal| {
                if fading || (decor.fade_handles && signal.pressed) {
                    let rect = ui.rect(&response.handle);
                    let (px, py) = signal.press_point;
                    let local = px - rect.x;
                    let edge = (signal.pressed
                        && signal.button == Some(Signal::BUTTON_PRIMARY)
                        && rect.contains(signal.press_point)
                        && (1.0..1.0 + FADE_HANDLE).contains(&(py - rect.y))
                        && trim_grip(geo, (start, length), local, rect.w) == DragEdge::Move)
                        .then(|| fade_handles(geo, x, start, length, fades))
                        .flatten()
                        .and_then(|handles| {
                            handles
                                .iter()
                                .position(|&handle| (local - handle).abs() < FADE_HANDLE * 0.5)
                        });
                    if fading || edge.is_some() {
                        drag_fade(
                            signal,
                            id,
                            revision,
                            edge,
                            (length, geo.scale),
                            ui.key_pressed(KeyCode::Escape, pxui::Mods::ANY),
                            &mut fades,
                            intents,
                        );
                        return true;
                    }
                }
                let edge = if signal.pressed {
                    let rect = ui.rect(&response.handle);
                    trim_grip(geo, (start, length), signal.press_point.0 - rect.x, rect.w)
                } else {
                    DragEdge::Move
                };
                let pointer = if signal.released {
                    signal.release_point
                } else {
                    signal.pointer
                };
                let options = DragOptions {
                    lane: drop.and_then(|(project, source, kind, viewport)| {
                        compatible_drop(project, cache, geo, viewport, pointer, source, kind)
                    }),
                    bypass_snap: ui.key_down(KeyCode::Meta) || ui.key_down(KeyCode::Ctrl),
                    leave_overlays: video && ui.key_down(KeyCode::Char('`')),
                };
                (capturing || !blade && (!signal.pressed || !shift))
                    && move_drag(
                        signal, id, revision, start, length, edge, geo.scale, playhead, cache,
                        options, intents,
                    )
            });
            if response.clicked && !dragged {
                if blade {
                    let signal = signal.as_ref().expect("clicked button has a signal");
                    let at = if signal.clicked {
                        let rect = ui.rect(&response.handle);
                        blade_frame(geo, x + signal.release_point.0 - rect.x)
                    } else {
                        playhead
                    };
                    if inside(start, length, at) {
                        intents.split = Some((id, at));
                    }
                } else {
                    if expandable
                        && !shift
                        && signal.as_ref().is_some_and(|signal| signal.double_clicked)
                        && let TimelineId::Clip(id) = id
                    {
                        intents.state.toggle_take(id);
                    }
                    if shift {
                        intents.state.toggle(id);
                    } else {
                        intents.state.set_selection(Some(id), None);
                        if let Some(focus) = double_text {
                            intents.inspector_focus = Some(focus);
                        }
                    }
                }
            }
            if video {
                if let Some(thumbnail) = visual.thumbnail {
                    film_strip(
                        ui,
                        "clip-film",
                        (1.0, 17.0, (width - 2.0).max(0.0), (height - 22.0).max(0.0)),
                        thumbnail,
                        255,
                    );
                } else {
                    for (index, pixel) in (1..20)
                        .map(|i| (i, i as f32 * 24.0))
                        .take_while(|&(_, x)| x < width - 1.0)
                    {
                        tile(
                            ui,
                            100 + index,
                            pixel,
                            17.0,
                            2.0_f32.min(width - pixel),
                            height - 22.0,
                            PANEL,
                        );
                    }
                }
            } else if let Some(thumbnail) = visual.thumbnail {
                let thumb_width = width.min(64.0);
                film_strip(
                    ui,
                    "overlay-film",
                    (
                        width - thumb_width,
                        height * 0.5,
                        (thumb_width - 1.0).max(0.0),
                        height * 0.5 - 5.0,
                    ),
                    thumbnail,
                    135,
                );
            }
            if let Some(ref waveform) = visual.waveform {
                waveform_view(
                    ui,
                    "clip-waveform",
                    (1.0, 4.0, (width - 2.0).max(0.0), height - 12.0),
                    waveform,
                    tone,
                );
            }
            let ink = if selected { INK } else { tone };
            if fades != [0; 2] {
                block_fades(
                    ui,
                    (geo, x, width, height - 4.0),
                    (start, length),
                    fades,
                    ink,
                );
            }
            if narrow_caption {
                let label = ui.node(
                    "narrow-caption-label",
                    Node::DEFAULT
                        .at(2.0, 1.0)
                        .w_px((width - 2.0).max(0.0))
                        .h_px(13.0),
                );
                ui.within(&label, &mut |ui| {
                    let text = ui.label(name, SMALL.pad(0.0).role(Role::Unknown));
                    ui.style(
                        &text,
                        &States::of(Look::NONE.text(ink)),
                        pxui::ui::State::default(),
                    );
                });
            } else if separate_header {
                let head_height = if video { 16.0 } else { height - 4.0 };
                let head = if video {
                    film_header(width, tone, selected)
                } else {
                    Node::DEFAULT.w_px(width).h_px(head_height)
                };
                ui.stack(head, |ui| {
                    block_header(
                        ui,
                        &response.handle,
                        name,
                        if video { width - 2.0 } else { width },
                        head_height,
                        tone,
                        ink,
                        &decor,
                        fx,
                        insets,
                    );
                    if video {
                        tile(
                            ui,
                            8,
                            0.0,
                            15.0,
                            width - 2.0,
                            1.0,
                            Rgba::rgba(tone.r, tone.g, tone.b, 51),
                        );
                    }
                });
            }
            if let Some(ticks) = caption.filter(|ticks| !ticks.is_empty()) {
                let art = ui.node(
                    "caption-ticks",
                    Node::DEFAULT.at(0.0, 0.0).w_px(width).h_px(height - 4.0),
                );
                let ticks = Rc::clone(ticks);
                ui.canvas(&art, move |paint, rect| {
                    let offset = |at: i32| geo.labels + geo.x(at as f32) - x;
                    let first = ticks.partition_point(|tick| offset(tick.at) <= -3.0);
                    for tick in &ticks[first..] {
                        let dx = offset(tick.at);
                        if dx >= rect.w {
                            break;
                        }
                        let color =
                            Rgba::rgba(tone.r, tone.g, tone.b, if tick.emph { 255 } else { 140 });
                        let width = if tick.emph { 3.0 } else { 1.0 };
                        if dx + width > 0.0 {
                            paint.fill(
                                Rect::new(rect.x + dx, rect.y + rect.h - 9.0, width, 6.0),
                                Fill::new(color),
                            );
                        }
                    }
                });
            }
            if decor.motion {
                let left = geo.labels + geo.x(start as f32) - x;
                let right = geo.labels + geo.x(start.saturating_add(length) as f32) - x;
                motion_diamonds(ui, width, height - 4.0, left, right, tone);
            }
            if decor.fade_handles
                && let Some(handles) = fade_handles(geo, x, start, length, fades)
            {
                let active = intents
                    .state
                    .fade_drag
                    .is_some_and(|drag| drag.item == id && !drag.cancelled);
                for (edge, handle) in handles.into_iter().enumerate() {
                    if handle <= -FADE_HANDLE * 0.5 || handle >= width + FADE_HANDLE * 0.5 {
                        continue;
                    }
                    if fades[edge] != 0 || response.hovered || intents.state.contains(id) {
                        let left = handle - FADE_HANDLE * 0.5;
                        let look = Look::NONE
                            .fill(if fades[edge] != 0 { tone } else { INK })
                            .border(PANEL, 1.0);
                        ui.node(
                            38 + edge as u64,
                            Node::DEFAULT
                                .at(left, 1.0)
                                .w_px(FADE_HANDLE)
                                .h_px(FADE_HANDLE)
                                .look(look),
                        );
                    }
                }
                if active
                    || response.hovered && {
                        let rect = ui.rect(&response.handle);
                        let (px, py) = ui.signal(&response.handle).pointer;
                        let local = px - rect.x;
                        (1.0..1.0 + FADE_HANDLE).contains(&(py - rect.y))
                            && trim_grip(geo, (start, length), local, rect.w) == DragEdge::Move
                            && handles
                                .iter()
                                .any(|&handle| (local - handle).abs() < FADE_HANDLE * 0.5)
                    }
                {
                    ui.request_cursor(Cursor::HorizontalResize);
                }
            }
            if !blade && (response.hovered || intents.state.contains(id)) {
                let grip_y = (height - 14.0) * 0.5;
                if trim_grip(geo, (start, length), 0.0, width) == DragEdge::In {
                    tile(ui, 36, 2.0, grip_y, 2.0, 10.0, tone);
                }
                if trim_grip(geo, (start, length), width, width) == DragEdge::Out {
                    tile(ui, 37, width - 4.0, grip_y, 2.0, 10.0, tone);
                }
            }
        },
    );
}

fn film_strip(
    ui: &mut Ui,
    key: &'static str,
    (x, y, width, height): (f32, f32, f32, f32),
    image: Image,
    opacity: u8,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let node = ui.node(key, Node::DEFAULT.at(x, y).w_px(width).h_px(height));
    ui.canvas(&node, move |paint, rect| {
        let tile_width = (rect.h * image.width as f32 / image.height.max(1) as f32).max(40.0);
        let count = ((rect.w / tile_width).ceil() as usize).min(120);
        let style = ImageStyle::DEFAULT
            .fit(pxui::draw::Fit::Cover)
            .tint(Rgba::rgba(255, 255, 255, opacity));
        for index in 0..count {
            let position = index as f32 * tile_width;
            let width = (rect.w - position).min(tile_width) - 1.0;
            if width > 0.0 {
                let tile = Rect::new(rect.x + position, rect.y, width, rect.h);
                let _ = paint.image(tile, &image, &style);
            }
        }
    });
}

fn waveform_view(
    ui: &mut Ui,
    key: &'static str,
    (x, y, width, height): (f32, f32, f32, f32),
    peaks: &PeakColumns,
    color: Rgba,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let heights = peaks.heights;
    let count = peaks.count;
    let node = ui.node(key, Node::DEFAULT.at(x, y).w_px(width).h_px(height));
    ui.canvas(&node, move |paint, rect| {
        let step = rect.w / count as f32;
        let fill = Fill::new(Rgba::rgba(color.r, color.g, color.b, 128));
        let middle = rect.y + rect.h * 0.5;
        for (column, &peak) in heights[..count].iter().enumerate() {
            let half = (f32::from(peak) * rect.h / 510.0).max(0.5);
            paint.fill(
                Rect::new(
                    rect.x + step * column as f32,
                    middle - half,
                    (step - 1.0).max(1.0).min(step),
                    2.0 * half,
                ),
                fill,
            );
        }
    });
}

fn trim_grip(geo: Geometry, (start, length): (i32, i32), local_x: f32, width: f32) -> DragEdge {
    if width < 2.0 * TRIM_GRIP || length < MIN_TRIM_LENGTH {
        return DragEdge::Move;
    }
    if (0.0..TRIM_GRIP).contains(&local_x) && geo.x(start as f32) >= 0.0 {
        return DragEdge::In;
    }
    if local_x >= width - TRIM_GRIP && geo.x(start.saturating_add(length) as f32) <= geo.width {
        return DragEdge::Out;
    }
    DragEdge::Move
}
fn move_drag(
    signal: &Signal,
    id: TimelineId,
    revision: u64,
    start: i32,
    length: i32,
    edge: DragEdge,
    scale: f32,
    at: i32,
    cache: &Cache,
    options: DragOptions,
    intents: &mut Intents,
) -> bool {
    if signal.pressed && signal.button == Some(Signal::BUTTON_PRIMARY) {
        intents.state.drag = Some(ItemDrag {
            item: id,
            revision,
            edge,
            start,
            origin: signal.press_point,
            moved: false,
            to: edge.point(start, length),
            lane: None,
            snap: None,
        });
    }
    let Some(mut drag) = intents.state.drag.filter(|drag| drag.item == id) else {
        return false;
    };
    if drag.revision != revision || (!signal.held && !signal.released) {
        intents.state.drag = None;
        return drag.moved;
    }
    intents.drag_seen = true;
    let (x, y) = if signal.released {
        signal.release_point
    } else {
        signal.pointer
    };
    let distance = x - drag.origin.0;
    let overlay_move = drag.edge == DragEdge::Move && matches!(id, TimelineId::Item(_));
    let vertical = overlay_move && (y - drag.origin.1).abs() >= 6.0;
    if drag.moved || distance.abs() >= 3.0 || vertical {
        drag.moved = true;
        let point = drag.edge.point(drag.start, length);
        let bounds = drag.edge.bounds(drag.start, length);
        let to = point
            .saturating_add((distance / scale).round() as i32)
            .clamp(bounds.0, bounds.1);
        drag.lane = if overlay_move { options.lane } else { None };
        (drag.to, drag.snap) =
            if intents.state.free_snap || options.bypass_snap || (overlay_move && to == point) {
                (to, None)
            } else {
                cache.snap_drag(
                    id,
                    to,
                    (drag.edge == DragEdge::Move).then_some(length),
                    bounds,
                    at,
                    scale,
                )
            };
        intents.state.set_selection(Some(id), None);
        if signal.released && (drag.to != point || drag.lane.is_some()) {
            let ripple = !intents.state.free_move;
            match drag.edge {
                DragEdge::Move => {
                    intents.moved = Some(MoveIntent {
                        item: id,
                        to: (drag.to != point).then_some(drag.to),
                        ripple,
                        lane: drag.lane,
                        carry_overlays: !options.leave_overlays,
                    })
                }
                DragEdge::In | DragEdge::Out => {
                    let edge = if drag.edge == DragEdge::In {
                        "in"
                    } else {
                        "out"
                    };
                    intents.trimmed = Some((id, edge, drag.to.saturating_sub(point), ripple));
                }
            }
        }
    }
    intents.state.drag = signal.held.then_some(drag);
    drag.moved
}
fn blade_frame(geo: Geometry, x: f32) -> i32 {
    ((x - geo.labels) / geo.scale + geo.origin).round() as i32
}

fn inside(start: i32, length: i32, at: i32) -> bool {
    start < at && at < start.saturating_add(length)
}

pub(super) fn split_at_playhead(
    project: &Project,
    selection: Option<TimelineId>,
    at: i32,
) -> Option<TimelineId> {
    if let Some(TimelineId::Item(id)) = selection {
        let timing = project.items.timing(project.items.find(id)?)?;
        let start = anchor_start(project, timing.anchor).ok()?;
        return inside(start, timing.length, at).then_some(TimelineId::Item(id));
    }
    if let Some(TimelineId::Clip(id)) = selection {
        let timing = project.clips.timing(project.clips.find(id)?)?;
        return inside(timing.start, timing.length, at).then_some(TimelineId::Clip(id));
    }
    project.clips.order().iter().find_map(|&slot| {
        let timing = project.clips.timing(slot)?;
        if inside(timing.start, timing.length, at) {
            project.clips.id(slot).map(TimelineId::Clip)
        } else {
            None
        }
    })
}
fn film_header(width: f32, tone: Rgba, selected: bool) -> Node {
    plate(blend(tone, if selected { 32 } else { 18 }))
        .at(1.0, 1.0)
        .w_px((width - 2.0).max(0.0))
        .h_px(16.0)
}

fn paint_fx_badge(ui: &mut Ui, owner: &Handle, badge: &FxBadge, (x, y): (f32, f32), tint: Rgba) {
    let inherited = badge.kind == FxBadgeKind::Inherited;
    let (node, ink) = if inherited {
        (Node::DEFAULT, Rgba::rgba(tint.r, tint.g, tint.b, 204))
    } else {
        (plate(lane_tone(LaneKind::Camera)), Rgba::rgb(9, 9, 10))
    };
    let label = if badge.kind == FxBadgeKind::Override {
        "fx•"
    } else {
        "fx"
    };
    let handle = ui.row(
        node.at(x, y)
            .w_px(badge.width())
            .h_px(FX_BADGE_HEIGHT)
            .pad_x(3.0)
            .align(Align::Center),
        |ui| {
            let label = ui.label(label, CAPTION.pad(0.0).h_px(FX_BADGE_HEIGHT));
            ui.style(
                &label,
                &States::of(Look::NONE.text(ink)),
                UiState::default(),
            );
        },
    );
    if inherited {
        ui.canvas(&handle, move |paint, rect| {
            paint.dashed_rect(
                Rect::new(rect.x + 0.5, rect.y + 0.5, rect.w - 1.0, rect.h - 1.0),
                Stroke::new(ink),
            );
        });
    }
    // The block or lane keeps the pointer hit. Its tooltip waits only over the badge.
    if ui.hovered(owner, false) && ui.rect(&handle).contains(ui.signal(owner).pointer) {
        ui.tooltip(owner, &badge.tip);
    }
}

fn block_header(
    ui: &mut Ui,
    owner: &Handle,
    name: &str,
    width: f32,
    height: f32,
    tone: Rgba,
    ink: Rgba,
    decor: &BlockDecor,
    fx: Option<&FxBadge>,
    (leading, trailing): (f32, f32),
) {
    const BADGE_HEIGHT: f32 = 12.0;
    const GAP: f32 = 6.0;
    const PIN: f32 = 25.0;
    const BEHIND: f32 = 44.0;
    let mut available = width - leading - trailing;
    let mut reserve = |width: f32| {
        if width > available {
            return false;
        }
        available -= width + GAP;
        true
    };
    let fx = fx.filter(|badge| reserve(badge.width()));
    let pinned = decor.pinned && reserve(PIN);
    let behind = decor.behind && reserve(BEHIND);
    let speed = decor
        .speed
        .map(|speed| {
            let hundredths = (speed * 100.0).round() as u64;
            let whole = hundredths / 100;
            let fraction = hundredths % 100;
            let digits = whole.checked_ilog10().unwrap_or(0) as f32 + 1.0;
            let decimals = match fraction {
                0 => 0.0,
                n if n % 10 == 0 => 13.0,
                _ => 19.0,
            };
            (whole, fraction, 7.0 * (digits + 1.0) + 6.0 + decimals)
        })
        .filter(|&(_, _, width)| reserve(width));

    let mut x = leading;
    if available >= 10.0 {
        let duration = decor.duration.and_then(|(frames, fps)| {
            let centiseconds =
                (u64::try_from(frames).ok()? * 100 + u64::from(fps) / 2) / u64::from(fps.max(1));
            Some((centiseconds / 100, centiseconds % 100))
        });
        let detail_style = CAPTION.pad(0.0);
        let detail_width = match (duration, decor.placement) {
            (Some((seconds, _)), _) => {
                let digits = seconds.checked_ilog10().unwrap_or(0) + 1;
                f32::from((digits + 4) as u16) * 6.5
            }
            (None, Some(placement)) => ui
                .text_service()
                .measure(placement, detail_style, None)
                .map_or(0.0, |metrics| metrics.width.ceil()),
            (None, None) => 0.0,
        };
        let detail_width = if available >= detail_width + 26.0 {
            detail_width
        } else {
            0.0
        };
        let name_style = SMALL.pad(0.0);
        let budget = available
            - if detail_width > 0.0 {
                detail_width + GAP
            } else {
                0.0
            };
        let name_width = ui
            .text_service()
            .measure(name, name_style, None)
            .map_or(budget, |metrics| metrics.width.ceil().min(budget));
        ui.row(
            Node::DEFAULT
                .at(leading, 0.0)
                .w_px(available)
                .h_px(height)
                .gap(GAP)
                .align(Align::Center),
            |ui| {
                let label = ui.label(
                    name,
                    name_style.ellipsis().w_px(name_width).role(Role::Unknown),
                );
                ui.style(
                    &label,
                    &States::of(Look::NONE.text(ink)),
                    pxui::ui::State::default(),
                );
                if detail_width > 0.0 {
                    let style = detail_style.w_px(detail_width);
                    let label = match duration {
                        Some((secs, hundredths)) => {
                            ui.label_fmt(format_args!("{secs}.{hundredths:02}s"), style)
                        }
                        None => ui.label(decor.placement.unwrap_or_default(), style),
                    };
                    let quiet = Rgba::rgba(ink.r, ink.g, ink.b, 153);
                    ui.style(
                        &label,
                        &States::of(Look::NONE.text(quiet)),
                        pxui::ui::State::default(),
                    );
                }
            },
        );
        x += name_width + GAP;
        if detail_width > 0.0 {
            x += detail_width + GAP;
        }
    }
    let mut badge = |text: std::fmt::Arguments, width, background, foreground| {
        ui.row(
            plate(background)
                .at(x, (height - BADGE_HEIGHT) * 0.5)
                .w_px(width)
                .h_px(BADGE_HEIGHT)
                .pad_x(3.0)
                .align(Align::Center),
            |ui| {
                let label = ui.label_fmt(text, CAPTION.pad(0.0).h_px(BADGE_HEIGHT));
                ui.style(
                    &label,
                    &States::of(Look::NONE.text(foreground)),
                    pxui::ui::State::default(),
                );
            },
        );
        x += width + GAP;
    };
    if let Some((whole, fraction, width)) = speed {
        let background = blend(tone, 22);
        if fraction == 0 {
            badge(format_args!("{whole}×"), width, background, ink);
        } else if fraction % 10 == 0 {
            badge(
                format_args!("{whole}.{}×", fraction / 10),
                width,
                background,
                ink,
            );
        } else {
            badge(
                format_args!("{whole}.{fraction:02}×"),
                width,
                background,
                ink,
            );
        }
    }
    if pinned {
        badge(format_args!("PIN"), PIN, WARN, SURFACE);
    }
    if behind {
        badge(format_args!("BEHIND"), BEHIND, blend(tone, 22), ink);
    }
    if let Some(fx) = fx {
        paint_fx_badge(ui, owner, fx, (x, (height - FX_BADGE_HEIGHT) * 0.5), ink);
    }
}

fn motion_diamonds(ui: &mut Ui, width: f32, height: f32, left: f32, right: f32, tone: Rgba) {
    if right - left < 12.0 {
        return;
    }
    let node = ui.node(
        "motion-diamonds",
        Node::DEFAULT.at(0.0, 0.0).w_px(width).h_px(height),
    );
    ui.canvas(&node, move |paint, rect| {
        let y = rect.y + rect.h * 0.5;
        let radius = 3.5 * std::f32::consts::SQRT_2;
        let halo = radius + 1.5;
        for cx in [left + 6.0, right - 6.0] {
            if cx + halo <= 0.0 || cx - halo >= rect.w {
                continue;
            }
            let x = rect.x + cx;
            for (radius, color) in [(halo, PANEL), (radius, tone)] {
                let fill = Fill::new(color);
                paint.triangle((x - radius, y), (x, y - radius), (x + radius, y), fill);
                paint.triangle((x - radius, y), (x + radius, y), (x, y + radius), fill);
            }
        }
    });
}

fn fade_handles(
    geo: Geometry,
    x: f32,
    start: i32,
    length: i32,
    fades: [i32; 2],
) -> Option<[f32; 2]> {
    let width = length as f32 * geo.scale;
    if width < 34.0 {
        return None;
    }
    let left = geo.labels + geo.x(start as f32) - x;
    Some([
        left + (fades[0] as f32 * geo.scale).clamp(9.0, width * 0.5 - 4.0),
        left + (width - fades[1] as f32 * geo.scale).clamp(width * 0.5 + 4.0, width - 9.0),
    ])
}

fn drag_fade(
    signal: &Signal,
    item: TimelineId,
    revision: u64,
    edge: Option<usize>,
    (length, scale): (i32, f32),
    escape: bool,
    fades: &mut [i32; 2],
    intents: &mut Intents,
) {
    if let Some(edge) = edge {
        intents.state.set_selection(Some(item), None);
        intents.state.fade_drag = Some(FadeDrag {
            item,
            revision,
            edge,
            origin: signal.press_point.0,
            value: fades[edge],
            moved: false,
            cancelled: false,
        });
    }
    let Some(mut drag) = intents.state.fade_drag else {
        return;
    };
    if !signal.held && !signal.released {
        intents.state.fade_drag = None;
        return;
    }
    intents.fade_seen = true;
    drag.cancelled |= escape || drag.revision != revision;
    if !drag.cancelled {
        let x = if signal.released {
            signal.release_point.0
        } else {
            signal.pointer.0
        };
        let distance = x - drag.origin;
        drag.moved |= distance.abs() >= 3.0;
        if drag.moved {
            let delta = (distance / scale + 0.5).floor() as i32;
            let opposite = fades[1 - drag.edge];
            drag.value = if drag.edge == 0 {
                fades[0].saturating_add(delta)
            } else {
                fades[1].saturating_sub(delta)
            }
            .clamp(0, length.saturating_sub(opposite).max(0));
            if signal.released && drag.value != fades[drag.edge] {
                intents.toolbar = Some(ToolbarCommand::SetFade(item, drag.edge == 0, drag.value));
            }
            fades[drag.edge] = drag.value;
        }
    }
    intents.state.fade_drag = signal.held.then_some(drag);
}

fn block_fades(
    ui: &mut Ui,
    (geo, x, width, height): (Geometry, f32, f32, f32),
    (start, length): (i32, i32),
    fades: [i32; 2],
    ink: Rgba,
) {
    let left = geo.labels + geo.x(start as f32) - x;
    let right = geo.labels + geo.x(start.saturating_add(length) as f32) - x;
    let fade_width = |frames: i32| frames.clamp(0, length) as f32 * geo.scale;
    let incoming = left + fade_width(fades[0]);
    let outgoing = right - fade_width(fades[1]);
    let show_in = incoming > 0.0 && left < incoming;
    let show_out = outgoing < width && outgoing < right;
    if !show_in && !show_out {
        return;
    }
    let surface = ui.node(
        "block-fades",
        Node::DEFAULT.at(0.0, 0.0).w_px(width).h_px(height),
    );
    ui.canvas(&surface, move |paint, rect| {
        let fill = Fill::new(Rgba::rgba(0, 0, 0, 107));
        let edge = Stroke::new(Rgba::rgba(ink.r, ink.g, ink.b, 204));
        let bottom = rect.y + rect.h;
        if show_in {
            let x = rect.x + left;
            let to = (rect.x + incoming, rect.y);
            paint.triangle((x, rect.y), to, (x, bottom), fill);
            paint.line((x, bottom), to, edge);
        }
        if show_out {
            let x = rect.x + right;
            let from = (rect.x + outgoing, rect.y);
            paint.triangle(from, (x, rect.y), (x, bottom), fill);
            paint.line(from, (x, bottom), edge);
        }
    });
}

fn overview(ui: &mut Ui, cache: &Cache, at: i32, geo: Geometry, fps: u32, intents: &mut Intents) {
    let state = &mut intents.state;
    ui.row(
        Node {
            w: Size::Grow,
            h: Size::Px(FOOTER_HEIGHT),
            align: Align::Center,
            padding: Edges::xy(8.0, 0.0),
            ..plate(PANEL).look(Look::NONE.fill(PANEL).border(LINE, 1.0).sides(TOP))
        },
        |ui| {
            if geo.labels > COMPACT_LABEL_WIDTH {
                ui.label(
                    state.clipboard_note.as_deref().unwrap_or(
                        "Drag to select · right-click for commands · Cmd+K opens the command palette",
                    ),
                    CAPTION.w(Size::Grow).ellipsis(),
                );
            }
            let duration = geo.end.max(fps as f32);
            let width = geo.overview_width();
            ui.stack(
                plate(CONTROL)
                    .w_px(width)
                    .h_px(OVERVIEW_HEIGHT)
                    .flags(Flags::CLIP),
                |ui| {
                    for clip in &cache.overview {
                        tile(
                            ui,
                            100 + u64::from(clip.slot),
                            clip.x,
                            6.0,
                            clip.width,
                            6.0,
                            Rgba::rgba(VIDEO.r, VIDEO.g, VIDEO.b, 102),
                        );
                    }
                    let window = (geo.width / geo.scale / duration).min(1.0) * width;
                    ui.node(
                        "timeline-overview-window",
                        Node::DEFAULT
                            .at(geo.origin / duration * width, 0.0)
                            .w_px(window)
                            .h_px(OVERVIEW_HEIGHT)
                            .look(
                                Look::NONE
                                    .fill(Rgba::rgba(INK.r, INK.g, INK.b, 26))
                                    .border(INK, 1.0),
                            ),
                    );
                    tile(
                        ui,
                        4,
                        at as f32 / duration * width - 1.0,
                        0.0,
                        2.0,
                        OVERVIEW_HEIGHT,
                        ACCENT,
                    );
                    if geo.scale > geo.fit {
                        let scrub = ui.scrub(
                            "timeline-overview-pan",
                            (geo.origin / duration).clamp(0.0, 1.0),
                            0.0..=1.0,
                            &Scrubber::DEFAULT.name("Pan timeline overview"),
                        );
                        ui.set_node_layout(
                            &scrub.response.handle,
                            &Node::DEFAULT.w_px(width).h_px(OVERVIEW_HEIGHT),
                        );
                        let signal = ui.signal(&scrub.response.handle);
                        if signal.pressed && signal.button == Some(Signal::BUTTON_PRIMARY) {
                            let rect = ui.rect(&scrub.response.handle);
                            let hit = (signal.press_point.0 - rect.x) / width * duration;
                            let visible = geo.width / geo.scale;
                            let inside = (geo.origin..=geo.origin + visible).contains(&hit);
                            state.overview_drag = Some(OverviewDrag {
                                pointer: signal.press_point.0,
                                origin: if inside {
                                    geo.origin
                                } else {
                                    (hit - visible * 0.5).clamp(0.0, geo.max_origin(geo.scale))
                                },
                                restore: inside,
                            });
                        }
                        let origin = if let Some(drag) = state.overview_drag {
                            intents.overview_seen = Some(drag);
                            if !signal.held {
                                state.overview_drag = None;
                            }
                            let x = if signal.released {
                                signal.release_point.0
                            } else {
                                signal.pointer.0
                            };
                            (signal.held || signal.released)
                                .then_some(drag.origin + (x - drag.pointer) / width * duration)
                        } else if !signal.held && !signal.released && !signal.pressed {
                            scrub
                                .preview
                                .or(scrub.commit)
                                .map(|pan| pan * duration - geo.width / geo.scale * 0.5)
                        } else {
                            None
                        };
                        if let Some(origin) = origin {
                            state.manual_pan = true;
                            state.framed_range = None;
                            state.scale = geo.scale;
                            state.origin = origin.clamp(0.0, geo.max_origin(geo.scale));
                        }
                    }
                },
            );
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_only_take_paints_spine_overlay_and_camera_strip_without_warm_allocations() {
        use pxui::{FrameIn, UiDesc, draw::kind};
        use splice_core::history::{Store, WriteOptions};
        use std::fs;

        let sample = crate::editor_views::load_sample().unwrap();
        let media: MediaId = "med_webcam".parse().unwrap();
        let mut project = Project::default();
        project.media.push(
            sample
                .media
                .iter()
                .find(|entry| entry.id == media)
                .unwrap()
                .clone(),
        );
        let folder =
            std::env::temp_dir().join(format!("splice-camera-strip-{}", std::process::id()));
        fs::create_dir(&folder).unwrap();
        let mut store = Store::create(&folder.join("Camera.splice"), &project).unwrap();
        let ops = splice_json::parse(
            r#"[
            {"cmd":"clip.insert","media":"med_webcam"},
            {"cmd":"overlay.add","kind":"media","media":"med_webcam","at":10}
        ]"#,
        )
        .unwrap();
        store
            .apply(ops.as_array().unwrap(), &WriteOptions::default())
            .unwrap();
        let project = store.project();
        let clip = project.clips.id(project.clips.order()[0]).unwrap();
        let view = View {
            width: 960.0,
            height: 440.0,
            at: 20,
            end: 300,
            dragging: false,
            history: (false, false),
            state: State {
                expanded: Some(clip),
                ..State::default()
            },
        };
        let image = Image::rgba8(96, 54, std::ptr::null_mut(), std::ptr::dangling_mut());
        let mut previews = [MediaPreview {
            id: media,
            thumbnail: None,
            camera: None,
            camera_only: false,
            waveform: Vec::new(),
            mic: Vec::new(),
            system: Vec::new(),
            events: splice_av::TimelineEvents::default(),
        }];
        let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
        let mut cache = Cache::default();
        let mut draw = |ui: &mut Ui, previews: &[MediaPreview]| {
            ui.frame(&FrameIn::new(view.width, view.height), &mut |ui| {
                paint(
                    ui,
                    Some(project),
                    view.clone(),
                    &mut cache,
                    (previews, None),
                );
            })
            .unwrap();
        };
        let strip = |ui: &Ui, opacity| {
            ui.layers().any(|layer| {
                layer
                    .instances
                    .iter()
                    .any(|instance| instance.kind == kind::IMAGE && instance.color[3] == opacity)
            })
        };
        draw(&mut ui, &previews);
        assert!(
            [255, 135, 120]
                .into_iter()
                .all(|opacity| !strip(&ui, opacity))
        );
        previews[0].camera = Some(image);
        draw(&mut ui, &previews);
        assert!(strip(&ui, 120));
        assert!(!strip(&ui, 255) && !strip(&ui, 135));
        previews[0].camera_only = true;
        draw(&mut ui, &previews);
        for (row, opacity) in [("spine", 255), ("overlay", 135), ("camera", 120)] {
            assert!(strip(&ui, opacity), "missing {row} filmstrip");
        }
        mem_sys::set_peer(pxui::library_counter);
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                for _ in 0..12 {
                    draw(&mut ui, &previews);
                }
            },
        );
        store.close().unwrap();
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn edit_points_use_overlay_ends_and_fresh_markers_without_allocating() {
        let mut project = crate::editor_views::load_sample().unwrap();
        let mut cache = Cache::default();
        let check = |cache: &Cache, project: &Project| {
            assert_eq!(cache.edit_point(project, 192, 660, false), Some(180));
            assert_eq!(cache.edit_point(project, 192, 660, true), Some(195));
            assert_eq!(cache.edit_point(project, 0, 660, false), None);
            assert_eq!(cache.edit_point(project, 659, 660, true), None);
        };
        check(&cache, &project);
        let geo = Geometry::new(960.0, project.fps, 660, &State::default());
        cache.refresh(Some(&project), geo, project.fps);
        mem_sys::set_peer(pxui::library_counter);
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut || check(&cache, &project));
        project.markers[0].at = 190;
        project.rev += 1;
        assert_eq!(cache.edit_point(&project, 192, 660, false), Some(190));
    }

    #[test]
    fn follow_pages_in_both_directions_and_respects_a_manual_pan() {
        let mut state = State {
            scale: 4.0,
            origin: 100.0,
            manual_pan: true,
            ..State::default()
        };
        state.follow(960.0, 359, 660, 30);
        assert_eq!(state.origin, 100.0);
        state.resume_follow();
        state.follow(960.0, 359, 660, 30);
        assert_eq!(state.origin, 359.0);
        state.follow(960.0, 350, 660, 30);
        let geo = Geometry::new(960.0, 30, 660, &state);
        assert!((geo.x(350.0) - geo.width * 0.9).abs() < 0.01);
        state.follow(960.0, 0, 660, 30);
        let geo = Geometry::new(960.0, 30, 660, &state);
        assert_eq!(geo.x(0.0), 0.0);
    }

    struct WheelTest {
        ui: Ui,
        project: Project,
        view: View,
        cache: Cache,
        page: Option<Handle>,
        events: Vec<Event>,
        encoded: pxui_host::Events,
        time: f64,
    }

    impl WheelTest {
        fn new(width: f32, lanes: u64) -> Self {
            let mut project = Project::default();
            for serial in 1..=lanes {
                project.lanes.push(splice_model::Lane {
                    id: LaneId::from_serial(serial),
                    name: "Graphics".into(),
                    kind: LaneKind::Graphics,
                    effects: Vec::new(),
                });
            }
            project
                .clips
                .insert(splice_model::Clip {
                    id: ClipId::from_serial(1),
                    timing: ClipTiming {
                        start: 0,
                        source: 0,
                        length: 900,
                        speed: 1.0,
                    },
                    data: ClipData::default(),
                })
                .unwrap();
            let mut scene = Self {
                ui: Ui::new(&pxui::UiDesc::DEFAULT).unwrap(),
                project,
                view: View {
                    width,
                    height: 210.0,
                    at: 120,
                    end: 900,
                    dragging: false,
                    history: (false, false),
                    state: State {
                        scale: 4.0,
                        origin: 120.0,
                        ..State::default()
                    },
                },
                cache: Cache::default(),
                page: None,
                events: Vec::with_capacity(input::EVENT_RESERVE),
                encoded: pxui_host::Events::default(),
                time: 0.0,
            };
            scene.frame(&[]);
            scene.frame(&[]);
            scene
        }

        fn frame(&mut self, events: &[Event]) {
            self.events.clear();
            self.events.extend_from_slice(events);
            self.cache.wheel.begin(&self.events);
            self.events.retain(|event| {
                !self
                    .cache
                    .wheel
                    .route(&self.ui, &mut self.view.state, event, true)
            });
            self.time += 1.0 / 60.0;
            let compact = self.view.width <= 480.0;
            let input = pxui::FrameIn::new(self.view.width, 280.0)
                .time(self.time)
                .events(self.encoded.encode(&self.events));
            self.ui
                .frame(&input, &mut |ui| {
                    let mut timeline = |ui: &mut Ui| {
                        let intents = paint(
                            ui,
                            Some(&self.project),
                            self.view.clone(),
                            &mut self.cache,
                            (&[], None),
                        );
                        self.view.state = intents.state;
                        assert!(intents.scrub.is_none());
                        assert!(intents.moved.is_none());
                    };
                    if compact {
                        let page = ui.node(
                            "compact-editor-example",
                            Node::column()
                                .w(Size::Grow)
                                .h(Size::Grow)
                                .flags(Flags::SCROLL | Flags::CLIP | Flags::BLOCKING)
                                .scroll_step(18.0),
                        );
                        self.page = Some(page);
                        ui.within(&page, &mut |ui| {
                            ui.node("editor-header", Node::DEFAULT.h_px(40.0).shrink(0.0));
                            timeline(ui);
                            ui.node("editor-footer", Node::DEFAULT.h_px(100.0).shrink(0.0));
                        });
                    } else {
                        timeline(ui);
                    }
                })
                .unwrap();
        }

        fn geometry(&self) -> Geometry {
            Geometry::new(
                self.view.width,
                self.project.fps,
                self.view.end,
                &self.view.state,
            )
        }

        fn pointer(&self, ruler: bool, x: f32) -> Event {
            let area = self.cache.wheel.area.unwrap();
            let rect = self.ui.rect(if ruler { &area.ruler } else { &area.tracks });
            Event::PointerMoved((f64::from(rect.x + x), f64::from(rect.y + 9.0)))
        }

        fn vertical_offset(&self) -> f32 {
            self.ui
                .scroll(&self.cache.wheel.area.unwrap().tracks)
                .settled
        }
    }

    #[test]
    fn overview_pan_retains_the_grab_offset_and_escape_restores_an_inside_grab() {
        use input::MouseButton::LeftButton;

        mem_sys::set_peer(pxui::library_counter);
        for width in [480.0, 960.0] {
            let mut scene = WheelTest::new(width, 2);
            scene.ui.set_a11y(true);
            scene.frame(&[]);
            let rect = pxui::testing::find(&scene.ui, Role::Slider, "Pan timeline overview")
                .expect("zoomed timeline overview");
            let geo = scene.geometry();
            let origin = geo.origin;
            let at = |frame: f32| {
                (
                    f64::from(rect.x + frame / geo.end * rect.w),
                    f64::from(rect.y + rect.h * 0.5),
                )
            };
            let grab = at(origin + 30.0);
            scene.frame(&[Event::PointerPressed(LeftButton, grab)]);
            assert!((scene.geometry().origin - origin).abs() < 0.01);
            let moved = Event::PointerMoved(at(origin + 60.0));
            scene.frame(&[moved.clone()]);
            assert!((scene.geometry().origin - origin - 30.0).abs() < 0.01);
            assert!(scene.view.state.manual_pan);
            scene.ui.set_a11y(false);
            scene.frame(&[]);
            let meter =
                splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset);
            meter.warm(&mut || scene.frame(&[]));

            let prior = scene.view.state.clone();
            let mut intents = Intents {
                state: prior.clone(),
                ..Intents::default()
            };
            assert!(!intents.escape(&prior, scene.view.at));
            scene.view.state = intents.state;
            assert!((scene.geometry().origin - origin).abs() < 0.01);
            scene.frame(&[Event::PointerReleased(LeftButton, at(origin + 60.0))]);
            assert!((scene.geometry().origin - origin).abs() < 0.01);

            let outside = at(550.0);
            scene.frame(&[Event::PointerPressed(LeftButton, outside)]);
            let centered = 550.0 - geo.width / (2.0 * geo.scale);
            assert!((scene.geometry().origin - centered).abs() < 0.01);
            scene.frame(&[Event::PointerReleased(LeftButton, outside)]);
            assert!((scene.geometry().origin - centered).abs() < 0.01);
        }
    }

    #[test]
    fn timeline_wheel_prioritizes_vertical_tracks_at_both_rails_and_keeps_page_scroll() {
        for width in [480.0, 960.0] {
            let mut scene = WheelTest::new(width, 10);
            let area = scene.cache.wheel.area.unwrap();
            assert!(area.overflow);
            let track = scene.pointer(false, 14.25);
            let ruler = scene.pointer(true, scene.geometry().labels + 140.25);
            let max_scroll = scene.cache.rows_height - scene.ui.rect(&area.tracks).h;

            scene.frame(&[track.clone(), Event::Scrolled((0.0, 1.0))]);
            assert_eq!(scene.vertical_offset(), 0.0);
            assert_eq!(scene.geometry().origin, 120.0);
            assert!(!scene.view.state.manual_pan);
            scene.ui.set_scroll_offset(&area.tracks, 0.0);

            scene.frame(&[ruler, Event::Scrolled((0.0, -2.0))]);
            assert_eq!(scene.vertical_offset(), 2.0 * LANE_HEIGHT);
            assert_eq!(scene.geometry().origin, 120.0);
            scene.frame(&[
                track.clone(),
                Event::Scrolled((0.0, -4.0)),
                Event::TrackpadScrolled {
                    delta: (1.0, -13.0),
                    phase: ScrollPhase::Touched,
                    time: 1.0,
                },
            ]);
            assert_eq!(scene.vertical_offset(), 2.0 * LANE_HEIGHT + 13.0);
            assert_eq!(scene.geometry().origin, 120.0);
            scene.frame(&[Event::TrackpadScrolled {
                delta: (0.0, 0.0),
                phase: ScrollPhase::Lifted,
                time: 1.2,
            }]);

            scene.ui.set_scroll_offset(&area.tracks, max_scroll);
            scene.frame(&[track, Event::Scrolled((0.0, -1.0))]);
            assert_eq!(scene.vertical_offset(), max_scroll);
            assert_eq!(scene.geometry().origin, 120.0);
            assert!(!scene.view.state.manual_pan);
            scene.frame(&[
                Event::KeyPressed(Key::Shift),
                Event::Scrolled((-0.25, -2.0)),
                Event::KeyReleased(Key::Shift),
            ]);
            assert_eq!(scene.vertical_offset(), max_scroll);
            assert_eq!(scene.geometry().origin, 128.0);
            assert!(scene.view.state.manual_pan);

            scene.frame(&[Event::Scrolled((1.0, -1.0))]);
            assert_eq!(scene.geometry().origin, 132.0);
            assert_eq!(scene.vertical_offset(), max_scroll);
            scene.frame(&[
                Event::Scrolled((0.0, 2.0)),
                Event::TrackpadScrolled {
                    delta: (6.5, 1.5),
                    phase: ScrollPhase::Touched,
                    time: 1.3,
                },
            ]);
            let panned = 132.0 + 6.5 / 4.0;
            assert_eq!(scene.geometry().origin, panned);
            assert_eq!(scene.vertical_offset(), max_scroll);

            if let Some(page) = scene.page {
                assert_eq!(scene.ui.scroll(&page).settled, 0.0);
                scene.frame(&[
                    Event::PointerMoved((200.0, 10.0)),
                    Event::Scrolled((0.0, -1.0)),
                ]);
                assert_eq!(scene.ui.scroll(&page).settled, 18.0);
                assert_eq!(scene.vertical_offset(), max_scroll);
                assert_eq!(scene.geometry().origin, panned);
            }
        }
    }

    #[test]
    fn timeline_wheel_zooms_at_fractional_track_and_ruler_points_without_warm_allocations() {
        mem_sys::set_peer(pxui::library_counter);
        for width in [480.0, 960.0] {
            let mut scene = WheelTest::new(width, 0);
            assert!(!scene.cache.wheel.area.unwrap().overflow);
            let x = 113.25;
            let local_x = scene.geometry().labels + x;
            let mut expected = 4.0_f32;
            let anchored = scene.geometry().origin + x / expected;

            for (ruler, key, wheel, exponent) in [
                (true, Key::Ctrl, Event::Scrolled((0.0, 1.0)), 0.1_f32),
                (
                    false,
                    Key::Meta,
                    Event::TrackpadScrolled {
                        delta: (1.0, 39.0),
                        phase: ScrollPhase::Touched,
                        time: 1.0,
                    },
                    39.0 * 0.012,
                ),
                (
                    true,
                    Key::Meta,
                    Event::TrackpadScrolled {
                        delta: (0.0, 40.0),
                        phase: ScrollPhase::Moved,
                        time: 1.01,
                    },
                    40.0 * 0.0025,
                ),
            ] {
                let pointer = scene.pointer(ruler, local_x);
                scene.frame(&[
                    pointer,
                    Event::KeyPressed(key),
                    Event::KeyPressed(Key::Shift),
                    wheel,
                    Event::KeyReleased(Key::Shift),
                    Event::KeyReleased(key),
                ]);
                expected *= exponent.exp();
                let geo = scene.geometry();
                assert!((geo.scale - expected).abs() < 0.00001);
                assert!((geo.x(anchored) - x).abs() < 0.0001);
                assert!(!scene.view.state.manual_pan);
                assert_eq!(scene.vertical_offset(), 0.0);
            }

            scene
                .view
                .state
                .open_menu(ContextTarget::Background(100, None), (20.0, 12.0));
            scene.frame(&[]);
            assert!(scene.view.state.menu_open());
            let before = scene.geometry();
            let clear = scene.pointer(true, width - 34.0);
            scene.frame(&[clear, Event::Scrolled((0.0, -2.0))]);
            assert_eq!(scene.geometry().origin, before.origin + 32.0 / before.scale);
            assert!(!scene.view.state.menu_open());
            assert!(scene.view.state.manual_pan);
            scene.frame(&[Event::Scrolled((1000.0, 0.0))]);
            let geo = scene.geometry();
            assert_eq!(geo.origin, geo.max_origin(geo.scale));

            let right = scene.pointer(true, width - 3.0);
            scene.frame(&[
                right,
                Event::KeyPressed(Key::Meta),
                Event::Scrolled((0.0, -1000.0)),
                Event::KeyReleased(Key::Meta),
            ]);
            assert_eq!(scene.geometry().scale, geo.fit);
            assert_eq!(scene.geometry().origin, 0.0);
            let gutter = scene.pointer(true, 12.0);
            scene.frame(&[
                gutter,
                Event::KeyPressed(Key::Ctrl),
                Event::Scrolled((0.0, 1000.0)),
                Event::KeyReleased(Key::Ctrl),
            ]);
            assert_eq!(scene.geometry().scale, geo.max_scale);
            assert_eq!(scene.geometry().origin, 0.0);
            assert!(scene.view.state.manual_pan);

            scene.view.state.scale = 4.0;
            scene.view.state.origin = 120.0;
            let track = scene.pointer(false, local_x);
            scene.frame(&[track]);
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    for _ in 0..12 {
                        scene.frame(&[Event::Scrolled((0.25, 0.0))]);
                        scene.frame(&[
                            Event::KeyPressed(Key::Meta),
                            Event::TrackpadScrolled {
                                delta: (0.0, 0.25),
                                phase: ScrollPhase::Moved,
                                time: 2.0,
                            },
                        ]);
                        scene.frame(&[
                            Event::TrackpadScrolled {
                                delta: (0.0, -0.25),
                                phase: ScrollPhase::Moved,
                                time: 2.01,
                            },
                            Event::KeyReleased(Key::Meta),
                        ]);
                        scene.frame(&[Event::Scrolled((-0.25, 0.0))]);
                    }
                });
        }
    }

    fn clear_actions(result: &std::cell::RefCell<Intents>) {
        let state = result.borrow().state.clone();
        *result.borrow_mut() = Intents {
            state,
            ..Intents::default()
        };
    }

    fn menu_project() -> Project {
        use splice_model::{Clip, Item, ItemTiming, Lane, Media, Overlay, Probe, Transcript, Word};

        let mut project = Project {
            fps: 60,
            serial: 100,
            ..Project::default()
        };
        let media = MediaId::from_serial(1);
        project.media.push(Media {
            id: media,
            kind: MediaKind::Take,
            path: "take.mov".into(),
            bookmark: None,
            size: 0,
            mtime: 0,
            take: Some(Default::default()),
            probe: Probe {
                fps: 60.0,
                frames: 400,
                width: 960,
                height: 540,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
        });
        for (serial, start, name) in [(1, 0, "Menu Take"), (2, 120, "Next Take")] {
            let mut data = ClipData {
                media: Some(media),
                ..ClipData::default()
            };
            data.appearance.name = Some(name.into());
            data.enabled = splice_model::ComponentStates {
                screen: Some(true),
                mic: Some(true),
                system: Some(false),
                ..Default::default()
            };
            project
                .clips
                .insert(Clip {
                    id: ClipId::from_serial(serial),
                    timing: ClipTiming {
                        start,
                        source: start,
                        length: 120,
                        speed: 1.0,
                    },
                    data,
                })
                .unwrap();
        }
        let captions = LaneId::from_serial(2);
        let overlays = LaneId::from_serial(1);
        for (id, name, kind) in [
            (captions, "Captions", LaneKind::Captions),
            (overlays, "Overlays", LaneKind::Overlay),
        ] {
            project.lanes.push(Lane {
                id,
                name: name.into(),
                kind,
                effects: Vec::new(),
            });
        }
        let clip = ClipId::from_serial(1);
        let words = ["One", "Two", "Three", "Four"].map(|word| word.to_owned());
        project.transcripts.push(Transcript {
            media,
            hash: 1,
            language: "en".into(),
            words: words
                .into_iter()
                .enumerate()
                .map(|(index, text)| {
                    let from = index as i32 * 24 + 10;
                    Word {
                        id: WordId::new(media, index as u32 + 1),
                        text,
                        from,
                        to: from + 13,
                        confidence: None,
                    }
                })
                .collect(),
        });
        for (serial, length, source, lane, kind) in [
            (
                1,
                54,
                10,
                overlays,
                ItemKind::Media(Overlay {
                    media,
                    source: 0,
                    placement: Placement::Card,
                    behind_presenter: false,
                    muted: true,
                    framing: Default::default(),
                }),
            ),
            (
                2,
                37,
                10,
                captions,
                ItemKind::Caption {
                    words: [1, 2].map(|word| WordId::new(media, word)).to_vec(),
                    edits: vec![splice_model::CaptionEdit {
                        word: WordId::new(media, 1),
                        text: None,
                        emoji: None,
                        emph: false,
                        hidden: true,
                    }],
                    text: None,
                },
            ),
            (
                3,
                37,
                58,
                captions,
                ItemKind::Caption {
                    words: [3, 4].map(|word| WordId::new(media, word)).to_vec(),
                    edits: Vec::new(),
                    text: None,
                },
            ),
        ] {
            let mut data = ItemData {
                lane,
                kind,
                appearance: Default::default(),
            };
            data.appearance.name = Some(
                match serial {
                    1 => "Menu Overlay",
                    2 => "Menu Cue",
                    _ => "Next Cue",
                }
                .into(),
            );
            project
                .items
                .insert(Item {
                    id: ItemId::from_serial(serial),
                    timing: ItemTiming {
                        anchor: Anchor::ClipSource { clip, source },
                        length,
                    },
                    data,
                })
                .unwrap();
        }
        project
    }

    fn test_menu_frame(ui: &mut Ui, events: &[pxui::Event], draw: &mut dyn FnMut(&mut Ui)) {
        ui.frame(&pxui::FrameIn::new(960.0, 560.0).events(events), draw)
            .unwrap();
    }

    fn test_menu_point(ui: &Ui, role: Role, name: &str) -> (f64, f64) {
        let rect = pxui::testing::find(ui, role, name).unwrap_or_else(|| {
            let rows: Vec<_> = ui
                .a11y_nodes()
                .iter()
                .filter(|node| node.is_role(role))
                .map(|node| ui.a11y_name_of(node))
                .collect();
            panic!("missing {name}; visible rows {rows:?}")
        });
        (
            f64::from(rect.x + rect.w * 0.5),
            f64::from(rect.y + rect.h * 0.5),
        )
    }

    fn test_menu_click(ui: &mut Ui, point: (f64, f64), button: u32, draw: &mut dyn FnMut(&mut Ui)) {
        use pxui::Event;
        for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
            test_menu_frame(ui, &[Event::pointer(kind, button, point.0, point.1)], draw);
        }
    }

    fn test_open_submenu(ui: &mut Ui, title: &str, draw: &mut dyn FnMut(&mut Ui)) {
        use pxui::Event;
        let (x, y) = test_menu_point(ui, Role::MenuItem, title);
        test_menu_frame(ui, &[Event::pointer(Event::POINTER_MOVED, 0, x, y)], draw);
        test_menu_frame(ui, &[], draw);
    }

    #[test]
    fn inspector_focus_from_context_and_double_click_opens_the_exact_editor() {
        use crate::inspector::{self, Focus, Scope, Selection};
        use splice_core::{edit, package::journal::Source};
        use std::cell::{Cell, RefCell};

        let mut project = menu_project();
        let op =
            splice_json::parse(r#"{"cmd":"title.add","text":"Focus title","at":30,"length":45}"#)
                .unwrap();
        let report = edit::apply(&mut project, &[op], Source::Human, false).unwrap();
        let Target::Timeline(TimelineId::Item(title)) = report.changes[0].created[0] else {
            panic!("expected a title");
        };
        let clip = ClipId::from_serial(1);
        let caption = ItemId::from_serial(2);
        let view = View {
            width: 960.0,
            height: 560.0,
            at: 30,
            end: 240,
            dragging: false,
            history: (false, false),
            state: State {
                scale: 3.0,
                ..Default::default()
            },
        };
        let mut cache = Cache::default();
        let result = RefCell::new(Intents {
            state: view.state.clone(),
            ..Default::default()
        });
        let draw = &mut |ui: &mut Ui| {
            let state = result.borrow().state.clone();
            result.borrow_mut().merge(paint(
                ui,
                Some(&project),
                View {
                    state,
                    ..view.clone()
                },
                &mut cache,
                (&[], None),
            ));
        };
        let mut timeline_ui = crate::viewer_ui().unwrap();
        timeline_ui.set_a11y(true);
        for _ in 0..3 {
            test_menu_frame(&mut timeline_ui, &[], draw);
        }
        let mut editor_ui = crate::viewer_ui().unwrap();
        editor_ui.set_a11y(true);
        mem_sys::set_peer(pxui::library_counter);

        for (name, owner, selected, focus, field, double) in [
            (
                "Menu Take",
                TimelineId::Clip(clip),
                Selection::Clip(clip),
                Focus::Speed(clip),
                "Custom speed",
                false,
            ),
            (
                "Focus title",
                TimelineId::Item(title),
                Selection::Item(title),
                Focus::Text(title),
                "Title text",
                false,
            ),
            (
                "Menu Cue",
                TimelineId::Item(caption),
                Selection::Item(caption),
                Focus::Text(caption),
                "Caption text",
                false,
            ),
            (
                "Menu Cue",
                TimelineId::Item(caption),
                Selection::Item(caption),
                Focus::Text(caption),
                "Caption text",
                true,
            ),
        ] {
            for _ in 0..2 {
                test_menu_frame(&mut timeline_ui, &[], draw);
            }
            let hit = test_menu_point(&timeline_ui, Role::Button, name);
            if double {
                use pxui::Event;
                test_menu_click(&mut timeline_ui, hit, 0, draw);
                test_menu_frame(
                    &mut timeline_ui,
                    &[Event::pointer(Event::POINTER_PRESSED, 0, hit.0, hit.1)],
                    draw,
                );
                assert!(timeline_ui.captured());
                assert!(result.borrow().inspector_focus.is_none());
                test_menu_frame(
                    &mut timeline_ui,
                    &[Event::pointer(Event::POINTER_RELEASED, 0, hit.0, hit.1)],
                    draw,
                );
            } else {
                test_menu_click(&mut timeline_ui, hit, 1, draw);
                let entry = if matches!(focus, Focus::Speed(_)) {
                    test_open_submenu(&mut timeline_ui, "Speed", draw);
                    "Custom…"
                } else {
                    "Edit Text…"
                };
                let menu_item = test_menu_point(&timeline_ui, Role::MenuItem, entry);
                test_menu_click(&mut timeline_ui, menu_item, 0, draw);
            }
            let mut intent = result.borrow_mut();
            assert_eq!(intent.state.selection(), Some(owner));
            assert_eq!(intent.inspector_focus.take(), Some(focus));
            assert!(intent.toolbar.is_none());
            drop(intent);

            let mut cache = inspector::captions::Cache::default();
            let selection = Cell::new(Selection::None);
            let pending = Cell::new(Some(focus));
            let focused = RefCell::new(inspector::Intents::default());
            let mut paint = |ui: &mut Ui| {
                cache.refresh(ui, Some(&project), selection.get());
                let next = inspector::paint(
                    ui,
                    Some(&project),
                    selection.get(),
                    Scope::Selection,
                    240,
                    (268.0, false),
                    (&[], true, None, Some((&cache, 30)), pending.take()),
                );
                assert!(
                    next.op.is_none(),
                    "focus and cancellation never write an edit"
                );
                if next.focused_field.is_some() {
                    focused.replace(next);
                }
            };
            let frame = pxui::FrameIn::new(268.0, 180.0);
            editor_ui.frame(&frame, &mut paint).unwrap();
            assert!(
                !editor_ui.text_input_focused(),
                "ignore a request for a different owner"
            );
            selection.set(selected);
            pending.set(Some(focus));
            editor_ui.frame(&frame, &mut paint).unwrap();
            assert!(
                editor_ui.text_input_focused(),
                "{field} must really be editing"
            );
            if inspector::reveal_focus(&mut editor_ui, &focused.borrow()) {
                editor_ui.frame(&frame, &mut paint).unwrap();
            }
            let node = editor_ui
                .a11y_nodes()
                .iter()
                .find(|node| node.is_role(Role::TextField) && editor_ui.a11y_name_of(node) == field)
                .expect("focused field");
            assert!(node.states & pxui::A11yStates::FOCUSED.bits() != 0);
            let view = editor_ui.rect(&focused.borrow().focus_scroll.unwrap());
            let rect = editor_ui.rect(&focused.borrow().focused_field.unwrap());
            assert!(
                rect.y >= view.y - 1.0 && rect.y + rect.h <= view.y + view.h + 1.0,
                "{field} {rect:?} outside {view:?}"
            );
            let mut encoded = pxui_host::Events::default();
            let escape = [
                Event::KeyPressed(Key::Escape),
                Event::KeyReleased(Key::Escape),
            ];
            editor_ui
                .frame(&frame.events(encoded.encode(&escape)), &mut paint)
                .unwrap();
            assert!(!editor_ui.text_input_focused());
            let mut idle = || {
                editor_ui.frame(&frame, &mut paint).unwrap();
            };
            for _ in 0..8 {
                idle();
            }
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut idle);
        }
    }

    #[test]
    fn clip_block_menu_dispatches_speed_mute_and_crop_and_reuses_warm_submenus() {
        use splice_core::{edit, package::journal::Source};
        use std::cell::RefCell;

        let project = menu_project();
        let mut cache = Cache::default();
        let view = View {
            width: 960.0,
            height: 560.0,
            at: 1,
            end: 240,
            dragging: false,
            history: (false, false),
            state: State {
                scale: 3.0,
                ..Default::default()
            },
        };
        let result = RefCell::new(Intents {
            state: view.state.clone(),
            ..Default::default()
        });
        let draw = &mut |ui: &mut Ui| {
            let state = result.borrow().state.clone();
            let intents = paint(
                ui,
                Some(&project),
                View {
                    state,
                    ..view.clone()
                },
                &mut cache,
                (&[], None),
            );
            result.borrow_mut().merge(intents);
        };
        let clip = ClipId::from_serial(1);
        let slot = project.clips.find(clip).unwrap();
        let next = project.clips.find(ClipId::from_serial(2)).unwrap();
        mem_sys::set_peer(pxui::library_counter);
        let mut ui = Ui::new(&pxui::UiDesc::DEFAULT).unwrap();
        ui.set_a11y(true);
        test_menu_frame(&mut ui, &[], draw);
        test_menu_frame(&mut ui, &[], draw);
        let hit = test_menu_point(&ui, Role::Button, "Menu Take");
        for ripple in [false, true] {
            result.borrow_mut().state.free_move = !ripple;
            test_menu_click(&mut ui, hit, 1, draw);
            assert_eq!(
                result.borrow().state.selection(),
                Some(TimelineId::Clip(clip))
            );
            let rows: Vec<_> = [
                "Split at Playhead",
                "Trim Start to Playhead",
                "Trim End to Playhead",
                "Speed",
                "Fades",
                "Show Components",
                "Mute Audio",
                "Lift to Overlay",
                "Crop…",
            ]
            .map(|row| test_menu_point(&ui, Role::MenuItem, row).1)
            .into();
            assert!(rows.windows(2).all(|rows| rows[0] < rows[1]));
            let split = test_menu_point(&ui, Role::MenuItem, "Split at Playhead");
            test_menu_click(&mut ui, split, 0, draw);
            assert!(result.borrow().state.menu_open());
            assert!(result.borrow().split.is_none());
            test_open_submenu(&mut ui, "Speed", draw);
            let speed = test_menu_point(&ui, Role::MenuItem, "2×");
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    for _ in 0..6 {
                        test_menu_frame(&mut ui, &[], draw);
                    }
                });
            test_menu_click(&mut ui, speed, 0, draw);
            let ops = result
                .borrow_mut()
                .toolbar
                .take()
                .expect("speed action")
                .operations();
            assert_eq!(
                ops[0].field("ripple").and_then(splice_json::Value::as_bool),
                Some(ripple)
            );
            let mut edited = project.clone();
            edit::apply(&mut edited, &ops, Source::Human, false).unwrap();
            assert_eq!(edited.clips.timing(slot).unwrap().length, 60);
            assert_eq!(
                edited.clips.timing(next).unwrap().start,
                if ripple { 60 } else { 120 }
            );
        }

        let mut decorated = project.clone();
        for (group, row) in [("Fades", "Fade In 0.5 s"), ("Color", "Blue")] {
            test_menu_click(&mut ui, hit, 1, draw);
            test_open_submenu(&mut ui, group, draw);
            let choice = test_menu_point(&ui, Role::MenuItem, row);
            test_menu_click(&mut ui, choice, 0, draw);
            let ops = result
                .borrow_mut()
                .toolbar
                .take()
                .expect("decoration action")
                .operations();
            edit::apply(&mut decorated, &ops, Source::Human, false).unwrap();
        }
        let decorated = &decorated.clips.data(slot).unwrap().appearance;
        assert_eq!(decorated.fade_in, 30);
        assert_eq!(decorated.color.as_deref(), Some("blue"));

        test_menu_click(&mut ui, hit, 1, draw);
        let mute = test_menu_point(&ui, Role::MenuItem, "Mute Audio");
        test_menu_click(&mut ui, mute, 0, draw);
        let ops = result
            .borrow_mut()
            .toolbar
            .take()
            .expect("audio batch")
            .operations();
        assert_eq!(ops.len(), 2);
        let mut edited = project.clone();
        edit::apply(&mut edited, &ops, Source::Human, false).unwrap();
        let tracks = &edited.clips.data(slot).unwrap().enabled;
        assert_eq!(
            [tracks.mic, tracks.system, tracks.screen],
            [Some(false), Some(false), Some(true)]
        );

        test_menu_click(&mut ui, hit, 1, draw);
        let crop = test_menu_point(&ui, Role::MenuItem, "Crop…");
        test_menu_click(&mut ui, crop, 0, draw);
        assert_eq!(result.borrow().crop, Some(TimelineId::Clip(clip)));
        assert!(result.borrow().toolbar.is_none());

        clear_actions(&result);
        test_menu_click(&mut ui, hit, 1, draw);
        let copy = test_menu_point(&ui, Role::MenuItem, "Copy");
        test_menu_click(&mut ui, copy, 0, draw);
        {
            let mut intents = result.borrow_mut();
            let action = intents.clipboard.take().unwrap();
            assert!(action == ClipboardAction::Copy);
            assert!(
                intents
                    .state
                    .clipboard_operation(action, &project, 1)
                    .is_none()
            );
        }
        test_menu_frame(&mut ui, &[], draw);
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut || test_menu_frame(&mut ui, &[], draw));
        test_menu_click(&mut ui, hit, 1, draw);
        let paste = test_menu_point(&ui, Role::MenuItem, "Paste");
        test_menu_click(&mut ui, paste, 0, draw);
        assert!(result.borrow_mut().clipboard.take() == Some(ClipboardAction::Paste));

        result.borrow_mut().state.open_menu(
            ContextTarget::Background(200, Some(project.spine.id)),
            (600.0, 320.0),
        );
        test_menu_frame(&mut ui, &[], draw);
        let here = test_menu_point(&ui, Role::MenuItem, "Paste Here");
        test_menu_click(&mut ui, here, 0, draw);
        assert!(result.borrow().clipboard == Some(ClipboardAction::PasteHere(200)));
    }

    #[test]
    fn overlay_and_caption_block_menus_keep_kind_and_anchor_state() {
        use splice_core::{edit, package::journal::Source};
        use std::cell::RefCell;

        let project = RefCell::new(menu_project());
        let mut cache = Cache::default();
        let view = View {
            width: 960.0,
            height: 560.0,
            at: 30,
            end: 240,
            dragging: false,
            history: (false, false),
            state: State {
                scale: 3.0,
                ..Default::default()
            },
        };
        let result = RefCell::new(Intents {
            state: view.state.clone(),
            ..Default::default()
        });
        let draw = &mut |ui: &mut Ui| {
            let state = result.borrow().state.clone();
            let project = project.borrow();
            let intents = paint(
                ui,
                Some(&project),
                View {
                    state,
                    ..view.clone()
                },
                &mut cache,
                (&[], None),
            );
            result.borrow_mut().merge(intents);
        };
        let apply = || {
            let ops = result
                .borrow_mut()
                .toolbar
                .take()
                .expect("chosen command")
                .operations();
            let mut project = project.borrow_mut();
            edit::apply(&mut project, &ops, Source::Human, false).unwrap();
            project.rev += 1;
            ops
        };
        let mut ui = Ui::new(&pxui::UiDesc::DEFAULT).unwrap();
        ui.set_a11y(true);
        test_menu_frame(&mut ui, &[], draw);
        test_menu_frame(&mut ui, &[], draw);

        let overlay = ItemId::from_serial(1);
        let slot = project.borrow().items.find(overlay).unwrap();
        let hit = test_menu_point(&ui, Role::Button, "Menu Overlay");
        test_menu_click(&mut ui, hit, 1, draw);
        assert!(pxui::testing::find(&ui, Role::MenuItem, "Speed").is_none());
        test_open_submenu(&mut ui, "Placement", draw);
        let choice = test_menu_point(&ui, Role::MenuItem, "corner-tr");
        test_menu_click(&mut ui, choice, 0, draw);
        apply();
        assert!(matches!(project.borrow().items.data(slot).unwrap().kind,
            ItemKind::Media(overlay) if overlay.placement == Placement::TopRight));
        for (label, pinned) in [("Pin to Timeline", true), ("Anchor to Clip Below", false)] {
            test_menu_click(&mut ui, hit, 1, draw);
            let choice = test_menu_point(&ui, Role::MenuItem, label);
            test_menu_click(&mut ui, choice, 0, draw);
            apply();
            assert_eq!(
                matches!(
                    project.borrow().items.timing(slot).unwrap().anchor,
                    Anchor::Pinned(_)
                ),
                pinned
            );
        }

        test_menu_frame(&mut ui, &[], draw);
        let hit = test_menu_point(&ui, Role::Button, "Menu Cue");
        test_menu_click(&mut ui, hit, 1, draw);
        assert_eq!(
            result.borrow().state.selection(),
            Some(TimelineId::Item(ItemId::from_serial(2)))
        );
        assert!(result.borrow().state.menu_open());
        assert!(pxui::testing::find(&ui, Role::MenuItem, "Pin to Timeline").is_none());
        test_open_submenu(&mut ui, "Emphasize Word", draw);
        assert!(pxui::testing::find(&ui, Role::MenuItem, "One").is_none());
        let word = test_menu_point(&ui, Role::MenuItem, "Two");
        test_menu_click(&mut ui, word, 0, draw);
        assert_eq!(
            apply()[0].field("cmd").and_then(splice_json::Value::as_str),
            Some("caption.set")
        );

        test_menu_click(&mut ui, hit, 1, draw);
        test_open_submenu(&mut ui, "Style", draw);
        let style = test_menu_point(&ui, Role::MenuItem, "Clean");
        test_menu_click(&mut ui, style, 0, draw);
        assert_eq!(
            apply()[0].field("cmd").and_then(splice_json::Value::as_str),
            Some("effect.set")
        );

        test_menu_click(&mut ui, hit, 1, draw);
        let split = test_menu_point(&ui, Role::MenuItem, "Split at Playhead");
        test_menu_click(&mut ui, split, 0, draw);
        let ops = apply();
        assert_eq!(
            ops[0].field("cmd").and_then(splice_json::Value::as_str),
            Some("caption.split")
        );
        assert_eq!(project.borrow().items.order().len(), 4);
        test_menu_frame(&mut ui, &[], draw);
        let hit = test_menu_point(&ui, Role::Button, "Menu Cue");
        test_menu_click(&mut ui, hit, 1, draw);
        let merge = test_menu_point(&ui, Role::MenuItem, "Merge with Next");
        test_menu_click(&mut ui, merge, 0, draw);
        assert_eq!(
            apply()[0].field("cmd").and_then(splice_json::Value::as_str),
            Some("caption.merge")
        );
        assert_eq!(project.borrow().items.order().len(), 3);
    }

    #[test]
    fn right_click_timeline_keeps_group_and_dispatches_context_actions() {
        use pxui::{Event, Role, testing};
        use splice_model::Clip;
        use std::cell::RefCell;

        let mut project = Project::default();
        for (serial, start, name) in [(1, 0, "First"), (2, 90, "Second")] {
            let mut data = ClipData::default();
            data.appearance.name = Some(name.into());
            project
                .clips
                .insert(Clip {
                    id: ClipId::from_serial(serial),
                    timing: ClipTiming {
                        start,
                        length: 90,
                        source: 0,
                        speed: 1.0,
                    },
                    data,
                })
                .unwrap();
        }
        let selected = [1, 2].map(|id| TimelineId::Clip(ClipId::from_serial(id)));
        let mut state = State::default();
        state.set_selected(selected.to_vec());
        let view = View {
            width: 320.0,
            height: 240.0,
            at: 0,
            end: 180,
            dragging: false,
            history: (false, false),
            state,
        };
        let result = RefCell::new(Intents {
            state: view.state.clone(),
            ..Intents::default()
        });
        let mut cache = Cache::default();
        let build = &mut |ui: &mut Ui| {
            let mut intents = result.borrow().clone();
            let state = intents.state.clone();
            intents.merge(paint(
                ui,
                Some(&project),
                View {
                    state,
                    ..view.clone()
                },
                &mut cache,
                (&[], None),
            ));
            *result.borrow_mut() = intents;
        };
        let mut ui = pxui::doc::frames(2, build);
        let item = testing::find(&ui, Role::Button, "Second").unwrap();
        let (x, y) = (
            f64::from(item.x + item.w * 0.5),
            f64::from(item.y + item.h * 0.5),
        );
        for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
            testing::frame(&mut ui, &[Event::pointer(kind, 1, x, y)], build);
        }
        assert!(result.borrow().state.menu_open());
        assert_eq!(result.borrow().state.selected(), &selected);
        let duplicate = testing::find(&ui, Role::MenuItem, "Duplicate").unwrap();
        testing::click(
            &mut ui,
            f64::from(duplicate.x + duplicate.w * 0.5),
            f64::from(duplicate.y + duplicate.h * 0.5),
            build,
        );
        assert!(
            matches!(&result.borrow().toolbar, Some(ToolbarCommand::Duplicate(ids)) if ids.as_ref() == selected)
        );
        assert!(!result.borrow().state.menu_open());

        clear_actions(&result);
        let ruler = testing::find(&ui, Role::Slider, "Timeline playhead").unwrap();
        let (x, y) = (
            f64::from(ruler.x + ruler.w * 0.5),
            f64::from(ruler.y + ruler.h * 0.5),
        );
        for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
            testing::frame(&mut ui, &[Event::pointer(kind, 1, x, y)], build);
        }
        let seek = testing::find(&ui, Role::MenuItem, "Move Playhead Here").unwrap();
        testing::click(
            &mut ui,
            f64::from(seek.x + seek.w * 0.5),
            f64::from(seek.y + seek.h * 0.5),
            build,
        );
        assert_eq!(result.borrow().scrub, Some((Phase::Ended, 94)));
    }
    #[test]
    fn marquee_and_shift_click_preserve_exact_group_and_warm_paint() {
        use pxui::{Event, Role, testing};
        use splice_model::Clip;
        use std::cell::RefCell;

        let mut project = Project::default();
        for (serial, start, name) in [(1, 18, "First"), (2, 78, "Second"), (3, 160, "Third")] {
            let mut data = ClipData::default();
            data.appearance.name = Some(name.into());
            project
                .clips
                .insert(Clip {
                    id: ClipId::from_serial(serial),
                    timing: ClipTiming {
                        start,
                        length: 36,
                        source: 0,
                        speed: 1.0,
                    },
                    data,
                })
                .unwrap();
        }
        let view = View {
            width: 320.0,
            height: 240.0,
            at: 0,
            end: 196,
            dragging: false,
            history: (false, false),
            state: State::default(),
        };
        let geo = Geometry::new(view.width, project.fps, view.end, &view.state);
        let selected = [1, 2, 3].map(|id| TimelineId::Clip(ClipId::from_serial(id)));
        let result = RefCell::new(Intents::default());
        let mut cache = Cache::default();
        let build = &mut |ui: &mut Ui| {
            let mut intents = result.borrow().clone();
            let state = intents.state.clone();
            intents.merge(paint(
                ui,
                Some(&project),
                View {
                    state,
                    ..view.clone()
                },
                &mut cache,
                (&[], None),
            ));
            *result.borrow_mut() = intents;
        };
        let mut ui = pxui::doc::frames(2, build);
        let first = testing::find(&ui, Role::Button, "First").unwrap();
        let third = testing::find(&ui, Role::Button, "Third").unwrap();
        let starting = f64::from(geo.labels + geo.x(5.0));
        let ending = f64::from(geo.labels + geo.x(125.0));
        let from_y = f64::from(first.y + 4.0);
        let to_y = f64::from(first.y + first.h - 4.0);
        for (kind, x, y) in [
            (Event::POINTER_PRESSED, starting, from_y),
            (Event::POINTER_MOVED, ending, to_y),
        ] {
            testing::frame(&mut ui, &[Event::pointer(kind, 0, x, y)], build);
        }
        assert!(result.borrow().state.marquee.is_some_and(Marquee::moved));
        assert!(result.borrow().state.selected().is_empty());
        testing::frame(
            &mut ui,
            &[Event::pointer(Event::POINTER_RELEASED, 0, ending, to_y)],
            build,
        );
        assert_eq!(result.borrow().state.selected(), &selected[..2]);

        // The input ABI names Shift as its special-key base (0x110000) plus 9.
        testing::frame(&mut ui, &[Event::key(Event::KEY_PRESSED, 0x11_0009)], build);
        let (x, y) = (
            f64::from(third.x + third.w * 0.5),
            f64::from(third.y + third.h * 0.5),
        );
        testing::click(&mut ui, x, y, build);
        assert_eq!(result.borrow().state.selected(), &selected);
        testing::click(&mut ui, x, y, build);
        assert_eq!(result.borrow().state.selected(), &selected[..2]);
        testing::frame(
            &mut ui,
            &[Event::key(Event::KEY_RELEASED, 0x11_0009)],
            build,
        );

        mem_sys::set_peer(pxui::library_counter);
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                for _ in 0..8 {
                    testing::frame(&mut ui, &[], build);
                }
            },
        );
        assert_eq!(result.borrow().state.selected(), &selected[..2]);
    }
    #[test]
    fn ready_media_thumbnails_and_source_waveforms_stay_allocation_free() {
        use pxui::{UiDesc, testing};
        use splice_model::{
            Clip, Ease, Effect, EffectId, EffectParam, Key, Media, MediaKind, Probe,
        };

        let peaks = peak_columns(&[1.0, 0.2, 0.4, 0.8], [0.5, 1.0], 6.0, 0.5);
        assert_eq!(peaks.count, 2);
        assert_eq!(&peaks.heights[..2], &[51, 102]);

        let media = MediaId::from_serial(1);
        let clip = ClipId::from_serial(2);
        let mut project = Project::default();
        project.media.push(Media {
            id: media,
            kind: MediaKind::Take,
            path: "Screen take".into(),
            bookmark: None,
            size: 0,
            mtime: 0,
            probe: Probe {
                fps: 30.0,
                frames: 240,
                width: 80,
                height: 60,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
            take: None,
        });
        let mut data = ClipData::default();
        data.media = Some(media);
        data.enabled.screen = Some(true);
        data.enabled.camera = Some(true);
        data.enabled.mic = Some(true);
        data.enabled.system = Some(true);
        data.appearance.effects.push(Effect {
            id: EffectId::from_serial(3),
            kind: "speed".into(),
            enabled: Some(true),
            params: vec![EffectParam {
                name: "rate".into(),
                value: EffectValue::Keys(vec![
                    Key {
                        at: 0,
                        value: 0.25,
                        ease: Ease::Linear,
                    },
                    Key {
                        at: 120,
                        value: 1.25,
                        ease: Ease::Linear,
                    },
                ]),
            }],
        });
        let slot = project
            .clips
            .insert(Clip {
                id: clip,
                timing: ClipTiming {
                    start: 0,
                    length: 120,
                    source: 60,
                    speed: 1.0,
                },
                data,
            })
            .unwrap();
        let previews = [MediaPreview {
            events: splice_av::TimelineEvents::default(),
            id: media,
            camera_only: false,
            thumbnail: Some(Image::rgba8(
                80,
                60,
                std::ptr::null_mut(),
                std::ptr::dangling_mut(),
            )),
            waveform: (0..120).map(|index| index as f32 / 120.0).collect(),
            camera: Some(Image::rgba8(
                64,
                48,
                std::ptr::null_mut(),
                std::ptr::dangling_mut(),
            )),
            mic: vec![0.25; 120],
            system: (0..120).map(|index| index as f32 / 120.0).collect(),
        }];
        let view = View {
            width: 320.0,
            height: 240.0,
            at: 0,
            end: 120,
            dragging: false,
            history: (false, false),
            state: State {
                expanded: Some(clip),
                ..State::default()
            },
        };
        let geo = Geometry::new(view.width, project.fps, view.end, &view.state);
        let (x, width) = geo.visible(0, 120).unwrap();
        let source = audio_peaks(
            &previews,
            &project,
            (media, 60, 1.0, 1.0),
            (geo, 0, x, width),
        )
        .unwrap();
        assert!(source.heights[0] >= 63);
        assert!(source.heights[source.count - 1] < 195);

        mem_sys::set_peer(pxui::library_counter);
        let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
        let mut cache = Cache::default();
        let timing = project.clips.timing(slot).unwrap();
        let [_, camera, mic, system, _, _] = components(project.clips.data(slot).unwrap());
        let camera = take_component_preview(
            &mut cache,
            &previews,
            &project,
            slot,
            timing,
            camera,
            true,
            (geo, x, width),
        );
        assert_eq!(camera.thumbnail.map(|image| image.width), Some(64));
        let mic = take_component_preview(
            &mut cache,
            &previews,
            &project,
            slot,
            timing,
            mic,
            true,
            (geo, x, width),
        )
        .waveform
        .unwrap();
        let system = take_component_preview(
            &mut cache,
            &previews,
            &project,
            slot,
            timing,
            system,
            true,
            (geo, x, width),
        )
        .waveform
        .unwrap();
        assert!(mic.heights[..mic.count].iter().all(|&peak| peak == 64));
        assert!(system.heights[system.count - 1] > mic.heights[mic.count - 1]);
        let expected_end = (60.0 + consumed(&project, slot, 120.0).unwrap()) / 240.0;
        assert!((cache.keyed_waves[0].source[system.count] - expected_end).abs() < 0.001);
        let mut frame = || {
            for at in 0..12 {
                testing::frame(&mut ui, &[], &mut |ui| {
                    paint(
                        ui,
                        Some(&project),
                        View { at, ..view.clone() },
                        &mut cache,
                        (&previews, None),
                    );
                });
            }
        };
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut frame);
    }
    #[test]
    fn framed_range_survives_resize_until_manual_zoom() {
        let mut state = State::default();
        state.frame_range([250, 500]);
        let wide = Geometry::new(936.0, 30, 1000, &state);
        let narrow = Geometry::new(640.0, 30, 1000, &state);
        for geo in [wide, narrow] {
            assert!(geo.x(250.0).abs() < 0.01);
            assert!((geo.x(500.0) - geo.width).abs() < 0.01);
        }
        narrow.zoom(&mut state, narrow.scale * 1.4, 350);
        assert!(state.framed_range.is_none());
    }
    #[test]
    fn virtual_tracks_keep_extent_and_build_only_the_scroll_window() {
        use pxui::{Event, Role, testing};
        use splice_model::{Clip, Lane};

        let mut project = Project::default();
        let id = ClipId::from_serial(1);
        let mut data = ClipData::default();
        data.enabled.screen = Some(true);
        data.enabled.mic = Some(false);
        project
            .clips
            .insert(Clip {
                id,
                timing: ClipTiming {
                    start: 0,
                    length: 120,
                    source: 0,
                    speed: 1.0,
                },
                data,
            })
            .unwrap();
        for index in 0..240 {
            project.lanes.push(Lane {
                id: LaneId::from_serial(index + 1),
                name: format!("Track {index}"),
                kind: LaneKind::Graphics,
                effects: Vec::new(),
            });
        }
        let state = State {
            expanded: Some(id),
            ..State::default()
        };
        let mut cache = Cache::default();
        cache.refresh_rows(&project, &state);
        let desired_height = 240.0 * LANE_HEIGHT + SPINE_HEIGHT + 18.0 + 24.0 + 30.0;
        assert_eq!(cache.rows_height, desired_height);
        let before = cache
            .rows
            .iter()
            .skip(cache.first_visible_row(0.0))
            .take_while(|row| row.top < 160.0 + TRACK_OVERSCAN)
            .count();
        assert!(before <= 9);
        let near_end = &cache.rows[cache.first_visible_row(desired_height - 160.0)..];
        assert!(matches!(
            near_end.last().unwrap().track,
            Track::Component(_, _)
        ));
        assert!(near_end.len() <= 9);
        let mut build = |ui: &mut Ui| {
            paint(
                ui,
                Some(&project),
                View {
                    width: 320.0,
                    height: 240.0,
                    at: 0,
                    end: 120,
                    dragging: false,
                    history: (false, false),
                    state: state.clone(),
                },
                &mut cache,
                (&[], None),
            );
        };
        let mut ui = pxui::doc::frames(2, &mut build);
        let viewport = ui
            .a11y_nodes()
            .iter()
            .find(|entry| entry.is_role(Role::ScrollArea))
            .map(|entry| (entry.id, entry.rect))
            .unwrap();
        assert!(testing::find(&ui, Role::Button, "Track 0").is_some());
        assert!(testing::find(&ui, Role::Button, "Track 100").is_none());
        let (x, y) = (
            f64::from(viewport.1.x + 42.0),
            f64::from(viewport.1.y + 42.0),
        );
        testing::frame(
            &mut ui,
            &[Event::pointer(Event::POINTER_MOVED, 0, x, y)],
            &mut build,
        );
        testing::frame(
            &mut ui,
            &[Event::pointer(Event::SCROLLED, 0, 0.0, -100.0)],
            &mut build,
        );
        assert!(testing::find(&ui, Role::Button, "Track 100").is_some());
        assert!(testing::find(&ui, Role::Button, "Track 0").is_none());
        let scroll = ui.box_of_a11y(viewport.0).unwrap();
        ui.set_scroll_offset(&scroll, 100_000.0);
        testing::frame(&mut ui, &[], &mut build);
        assert_eq!(ui.scroll(&scroll).settled, desired_height - viewport.1.h);
        assert!(testing::find(&ui, Role::Button, "Show microphone").is_some());
    }
    #[test]
    fn fade_handles_drag_a_warm_draft_commit_once_and_cancel() {
        use pxui::{Event, Role, testing};
        use splice_core::history::{Store, WriteOptions};
        use splice_model::{Clip, Media, MediaKind, Probe};
        use std::cell::RefCell;

        let id = ClipId::from_serial(2);
        let media = MediaId::from_serial(1);
        let mut project = Project {
            serial: 2,
            ..Project::default()
        };
        project.media.push(Media {
            id: media,
            kind: MediaKind::Video,
            path: "Fade source.mov".into(),
            bookmark: None,
            size: 0,
            mtime: 0,
            probe: Probe {
                fps: 30.0,
                frames: 240,
                width: 80,
                height: 60,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
            take: None,
        });
        let mut data = ClipData {
            media: Some(media),
            ..ClipData::default()
        };
        data.appearance.name = Some("Fadable".into());
        data.appearance.fade_out = 20;
        project
            .clips
            .insert(Clip {
                id,
                timing: ClipTiming {
                    start: 0,
                    length: 120,
                    source: 0,
                    speed: 1.0,
                },
                data,
            })
            .unwrap();

        let path =
            std::env::temp_dir().join(format!("splice-fade-handles-{}.splice", std::process::id()));
        let store = RefCell::new(Store::create(&path, &project).unwrap());
        let view = View {
            width: 320.0,
            height: 240.0,
            at: 0,
            end: 120,
            dragging: false,
            history: (false, false),
            state: State::default(),
        };
        let result = RefCell::new(Intents::default());
        let mut cache = Cache::default();
        let build = &mut |ui: &mut Ui| {
            let next = paint(
                ui,
                Some(store.borrow().project()),
                View {
                    state: result.borrow().state.clone(),
                    ..view.clone()
                },
                &mut cache,
                (&[], None),
            );
            result.borrow_mut().merge(next);
        };
        mem_sys::set_peer(pxui::library_counter);
        let mut ui = pxui::doc::frames(2, build);
        let block = testing::find(&ui, Role::Button, "Fadable").unwrap();
        let geo = Geometry::new(view.width, project.fps, view.end, &view.state);
        let handles = fade_handles(geo, geo.labels, 0, 120, [0, 20]).unwrap();
        let y = f64::from(block.y + 4.0);
        let incoming = f64::from(block.x + handles[0]);
        let outgoing = f64::from(block.x + handles[1]);
        let far = incoming + f64::from(150.0 * geo.scale);
        testing::frame(
            &mut ui,
            &[Event::pointer(Event::POINTER_PRESSED, 0, incoming, y)],
            build,
        );
        let motion = Event::pointer(Event::POINTER_MOVED, 0, far, y);
        testing::frame(&mut ui, &[motion], build);
        assert_eq!(
            result.borrow().state.selection(),
            Some(TimelineId::Clip(id))
        );
        assert_eq!(
            result.borrow().state.fade_drag.map(|drag| drag.value),
            Some(100)
        );
        assert!(result.borrow().toolbar.is_none());
        assert_eq!(store.borrow().project().rev, 0);
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut || testing::frame(&mut ui, &[motion], build));
        testing::frame(
            &mut ui,
            &[Event::pointer(Event::POINTER_RELEASED, 0, far, y)],
            build,
        );
        assert!(result.borrow().moved.is_none());
        assert!(result.borrow().trimmed.is_none());
        assert!(result.borrow().state.fade_drag.is_none());
        let op = result.borrow_mut().toolbar.take().unwrap().operation();
        store
            .borrow_mut()
            .apply(&[op], &WriteOptions::default())
            .unwrap();
        assert_eq!(store.borrow().project().rev, 1);
        let read_fades = || {
            let store = store.borrow();
            let clips = &store.project().clips;
            let a = &clips.data(clips.find(id).unwrap()).unwrap().appearance;
            [a.fade_in, a.fade_out]
        };
        assert_eq!(read_fades(), [100, 20]);

        let target = outgoing + f64::from(10.0 * geo.scale);
        for escape in [true, false] {
            clear_actions(&result);
            testing::frame(
                &mut ui,
                &[Event::pointer(Event::POINTER_PRESSED, 0, outgoing, y)],
                build,
            );
            testing::frame(
                &mut ui,
                &[Event::pointer(Event::POINTER_MOVED, 0, target, y)],
                build,
            );
            assert_eq!(
                result.borrow().state.fade_drag.map(|drag| drag.value),
                Some(10)
            );
            if escape {
                testing::key(&mut ui, KeyCode::Escape, build);
            } else {
                testing::frame(&mut ui, &[Event::new(Event::POINTER_CANCELLED)], build);
            }
            testing::frame(
                &mut ui,
                &[Event::pointer(Event::POINTER_RELEASED, 0, target, y)],
                build,
            );
            assert!(result.borrow().toolbar.is_none());
            assert!(result.borrow().state.fade_drag.is_none());
            assert_eq!(store.borrow().project().rev, 1);
            assert_eq!(read_fades(), [100, 20]);
        }
        store.into_inner().close().unwrap();
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn trim_grips_commit_deltas_and_cancel_without_moving() {
        use pxui::{Event, Role, testing};
        use splice_model::Clip;
        use std::cell::RefCell;

        let id = ClipId::from_serial(1);
        let mut data = ClipData::default();
        data.appearance.name = Some("Trimmable".into());
        let mut project = Project::default();
        project
            .clips
            .insert(Clip {
                id,
                timing: ClipTiming {
                    start: 30,
                    length: 120,
                    source: 0,
                    speed: 1.0,
                },
                data,
            })
            .unwrap();
        let view = View {
            width: 320.0,
            height: 240.0,
            at: 0,
            end: 300,
            dragging: false,
            history: (false, false),
            state: State {
                free_snap: true,
                free_move: true,
                ..State::default()
            },
        };
        let result = RefCell::new(Intents {
            state: view.state.clone(),
            ..Intents::default()
        });
        let mut cache = Cache::default();
        let build = &mut |ui: &mut Ui| {
            let mut intents = result.borrow().clone();
            intents.merge(paint(
                ui,
                Some(&project),
                View {
                    state: result.borrow().state.clone(),
                    ..view.clone()
                },
                &mut cache,
                (&[], None),
            ));
            *result.borrow_mut() = intents;
        };
        let mut ui = pxui::doc::frames(2, build);
        let clip = testing::find(&ui, Role::Button, "Trimmable").unwrap();
        let scale = Geometry::new(view.width, project.fps, view.end, &view.state).scale;
        let y = f64::from(clip.y + clip.h * 0.5);
        let leading = f64::from(clip.x + 3.0);
        let trailing = f64::from(clip.x + clip.w - 3.0);
        for (x, edge, frames) in [(leading, "in", 20), (trailing, "out", -16)] {
            let target = x + f64::from(frames as f32 * scale);
            clear_actions(&result);
            testing::frame(
                &mut ui,
                &[Event::pointer(Event::POINTER_PRESSED, 0, x, y)],
                build,
            );
            clear_actions(&result);
            testing::frame(
                &mut ui,
                &[Event::pointer(Event::POINTER_MOVED, 0, target, y)],
                build,
            );
            assert_eq!(result.borrow().trimmed, None);
            clear_actions(&result);
            testing::frame(
                &mut ui,
                &[Event::pointer(Event::POINTER_RELEASED, 0, target, y)],
                build,
            );
            assert_eq!(
                result.borrow().trimmed,
                Some((TimelineId::Clip(id), edge, frames, false))
            );
            assert_eq!(result.borrow().moved, None);
        }
        clear_actions(&result);
        testing::frame(
            &mut ui,
            &[Event::pointer(Event::POINTER_PRESSED, 0, trailing, y)],
            build,
        );
        clear_actions(&result);
        testing::frame(
            &mut ui,
            &[Event::pointer(Event::POINTER_MOVED, 0, trailing - 12.0, y)],
            build,
        );
        clear_actions(&result);
        testing::frame(
            &mut ui,
            &[Event {
                kind: Event::FOCUS_LOST,
                ..Event::default()
            }],
            build,
        );
        assert_eq!(result.borrow().trimmed, None);
        assert!(result.borrow().state.drag.is_none());
    }
    #[test]
    fn timeline_block_handles_pointer_drag_and_cancel() {
        use pxui::{Event, Role, testing};
        use splice_model::Clip;
        use std::cell::RefCell;

        let id = ClipId::from_serial(1);
        let mut data = ClipData::default();
        data.appearance.name = Some("Draggable".into());
        let mut project = Project::default();
        project
            .clips
            .insert(Clip {
                id,
                timing: ClipTiming {
                    start: 30,
                    length: 120,
                    source: 0,
                    speed: 1.0,
                },
                data,
            })
            .unwrap();
        let view = View {
            width: 320.0,
            height: 240.0,
            at: 0,
            end: 300,
            dragging: false,
            history: (false, false),
            state: State {
                free_snap: true,
                ..State::default()
            },
        };
        let result = RefCell::new(Intents {
            state: view.state.clone(),
            ..Intents::default()
        });
        let mut cache = Cache::default();
        let build = &mut |ui: &mut Ui| {
            let mut intents = result.borrow().clone();
            intents.merge(paint(
                ui,
                Some(&project),
                View {
                    state: result.borrow().state.clone(),
                    ..view.clone()
                },
                &mut cache,
                (&[], None),
            ));
            *result.borrow_mut() = intents;
        };
        let mut ui = pxui::doc::frames(2, build);
        let clip = testing::find(&ui, Role::Button, "Draggable").unwrap();
        let start = f64::from(clip.x + clip.w * 0.25);
        let y = f64::from(clip.y + clip.h * 0.5);
        let target = start
            + f64::from(40.0 * Geometry::new(view.width, project.fps, view.end, &view.state).scale);
        let press = Event::pointer(Event::POINTER_PRESSED, 0, start, y);
        let motion = Event::pointer(Event::POINTER_MOVED, 0, target, y);
        let release = Event::pointer(Event::POINTER_RELEASED, 0, target, y);
        clear_actions(&result);
        testing::frame(&mut ui, &[press], build);
        clear_actions(&result);
        testing::frame(&mut ui, &[motion], build);
        assert_eq!(result.borrow().state.drag.map(|drag| drag.to), Some(70));
        assert_eq!(result.borrow().moved, None);
        clear_actions(&result);
        testing::frame(&mut ui, &[release], build);
        assert_eq!(
            result.borrow().moved,
            Some(MoveIntent {
                item: TimelineId::Clip(id),
                to: Some(70),
                ripple: true,
                lane: None,
                carry_overlays: true
            })
        );

        clear_actions(&result);

        testing::frame(&mut ui, &[press], build);
        clear_actions(&result);
        testing::frame(&mut ui, &[motion], build);
        clear_actions(&result);
        testing::frame(
            &mut ui,
            &[Event {
                kind: Event::FOCUS_LOST,
                ..Event::default()
            }],
            build,
        );
        assert_eq!(result.borrow().moved, None);
        assert!(result.borrow().state.drag.is_none());
    }
    #[test]
    fn overlay_drag_moves_to_a_compatible_scrolled_lane_without_allocating_when_warm() {
        use pxui::{Event, testing};
        use splice_model::{Anchor, Item, ItemData, ItemId, ItemTiming, Lane};
        use std::cell::RefCell;

        let mut project = Project::default();
        let source = LaneId::from_serial(4);
        let destination = LaneId::from_serial(3);
        for (serial, name, kind) in [
            (1, "Top", LaneKind::Graphics),
            (2, "Spacer", LaneKind::Graphics),
            (3, "Destination", LaneKind::Graphics),
            (4, "Source", LaneKind::Graphics),
            (5, "Audio", LaneKind::Audio),
            (6, "Bottom", LaneKind::Audio),
        ] {
            project.lanes.push(Lane {
                id: LaneId::from_serial(serial),
                name: name.into(),
                kind,
                effects: Vec::new(),
            });
        }
        let id = ItemId::from_serial(1);
        project
            .items
            .insert(Item {
                id,
                timing: ItemTiming {
                    anchor: Anchor::Pinned(30),
                    length: 90,
                },
                data: ItemData {
                    lane: source,
                    kind: ItemKind::Title {
                        text: "Draggable title".into(),
                    },
                    appearance: Default::default(),
                },
            })
            .unwrap();
        let view = View {
            width: 300.0,
            height: 216.0,
            at: 0,
            end: 150,
            dragging: false,
            history: (false, false),
            state: State::default(),
        };
        let geometry = Geometry::new(view.width, project.fps, view.end, &view.state);
        let result = RefCell::new(Intents::default());
        let mut cache = Cache::default();
        let build = &mut |ui: &mut Ui| {
            ui.stack(
                Node::DEFAULT
                    .at(10.0, 12.0)
                    .w_px(view.width)
                    .h_px(view.height),
                |ui| {
                    let mut intents = result.borrow().clone();
                    intents.merge(paint(
                        ui,
                        Some(&project),
                        View {
                            state: intents.state.clone(),
                            ..view.clone()
                        },
                        &mut cache,
                        (&[], None),
                    ));
                    *result.borrow_mut() = intents;
                },
            );
        };
        let mut ui = pxui::doc::frames(2, build);
        let (scroll, viewport) = ui
            .a11y_nodes()
            .iter()
            .find(|entry| entry.is_role(Role::ScrollArea))
            .map(|entry| (ui.box_of_a11y(entry.id).unwrap(), entry.rect))
            .unwrap();
        let offset = 2.0 * LANE_HEIGHT + 6.0;
        ui.set_scroll_offset(&scroll, offset);
        testing::frame(&mut ui, &[], build);
        assert_eq!(ui.scroll(&scroll).painted, offset);

        let title = testing::find(&ui, Role::Button, "Draggable title").unwrap();
        let (x, y) = (
            f64::from(title.x + title.w * 0.5),
            f64::from(title.y + title.h * 0.5),
        );
        let target_y = f64::from(viewport.y + 4.0);
        let audio_y = target_y + f64::from(2.0 * LANE_HEIGHT);
        let press = Event::pointer(Event::POINTER_PRESSED, 0, x, y);
        let tone = lane_tone(LaneKind::Graphics);
        let target_fill = [tone.r, tone.g, tone.b, 24];

        mem_sys::set_peer(pxui::library_counter);
        for (dx, to) in [(0.0, None), (24.0, Some(54))] {
            clear_actions(&result);
            testing::frame(&mut ui, &[press], build);
            let wrong_lane = Event::pointer(Event::POINTER_MOVED, 0, x, audio_y);
            testing::frame(&mut ui, &[wrong_lane], build);
            assert!(
                result
                    .borrow()
                    .state
                    .drag
                    .is_some_and(|drag| drag.lane.is_none())
            );
            let header = Event::pointer(
                Event::POINTER_MOVED,
                0,
                f64::from(viewport.x + 8.0),
                target_y,
            );
            testing::frame(&mut ui, &[header], build);
            assert!(
                result
                    .borrow()
                    .state
                    .drag
                    .is_some_and(|drag| drag.lane.is_none())
            );

            let target_x = x + f64::from(dx * geometry.scale);
            let motion = Event::pointer(Event::POINTER_MOVED, 0, target_x, target_y);
            testing::frame(&mut ui, &[motion], build);
            assert_eq!(
                result.borrow().state.drag.and_then(|drag| drag.lane),
                Some(destination)
            );
            let highlight = ui
                .layers()
                .flat_map(|layer| layer.instances)
                .find(|instance| instance.color == target_fill)
                .unwrap();
            assert_eq!(
                highlight.rect,
                [
                    viewport.x + geometry.labels,
                    viewport.y,
                    viewport.x + viewport.w,
                    viewport.y + LANE_HEIGHT - 6.0,
                ],
            );
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    for _ in 0..8 {
                        testing::frame(&mut ui, &[motion], build);
                    }
                });
            assert_eq!(result.borrow().moved, None);

            let release = Event::pointer(Event::POINTER_RELEASED, 0, target_x, target_y);
            testing::frame(&mut ui, &[release], build);
            assert_eq!(
                result.borrow().moved,
                Some(MoveIntent {
                    item: TimelineId::Item(id),
                    to,
                    ripple: true,
                    lane: Some(destination),
                    carry_overlays: true,
                }),
            );
            assert!(result.borrow().state.drag.is_none());
        }
    }
    #[test]
    fn move_drag_snaps_position_and_commits_only_on_release() {
        let id = TimelineId::Clip(ClipId::from_serial(1));
        let mut cache = Cache::default();
        cache.add_snap_span(id, 30, 24);
        cache.add_snap_span(TimelineId::Clip(ClipId::from_serial(2)), 100, 20);
        cache.snap_points.sort_unstable_by_key(|point| point.frame);

        let mut intents = Intents::default();
        let press = Signal {
            pressed: true,
            held: true,
            button: Some(Signal::BUTTON_PRIMARY),
            pointer: (90.0, 0.0),
            press_point: (90.0, 0.0),
            ..Signal::default()
        };
        assert!(!move_drag(
            &press,
            id,
            1,
            30,
            24,
            DragEdge::Move,
            2.0,
            0,
            &cache,
            DragOptions::default(),
            &mut intents
        ));
        let motion = Signal {
            held: true,
            pointer: (180.0, 0.0),
            ..Signal::default()
        };
        assert!(move_drag(
            &motion,
            id,
            1,
            30,
            24,
            DragEdge::Move,
            2.0,
            0,
            &cache,
            DragOptions::default(),
            &mut intents
        ));
        assert_eq!(
            intents.state.drag.map(|drag| (drag.to, drag.snap)),
            Some((76, Some(100)))
        );
        assert_eq!(intents.moved, None);
        let release = Signal {
            released: true,
            release_point: (180.0, 0.0),
            ..Signal::default()
        };
        assert!(move_drag(
            &release,
            id,
            1,
            30,
            24,
            DragEdge::Move,
            2.0,
            0,
            &cache,
            DragOptions::default(),
            &mut intents
        ));
        assert_eq!(
            intents.moved,
            Some(MoveIntent {
                item: id,
                to: Some(76),
                ripple: true,
                lane: None,
                carry_overlays: true
            })
        );
        assert!(intents.state.drag.is_none());

        let mut intents = Intents::default();
        assert!(!move_drag(
            &press,
            id,
            1,
            30,
            24,
            DragEdge::Move,
            2.0,
            0,
            &cache,
            DragOptions::default(),
            &mut intents
        ));
        assert!(move_drag(
            &motion,
            id,
            1,
            30,
            24,
            DragEdge::Move,
            2.0,
            0,
            &cache,
            DragOptions::default(),
            &mut intents
        ));
        assert!(move_drag(
            &release,
            id,
            2,
            30,
            24,
            DragEdge::Move,
            2.0,
            0,
            &cache,
            DragOptions::default(),
            &mut intents
        ));
        assert_eq!(intents.moved, None);
        assert!(intents.state.drag.is_none());
    }
    #[test]
    fn fit_and_zoom_use_one_clock_for_ruler_and_clips() {
        let mut state = State::default();
        let fit = Geometry::new(936.0, 30, 1000, &state);
        assert_eq!(fit.labels, LABEL_WIDTH);
        assert!((fit.x(1000.0) / fit.width - 1.0 / 1.04).abs() < 0.001);
        assert!(fit.ticks(30) as f32 * fit.scale >= 70.0);
        fit.zoom(&mut state, fit.scale * 2.0, 450);
        let zoom = Geometry::new(936.0, 30, 1000, &state);
        assert!((zoom.x(450.0) - fit.x(450.0)).abs() < 0.01);
        assert_eq!(zoom.visible(0, 100), None);
        let (left, _) = zoom.visible(450, 50).unwrap();
        assert!((left - LABEL_WIDTH - zoom.x(450.0)).abs() < 0.01);
        assert_eq!(blade_frame(zoom, left + 10.0 * zoom.scale), 460);
        assert!(!inside(450, 50, 450));
        assert!(inside(450, 50, 460));
    }

    #[test]
    fn long_project_fit_keeps_major_ticks_spaced() {
        let fit = Geometry::new(936.0, 30, i32::MAX, &State::default());
        assert!(fit.ticks(30) as f32 * fit.scale >= 70.0);
    }

    #[test]
    fn passive_pointer_guides_follow_tools_and_disappear_during_input() {
        use pxui::{Event, draw::kind, testing};
        use splice_model::Clip;
        use std::cell::RefCell;

        let mut project = Project::default();
        let id = ClipId::from_serial(1);
        project
            .clips
            .insert(Clip {
                id,
                timing: ClipTiming {
                    start: 0,
                    length: 90,
                    source: 0,
                    speed: 1.0,
                },
                data: ClipData::default(),
            })
            .unwrap();
        let view = View {
            width: 320.0,
            height: 240.0,
            at: 40,
            end: 150,
            dragging: false,
            history: (false, false),
            state: State {
                scale: 2.0,
                ..State::default()
            },
        };
        let geo = Geometry::new(view.width, project.fps, view.end, &view.state);
        let state = RefCell::new(view.state.clone());
        let output = RefCell::new(Intents::default());
        let mut cache = Cache::default();
        let build = &mut |ui: &mut Ui| {
            let next = paint(
                ui,
                Some(&project),
                View {
                    state: state.borrow().clone(),
                    ..view.clone()
                },
                &mut cache,
                (&[], None),
            );
            *state.borrow_mut() = next.state.clone();
            *output.borrow_mut() = next;
        };
        let mut ui = pxui::doc::frames(2, build);
        let clip = testing::find(&ui, Role::Button, "Gap").unwrap();
        let x = f64::from(clip.x + geo.x(30.0));
        let y = f64::from(clip.y + clip.h * 0.5);
        let blank = f64::from(clip.x + geo.x(100.0));
        let motion = |x, y| Event::pointer(Event::POINTER_MOVED, 0, x, y);
        let skim = |ui: &Ui| {
            ui.layers()
                .flat_map(|layer| layer.instances)
                .find(|shape| shape.kind == kind::RECT && shape.color == [INK.r, INK.g, INK.b, 77])
                .map(|shape| shape.rect)
        };
        let blade = |ui: &Ui| {
            ui.layers()
                .flat_map(|layer| layer.instances)
                .find(|shape| {
                    shape.kind == kind::RECT
                        && shape.color == [INK.r, INK.g, INK.b, 255]
                        && shape.rect[0] == x as f32
                        && shape.rect[1] >= clip.y - 2.0
                        && shape.rect[3] - shape.rect[1] <= 4.0
                })
                .map(|shape| shape.rect)
        };

        testing::frame(&mut ui, &[motion(x, y)], build);
        assert_eq!(skim(&ui).unwrap()[0], x as f32);
        assert!(blade(&ui).is_none());
        assert!(output.borrow().split.is_none());
        state.borrow_mut().blade = true;
        testing::frame(&mut ui, &[motion(x, y)], build);
        assert_eq!(output.borrow().blade_hover.map(|hover| hover.0), Some(30));
        assert!(skim(&ui).is_some());
        assert_eq!(blade(&ui).unwrap()[1], clip.y - 2.0);
        assert!(output.borrow().split.is_none());
        testing::frame(&mut ui, &[motion(blank, y)], build);
        assert_eq!(skim(&ui).unwrap()[0], blank as f32);
        assert!(blade(&ui).is_none());
        testing::frame(&mut ui, &[motion(clip.x as f64 - 4.0, y)], build);
        assert!(skim(&ui).is_none());
        testing::frame(&mut ui, &[motion(x, 0.0)], build);
        assert!(skim(&ui).is_none());

        state.borrow_mut().blade = false;
        testing::frame(
            &mut ui,
            &[Event::pointer(Event::POINTER_PRESSED, 0, x, y)],
            build,
        );
        testing::frame(&mut ui, &[motion(x + 12.0, y)], build);
        assert!(state.borrow().drag.is_some());
        assert!(skim(&ui).is_none());
        testing::frame(
            &mut ui,
            &[Event::pointer(Event::POINTER_RELEASED, 0, x + 12.0, y)],
            build,
        );
        testing::frame(
            &mut ui,
            &[Event::pointer(Event::POINTER_PRESSED, 1, x, y)],
            build,
        );
        assert!(state.borrow().menu_open());
        assert!(skim(&ui).is_none());
        state.borrow_mut().context = None;
        testing::frame(
            &mut ui,
            &[Event::pointer(Event::POINTER_RELEASED, 1, x, y)],
            build,
        );
        testing::frame(&mut ui, &[motion(blank, y)], build);
        assert!(skim(&ui).is_some());

        mem_sys::set_peer(pxui::library_counter);
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                for delta in [0.0, 4.0, 8.0, 12.0] {
                    testing::frame(&mut ui, &[motion(blank + delta, y)], build);
                }
            },
        );
    }

    #[test]
    fn blade_click_uses_frame_under_pointer_and_emits_one_split() {
        use pxui::{Role, testing};
        use splice_model::{Clip, ClipData, ClipId, ClipTiming};
        use std::cell::RefCell;

        let id = ClipId::from_serial(1);
        let mut project = Project::default();
        project
            .clips
            .insert(Clip {
                id,
                timing: ClipTiming {
                    start: 0,
                    length: 300,
                    source: 0,
                    speed: 1.0,
                },
                data: ClipData::default(),
            })
            .unwrap();
        let view = View {
            width: 320.0,
            height: 220.0,
            at: 36,
            end: 300,
            dragging: false,
            history: (false, false),
            state: State::default(),
        };
        let result = RefCell::new(Intents {
            state: view.state.clone(),
            ..Intents::default()
        });
        let mut cache = Cache::default();
        let build = &mut |ui: &mut Ui| {
            let mut intents = result.borrow().clone();
            intents.merge(paint(
                ui,
                Some(&project),
                View {
                    state: result.borrow().state.clone(),
                    ..view.clone()
                },
                &mut cache,
                (&[], None),
            ));
            *result.borrow_mut() = intents;
        };
        let mut ui = pxui::doc::frames(2, build);
        let blade = testing::find(&ui, Role::Button, "Blade tool").unwrap();
        clear_actions(&result);
        testing::click(
            &mut ui,
            f64::from(blade.x + blade.w * 0.5),
            f64::from(blade.y + blade.h * 0.5),
            build,
        );
        assert!(result.borrow().state.blade);
        let gap = testing::find(&ui, Role::Button, "Gap").unwrap();
        let geo = Geometry::new(view.width, project.fps, view.end, &view.state);
        clear_actions(&result);
        testing::click(
            &mut ui,
            f64::from(gap.x + geo.x(120.0)),
            f64::from(gap.y + gap.h * 0.5),
            build,
        );
        assert_eq!(result.borrow().split, Some((TimelineId::Clip(id), 120)));
        clear_actions(&result);
        testing::frame(&mut ui, &[], build);
        assert_eq!(result.borrow().split, None);
    }
    struct LaneMenuUi {
        ui: Ui,
        cache: Cache,
        view: View,
        tick: f64,
    }

    impl LaneMenuUi {
        fn new(width: f32) -> Self {
            let mut ui = crate::viewer_ui().unwrap();
            ui.set_a11y(true);
            Self {
                ui,
                cache: Cache::default(),
                view: View {
                    width,
                    height: 420.0,
                    at: 192,
                    end: 600,
                    dragging: false,
                    history: (false, false),
                    state: State::default(),
                },
                tick: 1.0,
            }
        }

        fn frame(&mut self, project: &Project, events: &[pxui::Event]) -> Intents {
            self.frame_with_drawer(project, events, false)
        }

        fn frame_with_drawer(
            &mut self,
            project: &Project,
            events: &[pxui::Event],
            drawer: bool,
        ) -> Intents {
            let mut intents = Intents::default();
            let input = pxui::FrameIn::new(self.view.width, 640.0)
                .time(self.tick)
                .events(events);
            self.tick += 0.016;
            self.ui
                .frame(&input, &mut |ui| {
                    if drawer {
                        viewer_drawers::paint(
                            ui,
                            viewer_drawers::Drawer::Effects,
                            (Some(project), None),
                            [self.view.width, 640.0],
                            Point::new(12.0, 210.0),
                            viewer_drawers::EffectTargets::default(),
                        );
                    }
                    intents.merge(paint(
                        ui,
                        Some(project),
                        self.view.clone(),
                        &mut self.cache,
                        (&[], None),
                    ));
                })
                .unwrap();
            self.view.state = intents.state.clone();
            intents
        }

        fn find(&self, role: Role, text: &str) -> Rect {
            pxui::testing::find(&self.ui, role, text).unwrap_or_else(|| panic!("missing {text}"))
        }

        fn button(&self, lane: &str) -> Rect {
            let head = self.find(Role::Button, lane);
            self.ui
                .a11y_nodes()
                .iter()
                .find(|node| {
                    node.is_role(Role::Button)
                        && self.ui.a11y_name_of(node) == "Lane menu"
                        && node.rect.y >= head.y
                        && node.rect.y < head.y + head.h
                })
                .unwrap()
                .rect
        }

        fn click(&mut self, project: &Project, rect: Rect, button: u32) -> Intents {
            let x = f64::from(rect.x + rect.w * 0.5);
            let y = f64::from(rect.y + rect.h * 0.5);
            self.frame(
                project,
                &[pxui::Event::pointer(
                    pxui::Event::POINTER_PRESSED,
                    button,
                    x,
                    y,
                )],
            );
            let release = self.frame(
                project,
                &[pxui::Event::pointer(
                    pxui::Event::POINTER_RELEASED,
                    button,
                    x,
                    y,
                )],
            );
            self.frame(project, &[]);
            release
        }

        fn choose(&mut self, project: &Project, row: &str) -> Intents {
            self.click(project, self.find(Role::MenuItem, row), 0)
        }

        fn key(&mut self, project: &Project, key: Key) -> Intents {
            let mut events = pxui_host::Events::default();
            self.frame(
                project,
                events.encode(&[
                    input::Event::KeyPressed(key),
                    input::Event::KeyReleased(key),
                ]),
            )
        }

        fn type_text(&mut self, project: &Project, text: &'static str) -> Intents {
            self.frame(
                project,
                &[pxui::Event {
                    kind: pxui::Event::TEXT_INPUT,
                    text: pxui::Str::from(text),
                    ..Default::default()
                }],
            )
        }

        fn warm(&mut self, project: &Project) {
            for _ in 0..8 {
                self.frame(project, &[]);
            }
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    for _ in 0..8 {
                        self.frame(project, &[]);
                    }
                });
        }
    }

    #[test]
    fn lane_header_menu_matches_v13_and_opens_the_lane_in_selection_scope() {
        use crate::inspector::{self, Scope, Selection};
        use pxui::{Event, FrameIn, testing};
        use splice_core::{
            edit,
            history::{Store, WriteOptions},
            package::journal::Source,
        };
        use splice_json::Value as Json;

        mem_sys::set_peer(pxui::library_counter);
        for width in [960.0, 800.0, 480.0] {
            let mut project = crate::editor_views::load_sample().unwrap();
            let lane = project
                .lanes
                .iter()
                .find(|lane| lane.name == "Overlay 1")
                .unwrap()
                .id;
            let caption = project
                .lanes
                .iter()
                .find(|lane| lane.kind == LaneKind::Captions)
                .unwrap()
                .id;
            let mut pane = LaneMenuUi::new(width);
            let group: Vec<_> = project.clips.order()[..2]
                .iter()
                .filter_map(|&slot| project.clips.id(slot).map(TimelineId::Clip))
                .collect();
            pane.view.state.set_selected(group.clone());
            pane.frame(&project, &[]);
            pane.frame(&project, &[]);
            let head = pane.find(Role::Button, "Overlay 1");
            let button = pane.button("Overlay 1");
            let x = if width == 480.0 { 73.0 } else { 131.0 };
            assert_eq!(
                (button.x - head.x, button.y - head.y, button.w, button.h),
                (x, 3.5, 18.0, 18.0)
            );
            pane.warm(&project);
            println!(
                "LANE-MENU|{width}|offset {:?}|{}x{}|warm zero allocations",
                (button.x - head.x, button.y - head.y),
                button.w,
                button.h
            );

            pane.click(&project, button, 0);
            assert_eq!(pane.view.state.selected(), group);
            assert!(pane.view.state.menu_open());
            let entries: Vec<_> = pane
                .ui
                .a11y_nodes()
                .iter()
                .filter(|node| node.is_role(Role::MenuItem))
                .map(|node| pane.ui.a11y_name_of(node))
                .collect();
            assert_eq!(
                entries,
                [
                    "Lane Effects",
                    "Edit Lane Effects…",
                    "New Overlay Lane",
                    "Rename…",
                    "Select All in Lane",
                    "Delete Lane",
                ]
            );
            pane.warm(&project);

            let mut inspector_ui = crate::viewer_ui().unwrap();
            inspector_ui.set_a11y(true);
            let input = FrameIn::new(268.0, 640.0);
            let draw_inspector = |ui: &mut Ui, project: &Project, scope, selection| {
                ui.frame(&input, &mut |ui| {
                    inspector::paint(
                        ui,
                        Some(project),
                        selection,
                        scope,
                        600,
                        (268.0, false),
                        (&[], true, None, None, None),
                    );
                })
                .unwrap();
            };
            let mut scope = Scope::Canvas;
            draw_inspector(&mut inspector_ui, &project, scope, Selection::None);
            assert!(testing::find(&inspector_ui, Role::TextField, "Lane name").is_none());
            let intent = pane.choose(&project, "Edit Lane Effects…");
            if intent.inspect_selection {
                scope = Scope::Selection;
            }
            assert_eq!(scope, Scope::Selection);
            assert_eq!(pane.view.state.lane(), Some(lane));
            assert!(pane.view.state.selected().is_empty() && intent.toolbar.is_none());
            draw_inspector(&mut inspector_ui, &project, scope, Selection::Lane(lane));
            assert!(testing::find(&inspector_ui, Role::Heading, "Overlay 1").is_some());

            pane.click(&project, pane.button("Overlay 1"), 0);
            let submenu = pane.find(Role::MenuItem, "Lane Effects");
            pane.frame(
                &project,
                &[Event::pointer(
                    Event::POINTER_MOVED,
                    0,
                    f64::from(submenu.x + 10.0),
                    f64::from(submenu.y + submenu.h * 0.5),
                )],
            );
            pane.frame(&project, &[]);
            let add = pane
                .choose(&project, "Add Window")
                .toolbar
                .unwrap()
                .operation();
            assert_eq!(
                add.field("cmd").and_then(splice_json::Value::as_str),
                Some("effect.add")
            );
            assert_eq!(
                add.field("target").and_then(splice_json::Value::as_str),
                Some("lan_ov1")
            );
            edit::apply(&mut project, &[add], Source::Human, false).unwrap();
            project.rev += 1;
            assert!(
                project
                    .lanes
                    .iter()
                    .find(|l| l.id == lane)
                    .unwrap()
                    .effects
                    .iter()
                    .any(|fx| fx.kind == "window")
            );
            pane.frame(&project, &[]);

            if width == 480.0 {
                // Keyboard activation of the overflow button, plus the unchanged right-click route.
                for _ in 0..60 {
                    pane.key(&project, Key::Tab);
                    if pane.ui.a11y_nodes().iter().any(|node| {
                        node.rect == pane.button("Captions")
                            && node.states & pxui::A11yStates::FOCUSED.bits() != 0
                    }) {
                        break;
                    }
                }
                pane.key(&project, Key::Enter);
                pane.frame(&project, &[]);
                assert!(pane.view.state.menu_open());
                assert_eq!(pane.find(Role::MenuItem, "New Captions Lane").h, 22.0);
                let edit = pane.choose(&project, "Edit Lane Effects…");
                assert!(edit.inspect_selection && edit.state.lane() == Some(caption));
                pane.click(&project, pane.button("Captions"), 0);
                let submenu = pane.find(Role::MenuItem, "Lane Effects");
                let pos = Event::pointer(
                    Event::POINTER_MOVED,
                    0,
                    f64::from(submenu.x + 10.0),
                    f64::from(submenu.y + submenu.h * 0.5),
                );
                pane.frame(&project, &[pos]);
                pane.frame(&project, &[]);
                let add = pane
                    .choose(&project, "Add Caption Style")
                    .toolbar
                    .unwrap()
                    .operation();
                assert_eq!(
                    add.field("target").and_then(splice_json::Value::as_str),
                    Some("lan_cap")
                );
                assert_eq!(
                    add.field("type").and_then(splice_json::Value::as_str),
                    Some("captions")
                );
                edit::apply(&mut project, &[add], Source::Human, false).unwrap();
                project.rev += 1;

                pane.click(&project, pane.find(Role::Button, "Video · spine"), 1);
                pane.find(Role::MenuItem, "Select All Clips");
                pane.find(Role::MenuItem, "Show All Components");
                pane.key(&project, Key::Escape);
                pane.frame(&project, &[]);
                let click = pane.click(&project, pane.find(Role::Button, "Graphics"), 0);
                assert!(click.state.lane().is_some() && !click.inspect_selection);
                let right = pane.click(&project, pane.find(Role::Button, "Graphics"), 1);
                assert_eq!(right.state.lane(), click.state.lane());
                pane.find(Role::MenuItem, "New Graphics Lane");
                assert!(testing::find(&pane.ui, Role::MenuItem, "Lane Effects").is_none());
                pane.key(&project, Key::Escape);
            }
            if width == 800.0 {
                continue;
            }

            let path = std::env::temp_dir().join(format!(
                "splice-spine-menu-{}-{width}.splice",
                std::process::id(),
            ));
            let mut store = Store::create(&path, &project).unwrap();
            let project = store.project();
            let spine = project.spine.id;
            let open_spine = |pane: &mut LaneMenuUi, project: &Project| {
                pane.click(project, pane.find(Role::Button, "Video · spine"), 1);
            };
            open_spine(&mut pane, project);
            let rows = pane
                .ui
                .a11y_nodes()
                .iter()
                .filter(|node| node.is_role(Role::MenuItem))
                .map(|node| pane.ui.a11y_name_of(node))
                .collect::<Vec<_>>();
            assert_eq!(
                rows,
                [
                    "Show All Components",
                    "Select All Clips",
                    "Lane Effects",
                    "Edit Lane Effects…",
                    "New Overlay Lane",
                ]
            );
            pane.warm(project);
            assert!(
                pane.choose(project, "Show All Components")
                    .state
                    .all_components
            );
            open_spine(&mut pane, project);
            assert!(
                !pane
                    .choose(project, "Hide All Components")
                    .state
                    .all_components
            );
            open_spine(&mut pane, project);
            assert_eq!(
                pane.choose(project, "Select All Clips")
                    .state
                    .selected()
                    .len(),
                project.clips.order().len()
            );

            open_spine(&mut pane, project);
            let inspected = pane.choose(project, "Edit Lane Effects…");
            assert!(inspected.inspect_selection && inspected.toolbar.is_none());
            assert_eq!(inspected.state.lane(), Some(spine));
            assert!(inspected.state.selected().is_empty());
            draw_inspector(
                &mut inspector_ui,
                project,
                Scope::Selection,
                Selection::Lane(inspected.state.lane().unwrap()),
            );
            assert!(testing::find(&inspector_ui, Role::Heading, "Video").is_some());

            let open_effects = |pane: &mut LaneMenuUi, project: &Project| {
                open_spine(pane, project);
                let row = pane.find(Role::MenuItem, "Lane Effects");
                pane.frame(
                    project,
                    &[Event::pointer(
                        Event::POINTER_MOVED,
                        0,
                        f64::from(row.x + 10.0),
                        f64::from(row.y + row.h * 0.5),
                    )],
                );
                pane.frame(project, &[]);
            };
            let cursor_checked = |pane: &LaneMenuUi| {
                pane.ui
                    .a11y_nodes()
                    .iter()
                    .find(|node| {
                        node.is_role(Role::MenuItem) && pane.ui.a11y_name_of(node) == "Add Cursor"
                    })
                    .unwrap()
                    .states
                    & pxui::A11yStates::CHECKED.bits()
                    != 0
            };
            assert!(project.effects.iter().any(|effect| effect.kind == "cursor"));
            open_effects(&mut pane, project);
            let rows = pane
                .ui
                .a11y_nodes()
                .iter()
                .filter(|node| node.is_role(Role::MenuItem))
                .map(|node| pane.ui.a11y_name_of(node))
                .filter(|label| label.starts_with("Add "))
                .collect::<Vec<_>>();
            assert_eq!(
                rows,
                [
                    "Add Window",
                    "Add Cursor",
                    "Add Camera Layout",
                    "Add Keystroke Chips"
                ]
            );
            assert!(!cursor_checked(&pane));
            pane.warm(project);
            let op = pane
                .choose(project, "Add Cursor")
                .toolbar
                .unwrap()
                .operation();
            assert_eq!(op.field("cmd").and_then(Json::as_str), Some("effect.add"));
            assert_eq!(op.field("target").and_then(Json::as_str), Some("lan_spine"));
            assert_eq!(op.field("type").and_then(Json::as_str), Some("cursor"));
            store.apply(&[op], &WriteOptions::default()).unwrap();
            store.close().unwrap();
            let reopened = Store::open(&path).unwrap();
            assert!(reopened.verify().is_ok());
            assert!(
                reopened
                    .project()
                    .spine
                    .effects
                    .iter()
                    .any(|effect| effect.kind == "cursor")
            );
            pane.frame(reopened.project(), &[]);
            open_effects(&mut pane, reopened.project());
            assert!(cursor_checked(&pane));
            pane.warm(reopened.project());
            reopened.close().unwrap();
            std::fs::remove_dir_all(path).unwrap();
        }
    }

    #[test]
    fn lane_header_rename_commits_once_and_escape_restores_the_name() {
        use pxui::{Event, testing};
        use splice_core::history::{Store, WriteOptions};

        mem_sys::set_peer(pxui::library_counter);
        for width in [960.0, 800.0, 480.0] {
            let path = std::env::temp_dir().join(format!(
                "splice-lane-header-{}-{width}.splice",
                std::process::id(),
            ));
            let sample = crate::editor_views::load_sample().unwrap();
            let mut store = Store::create(&path, &sample).unwrap();
            let mut pane = LaneMenuUi::new(width);
            pane.frame(store.project(), &[]);
            pane.frame(store.project(), &[]);
            pane.click(store.project(), pane.button("Overlay 2"), 0);
            let opened = pane.choose(store.project(), "Rename…");
            assert!(opened.toolbar.is_none());
            assert!(pane.ui.text_input_focused());
            let field = pane.find(Role::TextField, "Lane name");
            let head = pane.find(Role::Button, "Overlay 2");
            assert_eq!(
                (field.x - head.x, field.y - head.y, field.h),
                (28.0, 3.5, 18.0)
            );
            assert_eq!(field.w, if width == 480.0 { 13.0 } else { 59.0 });
            assert!(field.x + field.w < pane.button("Overlay 2").x);
            pane.warm(store.project());
            println!(
                "LANE-FOCUS|{width}|{}x{}|warm zero allocations",
                field.w, field.h
            );

            let rev = store.project().rev;
            assert!(
                pane.type_text(store.project(), "  B-roll  ")
                    .toolbar
                    .is_none()
            );
            pane.warm(store.project());
            assert_eq!(store.project().rev, rev);
            let commit = pane.key(store.project(), Key::Enter).toolbar.unwrap();
            let op = commit.operation();
            assert_eq!(
                op.field("cmd").and_then(splice_json::Value::as_str),
                Some("lane.rename")
            );
            assert_eq!(
                op.field("lane").and_then(splice_json::Value::as_str),
                Some("lan_ov2")
            );
            assert_eq!(
                op.field("name").and_then(splice_json::Value::as_str),
                Some("B-roll")
            );
            store.apply(&[op], &WriteOptions::default()).unwrap();
            assert_eq!(store.project().rev, rev + 1);
            assert!(pane.frame(store.project(), &[]).toolbar.is_none());
            assert!(testing::find(&pane.ui, Role::TextField, "Lane name").is_none());
            let head = pane.find(Role::Button, "B-roll");

            pane.click(store.project(), head, 0);
            let x = f64::from(head.x + 32.0);
            let y = f64::from(head.y + head.h * 0.5);
            // Both presses must hit the same position for the retained click count.
            pane.tick += 0.5;
            for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
                pane.frame(store.project(), &[Event::pointer(kind, 0, x, y)]);
            }
            pane.frame(
                store.project(),
                &[Event::pointer(Event::POINTER_PRESSED, 0, x, y)],
            );
            assert!(!pane.ui.text_input_focused());
            pane.frame(
                store.project(),
                &[Event::pointer(Event::POINTER_RELEASED, 0, x, y)],
            );
            pane.frame(store.project(), &[]);
            assert!(pane.ui.text_input_focused());
            assert!(
                pane.type_text(store.project(), "Discard me")
                    .toolbar
                    .is_none()
            );
            assert!(pane.key(store.project(), Key::Escape).toolbar.is_none());
            assert!(pane.frame(store.project(), &[]).toolbar.is_none());
            pane.find(Role::Button, "B-roll");
            assert!(!pane.ui.text_input_focused());
            assert_eq!(store.project().rev, rev + 1);

            if width == 480.0 {
                pane.click(store.project(), pane.button("B-roll"), 0);
                pane.choose(store.project(), "Rename…");
                assert!(pane.type_text(store.project(), "  ").toolbar.is_none());
                assert!(pane.key(store.project(), Key::Enter).toolbar.is_none());
                pane.frame(store.project(), &[]);
                assert!(!pane.ui.text_input_focused());

                pane.click(store.project(), pane.button("B-roll"), 0);
                pane.choose(store.project(), "Rename…");
                pane.type_text(store.project(), "Cutaways");
                let graphics = pane.find(Role::Button, "Graphics");
                let blur = pane.frame(
                    store.project(),
                    &[Event::pointer(
                        Event::POINTER_PRESSED,
                        0,
                        f64::from(graphics.x + 32.0),
                        f64::from(graphics.y + graphics.h * 0.5),
                    )],
                );
                let op = blur.toolbar.unwrap().operation();
                assert_eq!(
                    op.field("name").and_then(splice_json::Value::as_str),
                    Some("Cutaways")
                );
                store.apply(&[op], &WriteOptions::default()).unwrap();
                assert_eq!(store.project().rev, rev + 2);
                assert!(pane.frame(store.project(), &[]).toolbar.is_none());
            }
            drop(store);
            std::fs::remove_dir_all(path).unwrap();
        }
    }

    #[test]
    fn lane_header_menu_keeps_the_dots_in_the_effect_drop_target() {
        use pxui::Event;
        use splice_core::{edit, package::journal::Source};

        let mut project = crate::editor_views::load_sample().unwrap();
        let lane = project
            .lanes
            .iter()
            .find(|lane| lane.name == "Overlay 1")
            .unwrap()
            .id;
        let mut pane = LaneMenuUi::new(480.0);
        for _ in 0..3 {
            pane.frame_with_drawer(&project, &[], true);
        }
        let from = pane.find(Role::Button, "Window");
        let target = pane.button("Overlay 1");
        let (x, y) = (
            f64::from(from.x + from.w * 0.5),
            f64::from(from.y + from.h * 0.5),
        );
        let (to_x, to_y) = (f64::from(target.x + 9.0), f64::from(target.y + 9.0));
        for event in [
            Event::pointer(Event::POINTER_PRESSED, 0, x, y),
            Event::pointer(Event::POINTER_MOVED, 0, to_x, to_y),
        ] {
            let preview = pane.frame_with_drawer(&project, &[event], true);
            assert!(preview.effect.is_none() && !preview.state.menu_open());
        }
        assert!(pane.ui.carrying().is_some());
        for _ in 0..8 {
            pane.frame_with_drawer(&project, &[], true);
        }
        mem_sys::set_peer(pxui::library_counter);
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                let preview = pane.frame_with_drawer(&project, &[], true);
                assert!(preview.effect.is_none() && !preview.state.menu_open());
            },
        );
        let release = Event::pointer(Event::POINTER_RELEASED, 0, to_x, to_y);
        let dropped = pane.frame_with_drawer(&project, &[release], true);
        assert!(dropped.toolbar.is_none() && !dropped.state.menu_open());
        let change = dropped
            .effect
            .expect("the lane header owns its menu button's drop area");
        assert_eq!(change.target, Target::Lane(lane));
        let operation = change.operation(&project).unwrap();
        edit::apply(&mut project, &[operation], Source::Human, false).unwrap();
        assert!(
            project
                .lanes
                .iter()
                .find(|row| row.id == lane)
                .unwrap()
                .effects
                .iter()
                .any(|fx| fx.kind == "window")
        );
    }

    #[test]
    fn native_lane_chrome_clips_and_scales_at_desktop_and_compact_widths() {
        use pxui::{FrameIn, UiDesc, draw::kind};
        use splice_model::{Clip, Item, ItemData, ItemId, ItemTiming, Lane, Zoom};

        let mut project = Project::default();
        let lane = LaneId::from_serial(1);
        project.lanes.push(Lane {
            id: lane,
            kind: LaneKind::Camera,
            name: "Motion".into(),
            effects: Vec::new(),
        });
        project
            .items
            .insert(Item {
                id: ItemId::from_serial(1),
                timing: ItemTiming {
                    anchor: Anchor::Pinned(30),
                    length: 100,
                },
                data: ItemData {
                    lane,
                    kind: ItemKind::Zoom(Zoom {
                        amount: 1.16,
                        focus: None,
                        mode: Default::default(),
                        spring: Default::default(),
                        follow: false,
                        motion_blur: 0.0,
                    }),
                    appearance: Default::default(),
                },
            })
            .unwrap();
        let mut data = ClipData {
            media: Some(MediaId::from_serial(1)),
            ..ClipData::default()
        };
        data.appearance.name = Some("Screen take".into());
        project
            .clips
            .insert(Clip {
                id: ClipId::from_serial(1),
                timing: ClipTiming {
                    start: 0,
                    length: 120,
                    source: 0,
                    speed: 1.0,
                },
                data,
            })
            .unwrap();

        mem_sys::set_peer(pxui::library_counter);
        for (width, origin, labels, diamond_halves) in [
            (960.0, 0.0, LABEL_WIDTH, 4),
            (480.0, 45.0, COMPACT_LABEL_WIDTH, 2),
        ] {
            let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
            ui.set_a11y(true);
            let mut cache = Cache::default();
            let view = View {
                width,
                height: 240.0,
                at: 60,
                end: 300,
                dragging: false,
                history: (false, false),
                state: State {
                    scale: 4.0,
                    origin,
                    ..State::default()
                },
            };
            let build = &mut |ui: &mut Ui| {
                paint(ui, Some(&project), view.clone(), &mut cache, (&[], None));
            };
            let input = FrameIn::new(width.into(), 240.0);
            ui.frame(&input, build).unwrap();
            ui.frame(&input, build).unwrap();
            let shapes = || ui.layers().flat_map(|layer| layer.instances);
            let icon = shapes()
                .filter(|shape| shape.kind == kind::LINE && shape.color == [191, 224, 234, 230])
                .collect::<Vec<_>>();
            assert_eq!(icon.len(), 4);
            assert!(icon.iter().all(|shape| shape.clip[2] == labels as i16));
            assert_eq!(
                shapes()
                    .filter(
                        |shape| shape.kind == kind::TRIANGLE && shape.color == [191, 224, 234, 255]
                    )
                    .count(),
                diamond_halves,
            );
            let name = ui
                .a11y_nodes()
                .iter()
                .find(|node| node.is_role(Role::Button) && ui.a11y_name_of(node) == "Motion")
                .unwrap();
            assert_eq!(name.rect.w, labels);
            assert!(
                ui.a11y_nodes()
                    .iter()
                    .any(|node| node.is_role(Role::Label) && ui.a11y_name_of(node) == "4.00s")
            );
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    for _ in 0..8 {
                        ui.frame(&input, build).unwrap();
                    }
                });
        }
    }

    #[test]
    fn narrow_caption_draws_clipped_labels_and_emphasized_word_ticks_when_warm() {
        use pxui::{draw::kind, testing};
        use splice_model::{
            CaptionEdit, Clip, Item, ItemId, Media, MediaKind, Probe, Transcript, Word, WordId,
        };

        let mut project = Project::default();
        let media = MediaId::from_serial(1);
        let clip = ClipId::from_serial(1);
        let lane = LaneId::from_serial(2);
        project.media.push(Media {
            id: media,
            kind: MediaKind::Video,
            path: "caption-source.mov".into(),
            bookmark: None,
            size: 0,
            mtime: 0,
            take: None,
            probe: Probe {
                fps: 30.0,
                frames: 150,
                width: 64,
                height: 64,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
        });
        project
            .clips
            .insert(Clip {
                id: clip,
                timing: ClipTiming {
                    start: 0,
                    length: 150,
                    source: 0,
                    speed: 1.0,
                },
                data: ClipData {
                    media: Some(media),
                    ..ClipData::default()
                },
            })
            .unwrap();
        let words = [20, 27, 30, 34].map(|at| Word {
            id: WordId::new(media, at as u32),
            from: at,
            to: at + 1,
            text: "word".into(),
            confidence: None,
        });
        let ids = words.each_ref().map(|word| word.id);
        project.transcripts.push(Transcript {
            media,
            hash: 1,
            language: "en".into(),
            words: Vec::from(words),
        });
        project.lanes.push(splice_model::Lane {
            id: lane,
            name: "Captions".into(),
            kind: LaneKind::Captions,
            effects: Vec::new(),
        });
        project
            .items
            .insert(Item {
                id: ItemId::from_serial(1),
                timing: splice_model::ItemTiming {
                    anchor: Anchor::ClipSource { clip, source: 20 },
                    length: 14,
                },
                data: ItemData {
                    lane,
                    kind: ItemKind::Caption {
                        words: ids.to_vec(),
                        edits: vec![CaptionEdit {
                            word: ids[2],
                            text: Some("accent".into()),
                            hidden: false,
                            emph: true,
                            emoji: None,
                        }],
                        text: None,
                    },
                    appearance: Default::default(),
                },
            })
            .unwrap();
        let view = View {
            width: 360.0,
            height: 240.0,
            at: 60,
            end: 150,
            dragging: false,
            history: (false, false),
            state: State {
                scale: 2.0,
                origin: 25.0,
                ..State::default()
            },
        };
        let mut cache = Cache::default();
        let build = &mut |ui: &mut Ui| {
            paint(ui, Some(&project), view.clone(), &mut cache, (&[], None));
        };
        let mut ui = pxui::doc::frames(2, build);
        let cue = testing::find(&ui, Role::Button, "word word accent word").unwrap();
        assert_eq!(cue.w, 18.0);
        let shapes = || ui.layers().flat_map(|layer| layer.instances);
        let geometry = Geometry::new(view.width, project.fps, view.end, &view.state);
        let step = geometry.ticks(project.fps) as f32;
        let first = (geometry.origin / step).ceil() * step;
        let x = geometry.labels + geometry.x(first);
        let y = cue.y - 2.0;
        assert!(shapes().any(|shape| shape.kind == kind::RECT
            && shape.color == [255, 255, 255, 10]
            && shape.rect == [x, y, x + 1.0, y + LANE_HEIGHT - 1.0]));
        assert!(shapes().any(|shape| shape.kind == kind::RECT
            && shape.color == [SURFACE.r, SURFACE.g, SURFACE.b, 255]
            && shape.rect
                == [
                    geometry.labels,
                    y + LANE_HEIGHT - 1.0,
                    view.width,
                    y + LANE_HEIGHT
                ]));
        let ticks = shapes()
            .filter(|shape| {
                shape.kind == kind::RECT
                    && shape.rect[3] - shape.rect[1] == 6.0
                    && shape.color[..3] == [242, 230, 166]
            })
            .collect::<Vec<_>>();
        assert_eq!(ticks.len(), 2);
        for (tick, (offset, width, alpha)) in
            ticks.into_iter().zip([(4.0, 1.0, 140), (10.0, 3.0, 255)])
        {
            assert_eq!(
                tick.rect,
                [
                    cue.x + offset,
                    cue.y + cue.h - 9.0,
                    cue.x + offset + width,
                    cue.y + cue.h - 3.0
                ]
            );
            assert_eq!(tick.color[3], alpha);
        }
        assert!(shapes().any(|shape| shape.kind == kind::MASK
            && shape.rect[0] >= cue.x
            && shape.rect[1] >= cue.y
            && shape.rect[1] < cue.y + 13.0
            && shape.clip[2] == (cue.x + cue.w) as i16));
        mem_sys::set_peer(pxui::library_counter);
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                for _ in 0..8 {
                    testing::frame(&mut ui, &[], build);
                }
            },
        );
    }

    #[test]
    fn recorded_events_follow_source_clock_and_visibility_without_warm_allocations() {
        use pxui::testing;
        use splice_model::{Clip, Ease, Effect, EffectId, EffectParam, Key};

        // The same native event-log envelope and pointer/key records used by splice_av tests.
        let path = std::env::temp_dir().join(format!(
            "splice-timeline-events-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, r#"{"t0":0,"source":{"kind":"display","rect":{"x":0,"y":0,"width":96,"height":64},"scale":1},"events":[
            {"t":0,"k":"move","x":0.25,"y":0.25},
            {"t":0.02,"k":"key","ch":"QQ","keyCode":12,"chipUntil":0.85},
            {"t":0.1,"k":"key","ch":"W","keyCode":13},
            {"t":0.6,"k":"down","x":0.75,"y":0.5,"b":0},{"t":0.65,"k":"up","x":0.75,"y":0.5,"b":0},
            {"t":0.75,"k":"key","ch":"E","keyCode":14},
            {"t":1.4,"k":"key","ch":"\u001c","keyCode":123,"mods":0},
            {"t":9,"k":"down","x":0.5,"y":0.75,"b":0},
            {"t":9,"k":"key","ch":"T","keyCode":17}]}"#).unwrap();
        let events = splice_av::load_timeline_events(&path, 30.0).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(events.clicks, [0.6, 9.0]);
        assert_eq!(
            (events.keys[1].index, events.keys[1].label.as_str()),
            (Some(1), "•")
        );
        assert_eq!(events.keys[0].chip_until, Some(0.85));
        assert_eq!(
            events.keys[0].authored_frames(
                SourceSpan {
                    start: 5,
                    end: 15,
                    source: 3.0,
                    step: 0.0,
                },
                &[],
                30.0,
            ),
            Some([5, 15]),
        );

        let mut project = Project::default();
        let id = ClipId::from_serial(1);
        let media = MediaId::from_serial(1);
        let mut data = ClipData {
            media: Some(media),
            ..ClipData::default()
        };
        data.enabled.pointer = Some(true);
        data.enabled.keys = Some(true);
        data.hidden_keys.push(2);
        for (serial, name, params) in [
            (1, "cursor", Vec::new()),
            (
                2,
                "keystrokes",
                vec![EffectParam {
                    name: "show".into(),
                    value: EffectValue::Text("typing".into()),
                }],
            ),
            (
                3,
                "speed",
                vec![EffectParam {
                    name: "rate".into(),
                    value: EffectValue::Keys(vec![Key {
                        at: 0,
                        value: 1.0,
                        ease: Ease::Linear,
                    }]),
                }],
            ),
        ] {
            data.appearance.effects.push(Effect {
                id: EffectId::from_serial(serial),
                kind: name.into(),
                enabled: Some(true),
                params,
            });
        }
        project
            .clips
            .insert(Clip {
                id,
                timing: ClipTiming {
                    start: 0,
                    length: 60,
                    source: 3,
                    speed: 1.25,
                },
                data,
            })
            .unwrap();
        let previews = [MediaPreview {
            id: media,
            thumbnail: None,
            waveform: Vec::new(),
            camera: None,
            camera_only: false,
            mic: Vec::new(),
            system: Vec::new(),
            events,
        }];
        let mut cache = Cache::default();
        let view = View {
            width: 320.0,
            height: 240.0,
            at: 50,
            end: 150,
            dragging: false,
            history: (false, false),
            state: State {
                expanded: Some(id),
                scale: 2.0,
                ..State::default()
            },
        };
        let result = std::cell::RefCell::new(Intents {
            state: view.state.clone(),
            ..Intents::default()
        });
        let build = &mut |ui: &mut Ui| {
            let state = result.borrow().state.clone();
            let next = paint(
                ui,
                Some(&project),
                View {
                    state,
                    ..view.clone()
                },
                &mut cache,
                (&previews, None),
            );
            result.borrow_mut().merge(next);
        };
        let mut ui = pxui::doc::frames(2, build);
        let shapes = || ui.layers().flat_map(|layer| layer.instances);
        let click = shapes()
            .find(|shape| {
                shape.color2 == [191, 224, 234, 255]
                    && (shape.rect[2] - shape.rect[0] - 9.0).abs() < 0.001
            })
            .unwrap();
        assert!((click.rect[0] - (COMPACT_LABEL_WIDTH + 24.0 - 4.5)).abs() < 0.001);
        let pointer_top = TOOLBAR_HEIGHT + RULER_HEIGHT + SPINE_HEIGHT + TAKE_HEAD_HEIGHT;
        assert_eq!(click.rect[1], pointer_top + (COMPONENT_HEIGHT - 9.0) * 0.5);
        let chips = ui
            .a11y_nodes()
            .iter()
            .filter(|node| node.is_role(Role::Label) && ui.a11y_name_of(node) == "•")
            .count();
        assert_eq!(
            chips, 2,
            "trimmed, offscreen, and excluded categories are omitted"
        );
        let authored = testing::find(&ui, Role::Button, "••").unwrap();
        assert_eq!((authored.x, authored.w), (COMPACT_LABEL_WIDTH, 36.0));
        let muted = shapes()
            .filter(|shape| shape.color2 == [QUIET.r, QUIET.g, QUIET.b, 255])
            .map(|shape| shape.rect)
            .collect::<Vec<_>>();
        assert!(
            muted
                .iter()
                .any(|rect| rect[0] == (COMPACT_LABEL_WIDTH + 31.2).round()
                    && rect[1] == pointer_top + COMPONENT_HEIGHT + 4.0),
            "{muted:?}"
        );
        mem_sys::set_peer(pxui::library_counter);
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                for _ in 0..8 {
                    testing::frame(&mut ui, &[], build);
                }
            },
        );
        let click = testing::find(&ui, Role::Button, "Jump to recorded click").unwrap();
        testing::click(
            &mut ui,
            f64::from(click.x + click.w / 2.0),
            f64::from(click.y + click.h / 2.0),
            build,
        );
        assert_eq!(result.borrow().scrub, Some((Phase::Ended, 12)));
        assert!(result.borrow().state.component().is_none());

        clear_actions(&result);
        let chip = ui
            .a11y_nodes()
            .iter()
            .filter(|node| node.is_role(Role::Button) && ui.a11y_name_of(node) == "•")
            .last()
            .unwrap()
            .rect;
        testing::click(
            &mut ui,
            f64::from(chip.x + chip.w / 2.0),
            f64::from(chip.y + chip.h / 2.0),
            build,
        );
        let selected = result.borrow();
        assert_eq!(
            selected.state.component(),
            Some((id, TakeComponent::Keys, Some(2)))
        );
        assert!(selected.inspect_selection);
        assert_eq!(selected.scrub, Some((Phase::Ended, 16)));
        let (_, hide) = component_visibility(&project, &selected.state, false).unwrap();
        let op = hide.operation();
        assert_eq!(
            op.field("cmd").and_then(splice_json::Value::as_str),
            Some("keys.hide")
        );
        assert_eq!(
            op.field("hidden").and_then(splice_json::Value::as_bool),
            Some(false)
        );
        drop(selected);
        clear_actions(&result);
        testing::click(
            &mut ui,
            f64::from(authored.x + 24.0),
            f64::from(authored.y + authored.h * 0.5),
            build,
        );
        assert_eq!(
            result.borrow().state.component(),
            Some((id, TakeComponent::Keys, Some(0)))
        );
        assert_eq!(result.borrow().scrub, Some((Phase::Ended, 0)));
    }

    #[test]
    fn expanded_take_exposes_only_present_components_and_emits_visibility_change() {
        use pxui::{Role, testing};
        use splice_model::Clip;
        use std::cell::RefCell;

        let id = ClipId::from_serial(1);
        let mut project = Project::default();
        let mut data = ClipData::default();
        data.appearance.name = Some("Recorded take".into());
        data.enabled.screen = Some(true);
        data.enabled.mic = Some(false);
        project
            .clips
            .insert(Clip {
                id,
                timing: ClipTiming {
                    start: 0,
                    length: 300,
                    source: 0,
                    speed: 1.0,
                },
                data,
            })
            .unwrap();
        let view = View {
            width: 320.0,
            height: 240.0,
            at: 0,
            end: 300,
            dragging: false,
            history: (false, false),
            state: State::default(),
        };
        let result = RefCell::new(Intents::default());
        let mut cache = Cache::default();
        let build = &mut |ui: &mut Ui| {
            let mut intents = result.borrow().clone();
            intents.merge(paint(
                ui,
                Some(&project),
                View {
                    state: result.borrow().state.clone(),
                    ..view.clone()
                },
                &mut cache,
                (&[], None),
            ));
            *result.borrow_mut() = intents;
        };
        let mut ui = pxui::doc::frames(2, build);
        assert!(testing::find(&ui, Role::Button, "Show microphone").is_none());
        let show = testing::find(&ui, Role::Button, "Show take components").unwrap();
        let viewport = ui
            .a11y_nodes()
            .iter()
            .find(|node| node.is_role(Role::ScrollArea))
            .unwrap()
            .rect;
        assert_eq!(show.x, viewport.x + 10.0);
        assert_eq!(show.y, viewport.y + (SPINE_HEIGHT - 18.0) * 0.5);
        clear_actions(&result);
        testing::click(
            &mut ui,
            f64::from(show.x + show.w * 0.5),
            f64::from(show.y + show.h * 0.5),
            build,
        );
        assert!(result.borrow().state.all_components);
        let mic = testing::find(&ui, Role::Button, "Show microphone").unwrap();
        let screen = testing::find(&ui, Role::Button, "Hide screen video").unwrap();
        assert!(testing::find(&ui, Role::Button, "Show camera video").is_none());
        let head_top = viewport.y + SPINE_HEIGHT;
        let screen_top = head_top + TAKE_HEAD_HEIGHT;
        let mic_top = screen_top + COMPONENT_HEIGHT;
        assert_eq!(screen.y, screen_top + (COMPONENT_HEIGHT - 18.0) * 0.5);
        assert_eq!(mic.y, mic_top + (AUDIO_COMPONENT_HEIGHT - 18.0) * 0.5);
        assert_eq!(screen.x, viewport.x + COMPACT_LABEL_WIDTH - 24.0);
        assert_eq!(mic.x, screen.x);
        let shapes = || ui.layers().flat_map(|layer| layer.instances);
        let bracket = [TAKE_SAGE.r, TAKE_SAGE.g, TAKE_SAGE.b, 102];
        let caps = shapes()
            .filter(|shape| shape.color == bracket && shape.rect[1] == head_top + 10.0)
            .count();
        assert_eq!(caps, 2);
        let mut hatch_count = 0;
        for shape in shapes().filter(|shape| shape.color == [QUIET.r, QUIET.g, QUIET.b, 26]) {
            hatch_count += 1;
            assert_eq!(shape.clip[0], (viewport.x + COMPACT_LABEL_WIDTH) as i16);
            assert_eq!(shape.clip[1], (mic_top + 2.0) as i16);
            assert_eq!(
                shape.clip[3],
                (mic_top + AUDIO_COMPONENT_HEIGHT - 2.0) as i16
            );
        }
        assert!(hatch_count > 1);
        assert!(shapes().any(|shape| shape.color == [227, 130, 118, 255]));
        clear_actions(&result);
        testing::click(
            &mut ui,
            f64::from(mic.x + mic.w * 0.5),
            f64::from(mic.y + mic.h * 0.5),
            build,
        );
        assert_eq!(result.borrow().component, Some((id, "mic", true)));
        mem_sys::set_peer(pxui::library_counter);
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                for _ in 0..8 {
                    testing::frame(&mut ui, &[], build);
                }
            },
        );
    }
    #[test]
    fn timeline_displays_real_badges_and_clipped_fades_without_allocating_when_warm() {
        use pxui::{Event, draw::kind, testing};
        use splice_model::{
            Appearance, Clip, Effect, EffectId, Item, ItemData, ItemId, ItemTiming, Lane, Marker,
            MarkerId, Overlay, Placement,
        };
        use std::cell::RefCell;

        let mut project = Project::default();
        let graphics = LaneId::from_serial(1);
        let overlays = LaneId::from_serial(2);
        let marker = MarkerId::from_serial(1);
        let media = MediaId::from_serial(1);
        for (id, kind) in [
            (graphics, LaneKind::Graphics),
            (overlays, LaneKind::Overlay),
        ] {
            project.lanes.push(Lane {
                id,
                name: "Track".into(),
                kind,
                effects: Vec::new(),
            });
        }
        project.markers.push(Marker {
            id: marker,
            at: 0,
            name: "Start".into(),
            color: None,
        });
        let title = ItemId::from_serial(1);
        for (id, lane, kind, timing, appearance) in [
            (
                title,
                graphics,
                ItemKind::Title {
                    text: "Pinned title".into(),
                },
                ItemTiming {
                    anchor: Anchor::Pinned(0),
                    length: 76,
                },
                Appearance {
                    effects: vec![Effect {
                        id: EffectId::from_serial(1),
                        kind: "window".into(),
                        enabled: Some(false),
                        params: Vec::new(),
                    }],
                    ..Appearance::default()
                },
            ),
            (
                ItemId::from_serial(2),
                overlays,
                ItemKind::Media(Overlay {
                    media,
                    source: 0,
                    placement: Placement::Card,
                    behind_presenter: true,
                    muted: false,
                    framing: Default::default(),
                }),
                ItemTiming {
                    anchor: Anchor::Marker { marker, offset: 0 },
                    length: 75,
                },
                Appearance::default(),
            ),
        ] {
            project
                .items
                .insert(Item {
                    id,
                    timing,
                    data: ItemData {
                        lane,
                        kind,
                        appearance,
                    },
                })
                .unwrap();
        }
        for (serial, start, speed, data) in [
            (
                1,
                0,
                1.25,
                ClipData {
                    media: Some(media),
                    appearance: Appearance {
                        name: Some("Fading clip".into()),
                        fade_in: 32,
                        fade_out: 16,
                        ..Appearance::default()
                    },
                    ..ClipData::default()
                },
            ),
            (2, 72, 1.0, ClipData::default()),
        ] {
            project
                .clips
                .insert(Clip {
                    id: ClipId::from_serial(serial),
                    timing: ClipTiming {
                        start,
                        length: 72,
                        source: 0,
                        speed,
                    },
                    data,
                })
                .unwrap();
        }
        let view = View {
            width: 320.0,
            height: 240.0,
            at: 50,
            end: 144,
            dragging: false,
            history: (false, false),
            state: State {
                origin: 10.0,
                scale: 2.0,
                free_snap: true,
                ..State::default()
            },
        };
        let result = RefCell::new(Intents {
            state: view.state.clone(),
            ..Intents::default()
        });
        let mut cache = Cache::default();
        let build = &mut |ui: &mut Ui| {
            let mut intents = result.borrow().clone();
            intents.merge(paint(
                ui,
                Some(&project),
                View {
                    state: intents.state.clone(),
                    ..view.clone()
                },
                &mut cache,
                (&[], None),
            ));
            *result.borrow_mut() = intents;
        };
        let mut ui = pxui::doc::frames(2, build);
        let mut fx = Rect::default();
        for badge in ["fx", "PIN", "BEHIND", "1.25×"] {
            let mut matches = ui
                .a11y_nodes()
                .iter()
                .filter(|node| node.is_role(Role::Label) && ui.a11y_name_of(node) == badge);
            let rect = matches
                .next()
                .unwrap_or_else(|| panic!("missing {badge} badge"))
                .rect;
            assert!(matches.next().is_none(), "duplicate {badge} badge");
            if badge == "fx" {
                fx = rect;
            }
        }
        let ramps = ui
            .layers()
            .flat_map(|layer| layer.instances)
            .filter(|shape| shape.kind == kind::TRIANGLE && shape.color == [0, 0, 0, 107])
            .collect::<Vec<_>>();
        assert_eq!(ramps.len(), 2);
        assert_eq!(ramps[0].rect[0], COMPACT_LABEL_WIDTH - 20.0);
        assert_eq!(ramps[0].clip[0], COMPACT_LABEL_WIDTH as i16);
        assert_eq!(ramps[1].rect[0], COMPACT_LABEL_WIDTH + 92.0);

        testing::click(
            &mut ui,
            f64::from(fx.x + fx.w * 0.5),
            f64::from(fx.y + fx.h * 0.5),
            build,
        );
        assert_eq!(
            result.borrow().state.selection(),
            Some(TimelineId::Item(title))
        );
        let label = testing::find(&ui, Role::Button, "Pinned title").unwrap();
        let x = f64::from(label.x + label.w - 3.0);
        let y = f64::from(label.y + label.h * 0.5);
        for (kind, x) in [
            (Event::POINTER_PRESSED, x),
            (Event::POINTER_MOVED, x - 16.0),
            (Event::POINTER_RELEASED, x - 16.0),
        ] {
            testing::frame(&mut ui, &[Event::pointer(kind, 0, x, y)], build);
        }
        assert_eq!(
            result.borrow().trimmed,
            Some((TimelineId::Item(title), "out", -8, true))
        );

        mem_sys::set_peer(pxui::library_counter);
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                for _ in 0..8 {
                    testing::frame(&mut ui, &[], build);
                }
            },
        );
    }
    #[test]
    fn track_guides_preserve_layer_order_and_clip_offscreen_stems() {
        use pxui::{draw::kind, testing};
        use splice_model::{Item, ItemData, ItemId, ItemTiming, Lane, Marker, MarkerId};

        let mut project = Project::default();
        for serial in 1..=7 {
            project.lanes.push(Lane {
                id: LaneId::from_serial(serial),
                name: "Graphics".into(),
                kind: LaneKind::Graphics,
                effects: Vec::new(),
            });
        }
        let marker = MarkerId::from_serial(1);
        for (serial, at) in [(1, 0), (2, 24), (3, 120)] {
            project.markers.push(Marker {
                id: MarkerId::from_serial(serial),
                at,
                name: "Guide".into(),
                color: None,
            });
        }
        for (serial, lane, frame) in [(1, 1, 0), (2, 3, 24), (3, 3, 48), (4, 4, 120)] {
            project
                .items
                .insert(Item {
                    id: ItemId::from_serial(serial),
                    timing: ItemTiming {
                        anchor: if serial == 3 {
                            Anchor::Pinned(frame)
                        } else {
                            Anchor::Marker {
                                marker,
                                offset: frame,
                            }
                        },
                        length: 9,
                    },
                    data: ItemData {
                        lane: LaneId::from_serial(lane),
                        kind: ItemKind::Title {
                            text: "Title".into(),
                        },
                        appearance: Default::default(),
                    },
                })
                .unwrap();
        }
        let mut view = View {
            width: 320.0,
            height: 240.0,
            at: 24,
            end: 150,
            dragging: false,
            history: (false, false),
            state: State {
                scale: 2.0,
                ..State::default()
            },
        };
        view.state.set_selected(
            [2, 3]
                .map(|serial| TimelineId::Item(ItemId::from_serial(serial)))
                .to_vec(),
        );
        let mut cache = Cache::default();
        let build = &mut |ui: &mut Ui| {
            paint(ui, Some(&project), view.clone(), &mut cache, (&[], None));
        };
        let mut ui = pxui::doc::frames(2, build);
        let (scroll, viewport) = ui
            .a11y_nodes()
            .iter()
            .find(|node| node.is_role(Role::ScrollArea))
            .map(|node| (ui.box_of_a11y(node.id).unwrap(), node.rect))
            .unwrap();
        ui.set_scroll_offset(&scroll, 52.0);
        testing::frame(&mut ui, &[], build);
        assert_eq!(ui.scroll(&scroll).painted, 52.0);

        let x = viewport.x + COMPACT_LABEL_WIDTH;
        let y = viewport.y;
        let bottom = y + viewport.h;
        let stem_end = y + 7.0 * LANE_HEIGHT + 5.0 - 52.0;
        let selected_x = x + 48.0;
        let instances = ui
            .layers()
            .flat_map(|layer| layer.instances)
            .collect::<Vec<_>>();
        let stem = instances
            .iter()
            .position(|instance| {
                instance.kind == kind::GRID && instance.color == [240, 214, 196, 140]
            })
            .unwrap();
        assert_eq!(instances[stem].rect, [x, y, x + 1.0, stem_end]);
        let diamond = instances
            .iter()
            .filter(|instance| {
                instance.kind == kind::TRIANGLE && instance.color == [240, 214, 196, 179]
            })
            .collect::<Vec<_>>();
        assert_eq!(diamond.len(), 2);
        for instance in diamond {
            assert!(instance.rect[0] < x);
            assert_eq!(instance.clip[0], x as i16);
            assert_eq!(instance.rect[1], stem_end - 1.5);
        }
        let markers = instances
            .iter()
            .enumerate()
            .filter(|(_, instance)| {
                instance.kind == kind::GRID && instance.color == [216, 200, 106, 89]
            })
            .collect::<Vec<_>>();
        assert_eq!(markers.len(), 2);
        let last_marker = markers[1].0;
        for ((index, instance), x) in markers.into_iter().zip([x, selected_x]) {
            assert!(index > stem);
            assert_eq!(instance.rect, [x, y, x + 1.0, bottom]);
        }
        let selected = instances
            .iter()
            .position(|instance| {
                instance.color == [240, 214, 196, 255]
                    && instance.rect
                        == [
                            selected_x,
                            y + 3.0 * LANE_HEIGHT - 3.0 - 52.0,
                            selected_x + 1.0,
                            stem_end,
                        ]
            })
            .unwrap();
        assert!(selected > last_marker);
        assert_eq!(
            instances
                .iter()
                .filter(|instance| {
                    instance.kind == kind::TRIANGLE && instance.color == [240, 214, 196, 255]
                })
                .count(),
            2
        );
        let needle = instances
            .iter()
            .enumerate()
            .filter(|(_, instance)| {
                instance.color == [255, 138, 92, 255]
                    && instance.rect[1] >= y
                    && instance.rect[3] <= bottom
            })
            .collect::<Vec<_>>();
        assert_eq!(needle.len(), 1);
        assert!(needle[0].0 > selected);
        assert_eq!(needle[0].1.rect, [selected_x, y, selected_x + 1.0, bottom]);
    }
    #[test]
    fn large_timeline_reuses_lane_and_overview_indexes_during_playback() {
        use pxui::{UiDesc, testing};
        use splice_model::{Anchor, Clip, Item, ItemData, ItemId, ItemTiming, Lane};

        let mut project = Project::default();
        let lane_count = 12;
        let clip_count = 1000;
        let end = 30_000;
        for serial in 1..=lane_count {
            project.lanes.push(Lane {
                id: LaneId::from_serial(serial),
                name: "Graphics".into(),
                kind: LaneKind::Graphics,
                effects: Vec::new(),
            });
        }
        for number in 0..clip_count {
            project
                .clips
                .insert(Clip {
                    id: ClipId::from_serial(number as u64 + 1),
                    timing: ClipTiming {
                        start: number * 30,
                        length: 30,
                        source: 0,
                        speed: 1.0,
                    },
                    data: ClipData::default(),
                })
                .unwrap();
        }
        for number in 0..2000 {
            project
                .items
                .insert(Item {
                    id: ItemId::from_serial(number as u64 + 1),
                    timing: ItemTiming {
                        anchor: Anchor::ClipSource {
                            clip: ClipId::from_serial(number as u64 / 2 + 1),
                            source: (number % 2) * 15,
                        },
                        length: 12,
                    },
                    data: ItemData {
                        lane: LaneId::from_serial(number as u64 % lane_count + 1),
                        kind: ItemKind::Title {
                            text: "Title".into(),
                        },
                        appearance: Default::default(),
                    },
                })
                .unwrap();
        }
        let mut cache = Cache::default();
        let view = View {
            width: 320.0,
            height: 240.0,
            at: 23_400,
            end,
            dragging: false,
            history: (false, false),
            state: State {
                origin: 23_370.0,
                scale: 2.0,
                ..State::default()
            },
        };
        let geo = Geometry::new(view.width, project.fps, end, &view.state);
        cache.refresh(Some(&project), geo, project.fps);
        assert_eq!(cache.lane(LaneId::from_serial(1)).len(), 167);
        assert_eq!(cache.lane(LaneId::from_serial(1))[0].start, Some(0));
        assert_eq!(cache.guides.anchors.len(), 2000);
        assert!(cache.overview.is_empty());
        let first = project.items.order()[0];
        project.items.timing_mut(first).unwrap().anchor = Anchor::Pinned(2);
        project.rev += 1;
        cache.refresh(Some(&project), geo, project.fps);
        assert_eq!(cache.lane(LaneId::from_serial(1))[0].start, Some(2));
        assert_eq!(cache.guides.anchors.len(), 1999);
        cache.refresh(None, geo, project.fps);
        assert!(cache.lane(LaneId::from_serial(1)).is_empty());

        mem_sys::set_peer(pxui::library_counter);
        let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
        let mut frame = || {
            for at in view.at..view.at + 20 {
                testing::frame(&mut ui, &[], &mut |ui| {
                    paint(
                        ui,
                        Some(&project),
                        View { at, ..view.clone() },
                        &mut cache,
                        (&[], None),
                    );
                });
            }
        };
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut frame);
        let stems = ui
            .layers()
            .flat_map(|layer| layer.instances)
            .filter(|instance| {
                instance.kind == pxui::draw::kind::GRID && instance.color == [240, 214, 196, 140]
            })
            .count();
        assert!((1..=6).contains(&stems));
    }
    #[test]
    fn fx_badges_resolve_scope_and_stay_warm_at_compact_and_desktop_widths() {
        use pxui::{Event, FrameIn, UiDesc, draw::kind, testing};
        use splice_model::{Clip, EffectId, Item, ItemTiming, Lane, Overlay, Placement, Probe};
        use std::cell::Cell;

        let effect = |id, kind: &str, enabled| Effect {
            id: EffectId::from_serial(id),
            kind: kind.into(),
            enabled,
            params: Vec::new(),
        };
        let mut project = Project {
            effects: vec![
                effect(1, "window", Some(true)),
                effect(2, "cursor", Some(true)),
                effect(3, "camera", Some(true)),
            ],
            ..Project::default()
        };
        project.spine.effects.push(effect(4, "cursor", Some(false)));
        let layers = LaneId::from_serial(1);
        let plain = LaneId::from_serial(2);
        let captions = LaneId::from_serial(3);
        for (id, name, lane_kind, effects) in [
            (
                layers,
                "Layers",
                LaneKind::Overlay,
                vec![effect(5, "window", None)],
            ),
            (plain, "Plain", LaneKind::Overlay, Vec::new()),
            (
                captions,
                "Captions",
                LaneKind::Captions,
                vec![effect(6, "captions", None)],
            ),
        ] {
            project.lanes.push(Lane {
                id,
                name: name.into(),
                kind: lane_kind,
                effects,
            });
        }
        let file = MediaId::from_serial(2);
        for (serial, media_kind) in [(1, MediaKind::Take), (2, MediaKind::Video)] {
            project.media.push(Media {
                id: MediaId::from_serial(serial),
                kind: media_kind,
                path: "clip.mov".into(),
                bookmark: None,
                size: 0,
                mtime: 0,
                probe: Probe {
                    fps: 30.0,
                    frames: 100,
                    width: 960,
                    height: 540,
                    audio: Vec::new(),
                    codec: None,
                    loudness: None,
                },
                take: (media_kind == MediaKind::Take).then(Default::default),
            });
        }
        for (serial, name, media) in [
            (1, "Take", Some(MediaId::from_serial(1))),
            (2, "File", Some(file)),
            (3, "Gap", None),
        ] {
            let mut data = ClipData {
                media,
                ..ClipData::default()
            };
            data.appearance.name = Some(name.into());
            if serial == 1 {
                data.enabled.pointer = Some(false);
                data.appearance.effects =
                    vec![effect(7, "window", None), effect(8, "cursor", None)];
            }
            project
                .clips
                .insert(Clip {
                    id: ClipId::from_serial(serial),
                    timing: ClipTiming {
                        start: (serial as i32 - 1) * 60,
                        length: 60,
                        source: 0,
                        speed: 1.0,
                    },
                    data,
                })
                .unwrap();
        }
        for number in 0..2_004 {
            let (lane, at) = match number {
                0 => (layers, 0),
                1 => (layers, 60),
                2 => (plain, 0),
                3 => (captions, 0),
                _ => (layers, 1_200 + number * 60),
            };
            let item_kind = if lane == captions {
                ItemKind::Caption {
                    words: Vec::new(),
                    edits: Vec::new(),
                    text: Some("Words".into()),
                }
            } else {
                ItemKind::Media(Overlay {
                    media: file,
                    source: 0,
                    placement: Placement::Card,
                    behind_presenter: false,
                    muted: true,
                    framing: Default::default(),
                })
            };
            let mut data = ItemData {
                lane,
                kind: item_kind,
                appearance: Default::default(),
            };
            if number == 1 {
                data.appearance
                    .effects
                    .push(effect(9, "window", Some(false)));
            }
            project
                .items
                .insert(Item {
                    id: ItemId::from_serial(number as u64 + 1),
                    timing: ItemTiming {
                        anchor: Anchor::Pinned(at),
                        length: 54,
                    },
                    data,
                })
                .unwrap();
        }

        mem_sys::set_peer(pxui::library_counter);
        for width in [480.0, 960.0] {
            let mut cache = Cache::default();
            let view = View {
                width,
                height: 280.0,
                at: 12,
                end: 124_000,
                dragging: false,
                history: (false, false),
                state: State {
                    scale: 2.25,
                    ..State::default()
                },
            };
            let geo = Geometry::new(width, 30, view.end, &view.state);
            cache.refresh(Some(&project), geo, 30);
            let clips = project.clips.order();
            let take = cache.clip_fx(clips[0]).unwrap();
            assert_eq!(take.kind, FxBadgeKind::Override);
            assert_eq!(
                take.tip,
                "Effects: Window (Project › this item) · Camera Layout (Project)"
            );
            assert_eq!(
                cache.clip_fx(clips[1]).unwrap().kind,
                FxBadgeKind::Inherited
            );
            assert_eq!(
                cache.clip_fx(clips[1]).unwrap().tip,
                "Effects: Window (Project)"
            );
            assert!(cache.clip_fx(clips[2]).is_none());
            let items = cache.lane(layers);
            assert_eq!(items[0].fx.as_ref().unwrap().kind, FxBadgeKind::Inherited);
            assert_eq!(
                items[0].fx.as_ref().unwrap().tip,
                "Effects: Window (Layers)"
            );
            assert_eq!(items[1].fx.as_ref().unwrap().kind, FxBadgeKind::Local);
            assert_eq!(
                items[1].fx.as_ref().unwrap().tip,
                "Effects: some turned off here"
            );
            assert!(cache.lane(plain)[0].fx.is_none());
            assert!(cache.lane(captions)[0].fx.is_none());
            assert_eq!(cache.lane_fx(captions).unwrap().kind, FxBadgeKind::Local);
            assert!(cache.lane_fx(plain).is_none());

            let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
            ui.set_a11y(true);
            let at = Cell::new(view.at);
            let selection = Cell::new(None);
            let build = &mut |ui: &mut Ui| {
                let intents = paint(
                    ui,
                    Some(&project),
                    View {
                        at: at.get(),
                        ..view.clone()
                    },
                    &mut cache,
                    (&[], None),
                );
                selection.set(intents.state.selection());
            };
            let input = FrameIn::new(width, 280.0);
            ui.frame(&input, build).unwrap();
            ui.frame(&input, build).unwrap();
            let take = testing::find(&ui, Role::Button, "Take").unwrap();
            let file = testing::find(&ui, Role::Button, "File").unwrap();
            let mut after_duration = |rect: Rect, name| {
                let name_width = ui
                    .text_service()
                    .measure(name, SMALL.pad(0.0), None)
                    .unwrap()
                    .width
                    .ceil();
                (rect.x + 17.0 + name_width + 6.0 + 32.5 + 6.0).floor()
            };
            let take_fx_x = after_duration(take, "Take");
            let take_fx = [take_fx_x, take.y + 3.0, take_fx_x + 26.0, take.y + 15.0];
            let file_fx = Rect::new(after_duration(file, "File"), file.y + 3.0, 20.0, 12.0);
            assert!(take_fx[2] < take.x + take.w - 9.0 - FADE_HANDLE * 0.5);
            assert!(file_fx.x + file_fx.w < file.x + file.w - 9.0 - FADE_HANDLE * 0.5);
            let shapes = || ui.layers().flat_map(|layer| layer.instances);
            assert!(
                shapes().any(|shape| shape.rect == take_fx && shape.color == [191, 224, 234, 255]),
                "width {width}: expected {take_fx:?}; take {take:?}; file {file:?}; cyan rects {:?}",
                shapes()
                    .filter(|shape| shape.color == [191, 224, 234, 255])
                    .map(|shape| (shape.rect, shape.clip))
                    .collect::<Vec<_>>()
            );
            assert!(testing::find(&ui, Role::Label, "fx•").is_some());
            assert!(testing::find(&ui, Role::Button, "Show take components").is_some());
            let inherited = shapes()
                .filter(|shape| shape.kind == kind::RECT && shape.color == [236, 230, 206, 204])
                .collect::<Vec<_>>();
            assert_eq!(inherited.len(), 10);
            for shape in inherited {
                let [left, top, right, bottom] = shape.rect;
                assert!(right - left == 1.0 || bottom - top == 1.0);
                assert!(
                    left >= file_fx.x
                        && top >= file_fx.y
                        && right <= file_fx.x + file_fx.w
                        && bottom <= file_fx.y + file_fx.h
                );
            }
            assert_eq!(
                ui.a11y_nodes()
                    .iter()
                    .filter(|node| node.is_role(Role::Label) && ui.a11y_name_of(node) == "fx")
                    .count(),
                6
            );

            let x = f64::from(file_fx.x + 4.0);
            let y = f64::from(file_fx.y + 5.0);
            let pointer = [Event::pointer(Event::POINTER_MOVED, 0, x, y)];
            for tick in 1..=8 {
                let input = FrameIn::new(width, 280.0)
                    .time(f64::from(tick) * 0.1)
                    .events(&pointer);
                ui.frame(&input, build).unwrap();
            }
            assert!(
                ui.a11y_nodes()
                    .iter()
                    .any(|node| ui.a11y_name_of(node) == "Effects: Window (Project)")
            );
            assert_eq!(selection.get(), None);

            let time = Cell::new(1.0);
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    for frame in 12..24 {
                        at.set(frame);
                        time.set(time.get() + 0.1);
                        let input = FrameIn::new(width, 280.0).time(time.get()).events(&pointer);
                        ui.frame(&input, build).unwrap();
                    }
                });
            for event in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
                time.set(time.get() + 0.1);
                let pointer = [Event::pointer(event, 0, x, y)];
                let input = FrameIn::new(width, 280.0).time(time.get()).events(&pointer);
                ui.frame(&input, build).unwrap();
            }
            assert_eq!(
                selection.get(),
                Some(TimelineId::Clip(ClipId::from_serial(2)))
            );
        }
    }

    #[test]
    fn warmed_timeline_advances_without_allocating() {
        use pxui::{UiDesc, testing};

        mem_sys::set_peer(pxui::library_counter);
        let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
        let project = Project::default();
        let mut cache = Cache::default();
        let mut frame = || {
            for at in 0..30 {
                testing::frame(&mut ui, &[], &mut |ui| {
                    paint(
                        ui,
                        Some(&project),
                        View {
                            width: 560.0,
                            height: 220.0,
                            at,
                            end: 1000,
                            dragging: false,
                            history: (false, false),
                            state: State::default(),
                        },
                        &mut cache,
                        (&[], None),
                    );
                });
            }
        };
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut frame);
    }
}
