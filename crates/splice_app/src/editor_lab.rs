use super::{editor_session, surface::ReplyFrame};

mod examples;
use pxui::{
    A11yStates, AreaCallbacks, Button, Fill, Flags, Handle, Image, ImageStyle, Label, Node, Rect,
    Role, TextArea, Ui,
    draw::flags::rect::{BOTTOM, LEFT, TOP},
    geom::{Edges, Rgba},
    theme::Font,
    ui::{Align, Along, Bar, Grid, Look, Size, State, States, Track},
};
use splice_core::history::Store;
use splice_json::{Kind, Value as Json};
use splice_mcp::{Tool, tool_catalog};
use splice_model::{
    Access, ClipId, LaneId, MarkerId, MediaId, MediaKind, Project, SessionId, TimelineId, WordId,
    timing,
};
use std::{collections::VecDeque, os::unix::ffi::OsStringExt, path::PathBuf};

const BG: Rgba = Rgba::rgb(9, 9, 10);
const PANEL: Rgba = Rgba::rgb(15, 15, 16);
const SURFACE: Rgba = Rgba::rgb(11, 11, 12);
const LINE: Rgba = Rgba::rgb(37, 37, 39);
const INK: Rgba = Rgba::rgb(244, 243, 238);
const MUTED: Rgba = Rgba::rgb(177, 177, 171);
const QUIET: Rgba = Rgba::rgb(130, 130, 125);
const MOTION: Rgba = Rgba::rgb(191, 224, 234);
const SAGE: Rgba = Rgba::rgb(211, 228, 205);
const ACCENT: Rgba = Rgba::rgb(255, 138, 92);
const ERROR: Rgba = Rgba::rgb(227, 130, 118);
const COMPACT_MAX: f32 = 820.0;
const DOCK_MAX: f32 = 1100.0;
const DOCK_RAIL: f32 = 180.0;
const WIDE_RAIL: f32 = 200.0;
const WIDE_SIDE: f32 = 320.0;
const LAB_PAD: f32 = 12.0;
const TOOL_ROW: f32 = 44.8;
const TOOL_LOOK: States = States::of(Look::NONE.text(INK))
    .hover(Look::NONE.fill(Rgba::rgb(34, 34, 36)).text(INK))
    .checked(
        Look::NONE
            .fill(Rgba::rgb(42, 42, 45))
            .text(INK)
            .border(ACCENT, 2.0)
            .sides(LEFT),
    )
    .focus(Look::NONE.border(ACCENT, 1.0).text(INK));
const CALL_LOOK: States = States::of(Look::NONE.text(INK))
    .hover(Look::NONE.fill(Rgba::rgb(34, 34, 36)).text(INK))
    .checked(Look::NONE.fill(Rgba::rgb(42, 42, 45)).text(INK))
    .focus(Look::NONE.border(ACCENT, 1.0).text(INK));
const RUN_LOOK: States = States::of(Look::NONE.fill(INK).text(BG))
    .hover(Look::NONE.fill(Rgba::rgb(236, 230, 206)).text(BG))
    .pressed(Look::NONE.fill(Rgba::rgb(236, 230, 206)).text(BG))
    .focus(Look::NONE.fill(INK).border(ACCENT, 1.0).text(BG))
    .disabled(Look::NONE.fill(SURFACE).text(QUIET));
const CONTROL: Look = Look::NONE
    .fill(Rgba::rgb(20, 20, 21))
    .border(LINE, 1.0)
    .text(INK);
const ACTION_LOOK: States = States::of(CONTROL)
    .hover(CONTROL.fill(Rgba::rgb(34, 34, 36)))
    .focus(CONTROL.border(ACCENT, 1.0));
const SCHEMA_LOOK: States = States::of(Look::NONE.text(QUIET))
    .hover(Look::NONE.text(INK))
    .focus(Look::NONE.text(INK).border(ACCENT, 1.0));
const FILL: Node = Node::column().w(Size::Grow).h(Size::Grow);
const BLOCK: Node = Node::column().w(Size::Grow).pad(LAB_PAD).gap(8.0);
const BAR: Node = Node::row()
    .w(Size::Grow)
    .h_px(30.0)
    .pad_x(8.0)
    .gap(8.0)
    .align(Align::Center)
    .look(Look::NONE.fill(PANEL).border(LINE, 1.0).sides(BOTTOM));
const HEADING: Label = super::EDITOR_HEADING.size(18.0).w(Size::Grow).ellipsis();
const CAPTION: Label = Label::MONO
    .size(10.0)
    .line_height(14.0)
    .w(Size::Grow)
    .wrap();
const SECTION: Label = CAPTION.caps(true).tracking(0.06);
const BODY: Label = Label::MONO
    .size(12.0)
    .line_height(16.8)
    .w(Size::Grow)
    .wrap();
const CODE: Label = Label::MONO
    .size(11.0)
    .line_height(17.05)
    .w(Size::Grow)
    .wrap_glyph();
const REPLY_NOTE: Node = Node::column()
    .w(Size::Grow)
    .padding(Edges::new(8.0, 4.0, 8.0, 4.0));
const SMALL: Label = Label::MONO.size(11.0);
const REQUEST: Look = Look::NONE
    .fill(SURFACE)
    .text(INK)
    .font(Font::BODY.size(11.0));
const MAX_DRAFT_BYTES: usize = 16 << 20;
// ponytail: cap the on-screen reply at 32 KiB until large replies need virtualized viewing.
const REPLY_PREVIEW_BYTES: usize = 32 << 10;
const CALL_LOG_LIMIT: usize = 30;
const CALL_LOG_ARG_BYTES: usize = 32 << 20;

pub(super) struct Lab {
    selected: usize,
    drafts: Vec<Option<String>>,
    examples: examples::Examples,
    request_error: Option<String>,
    calls: VecDeque<LoggedCall>,
    shown: Option<u64>,
    next_call: u64,
    demo_checked: Option<([u64; 2], bool)>,
    demo_ready: bool,
    demo_start: Option<bool>,
    demo: Option<Demo>,
    demo_note: Option<&'static str>,
    demo_unavailable: Option<&'static str>,
    demo_error: Option<String>,
}

struct LoggedCall {
    id: u64,
    args: Option<String>,
    recall_label: String,
    reply: Reply,
}

struct Reply {
    name: &'static str,
    error: bool,
    summary: String,
    diff: Vec<String>,
    remaining: usize,
    preview: String,
    truncated: bool,
    frames: Vec<ReplyFrame>,
}

pub(super) struct Call {
    pub(super) name: &'static str,
    pub(super) args: Json,
}

// The request/reply pane is independent of the tool rail and activity dock. On compact
// layouts the whole Lab body is the corresponding scroll area.
#[derive(Clone, Copy, Default)]
pub(super) struct Scroll {
    offset: f32,
    request_top: Option<f32>,
    compact: bool,
    restoring: bool,
    recalling: bool,
}

impl Scroll {
    pub(super) fn restore(&mut self) -> bool {
        self.restoring = self.offset > 0.0;
        self.recalling = false;
        self.restoring
    }

    /// Uses the settled layout after Ui::frame, including any real scroll input in that frame.
    /// Returns whether restoring or anchoring changed the offset and needs another paint.
    pub(super) fn settle(&mut self, ui: &mut Ui, intents: &Intents) -> bool {
        let Some((pane, compact)) = intents.viewport else {
            return false;
        };
        let bounds = ui.rect(&pane);
        if bounds.h <= 0.0 {
            return false;
        }
        let scroll = ui.scroll(&pane);
        let request = intents.request.map(|handle| ui.rect(&handle));
        let request_top = request.map(|rect| rect.y - bounds.y + scroll.painted);
        let target = if compact != self.compact {
            None
        } else if self.restoring {
            Some(self.offset)
        } else if self.recalling {
            // v13 keeps the visible request in place when recall reflows the tool description.
            request_top
                .zip(self.request_top)
                .map(|(new, old)| self.offset + new - old)
        } else {
            None
        };
        let offset = if let Some(target) = target
            && (target - scroll.settled).abs() > 0.01
        {
            ui.set_scroll_offset(&pane, target.max(0.0));
            ui.scroll(&pane).settled
        } else {
            scroll.settled
        };
        self.offset = offset;
        self.compact = compact;
        self.request_top = request_top
            .zip(request)
            .filter(|(top, rect)| *top - offset < bounds.h && *top - offset + rect.h > 0.0)
            .map(|(top, _)| top);
        self.restoring = false;
        self.recalling = offset > 0.0
            && self.request_top.is_some()
            && matches!(intents.action, Some(Action::Recall(_)));
        offset != scroll.settled
    }
}

#[derive(Default)]
pub(super) struct Intents {
    draft: Option<String>,
    action: Option<Action>,
    viewport: Option<(Handle, bool)>,
    request: Option<Handle>,
    pub(super) session: Option<editor_session::Action>,
}

impl Intents {
    pub(super) fn merge(&mut self, next: Self) {
        // PXUI may rebuild without events to settle the frame's layout.
        if next.draft.is_some() {
            self.draft = next.draft;
        }
        if next.action.is_some() {
            self.action = next.action;
        }
        if next.session.is_some() {
            self.session = next.session;
        }
        self.viewport = next.viewport.or(self.viewport);
        self.request = next.request.or(self.request);
    }

    pub(super) fn changed(&self) -> bool {
        self.draft.is_some() || self.action.is_some() || self.session.is_some()
    }
}

enum Action {
    Choose(usize),
    Recall(u64),
    Reset,
    Replace(String),
    Invalid(String),
    Run(Call),
    Demo(bool),
}

impl Default for Lab {
    fn default() -> Self {
        Self::new()
    }
}

impl Lab {
    pub(super) fn new() -> Self {
        let tools = tool_catalog();
        Self {
            selected: tools
                .iter()
                .position(|tool| tool.name == "edit.apply")
                .unwrap_or(0),
            drafts: vec![None; tools.len()],
            examples: examples::Examples::new(),
            request_error: None,
            calls: VecDeque::new(),
            shown: None,
            next_call: 0,
            demo_checked: None,
            demo_ready: false,
            demo_start: None,
            demo: None,
            demo_note: None,
            demo_unavailable: None,
            demo_error: None,
        }
    }

    pub(super) fn choose(&mut self, index: usize) {
        if index < self.drafts.len() && index != self.selected {
            self.selected = index;
            self.request_error = None;
        }
    }

    pub(super) fn set_draft(&mut self, draft: String) {
        if draft.len() > MAX_DRAFT_BYTES {
            self.request_error = Some("Request exceeds the 16 MiB text editor limit.".into());
            return;
        }
        self.drafts[self.selected] = Some(draft);
        self.request_error = None;
    }

    pub(super) fn apply(&mut self, intents: Intents) -> Option<Call> {
        if let Some(draft) = intents.draft {
            self.set_draft(draft);
        }
        match intents.action? {
            Action::Choose(index) => self.choose(index),
            Action::Recall(id) => {
                if let Some(call) = self.calls.iter().find(|call| call.id == id)
                    && let Some(args) = &call.args
                    && let Some(index) = tool_catalog()
                        .iter()
                        .position(|tool| tool.name == call.reply.name)
                {
                    let draft = args.clone();
                    self.choose(index);
                    self.set_draft(draft);
                    self.shown = Some(id);
                }
            }
            Action::Reset => {
                self.drafts[self.selected] = None;
                self.examples.invalidate();
                self.request_error = None;
            }
            Action::Replace(draft) => self.set_draft(draft),
            Action::Invalid(message) => self.request_error = Some(message),
            Action::Demo(watch) => self.demo_start = Some(watch),
            Action::Run(call) => {
                self.request_error = None;
                return Some(call);
            }
        }
        None
    }

    pub(super) fn refresh(
        &mut self,
        store: Option<&Store>,
        at: i32,
        end: i32,
        item: Option<TimelineId>,
        lane: Option<LaneId>,
    ) {
        self.examples.refresh(store, at, end, item, lane);
        self.refresh_demo(store);
    }

    pub(super) fn observe_viewer_jobs(&mut self, request: &Json, reply: &Json) {
        if request.field("command").and_then(Json::as_str) == Some("tool")
            && let Some(name) = request.field("name").and_then(Json::as_str)
        {
            let (payload, error) = tool_reply(reply);
            if !error {
                self.examples.observe_jobs(name, payload);
            }
        }
    }

    fn refresh_demo(&mut self, store: Option<&Store>) {
        self.demo_ready = false;
        self.demo_unavailable = None;
        let Some(store) = store else {
            self.demo_checked = None;
            self.demo_unavailable = Some("Open the sample project to run the demo.");
            return;
        };
        if self.demo.is_some() {
            return;
        }
        if let Some(reason) = demo_blocked(store) {
            self.demo_unavailable = Some(reason);
            return;
        }
        let project = store.project();
        let version = [project.rev, project.hash];
        let compatible = match self.demo_checked {
            Some((cached, compatible)) if cached == version => compatible,
            _ => {
                let compatible = demo_seed(project).is_some();
                self.demo_checked = Some((version, compatible));
                compatible
            }
        };
        self.demo_ready = compatible;
        self.demo_unavailable = (!compatible).then_some(DEMO_SEED_MISSING);
    }

    pub(super) fn demo_deadline(&self) -> Option<f64> {
        self.demo.as_ref().and_then(|demo| demo.due)
    }

