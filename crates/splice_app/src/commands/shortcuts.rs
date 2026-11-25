use super::{
    ADD_MARKER, ADD_MOTION, ADD_TITLE, BACK, BACK_SECOND, BLADE_TOOL, BindingDescription,
    CHORD_SEPARATOR, COPY, CROP, CUT, DELETE, DELETE_GAP, DUPLICATE, END, ESCAPE, FIT, FORWARD,
    FORWARD_SECOND, LANE_DOWN, LANE_UP, LIFT_TO_OVERLAY, NEXT_EDIT, NUDGE_BACK, NUDGE_BACK_TEN,
    NUDGE_FORWARD, NUDGE_FORWARD_TEN, PALETTE, PASTE, PASTE_EFFECTS, PLAY, PREVIOUS_EDIT, REDO,
    REPLACE_SPINE, SELECT_LANE_ITEMS, SELECT_TOOL, SESSION_UNDO, SHORTCUTS, SHUTTLE_FORWARD,
    SHUTTLE_REVERSE, SNAP, SPLIT, START, STOP, TRIM_END, TRIM_START, UNDO, ZOOM_IN, ZOOM_OUT,
    ZOOM_SELECTION, ZOOM_TIMELINE_IN, ZOOM_TIMELINE_OUT,
};
use splice_kit::CommandId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    Playback,
    Tools,
    Edit,
    Lanes,
    Add,
    View,
}
use Group::{Add, Edit, Lanes, Playback, Tools, View};

pub(crate) const GROUPS: [(Group, &str); 6] = [
    (Playback, "PLAYBACK"),
    (Tools, "TOOLS"),
    (Edit, "EDIT"),
    (Lanes, "LANES"),
    (Add, "ADD"),
    (View, "VIEW"),
];

pub(crate) struct ShortcutRow {
    pub(crate) group: Group,
    pub(crate) title: &'static str,
    pub(crate) command: &'static str,
    pub(crate) keys: String,
}

struct Row {
    group: Group,
    title: &'static str,
    command: &'static str,
    ids: &'static [CommandId],
    split: usize,
    hint: &'static str,
}

const fn row(
    group: Group,
    title: &'static str,
    command: &'static str,
    ids: &'static [CommandId],
) -> Row {
    Row {
        group,
        title,
        command,
        ids,
        split: 0,
        hint: "",
    }
}

impl Row {
    const fn split_at(mut self, split: usize) -> Self {
        self.split = split;
        self
    }

    const fn hint(mut self, hint: &'static str) -> Self {
        self.hint = hint;
        self
    }
}

