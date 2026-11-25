use crate::{
    Json, export::Job, friction::Recorder, object, project::Projects, text, timeline, tools,
    wire::*,
};
use splice_core::{
    history::{Store, StoreError, WriteOptions},
    package,
};
use splice_model::{ClipId, ItemId, JobId, LaneId, MarkerId, Project, TimelineId};
use std::{
    collections::HashSet,
    fmt, io,
    path::{Path, PathBuf},
};

/// Viewer-only intents handled on the window thread.
pub enum ViewCommand {
    /// Apply view changes and return the current selection as an array of ID strings.
    Show {
        /// Timeline frame to seek to. Leave the playhead unchanged when absent.
        at: Option<i32>,
        /// Selection in request order. An empty list clears it and absence preserves it.
        select: Option<Vec<ViewSelection>>,
        /// Recorded clip whose component rows should open.
        expand: Option<ClipId>,
        /// Nonempty timeline range to frame, with an exclusive end.
        range: Option<[i32; 2]>,
        /// Native view to open.
        view: Option<ViewTab>,
        /// Program viewer zoom to apply.
        zoom: Option<ViewerZoom>,
    },
    /// Render the viewer into a PNG.
    Snapshot {
        /// Requested timeline frame.
        at: i32,
        /// Absolute PNG output location.
        output: PathBuf,
    },
}

/// A tab in the native project window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewTab {
    /// The project editor.
    Editor,
    /// The Agent MCP lab.
    Agent,
    /// The Splice specification.
    Spec,
}

/// Zoom of the program viewer, independent of timeline framing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewerZoom {
    /// Fit the project canvas within the viewer.
    Fit,
    /// An absolute zoom factor, where 1.0 displays the canvas at 100%.
    Factor(f32),
}

/// A project object selectable in the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewSelection {
    /// A clip, overlay, or marker.
    Timeline(TimelineId),
    /// A secondary lane or the video spine.
    Lane(LaneId),
}

impl fmt::Display for ViewSelection {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeline(id) => fmt::Display::fmt(id, output),
            Self::Lane(id) => fmt::Display::fmt(id, output),
        }
    }
}

/// The viewer's store and MCP state. Call it only from the viewer's event loop.
pub struct Viewer {
    projects: Projects,
    friction: Recorder,
    owner: String,
}

