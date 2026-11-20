//! Native lowering of Splice's prepared playback plan.
#![cfg(target_os = "macos")]
#![deny(missing_docs)]

mod audio;
mod export;
mod gpu;
mod lower;
mod lut;
mod takes;

pub use export::{Export, ExportState};
pub use gpu::GpuProgram;

use apple_sys::composition;
use splice_eval::{EvalError, Evaluator};
use splice_model::{Frame, Project};
use std::{
    fmt,
    path::{Path, PathBuf},
};

pub use apple_sys::image::{png as encode_png, png_video as encode_frame_png};
pub use composition::{Codec, Encoding, Player};

/// A prepared timeline and its native composition for playback and delivery.
pub struct Composition {
    native: composition::Composition,
    evaluator: std::sync::Arc<Evaluator>,
    size: [u32; 2],
    end: Frame,
    text: Vec<TimedText>,
    plan: lower::Plan,
    /// Missing files drawn as placeholders, by media (never in an export).
    offline: Vec<(splice_model::MediaId, PathBuf)>,
    /// The frame-grab renderer, made on first use.
    offscreen: std::cell::RefCell<Option<Offscreen>>,
    /// The last read's decode, compose (draw and GPU completion) and readback times.
    timings: std::cell::Cell<[std::time::Duration; 3]>,
}

struct Offscreen {
    /// Keeps the device that owns these resources alive.
    _device: ogpu_core::backend::Device,
    queue: ogpu_core::backend::Queue,
    program: GpuProgram,
    texture: ogpu_core::backend::Texture,
    target: ogpu_core::backend::PreparedRenderTarget,
}

/// A headless OGPU device for frame grabs and export (the viewer shares its window's).
pub(crate) fn offscreen_device() -> Result<ogpu_core::backend::Device, Error> {
    let gpu = |error: ogpu_core::error::Error| Error::Input(ogpu_core::error::to_string(&error));
    ogpu_core::impl_::select(ogpu_metal::impl_::implementation());
    let driver = ogpu_core::impl_::create_driver().map_err(gpu)?;
    ogpu_core::backend::create_device(driver).map_err(gpu)
}

/// An RGBA canvas texture and its prepared render target, cleared to black.
pub(crate) fn offscreen_target(
    device: &ogpu_core::backend::Device,
    [width, height]: [u32; 2],
    alpha: f64,
) -> Result<
    (
        ogpu_core::backend::Texture,
        ogpu_core::backend::PreparedRenderTarget,
    ),
    Error,
> {
    use ogpu_core::types::{TextureDescriptor, TextureFormat, TextureUsage};
    let gpu = |error: ogpu_core::error::Error| Error::Input(ogpu_core::error::to_string(&error));
    let texture = ogpu_core::backend::create_texture(
        device,
        TextureDescriptor {
            label: Some("Splice canvas".into()),
            width: i64::from(width),
            height: i64::from(height),
            depth: 1,
            mip_levels: 1,
            sample_count: 1,
            format: TextureFormat::Rgba8Unorm,
            usage: vec![
                TextureUsage::TextureBinding,
                TextureUsage::RenderAttachment,
                TextureUsage::TextureCopySrc,
            ],
        },
    )
    .map_err(gpu)?;
    let target =
        splice_gpu::Compositor::target(device, &texture, [0.0, 0.0, 0.0, alpha]).map_err(gpu)?;
    Ok((texture, target))
}

/// A resolved title or caption shared by the export and viewer.
#[derive(Clone)]
pub struct TimedText {
    range: [Frame; 2],
    /// Resolved text, placement, color and opacity ramp.
    pub layer: composition::TextLayer,
    /// The title or caption it draws; its per-frame opacity and look come from the evaluator.
    item: Option<splice_model::ItemId>,
    /// Animator: start frame, frames to come in, preset.
    reveal: Option<(Frame, f64, splice_eval::Animate)>,
}

/// Where `media.analyze {kinds:["people"]}` writes a media's person matte.
pub fn matte_path(package: &Path, media: splice_model::MediaId) -> PathBuf {
    package
        .join("analysis")
        .join(media.to_string())
        .join("matte.mov")
}

