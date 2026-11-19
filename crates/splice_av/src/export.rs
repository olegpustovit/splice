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
    end: Frame,
    program: GpuProgram,
    queue: Queue,
    texture: Texture,
    target: PreparedRenderTarget,
    state: ExportState,
    cancel: bool,
    /// Keeps the device that owns these resources alive.
    _device: Device,
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
        size: [u32; 2],
        end: Frame,
    ) -> Result<Self, Error> {
        let device = offscreen_device()?;
        let queue = b::create_queue(&device).map_err(gpu)?;
        let program = GpuProgram::with_plan(&device, &plan, text)?;
        let (texture, target) = offscreen_target(&device, size)?;
        Ok(Self {
            reader,
            writer,
            plan,
            size,
            end,
            program,
            queue,
            texture,
            target,
            state: ExportState::Running(0.0),
            cancel: false,
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
        let out = self.writer.target(device)?;
        let drawn = b::native_texture(&self.texture).map_err(gpu)?.mtl_texture;
        let commands = b::begin_commands(&self.queue).map_err(gpu)?;
        let native = b::native_commands(&commands)
            .map_err(gpu)?
            .mtl_command_buffer;
        let encoded = self
            .program
            .draw(
                &self.plan,
                self.size,
                at,
                count,
                native,
                &commands,
                &mut self.target,
                true,
                true,
            )
            .and_then(|_| blit(native, &[drawn], &[out]).map_err(Error::from));
        if let Err(error) = encoded {
            let _ = b::abandon(&commands);
            return Err(error);
        }
        let receipt = b::commit(&commands).map_err(gpu)?;
        b::complete_through(&self.queue, receipt.epoch).map_err(gpu)?;
        self.writer.append(at)?;
        self.state = ExportState::Running(((at + 1) as f32 / self.end.max(1) as f32).min(1.0));
        Ok(true)
    }
}