impl Viewer {
    /// Takes ownership of the project's existing writer lock.
    pub fn new(store: Store, state: Option<PathBuf>) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let owner = format!(
            "{}-{}-{}",
            std::process::id(),
            now(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        Self {
            projects: Projects::viewer(store),
            friction: Recorder::new(state),
            owner,
        }
    }

    /// Borrows the current writer's store.
    pub fn store(&self) -> &Store {
        self.projects.viewer_store_ref()
    }

    /// Borrows the store for one of the viewer's own actions.
    pub fn store_mut(&mut self) -> &mut Store {
        self.projects.viewer_store()
    }

    /// Dispatches a local request and produces its private wire reply.
    pub fn handle(
        &mut self,
        request: &Json,
        at: i32,
        view: &mut dyn FnMut(&Store, ViewCommand) -> Result<Json, StoreError>,
    ) -> Json {
        let result = if request
            .field("owner")
            .is_some_and(|owner| owner.as_str() != Some(&self.owner))
        {
            Err(super::disconnected(&"The project viewer was reopened."))
        } else {
            match request.string("command") {
                Ok("tool") => self.tool(request, at, view),
                Ok("snapshot") => request.string("output").and_then(|output| {
                    view(
                        self.store(),
                        ViewCommand::Snapshot {
                            at: request.frame("at")?.unwrap_or(at),
                            output: PathBuf::from(output),
                        },
                    )
                }),
                Ok(command) => self.cli(command, request),
                Err(error) => Err(error),
            }
        };
        match result {
            Ok(value) => object(vec![("result", value), ("owner", text(&self.owner))]),
            Err(error) => object(vec![("error", error_payload(error))]),
        }
    }

    /// Advances agent exports for up to `budget`; the viewer calls it every frame. True while
    /// one is running, so the viewer keeps waking.
    pub fn tick(&mut self, budget: std::time::Duration) -> bool {
        self.projects.tick_exports(budget)
    }

    /// Finishes requested exports and closes the project writer.
    pub fn close(self) -> io::Result<()> {
        self.projects.close()
    }

    fn tool(
        &mut self,
        request: &Json,
        at: i32,
        view: &mut dyn FnMut(&Store, ViewCommand) -> Result<Json, StoreError>,
    ) -> Result<Json, StoreError> {
        let name = request.string("name")?;
        let args = request
            .field("args")
            .filter(|args| args.as_object().is_some())
            .ok_or_else(|| bad("args", "a tool arguments object"))?;
        if !tools::known(name, false) {
            return Err(bad("name", "a tool returned by tools/list"));
        }
        let client = request.optional_string("client")?.unwrap_or("mcp");
        let result = if name == "ui.show" {
            tools::reply(self.show(args, at, view), Vec::new())
        } else {
            tools::call(&mut self.projects, name, args)
        }
        .map_err(|error| {
            failure(
                "INVARIANT",
                &error.to_string(),
                "Inspect the project and try again.",
            )
        })?;
        self.friction.set_client(client);
        self.friction
            .call(&self.projects, name, args, &result, None);
        Ok(result)
    }

    fn show(
        &mut self,
        args: &Json,
        current: i32,
        view: &mut dyn FnMut(&Store, ViewCommand) -> Result<Json, StoreError>,
    ) -> Result<Json, StoreError> {
        args.optional_string("friction")?;
        let store = self
            .projects
            .lookup_id(args.optional_string("projectId")?.unwrap_or("prj_1"))?;
        let project = store.project();
        let range = args.field("range").map(parse_range).transpose()?;
        let at = args
            .field("at")
            .map(|value| splice_core::edit::resolve_view_frame(project, value))
            .transpose()?
            .or_else(|| range.map(|[from, _]| from));
        let at = at
            .map(|at| {
                timeline::duration(project).map(|end| at.clamp(0, end.saturating_sub(1).max(0)))
            })
            .transpose()?;
        let select = args
            .field("select")
            .map(|value| parse_selection(project, value))
            .transpose()?;
        let expand = args
            .optional_string("expand")?
            .map(|id| expandable_take(project, id))
            .transpose()?;
        let tab = args
            .optional_string("view")?
            .map(|name| match name {
                "edit" | "editor" => Ok(ViewTab::Editor),
                "mcp" | "agent" => Ok(ViewTab::Agent),
                "spec" => Ok(ViewTab::Spec),
                _ => Err(bad("view", "edit, editor, mcp, agent or spec")),
            })
            .transpose()?;
        let zoom = args.field("viewer").map(parse_zoom).transpose()?;
        let applied = at.is_some()
            || select.is_some()
            || expand.is_some()
            || range.is_some()
            || tab.is_some()
            || zoom.is_some();
        let selection = view(
            store,
            ViewCommand::Show {
                at,
                select,
                expand,
                range,
                view: tab,
                zoom,
            },
        )?;
        if selection.as_array().is_none() {
            return Err(failure(
                "INVARIANT",
                "The viewer did not return a selection array.",
                "Reopen the viewer and retry.",
            ));
        }
        Ok(object(vec![
            ("applied", Json::Bool(applied)),
            ("revision", rev(project.rev)),
            ("playhead", frame(at.unwrap_or(current))),
            ("selection", selection),
        ]))
    }

    fn cli(&mut self, command: &str, request: &Json) -> Result<Json, StoreError> {
        match command {
            "apply" => {
                let ops = request
                    .field("ops")
                    .and_then(Json::as_array)
                    .filter(|ops| !ops.is_empty())
                    .ok_or_else(|| bad("ops", "a nonempty operations array"))?;
                let options = WriteOptions {
                    time: now(),
                    dry_run: request.flag("dryRun")?,
                    if_revision: request
                        .field("ifRevision")
                        .map(|_| request.revision("ifRevision"))
                        .transpose()?,
                    key: request
                        .optional_string("idempotencyKey")?
                        .map(str::to_owned),
                    ..WriteOptions::default()
                };
                let store = self.store_mut();
                let result = crate::apply_batch(store, ops, &options)?;
                Ok(crate::checkpoint_edit(store, result))
            }
            "timeline" => {
                self.store_mut().checkpoint()?;
                Ok(package::snapshot(self.store().project()))
            }
            "check" => {
                let store = self.store();
                let verified = store.verify()?;
                let project = store.project();
                Ok(object(vec![
                    ("path", display(&store.path().display())),
                    ("revision", rev(project.rev)),
                    ("hash", hash(project.hash)),
                    ("ok", Json::Bool(true)),
                    ("stateHash", hash(verified)),
                    ("batches", count(store.records().len())),
                ]))
            }
            "frame" => crate::grab_frame(
                self.store(),
                request
                    .frame("at")?
                    .ok_or_else(|| bad("at", "a nonnegative frame"))?,
                Path::new(request.string("output")?),
            ),
            "export" => {
                let path = request.optional_string("output")?;
                let mut args = Vec::new();
                if let Some(preset) = request.optional_string("preset")? {
                    args.push(("preset".to_owned(), Json::String(preset.into())));
                }
                let end = timeline::duration(self.store().project())?;
                let output =
                    crate::presets::resolve(self.store().project(), end, &Json::Object(args))?
                        .pop()
                        .expect("one output");
                let id = JobId::from_serial(self.projects.exports.len() as u64 + 1);
                let job = Job::start(
                    self.store(),
                    id,
                    None,
                    &output,
                    path,
                    request.flag("overwrite")?,
                )?;
                let receipt = job.started();
                self.projects.exports.push(job);
                Ok(receipt)
            }
            "export-status" => {
                let id = request
                    .string("jobId")?
                    .parse::<JobId>()
                    .map_err(|_| bad("jobId", "an export job ID"))?;
                self.projects
                    .exports
                    .iter()
                    .find(|job| job.id == id && job.owner.is_none())
                    .ok_or_else(|| {
                        failure(
                            "NOT_FOUND",
                            "This CLI export is not retained.",
                            "Start export again.",
                        )
                    })?
                    .status(false)
            }
            _ => Err(bad("command", "a supported viewer command")),
        }
    }
}

fn parse_range(range: &Json) -> Result<[i32; 2], StoreError> {
    match (range.frame("from")?, range.frame("to")?) {
        (Some(from), Some(to)) if from < to => Ok([from, to]),
        _ => Err(bad(
            "range",
            "an object with from and to frames where from < to",
        )),
    }
}

fn parse_zoom(viewer: &Json) -> Result<ViewerZoom, StoreError> {
    let zoom = viewer
        .field("zoom")
        .ok_or_else(|| bad("viewer", "an object with a zoom field"))?;
    if zoom.as_str() == Some("fit") {
        return Ok(ViewerZoom::Fit);
    }
    zoom.as_f64()
        .map(|factor| factor as f32)
        .filter(|factor| factor.is_finite() && *factor > 0.0)
        .map(ViewerZoom::Factor)
        .ok_or_else(|| bad("viewer.zoom", "\"fit\" or a positive finite zoom factor"))
}

fn parse_selection(project: &Project, value: &Json) -> Result<Vec<ViewSelection>, StoreError> {
    let values = value
        .as_array()
        .ok_or_else(|| bad("select", "an array of timeline item or lane IDs"))?;
    let mut selection = Vec::with_capacity(values.len());
    let mut seen = HashSet::with_capacity(values.len());
    for value in values {
        let id = value
            .as_str()
            .ok_or_else(|| bad("select", "an array of timeline item or lane IDs"))?;
        let parsed = if let Ok(clip) = id.parse::<ClipId>() {
            project
                .clips
                .find(clip)
                .map(|_| ViewSelection::Timeline(TimelineId::Clip(clip)))
        } else if let Ok(item) = id.parse::<ItemId>() {
            project
                .items
                .find(item)
                .map(|_| ViewSelection::Timeline(TimelineId::Item(item)))
        } else if let Ok(marker) = id.parse::<MarkerId>() {
            project
                .markers
                .iter()
                .any(|entry| entry.id == marker)
                .then_some(ViewSelection::Timeline(TimelineId::Marker(marker)))
        } else if let Ok(lane) = id.parse::<LaneId>() {
            (lane == project.spine.id || project.lanes.iter().any(|entry| entry.id == lane))
                .then_some(ViewSelection::Lane(lane))
        } else {
            return Err(bad("select", "an array of timeline item or lane IDs"));
        };
        let parsed = parsed.ok_or_else(|| {
            failure(
                "NOT_FOUND",
                &format!("No timeline object {id}."),
                "Read timeline.snapshot and use a current ID.",
            )
        })?;
        if seen.insert(parsed) {
            selection.push(parsed);
        }
    }
    Ok(selection)
}

fn expandable_take(project: &Project, id: &str) -> Result<ClipId, StoreError> {
    let clip = id
        .parse::<ClipId>()
        .map_err(|_| bad("expand", "a recorded clip ID"))?;
    let data = project
        .clips
        .find(clip)
        .and_then(|slot| project.clips.data(slot))
        .ok_or_else(|| {
            failure(
                "NOT_FOUND",
                &format!("No clip {clip}."),
                "Read timeline.snapshot and use a current clip ID.",
            )
        })?;
    let components = &data.enabled;
    let present = [
        components.screen,
        components.camera,
        components.mic,
        components.system,
        components.pointer,
        components.keys,
    ];
    if present.into_iter().all(|component| component.is_none()) {
        return Err(failure(
            "WRONG_KIND",
            &format!("Clip {clip} has no recorded components."),
            "Use the ID of a recorded take with component rows.",
        ));
    }
    Ok(clip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::TestDirectory;
    use splice_json::parse;
    use splice_model::{Clip, ClipData, ClipTiming};

    #[test]
    fn show_seeks_to_range_start_and_applies_tab_or_fit_without_seeking() {
        let dir = TestDirectory::new("show-controls");
        let mut store = Store::create(&dir.0.join("view.splice"), &Project::default()).unwrap();
        let ops = parse(r#"[{"cmd":"title.add","at":0,"length":60,"text":"Title"}]"#).unwrap();
        store
            .apply(ops.as_array().unwrap(), &Default::default())
            .unwrap();
        let mut viewer = Viewer::new(store, None);
        let cases = [
            (
                r#"{"range":{"from":5,"to":25}}"#,
                (Some(5), Some([5, 25]), None, None),
            ),
            (
                r#"{"view":"edit"}"#,
                (None, None, Some(ViewTab::Editor), None),
            ),
            (
                r#"{"view":"editor"}"#,
                (None, None, Some(ViewTab::Editor), None),
            ),
            (
                r#"{"view":"agent"}"#,
                (None, None, Some(ViewTab::Agent), None),
            ),
            (
                r#"{"view":"spec"}"#,
                (None, None, Some(ViewTab::Spec), None),
            ),
            (
                r#"{"viewer":{"zoom":"fit"}}"#,
                (None, None, None, Some(ViewerZoom::Fit)),
            ),
        ];
        let selection = parse(r#"["lan_spine"]"#).unwrap();
        for (args, expected) in cases {
            let result = viewer
                .show(&parse(args).unwrap(), 15, &mut |_, command| {
                    let ViewCommand::Show {
                        at,
                        range,
                        view,
                        zoom,
                        select,
                        expand,
                    } = command
                    else {
                        panic!("expected the show command");
                    };
                    assert_eq!((at, range, view, zoom), expected, "{args}");
                    assert!(select.is_none() && expand.is_none());
                    Ok(selection.clone())
                })
                .unwrap();
            assert_eq!(result.field("applied"), Some(&Json::Bool(true)));
            assert_eq!(
                result.field("playhead"),
                Some(&frame(expected.0.unwrap_or(15)))
            );
            assert_eq!(result.field("selection"), Some(&selection));
            assert_eq!(result.field("revision"), Some(&rev(1)));
        }
        viewer.close().unwrap();
    }

    #[test]
    fn show_rejects_invalid_view_controls_before_dispatch() {
        let dir = TestDirectory::new("show-invalid-controls");
        let store = Store::create(&dir.0.join("view.splice"), &Project::default()).unwrap();
        let mut viewer = Viewer::new(store, None);
        for field in [
            r#""range":[]"#,
            r#""range":{"from":1}"#,
            r#""range":{"to":2}"#,
            r#""range":{"from":2,"to":1}"#,
            r#""range":{"from":1,"to":1}"#,
            r#""range":{"from":-1,"to":2}"#,
            r#""range":{"from":1,"to":2.5}"#,
            r#""range":{"from":1,"to":2147483648}"#,
            r#""view":"timeline""#,
            r#""view":1"#,
            r#""viewer":[]"#,
            r#""viewer":{}"#,
            r#""viewer":{"zoom":"1"}"#,
            r#""viewer":{"zoom":0}"#,
            r#""viewer":{"zoom":-1}"#,
            r#""viewer":{"zoom":1e50}"#,
            r#""viewer":{"zoom":1e-60}"#,
        ] {
            let args = parse(&format!(r#"{{"at":5,"select":["lan_spine"],{field}}}"#)).unwrap();
            let error = viewer
                .show(&args, 0, &mut |_, _| panic!("dispatched invalid {field}"))
                .unwrap_err();
            assert_eq!(error.code, "BAD_ARGS", "{field}");
        }
        viewer.close().unwrap();
    }

    #[test]
    fn show_validates_selection_ids_and_expands_only_recorded_components() {
        let mut project = Project::default();
        let id = ClipId::from_serial(1);
        let slot = project
            .clips
            .insert(Clip {
                id,
                timing: ClipTiming {
                    start: 0,
                    length: 100,
                    source: 0,
                    speed: 1.0,
                },
                data: ClipData::default(),
            })
            .unwrap();
        let ids = splice_json::parse(r#"["lan_spine","clp_1","clp_1"]"#).unwrap();
        assert_eq!(
            parse_selection(&project, &ids).unwrap(),
            [
                ViewSelection::Lane(LaneId::SPINE),
                ViewSelection::Timeline(TimelineId::Clip(id)),
            ]
        );
        let malformed = splice_json::parse(r#"["not-an-id"]"#).unwrap();
        assert_eq!(
            parse_selection(&project, &malformed).unwrap_err().code,
            "BAD_ARGS"
        );
        let missing = splice_json::parse(r#"["clp_2"]"#).unwrap();
        assert_eq!(
            parse_selection(&project, &missing).unwrap_err().code,
            "NOT_FOUND"
        );
        assert_eq!(
            expandable_take(&project, "clp_1").unwrap_err().code,
            "WRONG_KIND"
        );
        project.clips.data_mut(slot).unwrap().enabled.mic = Some(false);
        assert_eq!(expandable_take(&project, "clp_1").unwrap(), id);
    }
}
