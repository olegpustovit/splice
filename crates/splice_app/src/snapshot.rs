use super::{VIEWER_SIZE, commands, paint_viewer, playbar, program, project_name, viewer_error};
use crate::surface::{gpu_error, open_device, open_renderer};
use ogpu_core::{
    backend as be,
    backend::{ColorAttachment, RenderTarget},
    render_pass::{Load, Store as StorePixels},
    types::{Extent, Origin, TextureDescriptor, TextureFormat, TextureUsage},
};
use pxui::{Commands, FrameIn, Image, Pass, Renderer, Ui, UiDesc};
use splice_av::{Composition, encode_png};
use splice_core::history::{Store, StoreError};

/// Renders the viewer content at an exact timeline frame to a PNG without opening a window.
pub fn render(store: &Store, at: i32) -> Result<Vec<u8>, StoreError> {
    let composition = program::composition(store)?;
    let end = composition.as_ref().map_or(0, |program| program.end());
    if !(0..end.max(1)).contains(&at) {
        return Err(StoreError::new(
            "OUT_OF_RANGE",
            &format!(
                "Frame {at} is outside the viewer's range of 0 through {}.",
                end.saturating_sub(1).max(0)
            ),
            "Choose a timeline frame in that range.",
        ));
    }
    let mut ui = Ui::new(&UiDesc::DEFAULT).map_err(|error| viewer_error(&error))?;
    let mut gpu = Offscreen {
        device: open_device()?,
        renderer: None,
        queue: None,
        target: None,
        picture: None,
    };
    let native = be::native_leased(&gpu.device).map_err(gpu_error)?;
    let renderer = gpu.renderer.insert(open_renderer(&ui, native, 1)?);
    let queue = gpu
        .queue
        .insert(be::create_queue(&gpu.device).map_err(gpu_error)?);
    let image = if let Some(composition) = &composition {
        let picture = gpu.picture.insert(texture(
            &gpu.device,
            composition.size(),
            vec![TextureUsage::TextureBinding, TextureUsage::TextureCopyDst],
        )?);
        Some(load_picture(composition, at, picture)?)
    } else {
        None
    };
    let name = project_name(store);
    let input = FrameIn::new(VIEWER_SIZE[0] as f32, VIEWER_SIZE[1] as f32);
    ui.frame(&input, &mut |ui| {
        paint_viewer(
            ui,
            Some(&name),
            composition.as_ref().map(|composition| (at, composition)),
            image,
            playbar::View {
                at,
                end,
                fps: store.project().fps,
                play: commands::states(end > 0, false, (false, false))[commands::PLAY.0 as usize],
                dragging: false,
            },
        );
    })
    .map_err(|error| viewer_error(&error))?;

    let target = gpu.target.insert(texture(
        &gpu.device,
        VIEWER_SIZE,
        vec![TextureUsage::RenderAttachment, TextureUsage::TextureCopySrc],
    )?);
    renderer
        .resize(VIEWER_SIZE[0], VIEWER_SIZE[1])
        .map_err(|error| viewer_error(&error))?;
    let commands = be::begin_commands(queue).map_err(gpu_error)?;
    let native = be::native_commands(&commands)
        .map_err(gpu_error)?
        .mtl_command_buffer;
    renderer
        .prepare(
            &Commands {
                mtl_command_buffer: native,
                ..Commands::default()
            },
            1,
            0,
        )
        .map_err(|error| viewer_error(&error))?;
    let color = ui.theme().tokens.panel;
    let encoder = be::render_encoder(
        &commands,
        &RenderTarget {
            colors: vec![ColorAttachment {
                texture: target.clone(),
                resolve: None,
                load: Load::Clear,
                store: StorePixels::Store,
                clear: (
                    f64::from(color.r) / 255.0,
                    f64::from(color.g) / 255.0,
                    f64::from(color.b) / 255.0,
                    1.0,
                ),
            }],
            depth: None,
        },
    )
    .map_err(gpu_error)?;
    let pass = be::native_pass(&encoder).map_err(gpu_error)?;
    renderer
        .draw(&Pass {
            mtl_command_buffer: native,
            mtl_render_encoder: pass.mtl_render_encoder,
            width_px: pass.width_px,
            height_px: pass.height_px,
            ..Pass::default()
        })
        .map_err(|error| viewer_error(&error))?;
    be::end_render(&encoder).map_err(gpu_error)?;
    let receipt = be::commit(&commands).map_err(gpu_error)?;
    be::complete_through(queue, receipt.epoch).map_err(gpu_error)?;
    let pixels = be::read_texture(target, i64::from(VIEWER_SIZE[0]) * 4).map_err(gpu_error)?;
    encode_png(&pixels, VIEWER_SIZE, 0).map_err(|error| viewer_error(&error))
}

struct Offscreen {
    device: be::Device,
    renderer: Option<Renderer>,
    queue: Option<be::Queue>,
    target: Option<be::Texture>,
    picture: Option<be::Texture>,
}

impl Drop for Offscreen {
    fn drop(&mut self) {
        drop(self.renderer.take());
        for texture in [self.target.take(), self.picture.take()]
            .into_iter()
            .flatten()
        {
            let _ = be::destroy_texture(&texture);
        }
        if let Some(queue) = &self.queue {
            let _ = be::destroy_queue(queue);
        }
        let _ = be::destroy_device(&self.device);
    }
}

fn texture(
    device: &be::Device,
    [width, height]: [u32; 2],
    usage: Vec<TextureUsage>,
) -> Result<be::Texture, StoreError> {
    be::create_texture(
        device,
        TextureDescriptor {
            format: TextureFormat::Rgba8Unorm,
            label: None,
            width: i64::from(width),
            height: i64::from(height),
            depth: 1,
            sample_count: 1,
            mip_levels: 1,
            usage,
        },
    )
    .map_err(gpu_error)
}

fn load_picture(
    composition: &Composition,
    at: i32,
    texture: &be::Texture,
) -> Result<Image, StoreError> {
    let [width, height] = composition.size();
    let count = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| viewer_error(&"The program picture is too large."))?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(count)
        .map_err(|error| viewer_error(&error))?;
    pixels.resize(count, 0);
    composition
        .read_picture(at, &mut pixels)
        .map_err(|error| viewer_error(&error))?;
    be::write_texture(
        texture,
        None,
        Origin { x: 0, y: 0, z: 0 },
        Extent {
            width: i64::from(width),
            height: i64::from(height),
            depth: 1,
        },
        i64::from(width) * 4,
        &pixels,
    )
    .map_err(gpu_error)?;
    let native = be::native_texture(texture).map_err(gpu_error)?;
    Ok(Image::rgba8(
        width,
        height,
        std::ptr::null_mut(),
        native.mtl_texture,
    ))
}
