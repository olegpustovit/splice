use splice_core::{
    history::Store,
    package::journal::{Change, Source},
};
use splice_json::Value as Json;
use splice_mcp::{take_events_available, tool_catalog};
use splice_model::{
    Access, ClipId, ClipTiming, ItemId, JobId, LaneId, Media, MediaId, MediaKind, Project,
    SessionId, TimelineId,
};
use std::{
    collections::VecDeque,
    fmt::{Arguments, Write},
};

// A native Viewer binds its one Store as the first project in MCP.
const PROJECT_ID: &str = "prj_1";
const OPEN: &str = "Open a project in the editor to run this example.";
const LIVE: &str = "This example requires a writable project with Live agent access.";
const SESSION: &str = "Start an agent session to run this example.";
const JOB_LIMIT: usize = 30;
const SAMPLE_EDIT_OPS: &str = r#"[
    {
      "cmd": "item.split",
      "item": "clp_take",
      "at": 372,
      "ref": "$cut"
    },
    {
      "cmd": "overlay.add",
      "kind": "media",
      "media": "med_broll",
      "anchor": {
        "item": "$cut.right",
        "edge": "start"
      },
      "length": 90
    },
    {
      "cmd": "item.delete",
      "items": [
        "ovl_label"
      ],
      "ripple": false
    }
  ]"#;

pub(super) struct Examples {
    key: Option<Key>,
    dirty: bool,
    subjects: Subjects,
    rows: Vec<Example>,
    jobs: VecDeque<Job>,
}

pub(super) struct Example {
    pub(super) text: String,
    pub(super) missing: Option<&'static str>,
}

#[derive(Clone, Copy, PartialEq)]
struct Key {
    revision: u64,
    hash: u64,
    writable: bool,
    journal_problem: bool,
    open: Option<SessionId>,
    latest: Option<SessionId>,
    at: i32,
    end: i32,
    item: Option<TimelineId>,
    lane: Option<LaneId>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Job {
    id: JobId,
    recording: bool,
    active: bool,
}

impl Examples {
    pub(super) fn new() -> Self {
        Self {
            key: None,
            dirty: false,
            subjects: Subjects::default(),
            rows: tool_catalog()
                .iter()
                .map(|tool| {
                    let mut example = Example {
                        text: String::with_capacity(512),
                        missing: None,
                    };
                    example.detached(tool.name);
                    example
                })
                .collect(),
            jobs: VecDeque::new(),
        }
    }

    pub(super) fn get(&self, index: usize) -> &Example {
        &self.rows[index]
    }

    pub(super) fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub(super) fn refresh(
        &mut self,
        store: Option<&Store>,
        at: i32,
        end: i32,
        item: Option<TimelineId>,
        lane: Option<LaneId>,
    ) {
        let Some(store) = store else {
            self.jobs.clear();
            if self.key.take().is_some() {
                for (example, tool) in self.rows.iter_mut().zip(tool_catalog()) {
                    example.detached(tool.name);
                }
            }
            return;
        };
        let project = store.project();
        let end = end.max(0);
        let key = Key {
            revision: project.rev,
            hash: project.hash,
            writable: project.access == Access::Live && store.problem().is_none(),
            journal_problem: store.problem().is_some(),
            open: store
                .sessions()
                .iter()
                .rev()
                .find(|session| session.open)
                .map(|session| session.id),
            latest: store.sessions().last().map(|session| session.id),
            at: at.clamp(0, end.saturating_sub(1).max(0)),
            end,
            item,
            lane,
        };
        if !self.dirty && self.key == Some(key) {
            return;
        }
        // A clock tick only moves the ranges and marker example. Subject lookup can read
        // the take's event path, so run it on snapshot/selection changes or explicit Reset.
        let context_changed = self.dirty
            || self.key.is_none_or(|previous| {
                Key {
                    at: key.at,
                    ..previous
                } != key
            });
        if context_changed {
            self.subjects = Subjects::new(store, key);
        }
        self.subjects.marker_at(project, key.at);
        self.key = Some(key);
        self.dirty = false;
        for (example, tool) in self.rows.iter_mut().zip(tool_catalog()) {
            if context_changed
                || matches!(
                    tool.name,
                    "takes.events"
                        | "view.frames"
                        | "view.check"
                        | "edit.apply"
                        | "ui.show"
                        | "export.start"
                )
            {
                example.live(tool.name, key, &self.subjects, &self.jobs);
            }
        }
    }

    pub(super) fn observe_jobs(&mut self, name: &str, payload: &Json) {
        if !matches!(
            name,
            "export.start"
                | "record.start"
                | "record.stop"
                | "jobs.status"
                | "jobs.cancel"
                | "transcript.get"
                | "media.analyze"
        ) {
            return;
        }
        self.observe_job(name, payload);
        if let Some(jobs) = payload.field("jobs").and_then(Json::as_array) {
            for job in jobs {
                self.observe_job(name, job);
            }
        }
    }