// The v13 layout and command tags describe actions. The window catalog names the routes.
// The unlabeled timeline zoom defaults stay hidden, but explicit remaps get a row.
const ROWS: &[Row] = &[
    row(Playback, "Play / pause", "ui.play", &[PLAY]),
    row(
        Playback,
        "Reverse · stop · forward (repeat for 2× 4×)",
        "ui.play",
        &[SHUTTLE_REVERSE, STOP, SHUTTLE_FORWARD],
    ),
    row(
        Playback,
        "Step frame / second",
        "ui.show",
        &[BACK, FORWARD, BACK_SECOND, FORWARD_SECOND],
    )
    .split_at(2),
    row(
        Playback,
        "Previous / next edit point",
        "ui.show",
        &[PREVIOUS_EDIT, NEXT_EDIT],
    ),
    row(Playback, "Start / end", "ui.show", &[START, END]),
    row(Tools, "Select", "ui.tool", &[SELECT_TOOL]),
    row(
        Tools,
        "Blade (sticky · momentary)",
        "ui.tool",
        &[BLADE_TOOL],
    )
    .hint("hold ⌥"),
    row(
        Tools,
        "Snapping · bypass while dragging",
        "ui.snapping",
        &[SNAP],
    )
    .hint("hold ⌘"),
    row(
        Tools,
        "Move spine clip without its overlays",
        "item.move",
        &[],
    )
    .hint("hold `"),
    row(Tools, "Command palette", "ui.palette", &[PALETTE]),
    row(
        Edit,
        "Split at playhead (a caption: caption.split)",
        "item.split",
        &[SPLIT],
    ),
    row(
        Edit,
        "Trim start / end to playhead",
        "item.trim",
        &[TRIM_START, TRIM_END],
    ),
    row(
        Edit,
        "Delete (magnetic closes gap) · leave gap",
        "item.delete",
        &[DELETE, DELETE_GAP],
    )
    .split_at(1),
    row(
        Edit,
        "Cut · copy · paste",
        "ui.clipboard",
        &[CUT, COPY, PASTE],
    ),
    row(
        Edit,
        "Paste effects from the copied item (merges)",
        "effect.paste",
        &[PASTE_EFFECTS],
    ),
    row(Edit, "Duplicate", "item.duplicate", &[DUPLICATE]),
    row(
        Edit,
        "Nudge 1 / 10 frames",
        "item.move",
        &[NUDGE_BACK, NUDGE_FORWARD, NUDGE_BACK_TEN, NUDGE_FORWARD_TEN],
    )
    .split_at(2),
    row(Edit, "Select all in lane", "ui.show", &[SELECT_LANE_ITEMS]),
    row(Edit, "Undo / redo", "history.undo", &[UNDO, REDO]),
    row(
        Edit,
        "Undo all agent edits",
        "history.undoSession",
        &[SESSION_UNDO],
    ),
    row(Lanes, "Lift to overlay", "clip.lift", &[LIFT_TO_OVERLAY]),
    row(
        Lanes,
        "Replace spine clip",
        "overlay.replaceSpine",
        &[REPLACE_SPINE],
    ),
    row(
        Lanes,
        "Overlay lane up / down",
        "item.lane",
        &[LANE_UP, LANE_DOWN],
    ),
    row(Add, "Title at playhead", "title.add", &[ADD_TITLE]),
    row(
        Add,
        "Scale animation at playhead",
        "motion.add",
        &[ADD_MOTION],
    ),
    row(Add, "Marker at playhead", "marker.add", &[ADD_MARKER]),
    row(
        View,
        "Zoom in / out the active pane (viewer or timeline)",
        "ui.show",
        &[ZOOM_IN, ZOOM_OUT],
    ),
    row(View, "Zoom In on Timeline", "ui.show", &[ZOOM_TIMELINE_IN]),
    row(
        View,
        "Zoom Out on Timeline",
        "ui.show",
        &[ZOOM_TIMELINE_OUT],
    ),
    row(
        View,
        "Fit the active pane (viewer or timeline)",
        "ui.show",
        &[FIT],
    ),
    row(
        View,
        "Zoom the viewer around the pointer; scroll or drag pans; double-click toggles Fit / 100%",
        "ui.viewer",
        &[],
    )
    .hint("pinch · ⌘ scroll"),
    row(
        Edit,
        "Crop selected clip or clip at playhead (↩ apply · Esc cancel · ⇧ keeps ratio)",
        "clip.crop",
        &[CROP],
    ),
    row(View, "Zoom to selection", "ui.show", &[ZOOM_SELECTION]),
    row(View, "Zoom around pointer", "ui.show", &[]).hint("⌘ scroll · pinch"),
    row(View, "This list", "ui.help", &[SHORTCUTS]),
    row(
        View,
        "Cancel drag · leave blade · clear selection",
        "ui.show",
        &[ESCAPE],
    ),
];

