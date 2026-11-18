use crate::{
    Json, export::Job, friction::Recorder, object, platform, project::Projects, text, timeline,
    tools, wire::*,
};
use splice_core::{
    history::{Store, StoreError, WriteOptions},
    package,
};
use splice_model::JobId;
use std::{
    io,
    path::{Path, PathBuf},
};

/// Viewer-only intents handled on the window thread.
pub enum ViewCommand {
    /// Move to the given timeline frame.
    Seek(i32),
    /// Render the viewer into a PNG.
    Snapshot {
        /// Requested timeline frame.
        at: i32,
        /// Absolute PNG output location.
        output: PathBuf,
    },
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
        if !tools::known(name) {
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
        let at = if let Some(value) = args.field("at") {
            Some(splice_core::edit::resolve_view_frame(
                store.project(),
                value,
            )?)
        } else if let Some(range) = args.field("range") {
            Some(
                range
                    .frame("from")?
                    .ok_or_else(|| bad("range", "an object with from and to frames"))?,
            )
        } else {
            None
        };
        let mut frame_at = current;
        if let Some(at) = at {
            frame_at = at.clamp(
                0,
                timeline::duration(store.project())?
                    .saturating_sub(1)
                    .max(0),
            );
            view(store, ViewCommand::Seek(frame_at))?;
        }
        let mut fields = vec![
            ("applied", Json::Bool(at.is_some())),
            ("revision", rev(store.project().rev)),
            ("playhead", frame(frame_at)),
            ("selection", Json::Array(Vec::new())),
        ];
        if at.is_none()
            && ["select", "expand", "view", "viewer"]
                .iter()
                .any(|key| args.field(key).is_some())
        {
            fields.push(("reason", text("viewer controls the playhead only")));
        }
        Ok(object(fields))
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
                let hevc = platform::export_options(request.optional_string("preset")?, path)?;
                let id = JobId::from_serial(self.projects.exports.len() as u64 + 1);
                let job = Job::start(
                    self.store(),
                    id,
                    None,
                    hevc,
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
                    .status()
            }
            _ => Err(bad("command", "a supported viewer command")),
        }
    }
}