    fn observe_job(&mut self, name: &str, payload: &Json) {
        let Some(id) = payload
            .field("jobId")
            .or_else(|| payload.field("id"))
            .and_then(Json::as_str)
            .and_then(|id| id.parse().ok())
        else {
            return;
        };
        let Some(state) = payload.field("state").and_then(Json::as_str) else {
            return;
        };
        let previous = self.jobs.iter().position(|job| job.id == id);
        let recording = matches!(name, "record.start" | "record.stop")
            || payload.field("kind").and_then(Json::as_str) == Some("record")
            || previous.is_some_and(|index| self.jobs[index].recording);
        let job = Job {
            id,
            recording,
            active: matches!(
                state,
                "pending" | "queued" | "running" | "recording" | "paused"
            ),
        };
        if previous == Some(0) && self.jobs.front() == Some(&job) {
            return;
        }
        if let Some(index) = previous {
            self.jobs.remove(index);
        } else if self.jobs.len() == JOB_LIMIT {
            self.jobs.pop_back();
        }
        self.jobs.push_front(job);
        self.invalidate();
    }
}

#[derive(Default)]
struct Subjects {
    item: Option<TimelineId>,
    lane: Option<LaneId>,
    take: Option<(TimelineId, ClipTiming)>,
    transcript: Option<TimelineId>,
    media_clip: Option<TimelineId>,
    sample_edit: bool,
    live_batch: bool,
    spine_end: Option<i32>,
    marker_at: i32,
    marker: Option<TimelineId>,
    fps: i32,
}

impl Subjects {
    fn new(store: &Store, key: Key) -> Self {
        let project = store.project();
        let clips = || {
            project
                .clips
                .order()
                .iter()
                .filter_map(|&slot| project.clips.id(slot).map(TimelineId::Clip))
        };
        let items = || {
            project
                .items
                .order()
                .iter()
                .filter_map(|&slot| project.items.id(slot).map(TimelineId::Item))
        };
        let selected = key.item.filter(|&id| match id {
            TimelineId::Clip(id) => project.clips.find(id).is_some(),
            TimelineId::Item(id) => project.items.find(id).is_some(),
            TimelineId::Marker(id) => project.markers.iter().any(|marker| marker.id == id),
        });
        let lane = key
            .lane
            .filter(|&id| project.lanes.iter().any(|lane| lane.id == id));
        let item = selected.or_else(|| {
            if lane.is_some() {
                return None;
            }
            clips().chain(items()).next().or_else(|| {
                project
                    .markers
                    .first()
                    .map(|marker| TimelineId::Marker(marker.id))
            })
        });
        let lane = lane.or_else(|| project.lanes.first().map(|lane| lane.id));
        let take = selected.into_iter().chain(clips()).find_map(|id| {
            let TimelineId::Clip(clip) = id else {
                return None;
            };
            let slot = project.clips.find(clip)?;
            let media = media(project, id)?;
            take_events_available(store.path(), media)
                .then(|| (id, project.clips.timing(slot).expect("live clip")))
        });
        let transcript = selected
            .into_iter()
            .chain(clips())
            .chain(items())
            .find(|&id| {
                media(project, id).is_some_and(|media| {
                    project
                        .transcripts
                        .iter()
                        .any(|transcript| transcript.media == media.id)
                })
            });
        let media_clip = selected
            .into_iter()
            .chain(clips())
            .find(|&id| matches!(id, TimelineId::Clip(_)) && media(project, id).is_some());
        let live_batch = key.open.is_some()
            && store.records().iter().rev().any(|record| {
                record.batch.session == key.open
                    && record.batch.source == Source::Agent
                    && matches!(record.batch.change, Change::Edit { .. })
                    && store.is_active(record.rev)
            });
        let spine_end = project
            .clips
            .order()
            .last()
            .and_then(|&slot| project.clips.timing(slot))
            .and_then(ClipTiming::end);
        let fps = i32::try_from(project.fps).unwrap_or(i32::MAX).max(1);
        Self {
            item,
            lane,
            take,
            transcript,
            media_clip,
            sample_edit: sample_edit(project),
            live_batch,
            spine_end,
            fps,
            ..Self::default()
        }
    }