/// Resolve user labels once. Both help surfaces borrow the resulting rows on every frame.
pub(super) fn describe(bindings: &[BindingDescription]) -> Vec<ShortcutRow> {
    let grave = "hold `";
    let grave_available = !bindings
        .iter()
        .any(|binding| binding.keys.split(CHORD_SEPARATOR).any(|key| key == "`"));
    let mut rows = Vec::with_capacity(ROWS.len());
    for row in ROWS {
        let mut keys = String::new();
        let mut separator = "";
        for (index, id) in row.ids.iter().enumerate() {
            if index != 0 && index == row.split {
                separator = CHORD_SEPARATOR;
            }
            let Some(binding) = bindings.iter().find(|binding| binding.id == *id) else {
                continue;
            };
            let labels = binding.keys.split(CHORD_SEPARATOR);
            // Prefer Backspace when both native delete aliases still reach this action.
            let omit = match *id {
                DELETE if labels.clone().any(|key| key == "⌫") => "⌦",
                DELETE_GAP if labels.clone().any(|key| key == "⇧⌫") => "⇧⌦",
                ZOOM_TIMELINE_IN => "=",
                ZOOM_TIMELINE_OUT => "−",
                _ => "",
            };
            for key in labels.filter(|key| *key != omit) {
                if *id == ZOOM_TIMELINE_OUT && key == "-" {
                    continue;
                }
                if !keys.is_empty() {
                    keys.push_str(separator);
                }
                keys.push_str(if *id == ZOOM_OUT && key == "⌘-" {
                    "⌘−"
                } else {
                    key
                });
                separator = CHORD_SEPARATOR;
            }
            if !keys.is_empty() {
                separator = " ";
            }
        }
        if !row.hint.is_empty() && (row.hint != grave || grave_available) {
            if !keys.is_empty() {
                keys.push_str(CHORD_SEPARATOR);
            }
            keys.push_str(row.hint);
        }
        if !keys.is_empty() {
            rows.push(ShortcutRow {
                group: row.group,
                title: row.title,
                command: row.command,
                keys,
            });
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::Keys;

    #[test]
    fn default_rows_follow_authored_spec_and_modal_order() {
        let keys = Keys::new(None);
        let rows = keys.shortcuts();
        assert_eq!(keys.bindings().len(), 50);
        assert_eq!(
            rows.iter()
                .map(|row| (row.keys.as_str(), row.command))
                .collect::<Vec<_>>(),
            [
                ("Space", "ui.play"),
                ("J K L", "ui.play"),
                ("← → · ⇧← ⇧→", "ui.show"),
                ("↑ ↓", "ui.show"),
                ("Home End", "ui.show"),
                ("V", "ui.tool"),
                ("S · hold ⌥", "ui.tool"),
                ("N · hold ⌘", "ui.snapping"),
                ("hold `", "item.move"),
                ("⌘K", "ui.palette"),
                ("C", "item.split"),
                ("Q W", "item.trim"),
                ("⌫ · ⇧⌫", "item.delete"),
                ("⌘X ⌘C ⌘V", "ui.clipboard"),
                ("⌥⌘V", "effect.paste"),
                ("⌘D", "item.duplicate"),
                (", . · ⇧, ⇧.", "item.move"),
                ("⌘A", "ui.show"),
                ("⌘Z ⇧⌘Z", "history.undo"),
                ("⌥⌘Z", "history.undoSession"),
                ("⌥⌘↑", "clip.lift"),
                ("⌥⌘↓", "overlay.replaceSpine"),
                ("⌥↑ ⌥↓", "item.lane"),
                ("T", "title.add"),
                ("Z", "motion.add"),
                ("M", "marker.add"),
                ("⌘= ⌘−", "ui.show"),
                ("⇧Z", "ui.show"),
                ("pinch · ⌘ scroll", "ui.viewer"),
                ("⇧C", "clip.crop"),
                ("F", "ui.show"),
                ("⌘ scroll · pinch", "ui.show"),
                ("?", "ui.help"),
                ("Esc", "ui.show"),
            ]
        );
        let groups: Vec<_> = GROUPS
            .iter()
            .map(|(group, name)| (*name, rows.iter().filter(|row| row.group == *group).count()))
            .collect();
        assert_eq!(
            groups,
            [
                ("PLAYBACK", 5),
                ("TOOLS", 5),
                ("EDIT", 11),
                ("LANES", 3),
                ("ADD", 3),
                ("VIEW", 7)
            ]
        );
        let edit: Vec<_> = rows
            .iter()
            .filter(|row| row.group == Edit)
            .map(|row| row.keys.as_str())
            .collect();
        assert_eq!(
            edit,
            [
                "C",
                "Q W",
                "⌫ · ⇧⌫",
                "⌘X ⌘C ⌘V",
                "⌥⌘V",
                "⌘D",
                ", . · ⇧, ⇧.",
                "⌘A",
                "⌘Z ⇧⌘Z",
                "⌥⌘Z",
                "⇧C"
            ]
        );
    }
}