    pub(super) fn poll_demo(&mut self, now: f64, store: Option<&Store>, end: i32) -> Option<Call> {
        let Some(store) = store else {
            self.demo_start = None;
            if self.demo.take().is_some() {
                self.demo_note = Some("Demo stopped because the project closed.");
            }
            return None;
        };
        if let Some(watch) = self.demo_start.take()
            && self.demo.is_none()
        {
            self.demo_error = None;
            if let Some(reason) = demo_blocked(store) {
                self.demo_note = Some(reason);
            } else if let Some(seed) = demo_seed(store.project()) {
                self.demo = Some(Demo::new(seed, store.project().rev, now, watch));
                self.demo_ready = false;
                self.demo_note = Some("Demo running");
            } else {
                self.demo_note = Some(DEMO_SEED_MISSING);
            }
        }
        let demo = self.demo.as_mut()?;
        if !demo.due.is_some_and(|due| due <= now) {
            return None;
        }
        let mut open = store.sessions().iter().filter(|session| session.open);
        if store.project().access != Access::Live
            || store.problem().is_some()
            || store.project().rev != demo.revision
            || open.next().map(|session| session.id) != demo.session
            || open.next().is_some()
        {
            self.demo = None;
            self.demo_note = Some("Demo stopped because the project or session changed.");
            return None;
        }
        let call = demo.call(end);
        if call.is_none() {
            self.demo = None;
            self.demo_note = Some("Demo stopped because a frame or session is unavailable.");
        }
        call
    }

    pub(super) fn demo_replied(&mut self, now: f64, reply: &Json) {
        let Some(demo) = self.demo.as_mut().filter(|demo| demo.due.is_none()) else {
            return;
        };
        let (payload, error) = tool_reply(reply);
        if error {
            self.demo = None;
            self.demo_note = Some("Demo stopped. See Call log for the tool reply.");
            if let Some(call) = self.calls.front().filter(|call| call.reply.error) {
                self.demo_error = Some(format!("Demo stopped. {}", call.reply.summary));
            }
        } else if demo.step == DemoStep::Start
            && payload.field("resumed").and_then(Json::as_bool) == Some(true)
        {
            self.demo = None;
            self.demo_note =
                Some("Demo stopped because the start call resumed an existing session.");
        } else if !demo.advance(now, payload) {
            self.demo = None;
            self.demo_note = Some("Demo stopped because a tool returned an unexpected result.");
        } else if demo.step == DemoStep::Done {
            self.demo = None;
            self.demo_note =
                Some("Demo complete. Undo in the session banner reverts both batches.");
        }
    }

    // Accept the public tool result and the window thread's private reply envelope.
    pub(super) fn set_reply(&mut self, name: &'static str, args: &Json, reply: &Json) {
        let (payload, mut error) = tool_reply(reply);
        if !error {
            self.examples.observe_jobs(name, payload);
        }
        let mut compact = String::new();
        let (preview, truncated) = match splice_json::write(payload, &mut compact) {
            Ok(()) => reply_preview(&compact),
            Err(reason) => {
                error = true;
                (
                    format!("Could not display the structured reply. {reason}"),
                    false,
                )
            }
        };
        let (summary, diff, remaining) = result_feedback(name, error, payload, &compact);
        let frames = if name == "view.frames" && !error {
            reply_frames(payload)
        } else {
            Vec::new()
        };
        let mut arguments = String::new();
        let args = (splice_json::write(args, &mut arguments).is_ok()
            && arguments.len() <= MAX_DRAFT_BYTES
            && arguments.capacity() <= CALL_LOG_ARG_BYTES)
            .then_some(arguments);
        let bytes = args.as_ref().map_or(0, String::capacity);
        while self.calls.len() == CALL_LOG_LIMIT
            || self
                .calls
                .iter()
                .map(|call| call.args.as_ref().map_or(0, String::capacity))
                .sum::<usize>()
                + bytes
                > CALL_LOG_ARG_BYTES
        {
            self.calls.pop_back();
        }
        let id = self.next_call;
        self.next_call += 1;
        self.calls.push_front(LoggedCall {
            id,
            args,
            recall_label: format!("Load {name} arguments"),
            reply: Reply {
                name,
                error,
                summary,
                diff,
                remaining,
                preview,
                truncated,
                frames,
            },
        });
        self.shown = Some(id);
    }

    fn shown_call(&self) -> Option<&LoggedCall> {
        self.calls.iter().find(|call| Some(call.id) == self.shown)
    }

    fn reply(&self) -> Option<&Reply> {
        self.shown_call().map(|call| &call.reply)
    }

    pub(super) fn frame_reply(&self) -> Option<(u64, &[ReplyFrame])> {
        let call = self.shown_call()?;
        (!call.reply.frames.is_empty()).then_some((call.id, call.reply.frames.as_slice()))
    }

    fn draft(&self) -> &str {
        self.drafts[self.selected]
            .as_deref()
            .unwrap_or(&self.examples.get(self.selected).text)
    }

    fn example_problem(&self) -> Option<&'static str> {
        if self.drafts[self.selected].is_some() {
            return None;
        }
        self.examples.get(self.selected).missing
    }
}

fn tool_reply(reply: &Json) -> (&Json, bool) {
    let result = reply
        .field("result")
        .filter(|result| result.field("structuredContent").is_some())
        .unwrap_or(reply);
    let payload = result.field("structuredContent").unwrap_or(result);
    let error = result
        .field("isError")
        .and_then(Json::as_bool)
        .unwrap_or_else(|| payload.field("error").is_some() || payload.field("code").is_some());
    (payload, error)
}

fn reply_frames(payload: &Json) -> Vec<ReplyFrame> {
    if let Some(path) = payload
        .field("sheet")
        .and_then(Json::as_str)
        .and_then(frame_path)
    {
        return vec![ReplyFrame {
            path,
            at: None,
            aspect: 1.0,
        }];
    }
    payload
        .field("frames")
        .and_then(Json::as_array)
        .unwrap_or_default()
        .iter()
        .take(16)
        .filter_map(|frame| {
            let path = frame_path(frame.field("uri")?.as_str()?)?;
            let at = i32::try_from(frame.field("frame")?.as_i64()?).ok()?;
            let width = u32::try_from(frame.field("width")?.as_u64()?).ok()?;
            let height = u32::try_from(frame.field("height")?.as_u64()?).ok()?;
            if width == 0 || height == 0 {
                return None;
            }
            Some(ReplyFrame {
                path,
                at: Some(at),
                aspect: width as f32 / height as f32,
            })
        })
        .collect()
}

fn frame_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?.as_bytes();
    if path.first() != Some(&b'/') || path.len() > 8192 {
        return None;
    }
    let mut decoded = Vec::with_capacity(path.len());
    let mut bytes = path.iter().copied();
    while let Some(byte) = bytes.next() {
        let byte = if byte == b'%' {
            let hi = char::from(bytes.next()?).to_digit(16)?;
            let lo = char::from(bytes.next()?).to_digit(16)?;
            (hi * 16 + lo) as u8
        } else {
            byte
        };
        if byte == 0 {
            return None;
        }
        decoded.push(byte);
    }
    let path = PathBuf::from(std::ffi::OsString::from_vec(decoded));
    (path.extension().and_then(|ext| ext.to_str()) == Some("png")).then_some(path)
}

struct DemoSeed {
    hash: u64,
    words: [WordId; 4],
}

const DEMO_SEED_MISSING: &str =
    "The demo requires the sample take, b-roll, Demo beat and saved transcript.";

fn demo_blocked(store: &Store) -> Option<&'static str> {
    if store.problem().is_some() {
        Some("The demo requires a writable project. Inspect its journal first.")
    } else if store.project().access != Access::Live {
        Some("The demo requires Live agent access.")
    } else if store.sessions().iter().any(|session| session.open) {
        Some(
            "End the open agent session before running the demo. The demo uses its own undo history.",
        )
    } else {
        None
    }
}

