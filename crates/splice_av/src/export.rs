//! Export on OGPU (PLAN.md T12.7): AVFoundation decodes through the broker compositor and mixes the
//! audio; `splice_gpu` draws each frame; AVAssetWriter encodes. OGPU's Metal backend owns its
//! handles on the main thread, so an export is a stepper its owner advances from its own loop
//! (the MCP server, the viewer, the CLI) between other work.
use crate::{Error, TimedText, gpu::GpuProgram, lower::Plan, offscreen_device, offscreen_target};
use apple_sys::composition::{Reader, Writer, blit};
use ogpu_core::backend::{self as b, Device, PreparedRenderTarget, Queue, Texture};
use splice_model::Frame;
use std::time::{Duration, Instant};

/// Export progress or result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExportState {
    /// Fraction of frames encoded.
    Running(f32),
    /// The file is complete.
    Complete,
    /// Cancelled; the file was removed.
    Cancelled,
}

/// An export in progress. Call `step` from the main thread until it stops running.
pub struct Export {
    reader: Reader,
    writer: Writer,
    plan: Plan,
    size: [u32; 2],
    program: GpuProgram,
    queue: Queue,
    texture: Texture,
    target: PreparedRenderTarget,
    state: ExportState,
    cancel: bool,
    /// Frames written, half-open.
    range: [Frame; 2],
    /// Timeline and output frames per second: each timeline frame is drawn once and appended
    /// as the output frames that sample it (`splice_eval::resample`).
    rates: [u32; 2],
    /// Video goes through the GPU; audio-only exports just drain the reader.
    video: bool,
    /// Keeps the device that owns these resources alive.
    _device: Device,
}

/// A range drawn in order into RGBA buffers: an export's decode and draw without an encoder
/// (`Composition::frames`, for GIFs).
pub struct Frames {
    reader: Reader,
    plan: Plan,
    size: [u32; 2],
    program: GpuProgram,
    queue: Queue,
    texture: Texture,
    target: PreparedRenderTarget,
    /// Keeps the device that owns these resources alive.
    _device: Device,
}

impl Frames {
    pub(crate) fn start(
        reader: Reader,
        plan: Plan,
        text: &[TimedText],
        size: [u32; 2],
    ) -> Result<Self, Error> {
        let device = offscreen_device()?;
        let queue = b::create_queue(&device).map_err(gpu)?;
        let program = GpuProgram::with_plan(&device, &plan, text)?;
        let (texture, target) = offscreen_target(&device, size, 1.0)?;
        Ok(Self {
            reader,
            plan,
            size,
            program,
            queue,
            texture,
            target,
            _device: device,
        })
    }

    /// Decodes the next frame and, given a buffer (RGBA8 of the canvas size), draws it there.
    /// Returns its timeline frame, or `None` at the end of the range.
    pub fn next(&mut self, rgba: Option<&mut [u8]>) -> Result<Option<Frame>, Error> {
        let device = self.program.mtl_device();
        let Some((at, count)) = self.reader.next(device, self.program.sources_mut())? else {
            return Ok(None);
        };
        let Some(rgba) = rgba else {
            return Ok(Some(at));
        };
        let [width, height] = self.size.map(|n| n as usize);
        if rgba.len() != width * height * 4 {
            return Err(Error::Input(
                "Provide an RGBA buffer of the canvas size.".into(),
            ));
        }
        let commands = b::begin_commands(&self.queue).map_err(gpu)?;
        let native = b::native_commands(&commands)
            .map_err(gpu)?
            .mtl_command_buffer;
        if let Err(error) = self.program.draw(
            &self.plan,
            self.size,
            at,
            count,
            native,
            &commands,
            &mut self.target,
            false,
        ) {
            let _ = b::abandon(&commands);
            return Err(error);
        }
        let receipt = b::commit(&commands).map_err(gpu)?;
        b::complete_through(&self.queue, receipt.epoch).map_err(gpu)?;
        b::read_texture_into(&self.texture, (width * 4) as i64, rgba).map_err(gpu)?;
        Ok(Some(at))
    }
}

