use super::viewer_error;
use apple_sys::composition::{Player, ProgramTexture};
use metal_sys::frames::Frames;
use ogpu_core::backend as be;
use pxui::{
    Commands, Gpu, Image, Pass, Renderer, TargetDesc, Ui,
    images::{Images, Want},
};
use pxui_host::Window;
use splice_core::history::StoreError;

/// Owns the sampled textures as long as the PXUI renderer can reference them.
pub(super) struct Surface {
    device: be::Device,
    frames: Option<Frames>,
    renderer: Option<Renderer>,
    slots: Vec<Slot>,
    images: Images,
    current: Option<usize>,
    drawable: (u32, u32),
}

struct Slot {
    texture: ProgramTexture,
    sampled: u64,
}

impl Surface {
    pub(super) fn open(
        window: &mut Window,
        ui: &Ui,
        size: Option<[u32; 2]>,
    ) -> Result<Self, StoreError> {
        let mut surface = Self {
            device: open_device()?,
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
        let device_ptr = native.mtl_device;
        surface.renderer = Some(open_renderer(ui, native, 2)?);
        if let Some([width, height]) = size {
            for id in 1..=3 {
                let texture =
                    ProgramTexture::new([width, height]).map_err(|error| viewer_error(&error))?;
                let image = texture.image_for_device(device_ptr).ok_or_else(|| {
                    viewer_error(&"The program texture is on another Metal device.")
                })?;
                surface.images.store(
                    Want::new(id, 1024).expect("fixed video image identity"),
                    Some(Image::rgba8(
                        width,
                        height,
                        std::ptr::null_mut(),
                        image.as_ptr(),
                    )),
                );
                surface.slots.push(Slot {
                    texture,
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

    pub(super) fn sample(&mut self, player: &mut Player) -> Result<Option<i32>, StoreError> {
        // Frames::begin has waited for the reused in-flight slot. A paused frame can
        // be sampled again at any serial, so track each texture's most recent use.
        let completed = self.frames.as_ref().expect("open surface").completed();
        let index = self
            .slots
            .iter()
            .position(|slot| slot.sampled <= completed)
            .ok_or_else(|| viewer_error(&"No completed video texture slot is available."))?;
        let frame = player
            .copy_into(&mut self.slots[index].texture)
            .map_err(|error| viewer_error(&error))?;
        if frame.is_some() {
            self.current = Some(index);
        }
        Ok(frame)
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
