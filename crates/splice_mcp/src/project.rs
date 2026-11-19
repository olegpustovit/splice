use super::{Json, object, text, timeline, wire::*};
use splice_core::{
    history::{Store, StoreError},
    package,
};
use splice_model::{Access, Aspect, LaneKind, Media, MediaId, MediaKind, Project};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct Projects {
    open: Vec<OpenProject>,
    viewer_only: bool,
    #[cfg(target_os = "macos")]
    pub(super) exports: Vec<super::export::Job>,
    #[cfg(target_os = "macos")]
    pub(super) recordings: Vec<super::record::Job>,
}

enum OpenProject {
    Local(Store),
    #[cfg(target_os = "macos")]
    Remote(PathBuf, String),
}

impl OpenProject {
    fn path(&self) -> &Path {
        match self {
            Self::Local(store) => store.path(),
            #[cfg(target_os = "macos")]
            Self::Remote(path, _) => path,
        }
    }

    fn local(&self) -> Option<&Store> {
        match self {
            Self::Local(store) => Some(store),
            #[cfg(target_os = "macos")]
            Self::Remote(..) => None,
        }
    }

    fn local_mut(&mut self) -> Option<&mut Store> {
        match self {
            Self::Local(store) => Some(store),
            #[cfg(target_os = "macos")]
            Self::Remote(..) => None,
        }
    }

    fn into_local(self) -> Option<Store> {
        match self {
            Self::Local(store) => Some(store),
            #[cfg(target_os = "macos")]
            Self::Remote(..) => None,
        }
    }
}