/// Where `media.analyze {kinds:["matte"]}` writes a take camera's person matte.
pub fn camera_matte_path(package: &Path, media: splice_model::MediaId) -> PathBuf {
    matte_path(package, media).with_file_name("camera-matte.mov")
}

impl TimedText {
    /// Samples the opacity ramp at a frame in the cue's interval.
    pub fn opacity(&self, at: Frame) -> f32 {
        let fraction = (at - self.range[0]) as f32 / (self.range[1] - self.range[0]) as f32;
        let [a, b] = self.layer.opacity;
        a + (b - a) * fraction
    }
}

/// Why the program could not be prepared or rendered.
#[derive(Debug)]
pub enum Error {
    /// The evaluator could not prepare the project.
    Plan(EvalError),
    /// A rendered layer refers to a missing input.
    Input(String),
    /// A native service rejected the composition or requested frame.
    Native(apple_sys::Error),
    /// A source file the timeline uses is missing.
    Offline(PathBuf),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(error) => error.fmt(f),
            Self::Input(message) => f.write_str(message),
            Self::Native(error) => error.fmt(f),
            Self::Offline(path) => write!(f, "Media is offline: {}", path.display()),
        }
    }
}

impl std::error::Error for Error {}

impl From<apple_sys::Error> for Error {
    fn from(error: apple_sys::Error) -> Self {
        Self::Native(error)
    }
}

impl Composition {
    /// Builds the program for delivery from a read-only project revision and its package
    /// directory. A missing source file is `Error::Offline`: an export never has placeholders.
    pub fn prepare(project: &Project, package: &Path) -> Result<Self, Error> {
        Self::build(project, package, false, false)
    }

    /// The program for renders an agent sees (view.frames, view.check): keystroke chips land
    /// where and when export draws them, with every typed character replaced by a dot; key
    /// text stays human-only (D19), the geometry stays the export's (A1). Offline media draws
    /// as a placeholder at the item's geometry; `offline` lists it.
    pub fn prepare_for_agent(project: &Project, package: &Path) -> Result<Self, Error> {
        Self::build(project, package, true, true)
    }

    /// The program the viewer shows: export's, with offline media drawn as placeholders.
    pub fn prepare_for_viewer(project: &Project, package: &Path) -> Result<Self, Error> {
        Self::build(project, package, false, true)
    }

    fn build(
        project: &Project,
        package: &Path,
        redact_keys: bool,
        placeholders: bool,
    ) -> Result<Self, Error> {
        let pointers = lower::pointers(project, package)?;
        let evaluator =
            std::sync::Arc::new(Evaluator::prepare_with(project, &pointers).map_err(Error::Plan)?);
        let cuts = evaluator.breakpoints();
        let end = *cuts.last().unwrap_or(&0);
        let (native, text, plan, offline) = lower::build(
            project,
            package,
            &evaluator,
            &cuts,
            end,
            redact_keys,
            placeholders,
        )?;
        Ok(Self {
            native,
            evaluator,
            size: project.size,
            end,
            text,
            plan,
            offline,
            offscreen: std::cell::RefCell::new(None),
            timings: Default::default(),
        })
    }

    /// Output dimensions in canvas pixels.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Exclusive end of the composition in timeline frames.
    pub fn end(&self) -> Frame {
        self.end
    }

    /// Missing source files drawn as placeholders (or silent, for audio), with their media.
    pub fn offline(&self) -> &[(splice_model::MediaId, PathBuf)] {
        &self.offline
    }

    /// Playback evaluator for overlays drawn by the viewer.
    pub fn evaluator(&self) -> &Evaluator {
        &self.evaluator
    }

    /// Title and caption cards visible at this timeline frame, in paint order.
    pub fn text_at(&self, at: Frame) -> &[TimedText] {
        let first = self.text.partition_point(|cue| cue.range[1] <= at);
        let rest = &self.text[first..];
        &rest[..rest.partition_point(|cue| cue.range[0] <= at)]
    }

    /// Creates a paused playback output for this revision.
    pub fn player(&self) -> Result<Player, Error> {
        self.native.player().map_err(Into::into)
    }