fn demo_seed(project: &Project) -> Option<DemoSeed> {
    if project.fps != 30 {
        return None;
    }
    let take = "clp_take".parse::<ClipId>().ok()?;
    let screen = "med_screen".parse::<MediaId>().ok()?;
    let broll = "med_broll".parse::<MediaId>().ok()?;
    let beat = "mrk_2".parse::<MarkerId>().ok()?;
    let slot = project.clips.find(take)?;
    let clip = project.clips.timing(slot)?;
    if project.clips.data(slot)?.media != Some(screen)
        || (clip.start, clip.source, clip.length, clip.speed) != (120, 0, 420, 1.0)
        || !project
            .media
            .iter()
            .any(|m| m.id == screen && m.kind == MediaKind::Take)
        || !project.media.iter().any(|m| {
            m.id == broll
                && m.kind == MediaKind::Video
                && f64::from(m.probe.frames) >= m.probe.fps * 2.0
        })
        || !project
            .markers
            .iter()
            .any(|marker| marker.id == beat && marker.at == 372 && marker.name == "Demo beat")
    {
        return None;
    }
    let transcript = project.transcripts.iter().find(|t| t.media == screen)?;
    let word = |id: &str, text: &str, from, to| {
        let id = id.parse::<WordId>().ok()?;
        transcript
            .words
            .iter()
            .find(|word| word.id == id && word.text == text && word.from == from && word.to == to)
            .map(|word| word.id)
    };
    let words = [
        word("w:screen:12", "palette", 165, 180)?,
        word("w:screen:20", "open", 307, 316)?,
        word("w:screen:7", "Um", 97, 102)?,
        word("w:screen:31", "Uh", 482, 487)?,
    ];
    // Match the visible occurrences the demo's word anchors will actually resolve.
    let span = |word| {
        timing::word_span(project, word, Some(transcript.hash), None)
            .ok()
            .flatten()
    };
    let saved = span(words[1])?;
    if span(words[0])?.clip != take
        || saved.clip != take
        || span(words[2])?.clip != take
        || span(words[3]).is_some()
        || project
            .markers
            .iter()
            .any(|marker| marker.at == saved.start)
    {
        return None;
    }
    Some(DemoSeed {
        hash: transcript.hash,
        words,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DemoStep {
    Watch,
    Start,
    Transcript,
    Preview,
    Commit,
    Cut,
    Show,
    Frames,
    Done,
}

struct Demo {
    seed: DemoSeed,
    revision: u64,
    step: DemoStep,
    session: Option<SessionId>,
    playhead: Option<i32>,
    due: Option<f64>,
}

impl Demo {
    fn new(seed: DemoSeed, revision: u64, now: f64, watch: bool) -> Self {
        Self {
            seed,
            revision,
            step: if watch {
                DemoStep::Watch
            } else {
                DemoStep::Start
            },
            session: None,
            playhead: None,
            due: Some(now),
        }
    }

    fn call(&mut self, end: i32) -> Option<Call> {
        self.due.take()?;
        let [palette, save, first, second] = self.seed.words;
        let hash = self.seed.hash;
        let (name, args) = match self.step {
            DemoStep::Watch => ("ui.show", r#"{"projectId":"prj_1","view":"editor"}"#.into()),
            DemoStep::Start => (
                "session.start",
                r#"{"projectId":"prj_1","agent":"Agent Lab demo"}"#.into(),
            ),
            DemoStep::Transcript => (
                "transcript.get",
                r#"{"projectId":"prj_1","clip":"clp_take","limit":20}"#.into(),
            ),
            DemoStep::Preview | DemoStep::Commit => {
                let session = self.session?;
                let rev = self.revision;
                let options = if self.step == DemoStep::Preview {
                    r#""dryRun":true"#.to_owned()
                } else {
                    format!(
                        r#""summary":"Callouts for the palette and save beats","idempotencyKey":"demo-callouts-{rev}""#
                    )
                };
                (
                    "edit.apply",
                    format!(
                        r#"{{"projectId":"prj_1","sessionId":"{session}","ifRevision":"r{rev}",
                    {options},
                    "ops":[{{"cmd":"title.add","at":{{"word":"{palette}","transcriptHash":"{hash:016x}"}},"length":60,"text":"⌘K · command palette"}},
                    {{"cmd":"overlay.add","kind":"media","media":"med_broll","at":{{"marker":"mrk_2"}},"length":60,"placement":"corner-br"}},
                    {{"cmd":"marker.add","at":{{"word":"{save}","transcriptHash":"{hash:016x}"}},"name":"Save beat"}}]}}"#
                    ),
                )
            }
            DemoStep::Cut => {
                let session = self.session?;
                let rev = self.revision;
                (
                    "edit.apply",
                    format!(
                        r#"{{"projectId":"prj_1","sessionId":"{session}","ifRevision":"r{rev}",
                    "summary":"Cut filler words","ops":[{{"cmd":"cut.words","words":["{first}","{second}"],"transcriptHash":"{hash:016x}"}}]}}"#
                    ),
                )
            }
            DemoStep::Show => (
                "ui.show",
                format!(
                    r#"{{"projectId":"prj_1","at":{{"word":"{palette}","transcriptHash":"{hash:016x}"}},"select":[]}}"#
                ),
            ),
            DemoStep::Frames => {
                let at = self.playhead?.min(end.checked_sub(1)?);
                let second = at.saturating_add(90).min(end - 1);
                if at < 0 {
                    return None;
                }
                (
                    "view.frames",
                    format!(r#"{{"projectId":"prj_1","at":[{at},{second}]}}"#),
                )
            }
            DemoStep::Done => return None,
        };
        Some(Call {
            name,
            args: splice_json::parse(&args).expect("demo arguments"),
        })
    }

    fn advance(&mut self, now: f64, payload: &Json) -> bool {
        let revision = payload
            .field("revision")
            .and_then(Json::as_str)
            .and_then(|value| value.strip_prefix('r'))
            .and_then(|value| value.parse::<u64>().ok());
        let applied = payload.field("applied").and_then(Json::as_bool) == Some(true);
        let dry_run = payload.field("dryRun").and_then(Json::as_bool);
        let (next, pause) = match self.step {
            DemoStep::Watch if applied => (DemoStep::Start, 0.0),
            DemoStep::Start
                if revision == Some(self.revision)
                    && payload.field("resumed").and_then(Json::as_bool) != Some(true) =>
            {
                self.session = payload
                    .field("sessionId")
                    .and_then(Json::as_str)
                    .and_then(|id| id.parse().ok());
                if self.session.is_none() {
                    return false;
                }
                (DemoStep::Transcript, 0.6)
            }
            DemoStep::Transcript
                if payload
                    .field("hash")
                    .and_then(Json::as_str)
                    .and_then(|hash| u64::from_str_radix(hash, 16).ok())
                    == Some(self.seed.hash) =>
            {
                (DemoStep::Preview, 0.5)
            }
            DemoStep::Preview if revision == Some(self.revision) && dry_run == Some(true) => {
                (DemoStep::Commit, 0.7)
            }
            DemoStep::Commit | DemoStep::Cut
                if revision.is_some()
                    && revision == self.revision.checked_add(1)
                    && dry_run == Some(false)
                    && payload.field("replayed").and_then(Json::as_bool) != Some(true) =>
            {
                self.revision = revision.expect("checked revision");
                if self.step == DemoStep::Commit {
                    (DemoStep::Cut, 0.9)
                } else {
                    (DemoStep::Show, 0.7)
                }
            }
            DemoStep::Show if applied => {
                self.playhead = payload
                    .field("playhead")
                    .and_then(Json::as_i64)
                    .and_then(|at| i32::try_from(at).ok());
                if self.playhead.is_none() {
                    return false;
                }
                (DemoStep::Frames, 0.3)
            }
            DemoStep::Frames
                if payload
                    .field("frames")
                    .and_then(Json::as_array)
                    .is_some_and(|frames| !frames.is_empty()) =>
            {
                (DemoStep::Done, 0.0)
            }
            _ => return false,
        };
        self.step = next;
        self.due = (next != DemoStep::Done).then_some(now + pause);
        true
    }
}

pub(super) fn paint(
    ui: &mut Ui,
    context: Option<(u64, Option<SessionId>)>,
    lab: &Lab,
    images: &[Option<Image>],
    session: Option<(&editor_session::State, &Store)>,
    titlebar_height: f32,
) -> Intents {
    let size = ui.view_size();
    let tool = &tool_catalog()[lab.selected];
    let mut intents = Intents::default();
    let mut chosen = None;
    let pane = Look::NONE.fill(PANEL);
    if size.width <= COMPACT_MAX {
        let view = pane_scroll(ui, ("lab-compact", "Agent Lab"), FILL, &mut |ui| {
            ui.column(Node::column().w(Size::Grow).look(pane), |ui| {
                rail_header(ui);
                chosen = tool_list(ui, lab.selected, true);
                ui.node(
                    "rail-divider",
                    Node::DEFAULT
                        .w(Size::Grow)
                        .h_px(1.0)
                        .look(Look::NONE.fill(LINE)),
                );
            });
            header(ui, session, tool);
            detail(ui, context, lab, tool, images, &mut intents);
            ui.column(
                Node::column()
                    .w(Size::Grow)
                    .look(pane.border(LINE, 1.0).sides(TOP)),
                |ui| {
                    ui.node("activity-divider", Node::DEFAULT.h_px(1.0));
                    activity(ui, session, lab, &mut intents);
                },
            );
        });
        intents.viewport = Some((view, true));
    } else {
        let wide = size.width > DOCK_MAX;
        let rail = if wide { WIDE_RAIL } else { DOCK_RAIL };
        ui.column(FILL, |ui| {
            ui.row(FILL.gap(1.0).look(Look::NONE.fill(LINE)), |ui| {
                pane_scroll(
                    ui,
                    ("lab-tools", "MCP tools"),
                    FILL.w_px(rail - 1.0).look(pane),
                    &mut |ui| {
                        rail_header(ui);
                        chosen = tool_list(ui, lab.selected, false);
                    },
                );
                ui.column(FILL.look(Look::NONE.fill(BG)), |ui| {
                    header(ui, session, tool);
                    let view = pane_scroll(
                        ui,
                        ("lab-detail", "Agent request and result"),
                        FILL,
                        &mut |ui| {
                            detail(ui, context, lab, tool, images, &mut intents);
                        },
                    );
                    intents.viewport = Some((view, false));
                });
                if wide {
                    pane_scroll(
                        ui,
                        ("lab-reference", "Session activity"),
                        FILL.w_px(WIDE_SIDE - 1.0).look(pane),
                        &mut |ui| activity(ui, session, lab, &mut intents),
                    );
                }
            });
            if !wide {
                // The dock fills its content up to half the body. Longer logs scroll
                // independently so both rows of the desktop layout remain reachable.
                let limit = ((size.height - titlebar_height + 1.0) * 0.5).max(0.0);
                pane_scroll(
                    ui,
                    ("lab-history", "Session activity"),
                    FILL.h(Size::Fit)
                        .max_h(limit)
                        .look(pane.border(LINE, 1.0).sides(TOP)),
                    &mut |ui| {
                        ui.node("activity-divider", Node::DEFAULT.h_px(1.0));
                        activity(ui, session, lab, &mut intents);
                    },
                );
            }
        });
    }
    if let Some(index) = chosen {
        intents.action = Some(Action::Choose(index));
    }
    intents
}

fn pane_scroll(
    ui: &mut Ui,
    (key, name): (&'static str, &str),
    node: Node,
    draw: &mut dyn FnMut(&mut Ui),
) -> Handle {
    let viewport = ui.node(
        key,
        node.flags(Flags::SCROLL | Flags::CLIP | Flags::OVER_THUMB | Flags::BLOCKING)
            .role(Role::ScrollArea)
            .bar(Bar::new(Rgba::rgb(56, 56, 59), 4.0).gap(1.0)),
    );
    ui.a11y_name(&viewport, name);
    ui.within(&viewport, draw);
    viewport
}

fn activity(
    ui: &mut Ui,
    session: Option<(&editor_session::State, &Store)>,
    lab: &Lab,
    intents: &mut Intents,
) {
    side_heading(ui, "Agent session \u{b7} activity");
    ui.column(
        Node::column()
            .w(Size::Grow)
            .padding(Edges::new(8.0, 0.0, 8.0, 12.0)),
        |ui| {
            if session.is_some_and(|(state, _)| state.has_activity()) {
                intents.session = editor_session::batches(ui, session, false).or(intents.session);
            } else {
                let hint = ui.label("Start a session to see agent batches here.", BODY);
                tint(ui, hint, QUIET);
            }
        },
    );
    side_heading(ui, "Call log");
    let window = ui.view_size().width;
    let sidebar = if window > DOCK_MAX {
        WIDE_SIDE - 1.0
    } else {
        window
    };
    let summary_width = (sidebar - 40.0).max(1.0);
    let summary_style = CAPTION.lines(2).ellipsis();
    ui.column(Node::column().w(Size::Grow).pad_x(4.0).gap(1.0), |ui| {
        if lab.calls.is_empty() {
            let hint = ui.label("No calls yet.", BODY);
            tint(ui, hint, QUIET);
        }
        for call in &lab.calls {
            let summary = if call.args.is_some() {
                call.reply.summary.as_str()
            } else {
                "Arguments too large to reload."
            };
            let end = summary
                .char_indices()
                .nth(70)
                .map_or(summary.len(), |(end, _)| end);
            let summary = &summary[..end];
            let lines = ui
                .text_service()
                .measure(summary, summary_style, Some(summary_width))
                .map_or(1, |size| size.lines.clamp(1, 2));
            ui.id(call.id, |ui| {
                let button = Button::DEFAULT
                    .size(Size::Grow, Size::Px(24.8 + 14.0 * lines as f32))
                    .centered()
                    .content_align(Along::Start)
                    .pad(Edges::new(6.0, 4.0, 6.0, 4.0))
                    .look(&CALL_LOOK)
                    .selected(lab.shown == Some(call.id))
                    .disabled(call.args.is_none())
                    .name(&call.recall_label);
                if ui
                    .button_content("recall", &button, |ui, _| {
                        ui.column(Node::column().w(Size::Grow), |ui| {
                            ui.row(Node::row().w(Size::Grow).gap(6.0), |ui| {
                                let status = ui.label(
                                    if call.reply.error {
                                        "\u{2717}"
                                    } else {
                                        "\u{2713}"
                                    },
                                    BODY.w_px(14.0),
                                );
                                tint(ui, status, if call.reply.error { ERROR } else { SAGE });
                                let title = ui.label(call.reply.name, BODY.lines(1).ellipsis());
                                tint(ui, title, INK);
                            });
                            ui.row(
                                Node::row()
                                    .w(Size::Grow)
                                    .padding(Edges::new(20.0, 0.0, 0.0, 0.0)),
                                |ui| {
                                    let hint = ui.label(summary, summary_style);
                                    tint(ui, hint, QUIET);
                                },
                            );
                        });
                    })
                    .clicked
                {
                    intents.action = Some(Action::Recall(call.id));
                }
            });
        }
    });
}

fn side_heading(ui: &mut Ui, name: &str) {
    ui.row(
        Node::row()
            .w(Size::Grow)
            .h_px(27.0)
            .padding(Edges::new(8.0, 10.0, 8.0, 4.0)),
        |ui| {
            let heading = ui.label(name, SECTION.line_height(13.0));
            tint(ui, heading, QUIET);
        },
    );
}

fn rail_header(ui: &mut Ui) {
    ui.row(BAR.pad_x(12.0), |ui| {
        let label = ui.label_fmt(format_args!("{} tools", tool_catalog().len()), SMALL);
        tint(ui, label, QUIET);
    });
}

fn header(ui: &mut Ui, session: Option<(&editor_session::State, &Store)>, tool: &Tool) {
    ui.row(BAR, |ui| {
        let heading = ui.label(tool.name, HEADING);
        tint(ui, heading, INK);
        if let Some((session, store)) = session {
            let access = match store.project().access {
                Access::Live => "live",
                Access::Read => "read",
                Access::Off => "off",
            };
            session.lab_status(ui, access);
        }
    });
}

fn tool_list(ui: &mut Ui, selected: usize, compact: bool) -> Option<usize> {
    let mut chosen = None;
    let list = Node::column().w(Size::Grow).pad(4.0);
    let list = if compact {
        list.grid(Grid::new(&[Track::Fr(1.0), Track::Fr(1.0)], &[]))
    } else {
        list
    };
    ui.column(list, |ui| {
        let tools = tool_catalog();
        for group in ["Perception", "Session and history", "View", "Jobs"] {
            ui.column(
                Node::column()
                    .w(Size::Grow)
                    .min_h(28.0)
                    .padding(Edges::new(8.0, 10.0, 8.0, 4.0)),
                |ui| {
                    let label = ui.label(group, SECTION);
                    tint(ui, label, QUIET);
                },
            );
            let entries = tools.iter().enumerate();
            let primary = entries
                .clone()
                .filter(|(_, tool)| tool.name != "view.check");
            let extras = entries.filter(|(_, tool)| tool.name == "view.check");
            for (index, tool) in primary
                .chain(extras)
                .filter(|(_, tool)| tool_group(tool.name) == group)
            {
                let current = index == selected;
                let button = Button::DEFAULT
                    .size(Size::Grow, Size::Px(TOOL_ROW))
                    .centered()
                    .look(&TOOL_LOOK)
                    .selected(current)
                    .content_align(Along::Start)
                    .pad(Edges::new(8.0, 0.0, 8.0, 0.0))
                    .name(tool.name);
                if ui
                    .button_content(tool.name, &button, |ui, _| {
                        ui.column(Node::column().w(Size::Grow).gap(2.0), |ui| {
                            let name = ui.label(tool.name, BODY.lines(1).ellipsis());
                            tint(ui, name, INK);
                            let hint = ui.label(capability(tool.name), CAPTION.lines(1).ellipsis());
                            tint(ui, hint, if current { MUTED } else { QUIET });
                        });
                    })
                    .clicked
                {
                    chosen = Some(index);
                }
            }
        }
    });
    chosen
}

fn tool_group(name: &str) -> &'static str {
    match name {
        "session.start" | "session.end" | "edit.apply" | "history.list" | "history.undo" => {
            "Session and history"
        }
        "ui.show" | "view.check" => "View",
        "export.start" | "jobs.status" | "jobs.cancel" | "record.start" | "record.stop" => "Jobs",
        _ => "Perception",
    }
}

fn capability(name: &str) -> &'static str {
    match name {
        "project.info" => "read / open or create",
        "transcript.get" => "read / transcribe",
        "ui.show" => "edit (view only)",
        "timeline.snapshot" | "items.inspect" | "timeline.find" | "takes.events"
        | "media.analyze" | "view.frames" | "view.check" | "commands.describe" | "history.list"
        | "jobs.status" => "read",
        _ => "edit",
    }
}

fn annotation(ui: &mut Ui, text: std::fmt::Arguments<'_>) {
    ui.row(
        Node::row()
            .w(Size::Fit)
            .h_px(18.0)
            .center()
            .pad_x(6.0)
            .look(Look::NONE.border(Rgba::rgb(56, 56, 59), 1.0)),
        |ui| {
            let label = ui.label_fmt(text, CAPTION.w(Size::Fit));
            tint(ui, label, MUTED);
        },
    );
}

