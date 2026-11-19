use super::viewer_error;
use apple_sys::composition::Player;
use metal_sys::frames::Frames;
use ogpu_core::backend as be;
use ogpu_core::types::{TextureDescriptor, TextureFormat, TextureUsage};
use pxui::{
    Commands, Gpu, Image, Pass, Renderer, TargetDesc, Ui,
    images::{Images, Want},
};
use pxui_host::Window;
use splice_av::{Composition, GpuProgram};
use splice_core::history::StoreError;
use std::rc::Rc;

/// Owns the sampled textures as long as the PXUI renderer can reference them.
pub(super) struct Surface {
    device: be::Device,
    queue: be::Queue,
    /// The frame's command buffer, adopted once and readopted each frame without allocating.
    commands: Option<be::Commands>,
    /// The OGPU compositor for the current composition (T12.7).
    program: Option<(*const Composition, GpuProgram)>,
    frames: Option<Frames>,
    renderer: Option<Renderer>,
    slots: Vec<Slot>,
    images: Images,
    current: Option<usize>,
    drawable: (u32, u32),
}

struct Slot {
    texture: be::Texture,
    target: be::PreparedRenderTarget,
    sampled: u64,
}

impl Surface {
    pub(super) fn open(
        window: &mut Window,
        ui: &Ui,
        size: Option<[u32; 2]>,
    ) -> Result<Self, StoreError> {
        let device = open_device()?;
        let mut surface = Self {
            queue: be::create_queue(&device).map_err(gpu_error)?,
            device,
            commands: None,
            program: None,
            frames: None,
            renderer: None,
            slots: Vec::with_capacity(3),
            images: Images::default(),
            current: None,
            drawable: (0, 0),
        };
        let token = window.token().map_err(|error| viewer_error(&error))?;
        surface.frames =
            Some(Frames::new(&surface.device, token).map_err(|error| viewer_error(&error))?);
        let native = be::native_leased(&surface.device).map_err(gpu_error)?;
        surface.renderer = Some(open_renderer(ui, native, 2)?);
        if let Some([width, height]) = size {
            for id in 1..=3 {
                // The compositor draws into these and PXUI samples them: no copies.
                let texture = be::create_texture(
                    &surface.device,
                    TextureDescriptor {
                        label: Some("Splice program".into()),
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
                .map_err(gpu_error)?;
                let image = be::native_texture(&texture).map_err(gpu_error)?.mtl_texture;
                let target =
                    splice_gpu::Compositor::target(&surface.device, &texture, [0.0, 0.0, 0.0, 1.0])
                        .map_err(gpu_error)?;
                surface.images.store(
                    Want::new(id, 1024).expect("fixed video image identity"),
                    Some(Image::rgba8(width, height, std::ptr::null_mut(), image)),
                );
                surface.slots.push(Slot {
                    texture,
                    target,
                    sampled: 0,
                });
            }
        }
        Ok(surface)
    }

    pub(super) fn begin(&mut self, window: &Window) -> Result<bool, StoreError> {
        let (w, h) = window.extent().drawable;
        if w <= 0 || h <= 0 {
            return Ok(false);
        }
        self.frames
            .as_mut()
            .expect("open surface")
            .begin(w as u32, h as u32)
            .map_err(|error| viewer_error(&error))
    }

    /// Composites the player's newest frame into a free program texture, inside this frame's
    /// command buffer so PXUI samples it in the same submission.
    pub(super) fn sample(
        &mut self,
        composition: &Rc<Composition>,
        player: &mut Player,
    ) -> Result<Option<i32>, StoreError> {
        // Frames::begin has waited for the reused in-flight slot. A paused frame can
        // be sampled again at any serial, so track each texture's most recent use.
        let frames = self.frames.as_ref().expect("open surface");
        let completed = frames.completed();
        let index = self
            .slots
            .iter()
            .position(|slot| slot.sampled <= completed)
            .ok_or_else(|| viewer_error(&"No completed video texture slot is available."))?;
        let key = Rc::as_ptr(composition);
        if self.program.as_ref().is_none_or(|(known, _)| *known != key) {
            let program =
                GpuProgram::new(&self.device, composition).map_err(|error| viewer_error(&error))?;
            self.program = Some((key, program));
        }
        let native = be::NativeCommands {
            size: std::mem::size_of::<be::NativeCommands>() as u32,
            vk_command_buffer: std::ptr::null_mut(),
            mtl_command_buffer: frames.commands(),
            frame_serial: frames.serial(),
            inside_pass: 0,
        };
        let commands = match &self.commands {
            Some(commands) if be::readopt_commands(commands, native).map_err(gpu_error)? => {
                commands.clone()
            }
            _ => {
                let commands = be::adopt_commands(&self.queue, native).map_err(gpu_error)?;
                self.commands = Some(commands.clone());
                commands
            }
        };
        let (_, program) = self.program.as_mut().expect("program");
        let frame = program
            .render(
                composition,
                player,
                index as u32,
                frames.commands(),
                &commands,
                &mut self.slots[index].target,
            )
            .map_err(|error| viewer_error(&error));
        be::settle_commands(&commands);
        let frame = frame?;
        if frame.is_some() {
            self.current = Some(index);
            // Retire the slot only after this frame's command buffer completes.
            self.slots[index].sampled = frames.serial();
        }
        Ok(frame)
    }

    /// Reads the shown program texture back (tests and diagnostics; waits for the GPU).
    #[allow(dead_code)]
    pub(super) fn read_program(&mut self, rgba: &mut [u8]) -> Result<bool, StoreError> {
        let Some(current) = self.current else {
            return Ok(false);
        };
        if let Some(frames) = &mut self.frames {
            frames.wait().map_err(|error| viewer_error(&error))?;
        }
        let texture = &self.slots[current].texture;
        let width = be::native_texture(texture).map_err(gpu_error)?.width;
        be::read_texture_into(texture, i64::from(width) * 4, rgba).map_err(gpu_error)?;
        Ok(true)
    }

    pub(super) fn image(&self) -> Option<Image> {
        self.current
            .and_then(|slot| self.images.get(slot + 1, "program", 1.0, None))
    }

    pub(super) fn clear_program(&mut self) {
        self.current = None;
    }

    pub(super) fn draw(&mut self, window: &Window, ui: &Ui) -> Result<bool, StoreError> {
        let (w, h) = window.extent().drawable;
        let frames = self.frames.as_mut().expect("open surface");
        let renderer = self.renderer.as_mut().expect("open renderer");
        let drawable = (w as u32, h as u32);
        if self.drawable != drawable {
            renderer
                .resize(drawable.0, drawable.1)
                .map_err(|error| viewer_error(&error))?;
            self.drawable = drawable;
        }
        let commands = frames.commands();
        renderer
            .prepare(
                &Commands {
                    mtl_command_buffer: commands,
                    ..Commands::default()
                },
                frames.serial(),
                frames.completed(),
            )
            .map_err(|error| viewer_error(&error))?;
        let color = ui.theme().tokens.panel;
        let encoder = frames
            .pass([color.r, color.g, color.b])
            .map_err(|error| viewer_error(&error))?;
        renderer
            .draw(&Pass {
                mtl_command_buffer: commands,
                mtl_render_encoder: encoder,
                width_px: w as u32,
                height_px: h as u32,
                scale: window.scale() as f32,
                ..Pass::default()
            })
            .map_err(|error| viewer_error(&error))?;
        let submitted = frames.submit().map_err(|error| viewer_error(&error))?;
        if submitted && let Some(current) = self.current {
            self.slots[current].sampled = frames.serial();
        }
        Ok(submitted)
    }

    pub(super) fn close(mut self) -> Result<(), StoreError> {
        self.frames
            .as_mut()
            .expect("open surface")
            .wait()
            .map_err(|error| viewer_error(&error))
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        if let Some(frames) = &mut self.frames {
            let _ = frames.wait();
        }
        drop(self.renderer.take());
        self.program = None;
        self.commands = None;
        self.slots.clear();
        drop(self.frames.take());
        let _ = be::destroy_device(&self.device);
    }
}

pub(super) fn open_device() -> Result<be::Device, StoreError> {
    ogpu_core::impl_::select(ogpu_metal::impl_::implementation());
    let driver = ogpu_core::impl_::create_driver().map_err(gpu_error)?;
    be::create_device(driver).map_err(gpu_error)
}

pub(super) fn open_renderer(
    ui: &Ui,
    native: be::NativeDevice,
    format: u32,
) -> Result<Renderer, StoreError> {
    Renderer::new(
        ui,
        Gpu {
            backend: native.backend,
            mtl_device: native.mtl_device,
            queue: native.queue,
            owner_token: native.owner_token,
            release_owner: native.release_owner,
            ..Gpu::default()
        },
        TargetDesc { format, samples: 1 },
    )
    .map_err(|error| viewer_error(&error))
}

pub(super) fn gpu_error(error: ogpu_core::error::Error) -> StoreError {
    viewer_error(&ogpu_core::error::to_string(&error))
}
