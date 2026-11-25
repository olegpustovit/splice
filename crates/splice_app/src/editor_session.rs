use pxui::{
    Button, Fill, Label, Node, Ui,
    geom::{Edges, Rgba},
    ui::{Align, Look, SectionList, SectionPart, Size, State as UiState, States},
};
use splice_core::{
    history::{Session, Store},
    package::journal::{Change, Record, Source},
};
use splice_model::SessionId;

const BATCH_HEIGHT: f32 = 30.0;
pub(super) const LABEL_ADVANCE: f32 = 6.6;
const BANNER_PAD: f32 = 9.0;
const BANNER_GAP: f32 = 8.0;
const DOT_WIDTH: f32 = 7.0;
const UNDO_LABEL: &str = "Undo all agent edits ⌥⌘Z";
const UNDO_WIDTH: f32 = 166.0;
const END_WIDTH: f32 = 80.0;
const ACTIVITY_WIDTH: f32 = 62.0;
const QUIET: Rgba = Rgba::rgb(130, 130, 125);
const INK: Rgba = Rgba::rgb(244, 243, 238);
const ORANGE: Rgba = Rgba::rgb(255, 138, 92);
const LINE: Rgba = Rgba::rgb(56, 56, 59);
const CONTROL: Rgba = Rgba::rgb(20, 20, 21);
const BODY: Label = Label::MONO.size(11.0);
const CAPTION: Label = Label::MONO.size(10.0);
const BUTTON_BASE: Look = Look::NONE.fill(CONTROL).text(INK).border(LINE, 1.0);
const BUTTON: States = States::of(BUTTON_BASE)
    .hover(BUTTON_BASE.fill(Rgba::rgb(34, 34, 36)))
    .disabled(BUTTON_BASE.text(QUIET));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    OpenActivity(SessionId),
    End(SessionId),
    UndoAll(SessionId),
    UndoBatch {
        session: SessionId,
        revision: u64,
        if_revision: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Summary {
    id: SessionId,
    checkpoint: u64,
    open: bool,
    live: usize,
    batches: usize,
}

impl Summary {
    fn status(self) -> (&'static str, &'static str) {
        (
            if self.open {
                "Agent editing live"
            } else {
                "Agent session ended"
            },
            if self.live == 1 { "batch" } else { "batches" },
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Row {
    record: usize,
    live: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    revision: u64,
    latest: Option<SessionId>,
    open: Option<SessionId>,
    activity: Option<(SessionId, bool)>,
}

#[derive(Default)]
pub(super) struct State {
    key: Option<Key>,
    banner: Option<Summary>,
    activity: Option<Summary>,
    rows: Vec<Row>,
}

impl State {
    pub(super) fn banner_widths(&self, full: bool) -> (f32, f32) {
        let Some(session) = self.banner else {
            return (0.0, 0.0);
        };
        let mut controls = 2.0 * BANNER_PAD + DOT_WIDTH + BANNER_GAP + UNDO_WIDTH;
        if !full {
            return (controls, 0.0);
        }
        controls += 2.0 * BANNER_GAP + ACTIVITY_WIDTH;
        if session.open {
            controls += BANNER_GAP + END_WIDTH;
        }
        let id_digits = session.id.serial().checked_ilog(36).unwrap_or(0) + 1;
        let count_digits = session.live.checked_ilog10().unwrap_or(0) + 1;
        let (status, batch) = session.status();
        let glyphs = status.len() + " · ses_ ·  ".chars().count() + batch.len();
        (
            controls,
            ((glyphs as u32 + id_digits + count_digits) as f32 * LABEL_ADVANCE).ceil(),
        )
    }

    pub(super) fn open(&self) -> Option<SessionId> {
        self.banner
            .filter(|session| session.open)
            .map(|session| session.id)
    }

    pub(super) fn undoable(&self) -> Option<SessionId> {
        self.banner
            .filter(|session| session.live != 0)
            .map(|session| session.id)
    }

    pub(super) fn has_activity(&self) -> bool {
        self.activity.is_some()
    }

    pub(super) fn lab_status(&self, ui: &mut Ui, access: &str) {
        let open = self.banner.filter(|session| session.open);
        let dot = ui.node("lab-session-dot", Node::DEFAULT.w_px(7.0).h_px(7.0));
        ui.canvas(&dot, move |paint, rect| {
            let color = if open.is_some() { ORANGE } else { QUIET };
            paint.circle(
                (rect.x + 3.5, rect.y + 3.5),
                3.5,
                Some(Fill::new(color)),
                None,
            );
        });
        let label = if let Some(session) = open {
            ui.label_fmt(
                format_args!(
                    "{} · checkpoint r{} · {} batch{}",
                    session.id,
                    session.checkpoint,
                    session.batches,
                    if session.batches == 1 { "" } else { "es" }
                ),
                CAPTION.ellipsis(),
            )
        } else {
            ui.label_fmt(
                format_args!("no open session · access {access}"),
                CAPTION.ellipsis(),
            )
        };
        ui.style(
            &label,
            &States::of(Look::NONE.text(QUIET)),
            UiState::default(),
        );
    }

    pub(super) fn refresh(&mut self, store: Option<&Store>, activity: Option<SessionId>) {
        let Some(store) = store else {
            self.key = None;
            self.banner = None;
            self.activity = None;
            self.rows.clear();
            return;
        };
        let sessions = store.sessions();
        let open = sessions.iter().rfind(|session| session.open);
        let activity = activity.and_then(|id| sessions.iter().rfind(|session| session.id == id));
        let key = Key {
            revision: store.project().rev,
            latest: sessions.last().map(|session| session.id),
            open: open.map(|session| session.id),
            activity: activity.map(|session| (session.id, session.open)),
        };
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        self.banner = open.map(summary);
        self.activity = activity.map(summary);
        self.rows.clear();
        if sessions.is_empty() {
            return;
        }

        // Only look for a closed session when there is no open session to show.
        if self.banner.is_none() {
            let newest_live = store
                .records()
                .iter()
                .rev()
                .filter_map(|record| {
                    let id = agent_edit(record)?;
                    store.is_active(record.rev).then_some(id)
                })
                .max_by_key(|id| id.serial());
            self.banner = newest_live
                .and_then(|id| sessions.iter().rfind(|session| session.id == id))
                .map(summary);
        }
        for (index, record) in store.records().iter().enumerate().rev() {
            let Some(id) = agent_edit(record) else {
                continue;
            };
            let banner = self.banner.as_mut().filter(|session| session.id == id);
            let activity = self.activity.as_mut().filter(|session| session.id == id);
            if banner.is_none() && activity.is_none() {
                continue;
            }
            let live = store.is_active(record.rev);
            if let Some(banner) = banner {
                banner.live += usize::from(live);
                banner.batches += 1;
            }
            if let Some(activity) = activity {
                activity.live += usize::from(live);
                activity.batches += 1;
                self.rows.push(Row {
                    record: index,
                    live,
                });
            }
        }
    }
}

fn summary(session: &Session) -> Summary {
    Summary {
        id: session.id,
        checkpoint: session.checkpoint,
        open: session.open,
        live: 0,
        batches: 0,
    }
}

fn agent_edit(record: &Record) -> Option<SessionId> {
    if record.batch.source == Source::Agent && matches!(&record.batch.change, Change::Edit { .. }) {
        record.batch.session
    } else {
        None
    }
}

pub(super) fn banner(ui: &mut Ui, state: &State, status_width: f32) -> Option<Action> {
    let session = state.banner?;
    let full = status_width > 0.0;
    let mut action = None;
    ui.row(
        Node {
            w: Size::Fit,
            h: Size::Px(24.0),
            align: Align::Center,
            gap: BANNER_GAP,
            padding: Edges::new(BANNER_PAD, 0.0, BANNER_PAD, 0.0),
            look: Look::NONE
                .fill(Rgba::rgb(39, 27, 24))
                .border(ORANGE.with_alpha(115), 1.0),
            ..Node::DEFAULT
        },
        |ui| {
            let color = if session.open { ORANGE } else { QUIET };
            ui.node(
                "agent-live-dot",
                Node::DEFAULT
                    .w_px(DOT_WIDTH)
                    .h_px(DOT_WIDTH)
                    .look(Look::NONE.fill(color).radius(DOT_WIDTH * 0.5)),
            );
            if full {
                let (status, batch) = session.status();
                ui.label_fmt(
                    format_args!("{status} · {} · {} {batch}", session.id, session.live),
                    BODY.pad(0.0)
                        .w_px(status_width)
                        .line_height(15.4)
                        .ellipsis(),
                );
            }
            if control(
                ui,
                "agent-undo-all",
                UNDO_LABEL,
                [UNDO_WIDTH, 18.0],
                session.live != 0,
            ) {
                action = Some(Action::UndoAll(session.id));
            }
            if full
                && session.open
                && control(ui, "agent-end", "End session", [END_WIDTH, 18.0], true)
            {
                action = Some(Action::End(session.id));
            }
            if full
                && control(
                    ui,
                    "agent-activity",
                    "Activity",
                    [ACTIVITY_WIDTH, 18.0],
                    true,
                )
            {
                action = Some(Action::OpenActivity(session.id));
            }
        },
    );
    action
}

pub(super) fn batches(
    ui: &mut Ui,
    context: Option<(&State, &Store)>,
    modal: bool,
) -> Option<Action> {
    let Some((state, store, session)) =
        context.and_then(|(state, store)| Some((state, store, state.activity?)))
    else {
        ui.label("Start a session to see agent batches here.", BODY);
        return None;
    };
    let mut action = None;
    ui.row(
        Node::row().w(Size::Grow).h_px(24.0).center().gap(8.0),
        |ui| {
            let status = if session.open { "open" } else { "ended" };
            ui.label_fmt(
                format_args!(
                    "{} · {status} · checkpoint r{}",
                    session.id, session.checkpoint
                ),
                CAPTION.w(Size::Grow).ellipsis(),
            );
            if modal
                && session.open
                && control(ui, "activity-end", "End session", [80.0, 20.0], true)
            {
                action = Some(Action::End(session.id));
            }
            if control(
                ui,
                "activity-undo-all",
                "Undo all agent edits",
                [134.0, 20.0],
                session.live != 0,
            ) {
                action = Some(Action::UndoAll(session.id));
            }
        },
    );
    if state.rows.is_empty() {
        ui.label("No batches yet.", BODY);
        return action;
    }
    // ponytail: Page the fixed-height list if a session exceeds PXUI's million-row limit.
    let count = state.rows.len().min(1_000_000);
    let limit = if modal { 270.0 } else { 144.0 };
    ui.column(
        Node::column().h_px((count as f32 * BATCH_HEIGHT).min(limit)),
        |ui| {
            ui.section_list(
                "activity-batches",
                &SectionList::new(count)
                    .row_h(BATCH_HEIGHT)
                    .overlay_scrollbar(),
                &|index| store.records()[state.rows[index].record].rev as i64,
                &mut |ui, part, _| {
                    let SectionPart::Row(index) = part else {
                        return;
                    };
                    let row = state.rows[index];
                    let record = &store.records()[row.record];
                    if activity_row(ui, record, row.live) {
                        action = Some(Action::UndoBatch {
                            session: session.id,
                            revision: record.rev,
                            if_revision: store.project().rev,
                        });
                    }
                },
            );
        },
    );
    action
}

fn activity_row(ui: &mut Ui, record: &Record, live: bool) -> bool {
    let mut undo = false;
    ui.row(
        Node::DEFAULT
            .h_px(BATCH_HEIGHT - 2.0)
            .padding(Edges::new(0.0, 2.0, 6.0, 2.0))
            .center()
            .gap(6.0)
            .look(Look::NONE.fill(CONTROL)),
        |ui| {
            let stripe = if live { ORANGE } else { LINE };
            ui.node(
                "batch-stripe",
                Node::DEFAULT
                    .w_px(2.0)
                    .h_px(BATCH_HEIGHT - 2.0)
                    .look(Look::NONE.fill(stripe)),
            );
            let rev = ui.label_fmt(format_args!("r{}", record.rev), CAPTION.w_px(43.0));
            let title = ui.label(
                record
                    .batch
                    .label
                    .strip_prefix("Agent: ")
                    .unwrap_or(&record.batch.label),
                BODY.w(Size::Grow).ellipsis(),
            );
            if !live {
                let quiet = States::of(Look::NONE.text(QUIET));
                ui.style(&rev, &quiet, UiState::default());
                ui.style(&title, &quiet, UiState::default());
            }
            let button = Button::sized(if live { 40.0 } else { 62.0 }, 20.0)
                .look(&BUTTON)
                .label(CAPTION)
                .disabled(!live)
                .name(if live { "Undo batch" } else { "Reverted batch" });
            undo = ui
                .button(
                    "undo-batch",
                    if live { "Undo" } else { "reverted" },
                    &button,
                )
                .clicked;
        },
    );
    undo
}

fn control(
    ui: &mut Ui,
    key: &'static str,
    label: &'static str,
    [width, height]: [f32; 2],
    enabled: bool,
) -> bool {
    let button = Button::sized(width, height)
        .pad(Edges::new(6.0, 0.0, 6.0, 0.0))
        .look(&BUTTON)
        .label(CAPTION)
        .disabled(!enabled);
    ui.button(key, label, &button).clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use splice_core::history::{Scope, WriteOptions};
    use splice_model::Project;

    #[test]
    fn inline_activity_undo_removes_the_clicked_batch_in_a_closed_session() {
        use pxui::{Event, FrameIn, Role};
        let folder =
            std::env::temp_dir().join(format!("splice-inline-activity-{}", std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let mut store =
            Store::create(&folder.join("Activity.splice"), &Project::default()).unwrap();
        let session = store.start_session("fixture", 1).unwrap().id;
        for (at, name) in [(1, "First"), (2, "Second")] {
            let op = splice_json::parse(&format!(
                r#"{{"cmd":"marker.add","at":{at},"name":"{name}"}}"#
            ))
            .unwrap();
            let opts = WriteOptions {
                source: Source::Agent,
                session: Some(session),
                if_revision: Some(store.project().rev),
                ..WriteOptions::default()
            };
            store.apply(&[op], &opts).unwrap();
        }
        store.end_session(session).unwrap();
        let mut state = State::default();
        state.refresh(Some(&store), Some(session));
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let frame = FrameIn::new(500.0, 320.0);
        let draw = |ui: &mut Ui, events: &[Event]| {
            let mut action = None;
            ui.frame(&frame.events(events), &mut |ui| {
                action = batches(ui, Some((&state, &store)), false).or(action);
            })
            .unwrap();
            action
        };
        for _ in 0..3 {
            assert_eq!(draw(&mut ui, &[]), None);
        }
        let buttons: Vec<_> = ui
            .a11y_nodes()
            .iter()
            .filter(|node| node.is_role(Role::Button) && ui.a11y_name_of(node) == "Undo batch")
            .map(|node| node.rect)
            .collect();
        assert_eq!(buttons.len(), 2);
        let older = if buttons[0].y > buttons[1].y {
            buttons[0]
        } else {
            buttons[1]
        };
        let x = f64::from(older.x + older.w * 0.5);
        let y = f64::from(older.y + older.h * 0.5);
        draw(&mut ui, &[Event::pointer(Event::POINTER_PRESSED, 0, x, y)]);
        let expected = Action::UndoBatch {
            session,
            revision: 1,
            if_revision: 2,
        };
        assert_eq!(
            draw(&mut ui, &[Event::pointer(Event::POINTER_RELEASED, 0, x, y)]),
            Some(expected)
        );
        let receipt = store
            .undo(
                Scope::Batch(1),
                &WriteOptions {
                    session: Some(session),
                    if_revision: Some(2),
                    ..WriteOptions::default()
                },
            )
            .unwrap();
        assert_eq!(receipt.reverted, [1]);
        assert_eq!(store.project().markers[0].name, "Second");
        assert_eq!(store.project().markers.len(), 1);
        store.close().unwrap();
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn agent_banner_matches_v13_controls_and_routes_to_real_activity() {
        use crate::{
            EditorLayout, editor_lab,
            editor_titlebar::{self as titlebar, View},
        };
        use pxui::{Event, FrameIn, Rect, Role, testing};

        let folder =
            std::env::temp_dir().join(format!("splice-banner-route-{}.splice", std::process::id()));
        let mut store = Store::create(&folder, &Project::default()).unwrap();
        let session = store.start_session("fixture", 1).unwrap().id;
        for (at, name) in [(1, "First"), (2, "Second")] {
            let op = splice_json::parse(&format!(
                r#"{{"cmd":"marker.add","at":{at},"name":"{name}"}}"#
            ))
            .unwrap();
            let opts = WriteOptions {
                source: Source::Agent,
                session: Some(session),
                if_revision: Some(store.project().rev),
                label: Some(format!("Agent: {name}")),
                ..WriteOptions::default()
            };
            store.apply(&[op], &opts).unwrap();
        }
        store
            .undo(
                Scope::Batch(2),
                &WriteOptions {
                    session: Some(session),
                    ..WriteOptions::default()
                },
            )
            .unwrap();
        let lab = editor_lab::Lab::new();
        let mut state = State::default();
        mem_sys::set_peer(pxui::library_counter);
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let meter =
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset);
        for open in [true, false] {
            if !open {
                store.end_session(session).unwrap();
                let later = store.start_session("empty", 2).unwrap().id;
                store.end_session(later).unwrap();
            }
            for (width, height) in [
                (960.0, 62.0),
                (821.0, 62.0),
                (820.0, 34.0),
                (800.0, 34.0),
                (481.0, 55.0),
                (480.0, 34.0),
                (1280.0, 34.0),
            ] {
                let full = width > 820.0;
                let mut layout = EditorLayout::new([width, 640.0]);
                state.refresh(Some(&store), layout.activity_session(Some(&store)));
                let revision = Some(store.project().rev);
                let bar = titlebar::Layout::new(width, revision, Some(&state));
                assert_eq!(bar.height(), height, "open={open}, width={width}");
                let mut click = None;
                let draw = |ui: &mut Ui, layout: &EditorLayout, state: &State, events: &[Event]| {
                    let mut action = None;
                    ui.frame(&FrameIn::new(width, 640.0).events(events), &mut |ui| {
                        ui.column(Node::column().w(Size::Grow).h(Size::Grow), |ui| {
                            action = action.or(titlebar::paint(
                                ui,
                                Some("Field Notes / 01"),
                                revision,
                                layout.view,
                                &bar,
                                Some(state),
                            ));
                            if layout.view == View::Agent {
                                editor_lab::paint(
                                    ui,
                                    revision.map(|rev| (rev, state.open())),
                                    &lab,
                                    &[],
                                    Some((state, &store)),
                                );
                            } else {
                                ui.label("Editor content", BODY);
                            }
                        });
                    })
                    .unwrap();
                    action
                };
                for _ in 0..3 {
                    assert_eq!(draw(&mut ui, &layout, &state, &[]), None);
                }
                let rect = |ui: &Ui, name| {
                    testing::find(ui, Role::Button, name)
                        .unwrap_or_else(|| panic!("missing {name} at {width}"))
                };
                let tab = rect(&ui, "Spec");
                let undo = rect(&ui, UNDO_LABEL);
                let end = testing::find(&ui, Role::Button, "End session");
                let activity = testing::find(&ui, Role::Button, "Activity");
                let palette = rect(&ui, "Command palette");
                let shortcuts = rect(&ui, "Keyboard shortcuts");
                let (prefix, ending) = state.banner.unwrap().status();
                let status = testing::find(
                    &ui,
                    Role::Label,
                    &format!("{prefix} · {session} · 1 {ending}"),
                );
                assert_eq!(status.is_some(), full, "status at {width}");
                assert_eq!(end.is_some(), full && open);
                assert_eq!(activity.is_some(), full);
                assert_eq!([undo.w, undo.h], [166.0, 18.0]);
                if let Some(status) = status {
                    let expected = if open { 238.0 } else { 245.0 };
                    assert!((status.w - expected).abs() < 0.1, "{status:?}");
                    assert!((status.h - 15.4).abs() < 0.1, "{status:?}");
                    assert!((undo.x - status.x - status.w - 8.0).abs() < 0.1);
                }
                if let Some(end) = end {
                    assert_eq!([end.w, end.h], [80.0, 18.0]);
                }
                if let Some(activity) = activity {
                    assert_eq!([activity.w, activity.h], [62.0, 18.0]);
                }
                let mut previous = tab;
                for next in [
                    status,
                    Some(undo),
                    end,
                    activity,
                    Some(palette),
                    Some(shortcuts),
                ]
                .into_iter()
                .flatten()
                {
                    let inside = |r: Rect| {
                        r.x >= 0.0 && r.x + r.w <= width && r.y >= 0.0 && r.y + r.h <= height
                    };
                    assert!(inside(next), "open={open}, width={width}, {next:?}");
                    assert!(
                        next.x >= previous.x + previous.w - 0.01
                            || next.y >= previous.y + previous.h - 0.01,
                        "open={open}, width={width}: {previous:?} overlaps {next:?}",
                    );
                    previous = next;
                }
                assert!(
                    (shortcuts.x + shortcuts.w + if width <= 480.0 { 8.0 } else { 12.0 } - width)
                        .abs()
                        < 0.01
                );
                let editor = testing::find(&ui, Role::Label, "Editor content").unwrap();
                assert_eq!(editor.y, height);
                meter.warm(&mut || {
                    draw(&mut ui, &layout, &state, &[]);
                });
                let button = activity.unwrap_or_else(|| rect(&ui, "Agent · MCP"));
                let x = f64::from(button.x + button.w * 0.5);
                let y = f64::from(button.y + button.h * 0.5);
                for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
                    click =
                        draw(&mut ui, &layout, &state, &[Event::pointer(kind, 0, x, y)]).or(click);
                }
                match click {
                    Some(titlebar::Action::Session(Action::OpenActivity(id))) if full => {
                        assert_eq!(id, session);
                        layout.show_activity(id);
                    }
                    Some(titlebar::Action::View(View::Agent)) if !full => {
                        layout.show_view(View::Agent)
                    }
                    other => panic!("open={open}, width={width}: {other:?}"),
                }
                assert_eq!(layout.view, View::Agent);
                let picked = layout.activity_session(Some(&store));
                state.refresh(Some(&store), picked);
                for _ in 0..3 {
                    assert_eq!(draw(&mut ui, &layout, &state, &[]), None);
                }
                let agent_pane = ui
                    .a11y_nodes()
                    .iter()
                    .find(|node| {
                        node.is_role(Role::ScrollArea)
                            && ui.a11y_name_of(node) == if full { "MCP tools" } else { "Agent Lab" }
                    })
                    .expect("Agent view receives the real MCP pane")
                    .rect;
                assert_eq!(agent_pane.y, height);
                if full || open {
                    assert_eq!(picked, Some(session));
                    assert_eq!(
                        state.rows,
                        [
                            Row {
                                record: 1,
                                live: false
                            },
                            Row {
                                record: 0,
                                live: true
                            },
                        ]
                    );
                    assert!(testing::find(&ui, Role::Button, "Undo batch").is_some());
                    assert!(testing::find(&ui, Role::Button, "Reverted batch").is_some());
                }
                meter.warm(&mut || {
                    draw(&mut ui, &layout, &state, &[]);
                });
            }
        }
        store.close().unwrap();
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn session_banner_tracks_same_revision_end_and_activity_retains_undo_rows() {
        let folder = std::env::temp_dir().join(format!(
            "splice-session-banner-{}-{}.splice",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut store = Store::create(&folder, &Project::default()).unwrap();
        let mut state = State::default();
        state.refresh(Some(&store), None);
        assert!(state.banner.is_none());
        let session = store.start_session("fixture", 1).unwrap().id;
        state.refresh(Some(&store), Some(session));
        assert_eq!(
            state.banner.map(|session| (session.live, session.open)),
            Some((0, true))
        );
        let op = splice_json::parse(r#"{"cmd":"project.set","aspect":"1:1"}"#).unwrap();
        let options = WriteOptions {
            source: Source::Agent,
            session: Some(session),
            if_revision: Some(0),
            ..WriteOptions::default()
        };
        store.apply(&[op], &options).unwrap();
        state.refresh(Some(&store), Some(session));
        assert_eq!(
            state.banner.map(|session| (session.live, session.open)),
            Some((1, true))
        );
        store.end_session(session).unwrap();
        state.refresh(Some(&store), Some(session));
        assert_eq!(store.project().rev, 1);
        assert_eq!(
            state.banner.map(|session| (session.live, session.open)),
            Some((1, false))
        );
        assert_eq!(state.rows.len(), 1);
        assert!(state.rows[0].live);

        let mut render = |ui: &mut Ui| {
            banner(ui, &state, state.banner_widths(true).1);
            batches(ui, Some((&state, &store)), false);
        };
        mem_sys::set_peer(pxui::library_counter);
        let mut ui = pxui::Ui::new(&pxui::UiDesc::DEFAULT).unwrap();
        let input = pxui::FrameIn::new(700.0, 450.0);
        for _ in 0..8 {
            ui.frame(&input, &mut render).unwrap();
        }
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                ui.frame(&input, &mut render).unwrap();
            },
        );

        store
            .undo(
                Scope::Session,
                &WriteOptions {
                    session: Some(session),
                    ..WriteOptions::default()
                },
            )
            .unwrap();
        state.refresh(Some(&store), Some(session));
        assert!(state.banner.is_none());
        assert_eq!(state.rows.len(), 1);
        assert!(!state.rows[0].live);
        store.close().unwrap();
        std::fs::remove_dir_all(folder).unwrap();
    }
}