fn detail(
    ui: &mut Ui,
    context: Option<(u64, Option<SessionId>)>,
    lab: &Lab,
    tool: &Tool,
    images: &[Option<Image>],
    intents: &mut Intents,
) {
    ui.column(BLOCK.look(Look::NONE.fill(BG)), |ui| {
        let summary = if tool.name == "edit.apply" {
            "Atomic batch of registry commands, committed live as one undo step. Supply sessionId and ifRevision. Use dryRun for the diff and idempotencyKey for safe retries."
        } else {
            tool.description
        };
        let description = ui.label(summary, BODY);
        tint(ui, description, MUTED);
        ui.row(Node::row().w(Size::Grow).gap(4.0).row_gap(4.0).wrap(), |ui| {
            let hint = match capability(tool.name) {
                "read" => "readOnlyHint",
                "edit" if tool.name == "edit.apply" => "destructiveHint: per op",
                other => other,
            };
            annotation(ui, format_args!("{hint}"));
            if tool.input.iter().any(|(key, _)| *key == "idempotencyKey") {
                annotation(ui, format_args!("idempotentHint (with key)"));
            }
            if let Some((rev, _)) = context {
                annotation(ui, format_args!("rev r{rev}"));
            }
        });
        let schema_open = request(ui, context, lab, tool, intents);
        paint_reply(ui, lab.reply(), images);
        if schema_open {
            reference(ui, tool);
        }
    });
}

fn tint(ui: &mut Ui, label: Handle, color: Rgba) {
    ui.style(
        &label,
        &States::of(Look::NONE.text(color)),
        State::default(),
    );
}

fn request(
    ui: &mut Ui,
    context: Option<(u64, Option<SessionId>)>,
    lab: &Lab,
    tool: &Tool,
    intents: &mut Intents,
) -> bool {
    let source = lab.draft();
    let missing = lab.example_problem();
    let mut run = false;
    let mut reset = false;
    let mut revise = false;
    let mut start_demo = None;
    let sync_revision = tool.input.iter().any(|(field, _)| *field == "ifRevision");
    let run_width = 75.6;
    let reset_width = 133.2;
    let revision_width = 162.0;
    let demo_width = 133.2;
    let first_demo_fit = run_width
        + reset_width
        + demo_width
        + 24.0
        + if sync_revision {
            revision_width + 8.0
        } else {
            0.0
        };
    ui.row(
        Node::row().w(Size::Grow).gap(8.0).row_gap(8.0).wrap(),
        |ui| {
            run |= ui
                .button(
                    "lab-run",
                    "Run call",
                    &Button::sized(run_width, 24.0)
                        .look(&RUN_LOOK)
                        .label(BODY.w(Size::Fit))
                        .disabled(missing.is_some()),
                )
                .clicked;
            reset = ui
                .button(
                    "lab-reset",
                    "Reset to example",
                    &Button::sized(reset_width, 24.0)
                        .look(&ACTION_LOOK)
                        .label(BODY.w(Size::Fit)),
                )
                .clicked;
            if sync_revision {
                revise = ui
                    .button(
                        "lab-revision",
                        "Use current revision",
                        &Button::sized(revision_width, 24.0)
                            .look(&ACTION_LOOK)
                            .label(SMALL)
                            .disabled(context.is_none() || missing.is_some()),
                    )
                    .clicked;
            }
            if work_width(ui.view_size().width) >= first_demo_fit {
                ui.node("lab-demo-spacer", Node::DEFAULT.w(Size::Grow));
            }
            for (watch, label, width) in [
                (false, "Run demo session", demo_width),
                (true, "Run and watch in Editor", 183.6),
            ] {
                if ui
                    .button(
                        label,
                        label,
                        &Button::sized(width, 24.0)
                            .look(&ACTION_LOOK)
                            .label(BODY.w(Size::Fit))
                            .disabled(!lab.demo_ready),
                    )
                    .clicked
                {
                    start_demo = Some(watch);
                }
            }
        },
    );
    if let Some(note) = lab
        .demo_error
        .as_deref()
        .or(lab.demo_note)
        .or(lab.demo_unavailable)
    {
        let label = ui.label(note, CAPTION);
        tint(
            ui,
            label,
            if lab.demo_error.is_some() {
                ERROR
            } else {
                MUTED
            },
        );
    }
    let mut schema_open = false;
    ui.row(
        Node::row()
            .w(Size::Grow)
            .min_h(14.0)
            .align(Align::Start)
            .gap(8.0),
        |ui| {
            let caption = if let Some(reason) = missing {
                ui.label(reason, CAPTION)
            } else {
                ui.label(
                    "Request \u{b7} tools/call arguments (JSON)",
                    CAPTION.lines(1).ellipsis(),
                )
            };
            tint(ui, caption, if missing.is_some() { MUTED } else { QUIET });
            let button = ui.button(
                "lab-argument-schema",
                "Argument schema",
                &Button::sized(102.0, 14.0)
                    .label(CAPTION.w(Size::Fit))
                    .look(&SCHEMA_LOOK),
            );
            let handle = button.handle;
            let open = ui.state(&handle, 0) != 0;
            schema_open = open != button.clicked;
            if schema_open != open {
                ui.set_state(&handle, i64::from(schema_open));
            }
            let states = if schema_open {
                A11yStates::EXPANDABLE.with(A11yStates::EXPANDED)
            } else {
                A11yStates::EXPANDABLE
            };
            ui.set_role(&handle, Role::Button, states);
        },
    );
    let request = ui.column(
        Node::column().w(Size::Grow).h_px(180.0).look(REQUEST),
        |ui| {
            let edit = ui.text_area(
                tool.name,
                source,
                FILL.look(REQUEST),
                &TextArea::DEFAULT
                    .wrap(true)
                    .name("Request arguments (JSON)"),
                &AreaCallbacks::DEFAULT,
            );
            run |= edit.submitted;
            intents.draft = edit.changed.map(str::to_owned);
        },
    );
    intents.request = Some(request);
    // TextArea paints its own background over the wrapper. Draw the perimeter last.
    ui.canvas_over(&request, |paint, rect| {
        let Rect { x, y, w, h } = rect;
        let color = Fill::new(LINE);
        for edge in [
            Rect::new(x, y, w, 1.0),
            Rect::new(x, y + h - 1.0, w, 1.0),
            Rect::new(x, y + 1.0, 1.0, h - 2.0),
            Rect::new(x + w - 1.0, y + 1.0, 1.0, h - 2.0),
        ] {
            paint.fill(edge, color);
        }
    });
    let draft = intents.draft.as_deref().unwrap_or(source);
    intents.action = if let Some(watch) = start_demo {
        Some(Action::Demo(watch))
    } else if reset {
        Some(Action::Reset)
    } else if revise && let Some((revision, session)) = context {
        let session = session.filter(|_| tool.input.iter().any(|(field, _)| *field == "sessionId"));
        Some(match with_revision(draft, revision, session) {
            Ok(draft) => Action::Replace(draft),
            Err(error) => Action::Invalid(error),
        })
    } else if run && (missing.is_none() || intents.draft.is_some()) {
        Some(match parse_request(draft) {
            Ok(args) => Action::Run(Call {
                name: tool.name,
                args,
            }),
            Err(error) => Action::Invalid(error),
        })
    } else {
        None
    };
    let error = match &intents.action {
        Some(Action::Invalid(error)) => Some(error),
        None if intents.draft.is_none() => lab.request_error.as_ref(),
        _ => None,
    };
    if let Some(error) = error {
        ui.column(
            REPLY_NOTE.look(Look::NONE.fill(SURFACE).border(ERROR, 1.0).sides(LEFT)),
            |ui| {
                let message = ui.label(error, CAPTION);
                tint(ui, message, ERROR);
            },
        );
    }
    schema_open
}

fn reference(ui: &mut Ui, tool: &Tool) {
    ui.column(
        Node::column()
            .w(Size::Grow)
            .gap(4.0)
            .padding(Edges::new(0.0, 8.0, 0.0, 0.0)),
        |ui| {
            let label = ui.label("Argument schema", SECTION);
            tint(ui, label, QUIET);
            for &(field, schema) in tool.input {
                let required = if tool.required.contains(&field) {
                    " (required)"
                } else {
                    ""
                };
                let label = ui.label_fmt(format_args!("{field}{required}: {schema}"), CODE);
                tint(ui, label, MUTED);
            }
        },
    );
}

fn work_width(window: f32) -> f32 {
    let reserved = if window > DOCK_MAX {
        WIDE_RAIL + WIDE_SIDE
    } else if window > COMPACT_MAX {
        DOCK_RAIL
    } else {
        0.0
    };
    (window - reserved - 2.0 * LAB_PAD).max(2.0)
}

fn paint_frames(ui: &mut Ui, frames: &[ReplyFrame], images: &[Option<Image>]) {
    if frames.is_empty() {
        return;
    }
    let available = work_width(ui.view_size().width);
    ui.row(
        Node::row().w(Size::Grow).gap(8.0).row_gap(8.0).wrap(),
        |ui| {
            for (index, frame) in frames.iter().enumerate() {
                let image = images.get(index).copied().flatten();
                let aspect = image.map_or(frame.aspect, |image| {
                    image.width as f32 / image.height as f32
                });
                let w = (150.0 * aspect).clamp(2.0, available);
                let h = (w / aspect).clamp(2.0, 150.0);
                ui.id(index as u64, |ui| {
                    ui.column(Node::column().w_px(w), |ui| {
                        let node = ui.node(
                            "reply-frame",
                            Node::DEFAULT
                                .w_px(w)
                                .h_px(h)
                                .look(Look::NONE.fill(SURFACE).border(LINE, 1.0))
                                .role(Role::Image),
                        );
                        ui.a11y_name(&node, "Rendered frame preview");
                        if let Some(image) = image {
                            ui.canvas(&node, move |paint, rect| {
                                let inside = Rect::new(
                                    rect.x + 1.0,
                                    rect.y + 1.0,
                                    rect.w - 2.0,
                                    rect.h - 2.0,
                                );
                                let _ = paint.image(inside, &image, &ImageStyle::DEFAULT);
                            });
                        }
                        let label = if let Some(at) = frame.at {
                            ui.label_fmt(format_args!("{at} f"), CAPTION)
                        } else {
                            ui.label("Contact sheet", CAPTION)
                        };
                        tint(ui, label, QUIET);
                        if image.is_none() {
                            let missing = ui.label("Preview unavailable", CAPTION);
                            tint(ui, missing, QUIET);
                        }
                    });
                });
            }
        },
    );
}

fn paint_reply(ui: &mut Ui, reply: Option<&Reply>, images: &[Option<Image>]) {
    let (text, ink) = if let Some(reply) = reply {
        let (status, ink) = if reply.error {
            ("Error", ERROR)
        } else {
            ("Result", MOTION)
        };
        let caption = ui.label_fmt(format_args!("{status} · {}", reply.name), CAPTION);
        tint(ui, caption, QUIET);
        ui.column(
            REPLY_NOTE.look(Look::NONE.border(ink, 2.0).sides(LEFT)),
            |ui| {
                let summary = ui.label(&reply.summary, CAPTION.lines(3).ellipsis());
                tint(ui, summary, MUTED);
            },
        );
        paint_frames(ui, &reply.frames, images);
        if !reply.diff.is_empty() {
            ui.column(
                REPLY_NOTE
                    .gap(2.0)
                    .look(Look::NONE.fill(PANEL).border(LINE, 1.0)),
                |ui| {
                    let label = ui.label("Dry-run diff", SECTION);
                    tint(ui, label, QUIET);
                    for change in &reply.diff {
                        ui.label(change, CAPTION);
                    }
                    if reply.remaining != 0 {
                        ui.label_fmt(
                            format_args!("{} more in the structured reply", reply.remaining),
                            CAPTION,
                        );
                    }
                },
            );
        }
        if reply.truncated {
            let label = ui.label("Showing the first 32 KiB of the structured reply.", BODY);
            tint(ui, label, MUTED);
        }
        (reply.preview.as_str(), ink)
    } else {
        let caption = ui.label("Result", CAPTION);
        tint(ui, caption, QUIET);
        ui.node("empty-result-mirror", Node::DEFAULT.h_px(24.0));
        ("Run the call to see its structured result.", MOTION)
    };
    let height_limit = (ui.view_size().height * 0.46).max(90.0);
    ui.scroll_view(
        "lab-reply",
        Node::column()
            .w(Size::Grow)
            .min_h(90.0)
            .max_h(height_limit)
            .pad(8.0)
            .look(Look::NONE.fill(SURFACE).border(LINE, 1.0)),
        |ui| {
            let label = ui.label(text, CODE);
            tint(ui, label, ink);
        },
    );
}