impl Projects {
    #[cfg(any(target_os = "macos", test))]
    pub(super) fn viewer(store: Store) -> Self {
        Self {
            open: vec![OpenProject::Local(store)],
            viewer_only: true,
            #[cfg(target_os = "macos")]
            exports: Vec::new(),
            #[cfg(target_os = "macos")]
            recordings: Vec::new(),
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn viewer_store(&mut self) -> &mut Store {
        let OpenProject::Local(store) = &mut self.open[0] else {
            unreachable!("viewer owns its store")
        };
        store
    }

    #[cfg(target_os = "macos")]
    pub(super) fn viewer_store_ref(&self) -> &Store {
        self.open[0].local().expect("viewer owns its store")
    }

    #[cfg(target_os = "macos")]
    pub(super) fn remote(&self, id: &str) -> Option<(&Path, &str)> {
        let OpenProject::Remote(path, owner) = self.open.get(project_index(id)?)? else {
            return None;
        };
        Some((path, owner))
    }

    #[cfg(target_os = "macos")]
    pub(super) fn remember_remote(&mut self, path: PathBuf, owner: String) -> String {
        if let Some(index) = self.open.iter().position(|entry| entry.path() == path) {
            self.open[index] = OpenProject::Remote(path, owner);
            return project_id(index);
        }
        let id = project_id(self.open.len());
        self.open.push(OpenProject::Remote(path, owner));
        id
    }

    #[cfg(target_os = "macos")]
    pub(super) fn is_local(&self, path: &Path) -> bool {
        self.open
            .iter()
            .any(|entry| entry.path() == path && entry.local().is_some())
    }

    /// Packages this headless session holds the writer lock for.
    #[cfg(target_os = "macos")]
    pub(super) fn local_paths(&self) -> Vec<&Path> {
        if self.viewer_only {
            return Vec::new();
        }
        self.open
            .iter()
            .filter_map(|entry| entry.local().map(Store::path))
            .collect()
    }

    /// Closes the local writer for `path` so a viewer can take it. Later calls with the same
    /// project ID forward to that viewer.
    #[cfg(target_os = "macos")]
    pub(super) fn hand_off(&mut self, path: &Path) -> Result<(), StoreError> {
        let index = self
            .open
            .iter()
            .position(|entry| entry.path() == path && entry.local().is_some())
            .ok_or_else(|| {
                failure(
                    "NOT_FOUND",
                    "This session no longer holds the project.",
                    "Open the project again.",
                )
            })?;
        let id = project_id(index);
        if self.exports.iter().any(|job| job.active())
            || self.recordings.iter().any(|job| job.active(&id))
        {
            return Err(failure(
                "BUSY",
                "An agent export or recording is still running on this project.",
                "Wait for the agent's job to finish, then open the project again.",
            ));
        }
        let entry = std::mem::replace(
            &mut self.open[index],
            OpenProject::Remote(path.to_path_buf(), String::new()),
        );
        entry.into_local().expect("checked local entry").close()
    }

    /// Advances running exports for up to `budget` in total. True while any is running.
    #[cfg(target_os = "macos")]
    pub(super) fn tick_exports(&mut self, budget: std::time::Duration) -> bool {
        let running = self.exports.iter().filter(|job| job.active()).count() as u32;
        if running == 0 {
            return false;
        }
        for job in self.exports.iter_mut().filter(|job| job.active()) {
            job.step(budget / running);
        }
        self.exports.iter().any(|job| job.active())
    }

    pub(super) fn lookup(&mut self, args: &Json) -> Result<&mut Store, StoreError> {
        self.lookup_id(args.string("projectId")?)
    }

    pub(super) fn lookup_id(&mut self, id: &str) -> Result<&mut Store, StoreError> {
        let entry = project_index(id)
            .and_then(|index| self.open.get_mut(index))
            .ok_or_else(|| {
                failure(
                    "NOT_FOUND",
                    &format!("Project {id} is not open."),
                    "Call project.info with the package path.",
                )
            })?;
        let store = entry.local_mut().ok_or_else(|| {
            failure(
                "VIEWER_DISCONNECTED",
                "The viewer owns this project.",
                "Reopen the project with project.info.",
            )
        })?;
        read_access(store)?;
        Ok(store)
    }

    pub(super) fn info(&mut self, args: &Json) -> Result<Json, StoreError> {
        let create = args.flag("create")?;
        let id = args.optional_string("projectId")?;
        if let Some(path) = args.optional_string("path")? {
            if id.is_some() {
                return Err(bad("path", "the only project selector, or pass projectId"));
            }
            let path = package_path(path)?;
            let canonical = fs::canonicalize(&path);
            let existing = canonical
                .as_ref()
                .ok()
                .and_then(|path| self.open.iter().position(|entry| entry.path() == path));
            let index = if let Some(index) = existing
                && self.open[index].local().is_some()
            {
                index
            } else {
                if self.viewer_only {
                    return Err(failure(
                        "NOT_FOUND",
                        "This viewer does not own the requested package.",
                        "Open that package in a separate viewer or headless session.",
                    ));
                }
                let store = match canonical {
                    Ok(path) => Store::open(&path)?,
                    Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
                        Store::create(&path, &Project::default())?
                    }
                    Err(error) => return Err(path_error(&path, &error)),
                };
                read_access(&store)?;
                if let Some(index) = existing {
                    self.open[index] = OpenProject::Local(store);
                    index
                } else {
                    self.open.push(OpenProject::Local(store));
                    self.open.len() - 1
                }
            };
            let store = self.lookup_id(&project_id(index))?;
            return info(store, &project_id(index));
        }
        if create {
            return Err(bad(
                "path",
                "a new .splice package path when create is true",
            ));
        }
        let store = self.lookup(args)?;
        info(store, id.expect("lookup validates project ID"))
    }

    pub(super) fn context(&self, args: &Json, result: &Json) -> Option<&Store> {
        if let Some(id) = result
            .field("projectId")
            .or_else(|| args.field("projectId"))
        {
            return self.open.get(project_index(id.as_str()?)?)?.local();
        }
        if let Some(path) = args.field("path") {
            let path = fs::canonicalize(package_path(path.as_str()?).ok()?).ok()?;
            return self.open.iter().find(|store| store.path() == path)?.local();
        }
        match self.open.as_slice() {
            [store] => store.local(),
            _ => None,
        }
    }

    pub(super) fn close(self) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        drop(self.recordings);
        #[cfg(target_os = "macos")]
        for mut job in self.exports {
            job.wait();
        }
        for entry in self.open {
            if let Some(store) = entry.into_local()
                && store.problem().is_none()
            {
                store.close().map_err(io::Error::other)?;
            }
        }
        Ok(())
    }
}

fn project_index(id: &str) -> Option<usize> {
    id.strip_prefix("prj_")
        .and_then(|n| n.parse::<usize>().ok())
        .and_then(|n| n.checked_sub(1))
        .filter(|&i| project_id(i) == id)
}

fn project_id(index: usize) -> String {
    format!("prj_{}", index + 1)
}

pub(super) fn package_path(value: &str) -> Result<PathBuf, StoreError> {
    let path = if let Some(relative) = value.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(relative))
            .ok_or_else(|| bad("path", "an explicit .splice path when HOME is unavailable"))?
    } else {
        PathBuf::from(value)
    };
    if value.contains('\0') || path.extension().and_then(|ext| ext.to_str()) != Some("splice") {
        return Err(bad("path", "a path ending in .splice"));
    }
    Ok(path)
}

fn path_error(path: &Path, error: &io::Error) -> StoreError {
    if error.kind() == io::ErrorKind::NotFound {
        failure(
            "NOT_FOUND",
            &format!("No package at {}.", path.display()),
            "Pass an existing .splice path or add create:true to project.info.",
        )
    } else {
        failure(
            "IO",
            &format!("{}: {error}", path.display()),
            "Check the project path and filesystem permissions.",
        )
    }
}

fn read_access(store: &Store) -> Result<(), StoreError> {
    if store.project().access == Access::Off {
        return Err(failure(
            "PERMISSION_DENIED",
            "Agent access is off for this project.",
            "Ask the editor to enable read or live agent access.",
        ));
    }
    Ok(())
}

