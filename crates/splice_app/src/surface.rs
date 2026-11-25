use super::viewer_error;
use apple_sys::composition::Player;
use metal_sys::frames::Frames;
use ogpu_core::backend as be;
use ogpu_core::types::{Extent, Origin, TextureDescriptor, TextureFormat, TextureUsage};
use pxui::{
    Commands, Gpu, Image, Pass, Renderer, TargetDesc, Ui,
    images::{Images, Want},
};
use pxui_host::Window;
use splice_av::{Composition, GpuProgram};
use splice_core::history::StoreError;
use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

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
    matte: Option<StageImage>,
    crop: Option<StageImage>,
    thumbnails: Vec<StageImage>,
    lab_frames: LabFrames,
    current: Option<usize>,
    drawable: (u32, u32),
}

struct Slot {
    texture: be::Texture,
    target: Option<be::PreparedRenderTarget>,
    sampled: u64,
}

impl Surface {
    pub(super) fn device(&self) -> &be::Device {
        &self.device
    }

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
            matte: None,
            crop: None,
            thumbnails: Vec::new(),
            lab_frames: LabFrames::default(),
            current: None,
            drawable: (0, 0),
        };
        let token = window.token().map_err(|error| viewer_error(&error))?;
        surface.frames =
            Some(Frames::new(&surface.device, token).map_err(|error| viewer_error(&error))?);
        let native = be::native_leased(&surface.device).map_err(gpu_error)?;
        surface.renderer = Some(open_renderer(ui, native, 2)?);
        surface.matte = Some(StageImage::matte(&surface.device)?);
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
                surface.slots.push(Slot {
                    texture,
                    target: None,
                    sampled: 0,
                });
                let texture = &surface.slots.last().expect("created slot").texture;
                let image = be::native_texture(texture).map_err(gpu_error)?.mtl_texture;
                surface.images.store(
                    Want::new(id, 1024).expect("fixed video image identity"),
                    Some(Image::rgba8(width, height, std::ptr::null_mut(), image)),
                );
            }
        }
        surface.prepare_targets()?;
        Ok(surface)
    }

    pub(super) fn matte(&self) -> Option<Image> {
        self.matte.as_ref().map(StageImage::image)
    }

    pub(super) fn crop_preview(&self) -> Option<Image> {
        self.crop.as_ref().map(StageImage::image)
    }

    pub(super) fn set_crop_preview(
        &mut self,
        preview: Option<(&[u8], [u32; 2])>,
    ) -> Result<(), StoreError> {
        if self.crop.is_some() {
            self.frames
                .as_mut()
                .expect("open surface")
                .wait()
                .map_err(|error| viewer_error(&error))?;
        }
        self.crop = preview
            .map(|(pixels, size)| StageImage::upload(&self.device, size, pixels))
            .transpose()?;
        Ok(())
    }

    pub(super) fn add_thumbnail(
        &mut self,
        pixels: &[u8],
        size: [u32; 2],
    ) -> Result<Image, StoreError> {
        let preview = StageImage::upload(&self.device, size, pixels)?;
        let image = preview.image();
        self.thumbnails.push(preview);
        Ok(image)
    }

    pub(super) fn clear_thumbnails(&mut self) -> Result<(), StoreError> {
        if !self.thumbnails.is_empty() {
            self.frames
                .as_mut()
                .expect("open surface")
                .wait()
                .map_err(|error| viewer_error(&error))?;
            self.thumbnails.clear();
        }
        Ok(())
    }

    pub(super) fn lab_images(&self) -> &[Option<Image>] {
        &self.lab_frames.images
    }

    pub(super) fn refresh_lab_frames(
        &mut self,
        reply: Option<(u64, &[ReplyFrame], &Path)>,
    ) -> Result<(), StoreError> {
        if self.lab_frames.id == reply.map(|(id, _, _)| id) {
            return Ok(());
        }
        if !self.lab_frames.textures.is_empty() {
            self.frames
                .as_mut()
                .expect("open surface")
                .wait()
                .map_err(|error| viewer_error(&error))?;
        }
        self.lab_frames = LabFrames::load(&self.device, reply);
        Ok(())
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
        let key = Rc::as_ptr(composition);
        if self.program.as_ref().is_none_or(|(known, _)| *known != key) {
            self.clear_program()?;
            let program =
                GpuProgram::new(&self.device, composition).map_err(|error| viewer_error(&error))?;
            self.program = Some((key, program));
        }
        // Frames::begin has waited for the reused in-flight slot. A paused frame can
        // be sampled again at any serial, so track each texture's most recent use.
        let frames = self.frames.as_ref().expect("open surface");
        let completed = frames.completed();
        let index = self
            .slots
            .iter()
            .position(|slot| slot.sampled <= completed)
            .ok_or_else(|| viewer_error(&"No completed video texture slot is available."))?;
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
                self.slots[index].target.as_mut().expect("prepared slot"),
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

    pub(super) fn clear_program(&mut self) -> Result<(), StoreError> {
        self.current = None;
        if self.program.is_some() {
            self.frames
                .as_mut()
                .expect("open surface")
                .wait()
                .map_err(|error| viewer_error(&error))?;
            self.release_program();
            self.prepare_targets()?;
        }
        Ok(())
    }

    fn prepare_targets(&mut self) -> Result<(), StoreError> {
        for slot in &mut self.slots {
            slot.target = Some(
                splice_gpu::Compositor::target(&self.device, &slot.texture, [0.0, 0.0, 0.0, 1.0])
                    .map_err(gpu_error)?,
            );
        }
        Ok(())
    }

    fn release_program(&mut self) {
        // Every prepared pass and the program's group pass can keep the last commands alive.
        for slot in &mut self.slots {
            slot.target = None;
        }
        self.commands = None;
        self.program = None;
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
        let (encoder, _) = frames
            .pass([color.r, color.g, color.b], &[])
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
        let (drawable, _) = frames.acquire().map_err(|error| viewer_error(&error))?;
        if drawable && frames.present_pass(false, &[]).is_none() {
            frames.cancel();
            return Err(viewer_error(&"Could not open Metal presentation pass."));
        }
        let submitted = frames.submit().map_err(|error| viewer_error(&error))?;
        if let Some(current) = self.current {
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
            frames.cancel();
            let _ = frames.wait();
        }
        drop(self.renderer.take());
        drop(self.frames.take());
        drop(std::mem::take(&mut self.images));
        self.release_program();
        for slot in self.slots.drain(..) {
            if let Err(error) = be::destroy_texture(&slot.texture) {
                eprintln!("splice: could not release a Program texture: {error:?}");
            }
        }
        self.thumbnails.clear();
        drop(std::mem::take(&mut self.lab_frames));
        drop(self.crop.take());
        drop(self.matte.take());
        if let Err(error) = be::destroy_queue(&self.queue) {
            eprintln!("splice: could not release the viewer GPU queue: {error:?}");
        }
        if let Err(error) = be::destroy_device(&self.device) {
            eprintln!("splice: could not release the viewer GPU device: {error:?}");
        }
    }
}