fn gpu(error: ogpu_core::error::Error) -> Error {
    Error::Input(ogpu_core::error::to_string(&error))
}

impl Export {
    pub(crate) fn start(
        reader: Reader,
        writer: Writer,
        plan: Plan,
        text: &[TimedText],
        [size, rates]: [[u32; 2]; 2],
        encoding: &apple_sys::composition::Encoding,
    ) -> Result<Self, Error> {
        let device = offscreen_device()?;
        let queue = b::create_queue(&device).map_err(gpu)?;
        let mut program = GpuProgram::with_plan(&device, &plan, text)?;
        program.transparent = encoding.codec.alpha();
        let clear = if program.transparent { 0.0 } else { 1.0 };
        let (texture, target) = offscreen_target(&device, size, clear)?;
        Ok(Self {
            reader,
            writer,
            plan,
            size,
            program,
            queue,
            texture,
            target,
            state: ExportState::Running(0.0),
            cancel: false,
            range: encoding.range,
            rates,
            video: encoding.codec.video(),
            _device: device,
        })
    }

    /// The last known progress or result, without doing work.
    pub fn status(&self) -> ExportState {
        self.state
    }

    /// Requests cancellation; the next `step` removes the file.
    pub fn cancel(&mut self) {
        self.cancel = true;
    }

    /// Encodes frames until `budget` has passed or the export ends. Returns the new state.
    pub fn step(&mut self, budget: Duration) -> Result<ExportState, Error> {
        if !matches!(self.state, ExportState::Running(_)) {
            return Ok(self.state);
        }
        let started = Instant::now();
        loop {
            if self.cancel {
                self.writer.finish(true)?;
                self.state = ExportState::Cancelled;
                return Ok(self.state);
            }
            if !self.frame()? {
                self.state = if self.writer.finish(false)? {
                    ExportState::Complete
                } else {
                    ExportState::Cancelled
                };
                return Ok(self.state);
            }
            if started.elapsed() >= budget {
                return Ok(self.state);
            }
        }
    }

    /// Draws and appends the next frame. False at the end.
    fn frame(&mut self) -> Result<bool, Error> {
        let device = self.program.mtl_device();
        let Some((at, count)) = self.reader.next(device, self.program.sources_mut())? else {
            return Ok(false);
        };
        let [from, to] = self.range;
        let span = (to - from).max(1) as f32;
        if !self.video {
            // Audio-only: drain the video so the mix keeps flowing.
            self.state = ExportState::Running(((at - from + 1) as f32 / span).clamp(0.0, 1.0));
            return Ok(true);
        }
        // A lower output rate skips this frame; a higher one appends it more than once.
        let [timeline, output] = self.rates;
        let [first, last] = splice_eval::resample::outputs(at - from, timeline, output);
        let drawn = b::native_texture(&self.texture).map_err(gpu)?.mtl_texture;
        for n in first..last {
            let out = self.writer.target(device)?;
            let commands = b::begin_commands(&self.queue).map_err(gpu)?;
            let native = b::native_commands(&commands)
                .map_err(gpu)?
                .mtl_command_buffer;
            let drawn_here = if n == first {
                self.program.draw(
                    &self.plan,
                    self.size,
                    at,
                    count,
                    native,
                    &commands,
                    &mut self.target,
                    true,
                )
            } else {
                Ok(false)
            };
            let encoded =
                drawn_here.and_then(|_| blit(native, &[drawn], &[out]).map_err(Error::from));
            if let Err(error) = encoded {
                let _ = b::abandon(&commands);
                return Err(error);
            }
            let receipt = b::commit(&commands).map_err(gpu)?;
            b::complete_through(&self.queue, receipt.epoch).map_err(gpu)?;
            self.writer.append(n)?;
        }
        self.state = ExportState::Running(((at - from + 1) as f32 / span).clamp(0.0, 1.0));
        Ok(true)
    }
}
