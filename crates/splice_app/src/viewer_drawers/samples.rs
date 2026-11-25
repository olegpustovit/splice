use splice_core::{
    commands::CommandError,
    history::{EditResult, Store, StoreError, WriteOptions},
};
use splice_json::Value as Json;
use splice_model::{AudioTrack, Media, MediaId, MediaKind, Probe, TakeComponents};
use std::{
    fs::{self, File, Metadata},
    io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const SCREEN_FILES: &[&str] = &[
    "screen.mov",
    "camera.mov",
    "mic.m4a",
    "system.m4a",
    "events.json",
    "cursors/b2aa4304c7470ab1ee2a469d2be7c292ccb92dac49f6f07f59e9477979c31403.json",
    "cursors/b2aa4304c7470ab1ee2a469d2be7c292ccb92dac49f6f07f59e9477979c31403.png",
];
const WEBCAM_FILES: &[&str] = &["webcam.mov", "camera.mov", "mic.m4a"];
const FPS: f64 = 30.0;
const WIDTH: u32 = 640;
const CODEC: &str = "hevc";
const AUDIO_CHANNELS: Option<u32> = Some(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sample {
    Screen,
    Webcam,
}

impl Sample {
    fn folder(self) -> &'static str {
        match self {
            Self::Screen => "v13-sample-screen",
            Self::Webcam => "v13-sample-webcam",
        }
    }

    fn files(self) -> &'static [&'static str] {
        match self {
            Self::Screen => SCREEN_FILES,
            Self::Webcam => WEBCAM_FILES,
        }
    }

    fn primary(self) -> &'static str {
        self.files()[0]
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Screen => "Screen take 01",
            Self::Webcam => "Webcam take 01",
        }
    }

    fn probe_spec(self) -> (i32, u32, &'static [&'static str]) {
        match self {
            Self::Screen => (540, 400, &["mic", "system"]),
            Self::Webcam => (300, 360, &["mic"]),
        }
    }

    fn probe(self) -> Probe {
        let (frames, height, audio) = self.probe_spec();
        Probe {
            fps: FPS,
            frames,
            width: WIDTH,
            height,
            audio: audio
                .iter()
                .map(|name| AudioTrack {
                    name: (*name).into(),
                    channels: AUDIO_CHANNELS,
                })
                .collect(),
            codec: Some(CODEC.into()),
            loudness: None,
        }
    }

    fn component_files(self) -> [Option<&'static str>; 6] {
        let screen = self == Self::Screen;
        [
            screen.then_some(self.primary()),
            Some("camera.mov"),
            Some("mic.m4a"),
            screen.then_some("system.m4a"),
            screen.then_some("events.json"),
            screen.then_some("cursors"),
        ]
    }

    fn components(self) -> TakeComponents {
        let [screen, camera, mic, system, events, cursors] =
            self.component_files().map(|file| file.map(str::to_owned));
        TakeComponents {
            screen,
            camera,
            mic,
            system,
            events,
            cursors,
            ..TakeComponents::default()
        }
    }

    pub(super) fn identify(media: &Media) -> Option<Self> {
        let take = media.take.as_ref()?;
        if media.kind != MediaKind::Take
            || media.bookmark.is_some()
            || media.probe.fps != FPS
            || media.probe.width != WIDTH
            || media.probe.codec.as_deref() != Some(CODEC)
        {
            return None;
        }
        let components = [
            take.screen.as_deref(),
            take.camera.as_deref(),
            take.mic.as_deref(),
            take.system.as_deref(),
            take.events.as_deref(),
            take.cursors.as_deref(),
        ];
        [Self::Screen, Self::Webcam].into_iter().find(|sample| {
            let (frames, height, audio) = sample.probe_spec();
            sample.path_matches(&media.path)
                && components == sample.component_files()
                && media.probe.frames == frames
                && media.probe.height == height
                && media
                    .probe
                    .audio
                    .iter()
                    .map(|track| (track.name.as_str(), track.channels))
                    .eq(audio.iter().map(|&name| (name, AUDIO_CHANNELS)))
        })
    }

    fn path_matches(self, path: &str) -> bool {
        let Some((folder, file)) = path
            .strip_prefix("takes/")
            .and_then(|relative| relative.split_once('/'))
        else {
            return false;
        };
        file == self.primary()
            && (folder == self.folder()
                || folder
                    .strip_prefix(self.folder())
                    .and_then(|suffix| suffix.strip_prefix('-'))
                    .is_some_and(|suffix| {
                        !suffix.starts_with('0')
                            && suffix.bytes().all(|byte| byte.is_ascii_digit())
                            && suffix.parse::<u32>().is_ok_and(|number| number >= 2)
                    }))
    }
}

#[derive(Default)]
pub(super) struct Bundle {
    root: PathBuf,
    ready: [bool; 2],
}

impl Bundle {
    pub(super) fn new(resources: PathBuf) -> Self {
        let root = resources.join("v13-samples");
        let ready = [Sample::Screen, Sample::Webcam].map(|sample| complete(&root, sample));
        Self { root, ready }
    }

    pub(super) fn ready(&self, sample: Sample) -> bool {
        self.ready[match sample {
            Sample::Screen => 0,
            Sample::Webcam => 1,
        }]
    }