fn result_feedback(
    name: &str,
    error: bool,
    payload: &Json,
    compact: &str,
) -> (String, Vec<String>, usize) {
    let snippet = |text: &str, bytes| text[..text.floor_char_boundary(bytes)].to_owned();
    if error {
        let problem = payload.field("error").unwrap_or(payload);
        let code = problem
            .field("code")
            .and_then(Json::as_str)
            .unwrap_or("Error");
        let summary = problem.field("message").and_then(Json::as_str).map_or_else(
            || snippet(compact, 120),
            |message| format!("{code}: {}", snippet(message, 250)),
        );
        if let Some(fix) = problem.field("fix").and_then(Json::as_str) {
            return (format!("{summary} {}", snippet(fix, 160)), Vec::new(), 0);
        }
        return (summary, Vec::new(), 0);
    }
    if name != "edit.apply" {
        return (snippet(compact, 120), Vec::new(), 0);
    }
    let dry = payload.field("dryRun").and_then(Json::as_bool) == Some(true);
    let revision = payload.field("revision").and_then(Json::as_str);
    let mut summary = if dry {
        "Dry run".to_owned()
    } else if let Some(revision) = revision {
        format!("Committed {revision}")
    } else {
        "Edit result".to_owned()
    };
    if let Some(changes) = payload.field("changes").and_then(Json::as_array) {
        if let Some(text) = changes
            .first()
            .and_then(|change| change.field("summary"))
            .and_then(Json::as_str)
        {
            summary.push_str(": ");
            summary.push_str(&snippet(text, 230));
        }
        if changes.len() > 1 {
            use std::fmt::Write;
            let _ = write!(summary, " (+{} more)", changes.len() - 1);
        }
    }
    if let Some(effects) = payload
        .field("sideEffects")
        .and_then(Json::as_array)
        .filter(|effects| !effects.is_empty())
    {
        use std::fmt::Write;
        let _ = write!(summary, " · {} side effects", effects.len());
    }
    let rows = if dry {
        payload
            .field("diff")
            .and_then(Json::as_array)
            .unwrap_or(&[])
    } else {
        &[]
    };
    let mut diff = Vec::new();
    for row in rows.iter().take(4) {
        let Some(id) = row.field("id").and_then(Json::as_str) else {
            continue;
        };
        let before = row.field("before").and_then(diff_span);
        let after = row.field("after").and_then(diff_span);
        diff.push(format!(
            "{} · {} → {}",
            snippet(id, 32),
            before.as_deref().unwrap_or("—"),
            after.as_deref().unwrap_or("—")
        ));
    }
    let remaining = rows.len().saturating_sub(diff.len());
    (summary, diff, remaining)
}

fn diff_span(span: &Json) -> Option<String> {
    let from = span.field("from")?.as_i64()?;
    let to = span.field("to")?.as_i64()?;
    Some(format!("{from}–{to} f"))
}

fn parse_request(draft: &str) -> Result<Json, String> {
    if draft.len() > MAX_DRAFT_BYTES {
        return Err("Request exceeds the 16 MiB text editor limit.".into());
    }
    let args =
        splice_json::parse(draft).map_err(|error| format!("Invalid request JSON. {error}."))?;
    if !matches!(args, Json::Object(_)) {
        return Err("Request arguments must be a JSON object.".into());
    }
    Ok(args)
}

fn with_revision(draft: &str, revision: u64, session: Option<SessionId>) -> Result<String, String> {
    let mut updated = with_field(draft, "ifRevision", &format!("\"r{revision}\""))?;
    if let Some(session) = session {
        let mut encoded = String::new();
        splice_json::write(&Json::String(session.to_string()), &mut encoded)
            .map_err(|error| error.to_string())?;
        updated = with_field(&updated, "sessionId", &encoded)?;
    }
    Ok(updated)
}

fn with_field(draft: &str, field: &str, encoded: &str) -> Result<String, String> {
    let args = parse_request(draft)?;
    let fields = args.as_object().expect("parsed arguments object");
    let open = splice_json::next(draft, 0).expect("validated request");
    let mut updated = draft.to_owned();
    if let Some(index) = fields.iter().position(|(name, _)| name == field) {
        // Match the decoded key's position at the top level and replace only its value.
        let (mut depth, mut member, mut start, mut end) = (1, 0, open.end, open.end);
        loop {
            let token = splice_json::next(draft, end).expect("validated request");
            match token.kind {
                Kind::Colon if depth == 1 && member == index => {
                    start = splice_json::next(draft, token.end)
                        .expect("validated request")
                        .start;
                }
                Kind::Comma | Kind::ObjectEnd if depth == 1 && member == index => break,
                Kind::Comma if depth == 1 => member += 1,
                Kind::ObjectStart | Kind::ArrayStart => depth += 1,
                Kind::ObjectEnd | Kind::ArrayEnd => depth -= 1,
                Kind::End => unreachable!("validated member"),
                _ => {}
            }
            end = token.end;
        }
        updated.replace_range(start..end, encoded);
    } else if fields.is_empty() {
        let close = splice_json::next(draft, open.end).expect("validated request");
        updated.replace_range(
            open.end..close.start,
            &format!("\n  \"{field}\": {encoded}\n"),
        );
    } else {
        updated.insert_str(open.end, &format!("\n  \"{field}\": {encoded},"));
    }
    if updated.len() > MAX_DRAFT_BYTES {
        return Err(format!(
            "Adding {field} would exceed the 16 MiB text editor limit."
        ));
    }
    Ok(updated)
}