pub(super) fn access(level: Access) -> Json {
    text(match level {
        Access::Live => "live",
        Access::Read => "read",
        Access::Off => "off",
    })
}

pub(super) fn lane_kind(kind: LaneKind) -> &'static str {
    match kind {
        LaneKind::Graphics => "graphics",
        LaneKind::Captions => "captions",
        LaneKind::Overlay => "overlay",
        LaneKind::Motion => "motion",
        LaneKind::Audio => "audio",
    }
}

pub(super) fn media_path(store: &Store, media: &Media) -> PathBuf {
    store.path().join(&media.path)
}

pub(super) fn media_name(p: &Project, id: MediaId) -> String {
    p.media
        .iter()
        .find(|media| media.id == id)
        .and_then(|media| {
            Path::new(&media.path)
                .file_name()
                .and_then(|name| name.to_str())
        })
        .map_or_else(|| id.to_string(), str::to_owned)
}

fn info(store: &Store, id: &str) -> Result<Json, StoreError> {
    let p = store.project();
    let mut lanes: Vec<_> = p
        .lanes
        .iter()
        .map(|lane| {
            let items = p
                .items
                .order()
                .iter()
                .filter(|&&slot| p.items.data(slot).unwrap().lane == lane.id)
                .count();
            object(vec![
                ("id", display(&lane.id)),
                ("name", text(&lane.name)),
                ("kind", text(lane_kind(lane.kind))),
                ("items", count(items)),
            ])
        })
        .collect();
    lanes.push(object(vec![
        ("id", display(&p.spine.id)),
        ("name", text(&p.spine.name)),
        ("kind", text("spine")),
        ("items", count(p.clips.order().len())),
    ]));

    let mut offline = Vec::new();
    let media = p
        .media
        .iter()
        .map(|media| {
            let missing = !media_path(store, media).is_file();
            if missing {
                offline.push(object(vec![
                    ("id", display(&media.id)),
                    ("path", text(&media.path)),
                ]));
            }
            object(vec![
                ("id", display(&media.id)),
                ("name", Json::String(media_name(p, media.id))),
                (
                    "type",
                    text(match media.kind {
                        MediaKind::Take => "take",
                        MediaKind::Video => "video",
                        MediaKind::Audio => "audio",
                        MediaKind::Image => "image",
                        MediaKind::Compound => "compound",
                    }),
                ),
                ("frames", frame(media.probe.frames)),
                ("path", text(&media.path)),
                ("offline", Json::Bool(missing)),
            ])
        })
        .collect();

    let transcript = if let Some(first) = p.transcripts.first() {
        object(vec![
            ("state", text("ready")),
            ("hash", hash(first.hash)),
            ("words", count(first.words.len())),
        ])
    } else {
        object(vec![("state", text("unavailable")), ("words", count(0))])
    };
    let snapshot = package::snapshot(p);
    let mut effects = vec![
        ("project".into(), snapshot.field("effects").unwrap().clone()),
        (
            p.spine.id.to_string(),
            snapshot
                .field("spine")
                .unwrap()
                .field("effects")
                .unwrap()
                .clone(),
        ),
    ];
    for lane in snapshot.field("lanes").and_then(Json::as_array).unwrap() {
        let applied = lane.field("effects").unwrap();
        if applied
            .as_array()
            .is_some_and(|effects| !effects.is_empty())
        {
            effects.push((
                lane.field("id").and_then(Json::as_str).unwrap().into(),
                applied.clone(),
            ));
        }
    }
    let mut fields = vec![
        ("projectId", text(id)),
        ("revision", rev(p.rev)),
        ("fps", Json::Unsigned(u64::from(p.fps))),
        (
            "size",
            Json::Array(
                p.size
                    .map(|value| Json::Unsigned(u64::from(value)))
                    .to_vec(),
            ),
        ),
        (
            "aspect",
            text(match p.aspect {
                Aspect::Portrait => "9:16",
                Aspect::Landscape => "16:9",
                Aspect::Square => "1:1",
                Aspect::Social => "4:5",
                Aspect::Standard => "4:3",
            }),
        ),
        ("durationFrames", frame(timeline::duration(p)?)),
        ("access", access(p.access)),
        ("lanes", Json::Array(lanes)),
        ("media", Json::Array(media)),
        ("offlineMedia", Json::Array(offline)),
        ("transcript", transcript),
        ("effects", Json::Object(effects)),
        (
            "openSession",
            store
                .sessions()
                .iter()
                .rev()
                .find(|item| item.open)
                .map_or(Json::Null, |item| display(&item.id)),
        ),
    ];
    if let Some(problem) = store.problem() {
        fields.push(("warning", error_payload(problem.clone().into())));
    }
    Ok(object(fields))
}