const MATTE_SIDE: u32 = 128;

pub(super) struct ReplyFrame {
    pub(super) path: PathBuf,
    pub(super) at: Option<i32>,
    pub(super) aspect: f32,
}

/// The active Agent reply owns only its scaled previews. Paths stay in the call log for recall.
#[derive(Default)]
pub(super) struct LabFrames {
    id: Option<u64>,
    pub(super) images: Vec<Option<Image>>,
    textures: Vec<StageImage>,
}

impl LabFrames {
    pub(super) fn load(device: &be::Device, reply: Option<(u64, &[ReplyFrame], &Path)>) -> Self {
        let Some((id, files, package)) = reply else {
            return Self::default();
        };
        let root = package
            .canonicalize()
            .ok()
            .map(|path| path.join("cache").join("frames"));
        let mut frames = Self {
            id: Some(id),
            images: Vec::with_capacity(files.len()),
            textures: Vec::with_capacity(files.len()),
        };
        for file in files {
            let texture = (|| {
                let root = root.as_deref()?;
                let path = file.path.canonicalize().ok()?;
                if path.parent() != Some(root) || path.extension()?.to_str()? != "png" {
                    return None;
                }
                let (pixels, size) = apple_sys::image::decode_thumbnail(&path, 640).ok()?;
                StageImage::upload(device, size, &pixels).ok()
            })();
            frames.images.push(texture.as_ref().map(StageImage::image));
            frames.textures.extend(texture);
        }
        frames
    }
}

/// An image texture retained while the renderer may reference it.
pub(super) struct StageImage {
    texture: be::Texture,
    image: Image,
}

impl StageImage {
    pub(super) fn matte(device: &be::Device) -> Result<Self, StoreError> {
        let mut pixels = [0u8; MATTE_SIDE as usize * MATTE_SIDE as usize * 4];
        for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
            let x = index as u32 % MATTE_SIDE;
            let y = index as u32 / MATTE_SIDE;
            pixel.copy_from_slice(&matte_pixel(
                (x as f32 + 0.5) / MATTE_SIDE as f32,
                (y as f32 + 0.5) / MATTE_SIDE as f32,
            ));
        }
        Self::upload(device, [MATTE_SIDE; 2], &pixels)
    }

    fn upload(
        device: &be::Device,
        [width, height]: [u32; 2],
        pixels: &[u8],
    ) -> Result<Self, StoreError> {
        let mut image = Self {
            image: Image::default(),
            texture: be::create_texture(
                device,
                TextureDescriptor {
                    label: Some("Splice stage image".into()),
                    width: i64::from(width),
                    height: i64::from(height),
                    depth: 1,
                    mip_levels: 1,
                    sample_count: 1,
                    format: TextureFormat::Rgba8Unorm,
                    usage: vec![TextureUsage::TextureBinding, TextureUsage::TextureCopyDst],
                },
            )
            .map_err(gpu_error)?,
        };
        be::write_texture(
            &image.texture,
            None,
            Origin { x: 0, y: 0, z: 0 },
            Extent {
                width: i64::from(width),
                height: i64::from(height),
                depth: 1,
            },
            i64::from(width) * 4,
            pixels,
        )
        .map_err(gpu_error)?;
        let native = be::native_texture(&image.texture).map_err(gpu_error)?;
        image.image = Image::rgba8(width, height, std::ptr::null_mut(), native.mtl_texture);
        Ok(image)
    }

    pub(super) fn image(&self) -> Image {
        self.image
    }
}

impl Drop for StageImage {
    fn drop(&mut self) {
        let _ = be::destroy_texture(&self.texture);
    }
}

fn matte_pixel(x: f32, y: f32) -> [u8; 4] {
    let mix = ((x - 0.5) / 0.7).hypot((y - 0.45) / 0.8).min(0.75) / 0.75;
    let gray = (22.0 - 16.0 * mix).round() as u8;
    [gray, gray, (24.0 - 17.0 * mix).round() as u8, 255]
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