    fn marker_at(&mut self, project: &Project, at: i32) {
        self.marker_at = at.min(self.spine_end.unwrap_or(at));
        self.marker = project
            .markers
            .iter()
            .find(|marker| marker.at == self.marker_at)
            .map(|marker| TimelineId::Marker(marker.id));
    }
}

fn sample_edit(project: &Project) -> bool {
    let clip = "clp_take".parse::<ClipId>().expect("sample clip ID");
    let label = "ovl_label".parse::<ItemId>().expect("sample title ID");
    let broll = "med_broll".parse::<MediaId>().expect("sample b-roll ID");
    project
        .clips
        .find(clip)
        .and_then(|slot| project.clips.timing(slot))
        .is_some_and(|timing| timing.start < 372 && timing.end().is_some_and(|end| 372 < end))
        && project.items.find(label).is_some()
        && project.media.iter().any(|media| {
            media.id == broll
                && media.kind == MediaKind::Video
                && f64::from(media.probe.frames) * f64::from(project.fps) >= 90.0 * media.probe.fps
        })
}

fn media(project: &Project, id: TimelineId) -> Option<&Media> {
    use splice_model::ItemKind;
    let id = match id {
        TimelineId::Clip(id) => project.clips.data(project.clips.find(id)?)?.media?,
        TimelineId::Item(id) => match &project.items.data(project.items.find(id)?)?.kind {
            ItemKind::Media(overlay) => overlay.media,
            ItemKind::Sound(sound) => sound.media,
            _ => return None,
        },
        TimelineId::Marker(_) => return None,
    };
    project.media.iter().find(|media| media.id == id)
}

impl Example {
    fn begin(&mut self) {
        self.text.clear();
        self.text.push('{');
        self.missing = None;
    }

    fn field(&mut self, key: &str, value: Arguments<'_>) {
        if self.text.len() > 1 {
            self.text.push(',');
        }
        write!(&mut self.text, "\n  \"{key}\": {value}").expect("String formatting");
    }

    fn finish(&mut self) {
        if self.text.len() > 1 {
            self.text.push('\n');
        }
        self.text.push('}');
    }