    /// Reads one offscreen program frame, text included, into a caller-owned RGBA8 buffer.
    pub fn read_frame(&self, at: Frame, rgba: &mut [u8]) -> Result<(), Error> {
        self.read(at, rgba, 0)
    }

    /// Reads a frame, then composes the same decoded sources `repeats` more times back to back,
    /// as the viewer does at its frame rate. `last_frame_timings` then reports the fastest warm
    /// compose, free of the GPU waking after a from-scratch decode.
    pub fn read_frame_warm(&self, at: Frame, rgba: &mut [u8], repeats: u32) -> Result<(), Error> {
        self.read(at, rgba, repeats)
    }

    fn read(&self, at: Frame, rgba: &mut [u8], repeats: u32) -> Result<(), Error> {
        use ogpu_core::backend as b;
        let gpu =
            |error: ogpu_core::error::Error| Error::Input(ogpu_core::error::to_string(&error));
        let [width, height] = self.size.map(|n| n as usize);
        if !(0..self.end).contains(&at) || rgba.len() != width * height * 4 {
            return Err(Error::Input(
                "Provide a frame within the timeline and a matching RGBA buffer.".into(),
            ));
        }
        let mut slot = self.offscreen.borrow_mut();
        if slot.is_none() {
            let device = offscreen_device()?;
            let (texture, target) = offscreen_target(&device, self.size, 1.0)?;
            *slot = Some(Offscreen {
                queue: b::create_queue(&device).map_err(gpu)?,
                program: GpuProgram::with_plan(&device, &self.plan, &self.text)?,
                _device: device,
                texture,
                target,
            });
        }
        let offscreen = slot.as_mut().expect("offscreen renderer");
        let device_ptr = offscreen.program.mtl_device();
        let started = std::time::Instant::now();
        let count = self
            .native
            .grab(device_ptr, at, offscreen.program.sources_mut())?;
        let decoded = std::time::Instant::now();
        let mut compose = std::time::Duration::MAX;
        for _ in 0..=repeats {
            let began = std::time::Instant::now();
            let commands = b::begin_commands(&offscreen.queue).map_err(gpu)?;
            let native = b::native_commands(&commands)
                .map_err(gpu)?
                .mtl_command_buffer;
            if let Err(error) = offscreen.program.draw(
                &self.plan,
                self.size,
                at,
                count,
                native,
                &commands,
                &mut offscreen.target,
                false,
            ) {
                let _ = b::abandon(&commands);
                return Err(error);
            }
            let receipt = b::commit(&commands).map_err(gpu)?;
            b::complete_through(&offscreen.queue, receipt.epoch).map_err(gpu)?;
            compose = compose.min(began.elapsed());
        }
        let composed = std::time::Instant::now();
        b::read_texture_into(&offscreen.texture, (width * 4) as i64, rgba).map_err(gpu)?;
        self.timings
            .set([decoded - started, compose, composed.elapsed()]);
        Ok(())
    }

    /// Decode, compose (draw and GPU completion) and readback times of the last `read_frame`.
    pub fn last_frame_timings(&self) -> [std::time::Duration; 3] {
        self.timings.get()
    }

    /// Starts encoding the composition and its audio mix with `encoding` (its range clamped to
    /// the timeline). Advance the returned export with `step` on the main thread.
    pub fn start_export(
        &self,
        file: &Path,
        encoding: &composition::Encoding,
    ) -> Result<Export, Error> {
        if let Some((_, path)) = self.offline.first() {
            return Err(Error::Offline(path.clone()));
        }
        let mut encoding = encoding.clone();
        encoding.range = [
            encoding.range[0].clamp(0, self.end),
            encoding.range[1].clamp(0, self.end),
        ];
        if encoding.range[1] <= encoding.range[0] {
            encoding.range = [0, self.end];
        }
        let reader = self.native.reader(Some(encoding.range))?;
        let writer = composition::Writer::new(&reader, file, &encoding)?;
        Export::start(
            reader,
            writer,
            self.plan.clone(),
            &self.text,
            self.size,
            &encoding,
        )
    }
}