fn reply_preview(compact: &str) -> (String, bool) {
    let mut preview = String::with_capacity(compact.len().min(REPLY_PREVIEW_BYTES));
    let mut last = Kind::End;
    let mut depth = 0;
    let mut offset = 0;
    loop {
        let token = splice_json::next(compact, offset).expect("serialized reply");
        if token.kind == Kind::End {
            return (preview, false);
        }
        let opened = matches!(last, Kind::ObjectStart | Kind::ArrayStart);
        let closing = matches!(token.kind, Kind::ObjectEnd | Kind::ArrayEnd);
        if closing {
            depth -= 1;
        }
        if last == Kind::Comma || (closing && !opened) || (opened && !closing) {
            preview.push('\n');
            for _ in 0..depth {
                preview.push_str("  ");
            }
        } else if last == Kind::Colon {
            preview.push(' ');
        }
        let text = &compact[token.start..token.end];
        let room = REPLY_PREVIEW_BYTES.saturating_sub(preview.len());
        if text.len() > room {
            preview.truncate(REPLY_PREVIEW_BYTES.min(preview.len()));
            preview.push_str(&text[..text.floor_char_boundary(room)]);
            return (preview, true);
        }
        preview.push_str(text);
        if matches!(token.kind, Kind::ObjectStart | Kind::ArrayStart) {
            depth += 1;
        }
        offset = token.end;
        last = token.kind;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lab_stacks_request_and_reply_and_wraps_the_compact_tool_rail() {
        use pxui::{FrameIn, Role, testing};

        let lab = Lab::new();
        for width in [720.0, 960.0, 1280.0] {
            let mut ui = crate::viewer_ui().unwrap();
            ui.set_a11y(true);
            ui.frame(&FrameIn::new(width, 1200.0), &mut |ui| {
                paint(ui, None, &lab, &[], None, 0.0);
            })
            .unwrap();
            let request = testing::find(&ui, Role::TextArea, "Request arguments (JSON)").unwrap();
            let reply = testing::find(&ui, Role::Label, "Result").unwrap();
            let snapshot = testing::find(&ui, Role::Button, "timeline.snapshot").unwrap();
            let inspect = testing::find(&ui, Role::Button, "items.inspect").unwrap();
            assert_eq!(request.h, 180.0);
            let (x, y, r, b) = (
                request.x,
                request.y,
                request.x + request.w,
                request.y + request.h,
            );
            for edge in [
                [x, y, r, y + 1.0],
                [x, b - 1.0, r, b],
                [x, y + 1.0, x + 1.0, b - 1.0],
                [r - 1.0, y + 1.0, r, b - 1.0],
            ] {
                assert!(
                    ui.layers()
                        .flat_map(|layer| layer.instances.iter())
                        .any(|quad| quad
                            .rect
                            .into_iter()
                            .zip(edge)
                            .all(|(actual, expected)| (actual - expected).abs() < 0.5)
                            && quad.color == [LINE.r, LINE.g, LINE.b, LINE.a]),
                    "missing request perimeter {edge:?}"
                );
            }
            assert!((snapshot.h - 44.8).abs() < 0.01);
            assert!(
                ui.layers()
                    .flat_map(|layer| layer.instances.iter())
                    .any(|quad| quad.kind == pxui::draw::kind::MASK
                        && quad.rect[0] >= snapshot.x
                        && quad.rect[2] <= snapshot.x + snapshot.w
                        && quad.rect[1] >= snapshot.y
                        && quad.rect[3] <= snapshot.y + snapshot.h
                        && quad.color == [INK.r, INK.g, INK.b, INK.a])
            );
            assert!(
                testing::find(&ui, Role::Label, tool_catalog()[lab.selected].description).is_none()
            );
            assert_eq!(request.x, reply.x);
            assert!(reply.y >= request.y + request.h);
            if width <= 820.0 {
                assert_eq!(snapshot.y, inspect.y);
                assert!(inspect.x > snapshot.x && request.y > snapshot.y);
                assert!(request.x < 20.0);
            } else {
                assert_eq!(snapshot.x, inspect.x);
                assert!(inspect.y > snapshot.y);
                assert_eq!(request.x, if width > 1100.0 { 212.0 } else { 192.0 });
            }
        }
    }

    #[test]
    fn lab_layout_matches_the_v13_breakpoints_and_separates_scroll() {
        use pxui::{Event, FrameIn, testing};

        let lab = Lab::new();
        mem_sys::set_peer(pxui::library_counter);
        for (width, request_y, request_x) in [
            (960.0, 197.6, 192.0),
            (1280.0, 197.6, 212.0),
            (800.0, 819.0, 12.0),
            (480.0, 867.8, 12.0),
        ] {
            let compact = width <= COMPACT_MAX;
            let mut ui = crate::viewer_ui().unwrap();
            ui.set_a11y(true);
            let frame = FrameIn::new(width, 640.0);
            let draw = |ui: &mut Ui, events: &[Event]| {
                let mut intents = Intents::default();
                ui.frame(&frame.events(events), &mut |ui| {
                    ui.column(FILL, |ui| {
                        ui.node("titlebar", Node::DEFAULT.w(Size::Grow).h_px(34.0));
                        intents.merge(paint(ui, Some((0, None)), &lab, &[], None, 34.0));
                    });
                })
                .unwrap();
                intents
            };
            let near = |actual: f32, expected: f32| {
                assert!(
                    (actual - expected).abs() < 0.15,
                    "{width}: {actual} vs {expected}"
                );
            };
            let button = |ui: &Ui, name| testing::find(ui, Role::Button, name).unwrap();
            let area = |ui: &Ui, name| testing::find(ui, Role::ScrollArea, name);
            for _ in 0..8 {
                assert!(!draw(&mut ui, &[]).changed());
            }
            let intents = draw(&mut ui, &[]);
            let work = intents.viewport.unwrap().0;
            let request = intents.request.unwrap();
            let rect = ui.rect(&request);
            near(rect.x, request_x);
            near(rect.w, work_width(width));
            near(rect.y, request_y);
            near(rect.h, 180.0);

            let selected = button(&ui, "edit.apply");
            near(selected.x, 4.0);
            near(selected.y, if compact { 336.8 } else { 572.0 });
            near(selected.h, TOOL_ROW);
            for tool in tool_catalog() {
                button(&ui, tool.name);
            }
            assert!(button(&ui, "history.undo").y <= button(&ui, "session.end").y);
            let demo = button(&ui, "Run demo session");
            let run = button(&ui, "Run call");
            let watch = button(&ui, "Run and watch in Editor");
            if width == 480.0 {
                near(demo.x, run.x);
                near(demo.y, run.y + 32.0);
            } else {
                near(demo.y, run.y);
                near(watch.x + watch.w, width.min(960.0) - LAB_PAD);
            }

            let rail = if compact {
                assert!(
                    area(&ui, "MCP tools").is_none() && area(&ui, "Session activity").is_none()
                );
                assert_eq!(ui.rect(&work), area(&ui, "Agent Lab").unwrap());
                ui.rect(&work)
            } else {
                let rail = area(&ui, "MCP tools").unwrap();
                let side = area(&ui, "Session activity").unwrap();
                near(rail.y, 34.0);
                near(ui.rect(&work).y, 64.0);
                if width == 960.0 {
                    near(side.x, 0.0);
                    near(side.y, 539.4);
                    near(side.w, 960.0);
                    near(rail.y + rail.h, side.y);
                    assert!(selected.y >= rail.y + rail.h);
                } else {
                    near(side.x, 961.0);
                    near(side.y, 34.0);
                    near(side.w, 319.0);
                    near(side.h, 606.0);
                    assert!(selected.y + selected.h <= rail.y + rail.h);
                }
                rail
            };
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || assert!(!draw(&mut ui, &[]).changed()));

            let first = button(&ui, "project.info");
            let hover = Event::pointer(
                Event::POINTER_MOVED,
                0,
                (rail.x + 48.0).into(),
                (rail.y + 144.0).into(),
            );
            let wheel = Event::pointer(Event::SCROLLED, 0, 0.0, -4.0);
            assert!(!draw(&mut ui, &[hover]).changed());
            assert!(!draw(&mut ui, &[wheel]).changed());
            assert!(button(&ui, "project.info").y < first.y);
            if compact {
                assert!(ui.rect(&request).y < rect.y);
                assert!(ui.scroll(&work).settled > 0.0);
            } else {
                assert_eq!(ui.rect(&request), rect);
                assert_eq!(ui.scroll(&work).settled, 0.0);
            }

            if width == 960.0 {
                let schema = |ui: &Ui| testing::find(ui, Role::Label, "ARGUMENT SCHEMA");
                assert!(schema(&ui).is_none());
                let trigger = button(&ui, "Argument schema");
                for open in [true, false] {
                    for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
                        let click = Event::pointer(
                            kind,
                            0,
                            (trigger.x + trigger.w * 0.5).into(),
                            (trigger.y + trigger.h * 0.5).into(),
                        );
                        assert!(!draw(&mut ui, &[click]).changed());
                    }
                    assert_eq!(schema(&ui).is_some(), open);
                    assert_eq!(ui.rect(&request), rect);
                }
            }
        }
    }

    #[test]
    fn lab_activity_dock_uses_the_height_below_a_wrapped_titlebar() {
        use pxui::{FrameIn, testing};

        let mut lab = Lab::new();
        let args = splice_json::parse(r#"{"projectId":"prj_1"}"#).unwrap();
        let reply = splice_json::parse(r#"{"revision":"r0","access":"live"}"#).unwrap();
        for _ in 0..8 {
            lab.set_reply("project.info", &args, &reply);
        }
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        for (titlebar, dock) in [(34.0, 303.5), (62.0, 289.5)] {
            ui.frame(&FrameIn::new(960.0, 640.0), &mut |ui| {
                ui.column(FILL, |ui| {
                    ui.node("titlebar", Node::DEFAULT.h_px(titlebar));
                    paint(ui, Some((0, None)), &lab, &[], None, titlebar);
                });
            })
            .unwrap();
            let rail = testing::find(&ui, Role::ScrollArea, "MCP tools").unwrap();
            let side = testing::find(&ui, Role::ScrollArea, "Session activity").unwrap();
            assert_eq!(rail.y, titlebar);
            assert_eq!(side.h, dock);
            assert_eq!(rail.y + rail.h, side.y);
            assert_eq!(side.y + side.h, 640.0);
        }
    }

    #[test]
    fn dry_run_and_error_feedback_uses_structured_result() {
        use pxui::{Event, FrameIn, testing};

        let mut lab = Lab::new();
        let args = splice_json::parse(r#"{"dryRun":true}"#).unwrap();
        let dry = splice_json::parse(r#"{"result":{"isError":false,"structuredContent":{
            "revision":"r0","dryRun":true,"changes":[{"summary":"Added marker Beat"}],
            "sideEffects":[],"diff":[{"id":"mrk_1","before":null,"after":{"from":8,"to":8,"lane":null}}]
        }}}"#).unwrap();
        lab.set_reply("edit.apply", &args, &dry);
        let reply = lab.reply().unwrap();
        assert_eq!(reply.summary, "Dry run: Added marker Beat");
        let diff = "mrk_1 · — → 8–8 f";
        assert_eq!(reply.diff, [diff]);
        mem_sys::set_peer(pxui::library_counter);
        for width in [960.0, 480.0] {
            let mut ui = crate::viewer_ui().unwrap();
            ui.set_a11y(true);
            let frame = FrameIn::new(width, 606.0);
            let mut idle = || {
                ui.frame(&frame, &mut |ui| {
                    assert!(!paint(ui, Some((0, None)), &lab, &[], None, 0.0).changed());
                })
                .unwrap();
            };
            for _ in 0..8 {
                idle();
            }
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut idle);
            assert!(testing::find(&ui, Role::Label, diff).is_some());
            if width == 960.0 {
                let rect = testing::find(&ui, Role::TextArea, "Request arguments (JSON)").unwrap();
                assert_eq!((rect.x, rect.w, rect.h), (192.0, 756.0, 180.0));
                assert!((rect.y - 163.6).abs() <= 1.0, "{rect:?}");
            }
        }
        let rejected = splice_json::parse(
            r#"{"result":{"isError":true,"structuredContent":{
            "error":{"code":"STALE_REVISION","message":"Expected r1.","fix":"Refresh the revision."}
        }}}"#,
        )
        .unwrap();
        lab.set_reply("edit.apply", &args, &rejected);
        let reply = lab.reply().unwrap();
        assert_eq!(
            reply.summary,
            "STALE_REVISION: Expected r1. Refresh the revision."
        );
        assert!(reply.diff.is_empty());
        assert_eq!(lab.calls[1].reply.summary, "Dry run: Added marker Beat");
        assert_eq!(
            parse_request(lab.draft())
                .unwrap()
                .field("dryRun")
                .and_then(Json::as_bool),
            Some(true)
        );
        lab.set_draft("[]".into());
        let count = lab.calls.len();
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let frame = FrameIn::new(960.0, 606.0);
        let draw = |ui: &mut Ui, lab: &mut Lab, events: &[Event]| {
            let mut intents = Intents::default();
            ui.frame(&frame.events(events), &mut |ui| {
                intents.merge(paint(ui, Some((0, None)), lab, &[], None, 0.0));
            })
            .unwrap();
            lab.apply(intents)
        };
        for _ in 0..2 {
            assert!(draw(&mut ui, &mut lab, &[]).is_none());
        }
        let run = testing::find(&ui, Role::Button, "Run call").unwrap();
        for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
            let event = Event::pointer(
                kind,
                0,
                f64::from(run.x + run.w / 2.0),
                f64::from(run.y + run.h / 2.0),
            );
            assert!(draw(&mut ui, &mut lab, &[event]).is_none());
        }
        assert!(draw(&mut ui, &mut lab, &[]).is_none());
        assert_eq!(lab.calls.len(), count);
        assert!(
            testing::find(&ui, Role::Label, "Request arguments must be a JSON object.").is_some()
        );
    }

    #[test]
    fn settling_builds_keep_the_draft_and_run_intent() {
        let mut lab = Lab::default();
        let tool = &tool_catalog()[lab.selected];
        assert_eq!(tool.name, "edit.apply");
        let mut intents = Intents::default();
        assert!(!intents.changed());
        let session = SessionId::from_serial(2);
        let draft = with_revision(lab.draft(), 7, Some(session)).unwrap();
        let args = parse_request(&draft).unwrap();
        assert_eq!(
            args.field("sessionId"),
            Some(&Json::String(session.to_string()))
        );
        intents.merge(Intents {
            draft: Some(draft.clone()),
            action: Some(Action::Run(Call {
                name: tool.name,
                args: args.clone(),
            })),
            ..Intents::default()
        });
        intents.merge(Intents::default());
        assert!(intents.changed());
        let call = lab.apply(intents).unwrap();
        assert_eq!(call.name, tool.name);
        assert_eq!(call.args, args);
        assert_eq!(lab.draft(), draft);
    }

    #[test]
    fn live_examples_follow_real_store_changes_and_keep_explicit_drafts() {
        use pxui::{Event, FrameIn, testing};
        use splice_core::history::{Source, WriteOptions};
        use std::{fs, path::Path};

        let sample =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/sample.splice");
        let path =
            std::env::temp_dir().join(format!("splice-lab-examples-{}.splice", std::process::id()));
        fs::create_dir_all(path.join("transcripts")).unwrap();
        for file in ["project.json", "media.json", "transcripts/med_screen.json"] {
            fs::copy(sample.join(file), path.join(file)).unwrap();
        }
        fs::create_dir_all(path.join("takes/med_screen")).unwrap();
        fs::write(
            path.join("takes/med_screen/events.json"),
            r#"{"events":[]}"#,
        )
        .unwrap();
        let mut store = Store::open(&path).unwrap();
        let mut lab = Lab::new();
        let selected = Some(TimelineId::Clip("clp_take".parse().unwrap()));
        let playhead = 149;
        let end = 660;
        let refresh = |lab: &mut Lab, store: &Store| {
            lab.refresh(Some(store), playhead, end, selected, None);
        };
        let index = |name| {
            tool_catalog()
                .iter()
                .position(|tool| tool.name == name)
                .unwrap()
        };
        let args = |lab: &Lab, name| parse_request(&lab.examples.get(index(name)).text).unwrap();
        refresh(&mut lab, &store);
        assert!(lab.example_problem().is_some());
        assert!(args(&lab, "edit.apply").field("sessionId").is_none());
        assert!(lab.examples.get(index("record.stop")).missing.is_some());
        assert!(args(&lab, "record.stop").field("jobId").is_none());
        assert_eq!(
            args(&lab, "timeline.snapshot")
                .field("to")
                .and_then(Json::as_i64),
            Some(i64::from(end))
        );
        for name in ["takes.events", "media.analyze", "transcript.get"] {
            assert_eq!(
                args(&lab, name).field("clip").and_then(Json::as_str),
                Some("clp_take")
            );
        }
        assert_eq!(
            args(&lab, "items.inspect")
                .field("ids")
                .unwrap()
                .as_array()
                .unwrap()[0]
                .as_str(),
            Some("clp_take")
        );
        assert_eq!(
            args(&lab, "ui.show").field("at").and_then(Json::as_i64),
            Some(i64::from(playhead))
        );
        assert_eq!(
            args(&lab, "view.frames").field("at"),
            Some(&splice_json::parse("[119,149,179]").unwrap())
        );

        let first = store.start_session("example tester", 1).unwrap().id;
        refresh(&mut lab, &store);
        assert!(lab.example_problem().is_none());
        assert!(lab.examples.get(index("history.undo")).missing.is_some());
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                for at in [9, 98, 120, 149, 199, 372, 659] {
                    lab.refresh(Some(&store), at, end, selected, None);
                }
            },
        );
        assert!(
            lab.examples
                .get(index("ui.show"))
                .text
                .contains("\"at\": 659")
        );
        refresh(&mut lab, &store);

        let call_reply = |name, json| {
            let body = Json::Object(vec![
                ("command".into(), Json::String("tool".into())),
                ("name".into(), Json::String(String::from(name))),
            ]);
            let payload = splice_json::parse(json).unwrap();
            (body, payload)
        };
        let (request, reply) = call_reply(
            "record.start",
            r#"{"result":{"isError":false,"structuredContent":{"jobId":"job_a","kind":"record","state":"recording"}}}"#,
        );
        lab.observe_viewer_jobs(&request, &reply);
        refresh(&mut lab, &store);
        for name in ["record.stop", "jobs.status", "jobs.cancel"] {
            assert_eq!(
                args(&lab, name).field("jobId").and_then(Json::as_str),
                Some("job_a")
            );
        }
        assert_eq!(tool_catalog().len(), 21);
        for (i, tool) in tool_catalog().iter().enumerate() {
            let example = lab.examples.get(i);
            assert!(example.text.len() < 512, "{}", tool.name);
            let json = parse_request(&example.text).unwrap();
            assert_eq!(
                example.missing.is_some(),
                tool.name == "history.undo",
                "{}",
                tool.name
            );
            for (key, _) in json.as_object().unwrap() {
                assert!(
                    tool.input.iter().any(|(field, _)| *field == key),
                    "{key} in {}",
                    tool.name
                );
            }
            if example.missing.is_none() {
                for &key in tool.required {
                    assert!(
                        json.field(key).is_some(),
                        "{key} missing from {}",
                        tool.name
                    );
                }
            }
        }

        let mut state = editor_session::State::default();
        state.refresh(Some(&store), Some(first));
        let paint_lab = |ui: &mut Ui,
                         lab: &Lab,
                         store: &Store,
                         state: &editor_session::State,
                         frame: &FrameIn<'_>,
                         events: &[Event]| {
            let mut intents = Intents::default();
            ui.frame(&frame.events(events), &mut |ui| {
                intents.merge(paint(
                    ui,
                    Some((store.project().rev, state.open())),
                    lab,
                    &[],
                    Some((state, store)),
                    0.0,
                ));
            })
            .unwrap();
            intents
        };
        let warm = |lab: &mut Lab, store: &Store, state: &editor_session::State| {
            mem_sys::set_peer(pxui::library_counter);
            for width in [960.0, 480.0] {
                let mut ui = crate::viewer_ui().unwrap();
                let frame = FrameIn::new(width, 606.0);
                let mut idle = || {
                    refresh(lab, store);
                    assert!(!paint_lab(&mut ui, lab, store, state, &frame, &[]).changed());
                };
                for _ in 0..6 {
                    idle();
                }
                splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                    .warm(&mut idle);
            }
        };
        warm(&mut lab, &store, &state);

        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let frame = FrameIn::new(960.0, 606.0);
        let click = |ui: &mut Ui,
                     lab: &mut Lab,
                     store: &Store,
                     state: &editor_session::State,
                     label: &str| {
            for _ in 0..2 {
                assert!(!paint_lab(ui, lab, store, state, &frame, &[]).changed());
            }
            let hit = testing::find(ui, Role::Button, label).unwrap();
            let x = f64::from(hit.x + hit.w / 2.0);
            let y = f64::from(hit.y + hit.h / 2.0);
            let mut call = None;
            for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
                call = lab
                    .apply(paint_lab(
                        ui,
                        lab,
                        store,
                        state,
                        &frame,
                        &[Event::pointer(kind, 0, x, y)],
                    ))
                    .or(call);
            }
            call
        };
        let call = click(&mut ui, &mut lab, &store, &state, "Run call").unwrap();
        assert_eq!(call.name, "edit.apply");
        assert_eq!(
            call.args.field("ifRevision").and_then(Json::as_str),
            Some("r0")
        );
        assert_eq!(
            call.args.field("sessionId").and_then(Json::as_str),
            Some(first.to_string().as_str())
        );
        let ops = call.args.field("ops").unwrap().as_array().unwrap();
        assert_eq!(
            ops[0].field("at").and_then(Json::as_i64),
            Some(i64::from(playhead))
        );
        let mut options = WriteOptions {
            source: Source::Agent,
            session: Some(first),
            if_revision: Some(0),
            dry_run: call.args.field("dryRun").and_then(Json::as_bool).unwrap(),
            ..WriteOptions::default()
        };
        let preview = splice_mcp::apply_batch(&mut store, ops, &options).unwrap();
        assert_eq!(store.project().rev, 0);
        lab.set_reply(
            call.name,
            &call.args,
            &splice_mcp::edit_result_json(preview),
        );
        let dry_run = lab.calls.front().unwrap().id;
        warm(&mut lab, &store, &state);

        let typed = format!("  {}  ", lab.draft());
        lab.set_draft(typed.clone());
        options.dry_run = false;
        let edit = splice_mcp::apply_batch(&mut store, ops, &options).unwrap();
        assert_eq!(edit.revision, 1);
        refresh(&mut lab, &store);
        assert!(lab.examples.get(index("history.undo")).missing.is_none());
        store.end_session(first).unwrap();
        refresh(&mut lab, &store);
        assert!(lab.examples.get(index("history.undo")).missing.is_some());
        assert_eq!(
            args(&lab, "history.list")
                .field("sessionId")
                .and_then(Json::as_str),
            Some(first.to_string().as_str())
        );

        state.refresh(Some(&store), Some(first));
        assert_eq!(lab.draft(), typed);
        assert!(click(&mut ui, &mut lab, &store, &state, "Reset to example").is_none());
        refresh(&mut lab, &store);
        let ended = click(&mut ui, &mut lab, &store, &state, "Run call").unwrap();
        assert_eq!(ended.name, "edit.apply");
        assert_eq!(
            ended.args.field("sessionId").and_then(Json::as_str),
            Some(first.to_string().as_str())
        );
        let refused = splice_mcp::apply_batch(
            &mut store,
            ended.args.field("ops").unwrap().as_array().unwrap(),
            &WriteOptions {
                if_revision: Some(1),
                dry_run: true,
                ..options.clone()
            },
        )
        .unwrap_err();
        assert_eq!(refused.code, "NO_SESSION");
        assert_eq!(store.project().rev, 1);
        let reply = Json::Object(vec![
            ("isError".into(), Json::Bool(true)),
            (
                "structuredContent".into(),
                splice_mcp::error_payload(refused),
            ),
        ]);
        lab.set_reply(ended.name, &ended.args, &reply);
        assert!(lab.reply().unwrap().error);
        assert!(lab.reply().unwrap().summary.starts_with("NO_SESSION:"));
        lab.set_draft(typed.clone());

        let second = store.start_session("second", 2).unwrap().id;
        state.refresh(Some(&store), Some(second));
        refresh(&mut lab, &store);
        assert_eq!(lab.draft(), typed);
        assert!(click(&mut ui, &mut lab, &store, &state, "Reset to example").is_none());
        refresh(&mut lab, &store);
        let current = parse_request(lab.draft()).unwrap();
        assert_eq!(
            current.field("ifRevision").and_then(Json::as_str),
            Some("r1")
        );
        assert_eq!(
            current.field("sessionId").and_then(Json::as_str),
            Some(second.to_string().as_str())
        );
        assert_eq!(
            current.field("ops").unwrap().as_array().unwrap()[0]
                .field("cmd")
                .and_then(Json::as_str),
            Some("marker.set")
        );

        assert!(
            lab.apply(Intents {
                action: Some(Action::Recall(dry_run)),
                ..Intents::default()
            })
            .is_none()
        );
        refresh(&mut lab, &store);
        assert_eq!(parse_request(lab.draft()).unwrap(), call.args);

        let (request, reply) = call_reply(
            "record.stop",
            r#"{"result":{"isError":false,"structuredContent":{"jobId":"job_a","kind":"record","state":"done"}}}"#,
        );
        lab.observe_viewer_jobs(&request, &reply);
        refresh(&mut lab, &store);
        assert!(lab.examples.get(index("record.stop")).missing.is_some());
        assert!(lab.examples.get(index("jobs.cancel")).missing.is_some());
        assert_eq!(
            args(&lab, "jobs.status")
                .field("jobId")
                .and_then(Json::as_str),
            Some("job_a")
        );
        lab.choose(index("record.stop"));
        assert!(click(&mut ui, &mut lab, &store, &state, "Run call").is_none());
        assert!(
            testing::find(
                &ui,
                Role::Label,
                "Start a recording or enter the active recording's job ID.",
            )
            .is_some()
        );
        store.close().unwrap();
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn demo_uses_a_revision_key_and_explains_resumed_and_failed_starts() {
        let mut lab = Lab::new();
        let pending = || {
            Demo::new(
                DemoSeed {
                    hash: 0,
                    words: ["w:screen:12", "w:screen:20", "w:screen:7", "w:screen:31"]
                        .map(|id| id.parse().unwrap()),
                },
                7,
                0.0,
                false,
            )
        };

        let mut demo = pending();
        demo.step = DemoStep::Commit;
        demo.session = Some(SessionId::from_serial(1));
        let commit = demo.call(660).unwrap();
        assert_eq!(
            commit.args.field("idempotencyKey").and_then(Json::as_str),
            Some("demo-callouts-7")
        );
        assert_eq!(
            commit.args.field("ifRevision").and_then(Json::as_str),
            Some("r7")
        );

        let mut demo = pending();
        let start = demo.call(660).unwrap();
        lab.demo = Some(demo);
        let resumed = splice_json::parse(r#"{"result":{"isError":false,"structuredContent":{"sessionId":"ses_1","revision":"r7","resumed":true}}}"#).unwrap();
        lab.set_reply(start.name, &start.args, &resumed);
        lab.demo_replied(0.0, &resumed);
        assert!(lab.demo.is_none());
        assert_eq!(
            lab.demo_note,
            Some("Demo stopped because the start call resumed an existing session.")
        );

        let mut demo = pending();
        let start = demo.call(660).unwrap();
        lab.demo = Some(demo);
        let denied = splice_json::parse(r#"{"result":{"isError":true,"structuredContent":{"error":{"code":"ACCESS_DENIED","message":"Agent access is read-only.","fix":"Switch to Live."}}}}"#).unwrap();
        lab.set_reply(start.name, &start.args, &denied);
        lab.demo_replied(0.0, &denied);
        assert!(lab.demo.is_none());
        assert_eq!(
            lab.demo_error.as_deref(),
            Some("Demo stopped. ACCESS_DENIED: Agent access is read-only. Switch to Live.")
        );
    }

    #[test]
    fn demo_paces_real_store_batches_and_preserves_session_undo() {
        use crate::{EditorLayout, apply_view_command, editor_titlebar::View};
        use pxui::{Event, FrameIn, testing};
        use splice_core::history::{Scope, WriteOptions};
        use splice_mcp::live::Viewer;
        use std::{fs, path::Path};

        let sample =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/sample.splice");
        let path =
            std::env::temp_dir().join(format!("splice-lab-demo-{}.splice", std::process::id()));
        fs::create_dir_all(path.join("transcripts")).unwrap();
        for file in ["project.json", "media.json", "transcripts/med_screen.json"] {
            fs::copy(sample.join(file), path.join(file)).unwrap();
        }
        let mut viewer = Viewer::new(Store::open(&path).unwrap(), None);
        let base_items = viewer.store().project().items.order().len();
        let mut lab = Lab::new();
        lab.refresh_demo(Some(viewer.store()));
        assert!(
            lab.demo_ready,
            "the sample works with its media files absent"
        );

        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let frame = FrameIn::new(960.0, 606.0);
        let draw = |ui: &mut Ui, lab: &mut Lab, events: &[Event]| {
            let mut intents = Intents::default();
            ui.frame(&frame.events(events), &mut |ui| {
                intents.merge(paint(ui, Some((0, None)), lab, &[], None, 0.0));
            })
            .unwrap();
            assert!(lab.apply(intents).is_none());
        };
        draw(&mut ui, &mut lab, &[]);
        draw(&mut ui, &mut lab, &[]);
        let watch = testing::find(&ui, Role::Button, "Run and watch in Editor").unwrap();
        let x = f64::from(watch.x + watch.w * 0.5);
        let y = f64::from(watch.y + watch.h * 0.5);
        draw(
            &mut ui,
            &mut lab,
            &[Event::pointer(Event::POINTER_PRESSED, 0, x, y)],
        );
        draw(
            &mut ui,
            &mut lab,
            &[Event::pointer(Event::POINTER_RELEASED, 0, x, y)],
        );
        assert_eq!(lab.demo_start, Some(true));
        let mut layout = EditorLayout::new([960.0, 640.0]);
        layout.view = View::Agent;
        let mut now = 10.0;
        let expected = [
            ("ui.show", 0.0, 0),
            ("session.start", 0.6, 0),
            ("transcript.get", 0.5, 0),
            ("edit.apply", 0.7, 0),
            ("edit.apply", 0.9, 1),
            ("edit.apply", 0.7, 2),
            ("ui.show", 0.3, 2),
            ("view.frames", 0.0, 2),
        ];
        let mut session = None;
        for (index, (name, pause, revision)) in expected.into_iter().enumerate() {
            let call = lab.poll_demo(now, Some(viewer.store()), 600).unwrap();
            assert_eq!(call.name, name);
            assert!(lab.poll_demo(now, Some(viewer.store()), 600).is_none());
            let request = Json::Object(vec![
                ("command".into(), Json::String("tool".into())),
                ("name".into(), Json::String(name.into())),
                ("args".into(), call.args),
            ]);
            let args = request.field("args").unwrap();
            let reply = if name == "view.frames" {
                assert_eq!(
                    args.field("at"),
                    Some(&splice_json::parse("[276,366]").unwrap())
                );
                // The live viewer integration test renders on the platform main thread.
                splice_json::parse(
                    r#"{"result":{"isError":false,"structuredContent":{"revision":"r2","frames":[{"frame":276},{"frame":366}]}}}"#,
                )
                .unwrap()
            } else {
                viewer.handle(&request, 0, &mut |store, command| {
                    apply_view_command(&mut layout, None, store, command, None, None)
                })
            };
            let result = reply.field("result").expect("viewer tool response");
            assert_eq!(
                result.field("isError").and_then(Json::as_bool),
                Some(false),
                "{name}: {result:?}"
            );
            let payload = result.field("structuredContent").unwrap();
            assert_eq!(layout.view, View::Editor);
            assert_eq!(viewer.store().project().rev, revision);
            if name == "session.start" {
                session = payload
                    .field("sessionId")
                    .and_then(Json::as_str)
                    .map(|id| id.parse().unwrap());
            }
            if index == 3 {
                assert_eq!(viewer.store().project().items.order().len(), base_items);
                assert_eq!(payload.field("dryRun").and_then(Json::as_bool), Some(true));
                assert!(args.field("idempotencyKey").is_none());
            }
            if index == 4 {
                assert_eq!(
                    args.field("idempotencyKey").and_then(Json::as_str),
                    Some("demo-callouts-0")
                );
            }
            if index == 6 {
                assert_eq!(payload.field("playhead").and_then(Json::as_i64), Some(276));
            }
            lab.set_reply(call.name, args, &reply);
            lab.demo_replied(now, &reply);
            if index + 1 < expected.len() {
                assert_eq!(lab.demo_deadline(), Some(now + pause));
                assert!(
                    lab.poll_demo(now + pause - 0.0001, Some(viewer.store()), 600)
                        .is_none()
                );
            }
            now += pause;
        }
        assert!(lab.demo_deadline().is_none());
        assert_eq!(lab.calls.len(), expected.len());
        assert_eq!(
            lab.demo_note,
            Some("Demo complete. Undo in the session banner reverts both batches.")
        );
        let store = viewer.store_mut();
        assert_eq!(store.project().items.order().len(), base_items + 2);
        assert!(
            store
                .project()
                .markers
                .iter()
                .any(|marker| marker.name == "Save beat")
        );
        let options = WriteOptions {
            session,
            ..WriteOptions::default()
        };
        let receipt = store.undo(Scope::Session, &options).unwrap();
        assert_eq!(receipt.reverted, [1, 2]);
        assert_eq!(store.project().items.order().len(), base_items);
        assert!(
            !store
                .project()
                .markers
                .iter()
                .any(|marker| marker.name == "Save beat")
        );
        lab.refresh_demo(Some(store));
        assert!(!lab.demo_ready);
        lab.apply(Intents {
            action: Some(Action::Demo(false)),
            ..Intents::default()
        });
        assert!(lab.poll_demo(now, Some(store), 600).is_none());
        assert_eq!(store.sessions().len(), 1);
        assert!(store.sessions()[0].open);
        assert_eq!(store.project().rev, 3);
        viewer.close().unwrap();
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn recent_calls_reload_arguments_without_executing_and_paint_warm() {
        use pxui::{Event, FrameIn, Role, testing};

        let folder = std::env::temp_dir().join(format!("splice-lab-recall-{}", std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let mut store = Store::create(&folder.join("Recall.splice"), &Project::default()).unwrap();
        let session = store.start_session("fixture", 1).unwrap().id;
        let revision = store.project().rev;
        let mut state = editor_session::State::default();
        state.refresh(Some(&store), Some(session));
        let activity = Some((&state, &store));
        let mut lab = Lab::new();
        let context = Some((revision, Some(session)));
        let info = splice_json::parse(r#"{"projectId":"prj_1","details":["scope",3]}"#).unwrap();
        let edit =
            splice_json::parse(&with_revision(lab.draft(), revision, Some(session)).unwrap())
                .unwrap();
        let error =
            splice_json::parse(r#"{"structuredContent":{"code":"STALE_REVISION"},"isError":true}"#)
                .unwrap();
        for revision in 0..CALL_LOG_LIMIT {
            let reply = splice_json::parse(&format!(
                r#"{{"structuredContent":{{"projectId":"prj_1","access":"live","revision":{revision}}},"isError":false}}"#
            ))
            .unwrap();
            lab.set_reply("project.info", &info, &reply);
        }
        lab.set_reply("edit.apply", &edit, &error);
        assert_eq!(lab.calls.len(), CALL_LOG_LIMIT);
        assert_eq!(lab.calls.back().unwrap().id, 1);
        assert!(lab.reply().unwrap().error);

        mem_sys::set_peer(pxui::library_counter);
        for width in [960.0, 1280.0, 800.0, 480.0] {
            let mut ui = crate::viewer_ui().unwrap();
            ui.set_a11y(true);
            let warm_frame = FrameIn::new(width, 640.0);
            let mut idle = || {
                ui.frame(&warm_frame, &mut |ui| {
                    assert!(!paint(ui, context, &lab, &[], activity, 0.0).changed())
                })
                .unwrap();
            };
            for _ in 0..8 {
                idle();
            }
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    for _ in 0..8 {
                        idle();
                    }
                });
            if width == 960.0 {
                let dock = testing::find(&ui, Role::ScrollArea, "Session activity").unwrap();
                assert_eq!(dock.h, 320.5);
            }
            // Keep the compact page tall for hit-testing the stacked panel.
            let frame = FrameIn::new(width, if width < 820.0 { 1400.0 } else { 640.0 });
            let draw = |ui: &mut Ui, lab: &mut Lab, events: &[Event]| {
                let mut intents = Intents::default();
                ui.frame(&frame.events(events), &mut |ui| {
                    intents.merge(paint(ui, context, lab, &[], activity, 0.0));
                })
                .unwrap();
                lab.apply(intents)
            };
            assert!(draw(&mut ui, &mut lab, &[]).is_none());
            assert!(draw(&mut ui, &mut lab, &[]).is_none());
            assert!(testing::find(&ui, Role::Label, "AGENT SESSION · ACTIVITY").is_some());
            assert!(
                testing::find(
                    &ui,
                    Role::Label,
                    &format!("{session} · open · checkpoint r{revision}"),
                )
                .is_some()
            );
            let click = |ui: &mut Ui, lab: &mut Lab, name| {
                let rect = testing::find(ui, Role::Button, name).unwrap();
                let x = f64::from(rect.x + rect.w / 2.0);
                let y = f64::from(rect.y + rect.h / 2.0);
                assert!(
                    draw(ui, lab, &[Event::pointer(Event::POINTER_PRESSED, 0, x, y)]).is_none()
                );
                draw(ui, lab, &[Event::pointer(Event::POINTER_RELEASED, 0, x, y)])
            };
            assert!(click(&mut ui, &mut lab, "Load project.info arguments").is_none());
            assert_eq!(lab.reply().unwrap().name, "project.info");
            assert!(!lab.reply().unwrap().error);
            assert!(draw(&mut ui, &mut lab, &[]).is_none());
            let call = click(&mut ui, &mut lab, "Run call")
                .unwrap_or_else(|| panic!("Run call in {width}"));
            assert_eq!(call.name, "project.info");
            assert_eq!(call.args, info);
            assert_eq!(lab.calls.len(), CALL_LOG_LIMIT);
            assert!(click(&mut ui, &mut lab, "Load edit.apply arguments").is_none());
            assert!(draw(&mut ui, &mut lab, &[]).is_none());
            let call = click(&mut ui, &mut lab, "Run call")
                .unwrap_or_else(|| panic!("Run call in {width}"));
            assert_eq!(call.name, "edit.apply");
            assert_eq!(call.args, edit);
        }
        store.close().unwrap();
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn saved_frame_replies_restore_the_gallery_without_warm_allocations() {
        use pxui::{FrameIn, testing};

        let mut lab = Lab::new();
        let args = splice_json::parse(r#"{"projectId":"prj_1","at":[42]}"#).unwrap();
        let reply = splice_json::parse(
            r#"{"result":{"isError":false,"structuredContent":{
            "revision":"r0","frames":[{"frame":42,"width":1080,"height":1920,
            "uri":"file:///tmp/Frame%20Preview.splice/cache/frames/r0-42.png"}]
        }}}"#,
        )
        .unwrap();
        lab.set_reply("view.frames", &args, &reply);
        let (image_call, files) = lab.frame_reply().unwrap();
        assert_eq!(
            files[0].path.to_str(),
            Some("/tmp/Frame Preview.splice/cache/frames/r0-42.png")
        );
        assert!(frame_path("file://host/frame.png").is_none());
        assert!(frame_path("file:///tmp/frame%00.png").is_none());
        lab.set_reply("project.info", &args, &Json::Object(Vec::new()));
        assert!(lab.frame_reply().is_none());
        lab.apply(Intents {
            action: Some(Action::Recall(image_call)),
            ..Intents::default()
        });
        assert_eq!(lab.frame_reply().unwrap().0, image_call);

        let asset = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/render/playbar.png");
        let (pixels, size) = apple_sys::image::decode_thumbnail(&asset, 150).unwrap();
        assert_eq!(size, [150, 100]);
        assert_eq!(pixels.len(), 150 * 100 * 4);
        let images = [Some(Image::rgba8(
            360,
            640,
            std::ptr::null_mut(),
            std::ptr::dangling_mut(),
        ))];
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let input = FrameIn::new(960.0, 606.0);
        let mut draw = |ui: &mut Ui| {
            assert!(!paint(ui, Some((0, None)), &lab, &images, None, 0.0).changed());
        };
        mem_sys::set_peer(pxui::library_counter);
        for _ in 0..8 {
            ui.frame(&input, &mut draw).unwrap();
        }
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                ui.frame(&input, &mut draw).unwrap();
            },
        );
        let preview = testing::find(&ui, Role::Image, "Rendered frame preview").unwrap();
        assert!(
            (preview.w - 84.375).abs() < 0.1 && preview.h == 150.0,
            "{preview:?}"
        );
        assert!(testing::find(&ui, Role::Label, "42 f").is_some());
    }

    #[test]
    fn lab_wheel_scroll_is_recalled_in_the_live_and_fresh_gallery() {
        use pxui::{Event, FrameIn, testing};

        let folder = std::env::temp_dir().join(format!("splice-lab-scroll-{}", std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let mut store = Store::create(&folder.join("Gallery.splice"), &Project::default()).unwrap();
        let session = store.start_session("fixture", 1).unwrap().id;
        let mut state = editor_session::State::default();
        state.refresh(Some(&store), Some(session));
        let activity = Some((&state, &store));
        let context = Some((store.project().rev, Some(session)));
        let mut lab = Lab::new();
        lab.demo_note = Some("Demo complete. Undo in the session banner reverts both batches.");
        let args = splice_json::parse(r#"{"projectId":"prj_1","at":[276,366]}"#).unwrap();
        let reply = splice_json::parse(
            r#"{"structuredContent":{"frames":[
                {"frame":276,"width":1080,"height":1920,"uri":"file:///tmp/Gallery.splice/cache/frames/r2-276.png"},
                {"frame":366,"width":1080,"height":1920,"uri":"file:///tmp/Gallery.splice/cache/frames/r2-366.png"}]},
                "isError":false}"#,
        )
        .unwrap();
        let previous_args = splice_json::parse(r#"{"projectId":"prj_1"}"#).unwrap();
        let previous =
            splice_json::parse(r#"{"structuredContent":{"revision":"r0"},"isError":false}"#)
                .unwrap();
        for name in ["project.info", "timeline.snapshot"] {
            lab.set_reply(name, &previous_args, &previous);
        }
        lab.set_reply("view.frames", &args, &reply);
        let images = [Some(Image::rgba8(
            360,
            640,
            std::ptr::null_mut(),
            std::ptr::dangling_mut(),
        )); 2];
        let gallery = |ui: &Ui| testing::find(ui, Role::Image, "Rendered frame preview").unwrap();
        let draw = |ui: &mut Ui, lab: &Lab, scroll: &mut Scroll, events: &[Event]| {
            let mut intents = Intents::default();
            ui.frame(&FrameIn::new(960.0, 640.0).events(events), &mut |ui| {
                ui.column(FILL, |ui| {
                    ui.node("titlebar", Node::DEFAULT.w(Size::Grow).h_px(34.0));
                    intents.merge(paint(ui, context, lab, &images, activity, 0.0));
                });
            })
            .unwrap();
            scroll.settle(ui, &intents);
            intents
        };
        let mut live = crate::viewer_ui().unwrap();
        live.set_a11y(true);
        let mut scroll = Scroll::default();
        let view = draw(&mut live, &lab, &mut scroll, &[]);
        let pane = live.rect(&view.viewport.unwrap().0);
        let top = gallery(&live);
        assert!(top.y >= pane.y + pane.h, "{top:?} {pane:?}");
        let hover = Event::pointer(
            Event::POINTER_MOVED,
            0,
            f64::from(pane.x + pane.w - 7.0),
            f64::from(pane.y + pane.h * 0.6),
        );
        assert!(!draw(&mut live, &lab, &mut scroll, &[hover]).changed());
        assert_eq!(gallery(&live), top);
        assert_eq!(scroll.offset, 0.0);

        let wheel = Event::pointer(Event::SCROLLED, 0, 0.0, -12.0);
        assert!(!draw(&mut live, &lab, &mut scroll, &[wheel]).changed());
        let scrolled = gallery(&live);
        eprintln!(
            "initial {top:?}, wheel {scrolled:?}, pane {pane:?}, offset {}",
            scroll.offset
        );
        assert!(scroll.offset > 0.0);
        assert!(
            scrolled.y >= pane.y && scrolled.y + scrolled.h <= pane.y + pane.h,
            "{scrolled:?} {pane:?}, scroll {}",
            scroll.offset
        );
        assert_eq!(top.y - scrolled.y, scroll.offset);
        assert!((scrolled.w - 84.375).abs() < 0.01 && scrolled.h == 150.0);

        // A capture builds a new Ui, measures once, applies the settled offset, and paints.
        let mut capture = crate::viewer_ui().unwrap();
        capture.set_a11y(true);
        let mut saved = scroll;
        assert!(saved.restore());
        for _ in 0..3 {
            assert!(!draw(&mut capture, &lab, &mut saved, &[]).changed());
        }
        assert_eq!(gallery(&capture), scrolled);

        let before_recall = scroll.offset;
        let recall = testing::find(&live, Role::Button, "Load view.frames arguments").unwrap();
        let (x, y) = (
            f64::from(recall.x + recall.w / 2.0),
            f64::from(recall.y + recall.h / 2.0),
        );
        for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
            let intents = draw(
                &mut live,
                &lab,
                &mut scroll,
                &[Event::pointer(kind, 0, x, y)],
            );
            assert!(lab.apply(intents).is_none());
        }
        assert_eq!(tool_catalog()[lab.selected].name, "view.frames");
        for _ in 0..2 {
            assert!(!draw(&mut live, &lab, &mut scroll, &[]).changed());
        }
        assert_eq!(gallery(&live), scrolled);
        assert_ne!(scroll.offset, before_recall);
        eprintln!(
            "recall retains the preview at {scrolled:?}, offset {}",
            scroll.offset
        );

        // Rebuild another page before returning to Agent, as the live view switch does.
        live.frame(&FrameIn::new(960.0, 640.0), &mut |ui: &mut Ui| {
            ui.label("Spec", SECTION);
        })
        .unwrap();
        assert!(scroll.restore());
        for _ in 0..3 {
            assert!(!draw(&mut live, &lab, &mut scroll, &[]).changed());
        }
        assert_eq!(gallery(&live), scrolled);

        assert!(!draw(&mut live, &lab, &mut scroll, &[hover]).changed());
        let up = Event::pointer(Event::SCROLLED, 0, 0.0, 1.0);
        let down = Event::pointer(Event::SCROLLED, 0, 0.0, -1.0);
        mem_sys::set_peer(pxui::library_counter);
        let mut cycle = || {
            for events in [&[][..], &[up], &[down]] {
                assert!(!draw(&mut live, &lab, &mut scroll, events).changed());
            }
        };
        for _ in 0..8 {
            cycle();
        }
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut cycle);
        assert_eq!(gallery(&live), scrolled);
        store.close().unwrap();
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn editing_revision_preserves_other_arguments_and_keeps_the_last_tool_reply() {
        let tool = |name| {
            tool_catalog()
                .iter()
                .position(|tool| tool.name == name)
                .unwrap()
        };
        let mut lab = Lab::new();
        lab.choose(tool("edit.apply"));
        let draft = r#"{"ops":[{"ifRevision":"r3","weight":2.0}], "\u0069fRevision" : null }"#;
        let updated = with_revision(draft, 42, None).unwrap();
        assert_eq!(updated, draft.replace("null", "\"r42\""));
        assert!(parse_request("[]").is_err());
        assert!(parse_request("{").is_err());
        assert_eq!(
            parse_request(&with_revision("{}", 4, None).unwrap())
                .unwrap()
                .field("ifRevision")
                .and_then(Json::as_str),
            Some("r4")
        );

        lab.set_draft(updated.clone());
        let reply = splice_json::parse(r#"{"owner":"viewer","result":{"structuredContent":{"error":{"code":"STALE_REVISION","current":"r43"}},"isError":true}}"#).unwrap();
        lab.set_reply("edit.apply", &parse_request(&updated).unwrap(), &reply);
        lab.choose(tool("project.info"));
        assert_eq!(lab.draft(), "{}");
        lab.choose(tool("edit.apply"));
        assert_eq!(lab.draft(), updated);
        let reply = lab.reply().unwrap();
        assert!(reply.error);
        assert_eq!(reply.name, "edit.apply");
        assert_eq!(
            splice_json::parse(&reply.preview).unwrap(),
            splice_json::parse(r#"{"error":{"code":"STALE_REVISION","current":"r43"}}"#).unwrap()
        );

        assert!(
            lab.apply(Intents {
                action: Some(Action::Reset),
                ..Intents::default()
            })
            .is_none()
        );
        assert_eq!(
            parse_request(lab.draft())
                .unwrap()
                .field("dryRun")
                .and_then(Json::as_bool),
            Some(true)
        );
    }
}