    fn detached(&mut self, name: &str) {
        self.begin();
        match name {
            "edit.apply" | "history.undo" | "record.start" => {
                self.field("dryRun", format_args!("true"))
            }
            "commands.describe" => self.field("query", format_args!(r#""lane""#)),
            _ => {}
        }
        self.missing = Some(OPEN);
        self.finish();
    }

    fn live(&mut self, name: &str, key: Key, subjects: &Subjects, jobs: &VecDeque<Job>) {
        self.begin();
        if !matches!(
            name,
            "commands.describe" | "jobs.status" | "jobs.cancel" | "record.stop"
        ) {
            self.field("projectId", format_args!(r#""{PROJECT_ID}""#));
        }
        self.missing = self.params(name, key, subjects, jobs).err();
        self.finish();
    }

    fn params(
        &mut self,
        name: &str,
        key: Key,
        subjects: &Subjects,
        jobs: &VecDeque<Job>,
    ) -> Result<(), &'static str> {
        let need_write = if key.writable { Ok(()) } else { Err(LIVE) };
        let frames = if key.end > 0 {
            Ok(())
        } else {
            Err("Add a clip or overlay to put frames on the timeline.")
        };
        let open = key.open.ok_or(SESSION);
        let at = key.at;
        let fps = subjects.fps;
        let to = at
            .saturating_add(fps.saturating_mul(2))
            .saturating_add(1)
            .min(key.end);
        let from = to.saturating_sub(fps.saturating_mul(5)).max(0);
        match name {
            "project.info" => {}
            "timeline.snapshot" => {
                self.field("from", format_args!("0"));
                self.field("to", format_args!("{}", key.end));
                self.field("detail", format_args!(r#""concise""#));
                self.field("limit", format_args!("40"));
            }
            "items.inspect" => {
                if let Some(id) = subjects.item {
                    self.field("ids", format_args!(r#"["{id}"]"#));
                } else if let Some(id) = subjects.lane {
                    self.field("ids", format_args!(r#"["{id}"]"#));
                } else {
                    return Err("Add a timeline item, marker or lane to inspect.");
                }
            }
            "timeline.find" => {
                self.field("kind", format_args!(r#""edit""#));
                self.field("limit", format_args!("24"));
            }
            "takes.events" => {
                let (id, clip) = subjects.take.ok_or(
                    "Add a recorded clip with its event file available to read take events.",
                )?;
                let end = clip.end().expect("valid clip end");
                let start = at.clamp(clip.start, end - 1);
                self.field("clip", format_args!(r#""{id}""#));
                self.field("from", format_args!("{start}"));
                self.field(
                    "to",
                    format_args!("{}", start.saturating_add(fps.saturating_mul(5)).min(end)),
                );
                self.field("stride", format_args!("{}", (fps / 2).max(1)));
            }
            "transcript.get" => {
                let id = subjects
                    .transcript
                    .ok_or("A saved transcript on a clip or audio overlay is required.")?;
                let field = if matches!(id, TimelineId::Clip(_)) {
                    "clip"
                } else {
                    "item"
                };
                self.field(field, format_args!(r#""{id}""#));
                self.field("limit", format_args!("16"));
            }
            "media.analyze" => {
                let id = subjects
                    .media_clip
                    .ok_or("Add a media-backed clip to analyze.")?;
                self.field("clip", format_args!(r#""{id}""#));
                self.field("kinds", format_args!(r#"["loudness", "pauses"]"#));
            }
            "view.frames" => {
                frames?;
                let before = at.saturating_sub(fps).max(0);
                let after = at.saturating_add(fps).min(key.end - 1);
                if before < at && at < after {
                    self.field("at", format_args!("[{before}, {at}, {after}]"));
                } else if before < after {
                    self.field("at", format_args!("[{before}, {after}]"));
                } else {
                    self.field("at", format_args!("[{at}]"));
                }
            }
            "view.check" => {
                frames?;
                self.field("from", format_args!("{from}"));
                self.field("to", format_args!("{to}"));
            }
            "commands.describe" => self.field("query", format_args!(r#""lane""#)),
            "session.start" => {
                self.field("agent", format_args!(r#""Agent Lab""#));
                need_write?;
            }
            "session.end" => {
                if key.journal_problem {
                    return Err("The project needs a writable journal to end this session.");
                }
                let session = key.open.ok_or("No agent session is open to end.")?;
                self.field("sessionId", format_args!(r#""{session}""#));
            }
            "edit.apply" => {
                // Serial zero is unassigned. An explicit ID lets a fresh sample report the
                // server's session lookup error without opening a session as a side effect.
                let session = key
                    .open
                    .or(key.latest)
                    .unwrap_or_else(|| SessionId::from_serial(0));
                self.field("sessionId", format_args!(r#""{session}""#));
                self.field("ifRevision", format_args!(r#""r{}""#, key.revision));
                self.field("dryRun", format_args!("true"));
                if subjects.sample_edit {
                    self.field("ops", format_args!("{SAMPLE_EDIT_OPS}"));
                } else if let Some(id) = subjects.marker {
                    self.field("ops", format_args!(r#"[{{"cmd": "marker.set", "marker": "{id}", "name": "Example marker"}}]"#));
                } else {
                    self.field(
                        "ops",
                        format_args!(
                            r#"[{{"cmd": "marker.add", "at": {}, "name": "Example marker"}}]"#,
                            subjects.marker_at
                        ),
                    );
                }
                need_write?;
            }
            "history.list" => {
                if let Some(session) = key.open.or(key.latest) {
                    self.field("sessionId", format_args!(r#""{session}""#));
                }
                self.field("limit", format_args!("24"));
            }
            "history.undo" => {
                self.field("ifRevision", format_args!(r#""r{}""#, key.revision));
                self.field("scope", format_args!(r#""last""#));
                self.field("dryRun", format_args!("true"));
                need_write?;
                let session = open?;
                self.field("sessionId", format_args!(r#""{session}""#));
                if !subjects.live_batch {
                    return Err(
                        "The open session needs a live edit batch before it can be undone.",
                    );
                }
            }
            "ui.show" => {
                self.field("at", format_args!("{at}"));
                if let Some(id) = subjects.item {
                    self.field("select", format_args!(r#"["{id}"]"#));
                } else if let Some(id) = subjects.lane {
                    self.field("select", format_args!(r#"["{id}"]"#));
                }
            }
            "export.start" => {
                self.field("ifRevision", format_args!(r#""r{}""#, key.revision));
                if let Some(session) = key.open {
                    self.field("sessionId", format_args!(r#""{session}""#));
                }
                need_write?;
                frames?;
                self.field("preset", format_args!(r#""social-hevc""#));
                self.field("from", format_args!("{from}"));
                self.field("to", format_args!("{to}"));
            }
            "record.start" => self.field("dryRun", format_args!("true")),
            "jobs.status" | "jobs.cancel" | "record.stop" => {
                let job = match name {
                    "jobs.status" => jobs
                        .front()
                        .ok_or("Run a job or enter its returned job ID to check its status."),
                    "jobs.cancel" => jobs
                        .iter()
                        .find(|job| job.active)
                        .ok_or("Run a job or enter a running job's returned ID to cancel it."),
                    _ => jobs
                        .iter()
                        .find(|job| job.recording && job.active)
                        .ok_or("Start a recording or enter the active recording's job ID."),
                }?;
                self.field("jobId", format_args!(r#""{}""#, job.id));
            }
            _ => return Err("Choose this tool's arguments from its schema."),
        }
        Ok(())
    }
}