    pub(super) fn append(
        &self,
        store: &mut Store,
        sample: Sample,
        time: u64,
    ) -> Result<EditResult, StoreError> {
        let options = WriteOptions {
            time,
            ..WriteOptions::default()
        };
        store.check_write(&options)?;
        if !self.ready(sample) || !complete(&self.root, sample) {
            return Err(StoreError::new(
                "MEDIA_OFFLINE",
                "The bundled sample take is incomplete.",
                "Reinstall Splice to restore the sample media.",
            ));
        }

        let probe = sample.probe();
        let take = sample.components();
        let takes = store.path().join("takes");
        if real_directory(&takes)
            && let Some(id) = store.project().media.iter().find_map(|media| {
                let path = Path::new(&media.path);
                (media.kind == MediaKind::Take
                    && media.probe == probe
                    && media.take.as_ref() == Some(&take)
                    && sample.path_matches(&media.path)
                    && same_sample(&self.root, &store.path().join(path.parent()?), sample))
                .then_some(media.id)
            })
        {
            return store.apply(&[insert("media", id.to_string())], &options);
        }

        match fs::create_dir(&takes) {
            Ok(()) => {}
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists && real_directory(&takes) => {}
            Err(error) => return Err(storage(&error)),
        }
        let mut destination = None;
        for suffix in 1..=u32::MAX {
            let name = if suffix == 1 {
                sample.folder().to_owned()
            } else {
                format!("{}-{suffix}", sample.folder())
            };
            let relative = format!("takes/{name}");
            let directory = store.path().join(&relative);
            if store
                .project()
                .media
                .iter()
                .any(|media| store.path().join(&media.path).starts_with(&directory))
            {
                continue;
            }
            match fs::create_dir(&directory) {
                Ok(()) => {
                    destination = Some((relative, directory));
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(storage(&error)),
            }
        }
        let Some((relative, directory)) = destination else {
            return Err(StoreError::new(
                "EXISTS",
                "All directories for this sample take are occupied.",
                "Choose another project for the sample.",
            ));
        };
        let result = self
            .copy_to(sample, &directory, &takes, store.path())
            .and_then(|metadata| {
                let path = format!("{relative}/{}", sample.primary());
                let descriptor = Media {
                    id: MediaId::from_serial(0),
                    kind: MediaKind::Take,
                    path: path.clone(),
                    bookmark: None,
                    size: metadata.len(),
                    mtime: metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
                        .unwrap_or_default(),
                    probe,
                    take: Some(take),
                };
                store.apply_with_media(&[insert("path", path.clone())], &options, &mut |request| {
                    if request == path {
                        Ok(descriptor.clone())
                    } else {
                        Err(CommandError::new(
                            "BAD_MEDIA",
                            "The sample import requested a different file.".into(),
                            "Try inserting the sample again.".into(),
                        ))
                    }
                })
            });
        // A failed journal rollback may still have a durable reference to this directory.
        if result.is_err() && store.problem().is_none() {
            if let Err(error) = fs::remove_dir_all(&directory).and_then(|()| sync(&takes)) {
                eprintln!(
                    "splice: could not clean up {}: {error}",
                    directory.display()
                );
            }
        }
        result
    }

    fn copy_to(
        &self,
        sample: Sample,
        directory: &Path,
        takes: &Path,
        package: &Path,
    ) -> Result<Metadata, StoreError> {
        let copy = || {
            if sample == Sample::Screen {
                fs::create_dir(directory.join("cursors"))?;
            }
            for name in sample.files() {
                let mut input = File::open(self.root.join(name))?;
                let mut output = File::options()
                    .write(true)
                    .create_new(true)
                    .open(directory.join(name))?;
                if io::copy(&mut input, &mut output)? == 0 {
                    return Err(io::Error::other("A sample resource is empty."));
                }
                output.sync_all()?;
            }
            if sample == Sample::Screen {
                sync(&directory.join("cursors"))?;
            }
            for dir in [directory, takes, package] {
                sync(dir)?;
            }
            directory.join(sample.primary()).metadata()
        };
        copy().map_err(|error| storage(&error))
    }
}

fn real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

fn complete(root: &Path, sample: Sample) -> bool {
    real_directory(root)
        && (sample != Sample::Screen || real_directory(&root.join("cursors")))
        && sample.files().iter().all(|file| {
            fs::symlink_metadata(root.join(file))
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() != 0)
        })
}

fn same_sample(source: &Path, local: &Path, sample: Sample) -> bool {
    complete(local, sample)
        && sample.files().iter().all(|file| {
            fs::read(source.join(file)).is_ok_and(|bytes| {
                fs::metadata(local.join(file)).is_ok_and(|meta| meta.len() == bytes.len() as u64)
                    && fs::read(local.join(file)).is_ok_and(|copied| copied == bytes)
            })
        })
}

fn insert(field: &str, value: String) -> Json {
    Json::Object(vec![
        ("cmd".into(), Json::String("clip.insert".into())),
        (field.into(), Json::String(value)),
    ])
}

fn sync(directory: &Path) -> io::Result<()> {
    File::open(directory)?.sync_all()
}

fn storage(error: &io::Error) -> StoreError {
    StoreError::new(
        "IO",
        &format!("Could not copy the bundled sample take: {error}"),
        "Check that the project is writable and has free space, then retry.",
    )
}
