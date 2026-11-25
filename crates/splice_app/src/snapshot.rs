use super::{
    VIEWER_SIZE, commands, editor_lab, editor_session, editor_titlebar, paint_viewer, playbar,
    program, project_name, timeline, viewer_error, viewer_ui,
};
use crate::surface::{LabFrames, StageImage, gpu_error, open_device, open_renderer};
use ogpu_core::{
    backend as be,
    backend::{ColorAttachment, RenderTarget},
    render_pass::{Load, Store as StorePixels},
    types::{Extent, Origin, TextureDescriptor, TextureFormat, TextureUsage},
};
use pxui::{Commands, FrameIn, Image, Pass, Renderer};
use splice_av::{Composition, encode_png};
use splice_core::history::{Store, StoreError};
use splice_json::Value as Json;

/// Renders the viewer content at an exact timeline frame to a PNG without opening a window, with
/// the `offline` and `warning` fields when offline media drew as placeholders.
pub fn render(store: &Store, at: i32) -> Result<(Vec<u8>, Vec<(&'static str, Json)>), StoreError> {
    render_layout(
        store,
        at,
        super::EditorLayout::new(VIEWER_SIZE.map(|extent| extent as f32)),
        None,
        None,
    )
}

pub(super) fn render_layout(
    store: &Store,
    at: i32,
    mut layout: super::EditorLayout,
    agent: Option<(&editor_lab::Lab, &[commands::BindingDescription])>,
    media: Option<(&be::Device, &[timeline::MediaPreview])>,
) -> Result<(Vec<u8>, Vec<(&'static str, Json)>), StoreError> {
    let size = layout.size.map(|extent| extent.round().max(1.0) as u32);
    layout.size = size.map(|extent| extent as f32);
    layout.sheet = None;
    layout.drawer = None;
    let editor = layout.view == editor_titlebar::View::Editor;
    let restore_spec =
        layout.view == editor_titlebar::View::Spec && layout.view_selection.restore_scroll();
    let mut lab_scroll = layout.lab_scroll;
    let restore_lab =
        layout.view == editor_titlebar::View::Agent && agent.is_some() && lab_scroll.restore();
    let composition = if editor {
        program::composition(store)?
    } else {
        None
    };
    let end = composition.as_ref().map_or(0, |program| program.end());
    if editor && !(0..end.max(1)).contains(&at) {
        return Err(StoreError::new(
            "OUT_OF_RANGE",
            &format!(
                "Frame {at} is outside the viewer's range of 0 through {}.",
                end.saturating_sub(1).max(0)
            ),
            "Choose a timeline frame in that range.",
        ));
    }
    let mut ui = viewer_ui()?;
    // A live capture samples preview textures on the device that owns them.
    let (device, owns_device, previews) = match media {
        Some((device, previews)) => (device.clone(), false, previews),
        None => (open_device()?, true, &[][..]),
    };
    let mut gpu = Offscreen {
        device,
        owns_device,
        renderer: None,
        queue: None,
        target: None,
        picture: None,
        matte: None,
        lab_frames: LabFrames::default(),
        pending: None,
    };
    if editor {
        gpu.matte = Some(StageImage::matte(&gpu.device)?);
    }
    if layout.view == editor_titlebar::View::Agent {
        let reply = agent
            .and_then(|(lab, _)| lab.frame_reply())
            .map(|(id, files)| (id, files, store.path()));
        gpu.lab_frames = LabFrames::load(&gpu.device, reply);
    }
    let matte = gpu.matte.as_ref().map(StageImage::image);
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
    let mut session = editor_session::State::default();
    let activity = if layout.view == editor_titlebar::View::Agent {
        store.sessions().last().map(|session| session.id)
    } else {
        None
    };
    session.refresh(Some(store), activity);
    let (lab, bindings) = agent.map_or((None, &[][..]), |(lab, bindings)| (Some(lab), bindings));
    let input = FrameIn::new(layout.size[0], layout.size[1]);
    let mut cache = super::ViewerCache::for_app();
    let mut build = |ui: &mut pxui::Ui| {
        paint_viewer(
            ui,
            Some(&name),
            (
                Some(store.project()),
                lab.map(|lab| (lab, gpu.lab_frames.images.as_slice())),
                None,
                Some((&session, store)),
                bindings,
            ),
            layout.clone(),
            ([matte, image, None], &[]),
            (
                &mut cache,
                previews,
                composition.as_deref().map(Composition::evaluator),
            ),
            playbar::View {
                at,
                end,
                fps: store.project().fps,
                play: commands::states(end > 0, false, (false, false))[commands::PLAY.0 as usize],
                rate: 0,
                dragging: false,
            },
        )
        .lab
    };
    // A fresh UI needs a measured scroll area before restoring the live offset.
    for _ in 0..if restore_spec || restore_lab { 3 } else { 1 } {
        let mut intents = editor_lab::Intents::default();
        ui.frame(&input, &mut |ui| intents.merge(build(ui)))
            .map_err(|error| viewer_error(&error))?;
        if restore_lab {
            lab_scroll.settle(&mut ui, &intents);
        }
    }

    let target = gpu.target.insert(texture(
        &gpu.device,
        size,
        vec![TextureUsage::RenderAttachment, TextureUsage::TextureCopySrc],
    )?);
    renderer
        .resize(size[0], size[1])
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
    gpu.pending = Some(receipt.epoch);
    be::complete_through(queue, receipt.epoch).map_err(gpu_error)?;
    gpu.pending = None;
    let pixels = be::read_texture(target, i64::from(size[0]) * 4).map_err(gpu_error)?;
    let png = encode_png(&pixels, size, 0).map_err(|error| viewer_error(&error))?;
    let offline = composition
        .as_deref()
        .map_or_else(Vec::new, splice_mcp::offline_fields);
    Ok((png, offline))
}

struct Offscreen {
    device: be::Device,
    owns_device: bool,
    renderer: Option<Renderer>,
    queue: Option<be::Queue>,
    target: Option<be::Texture>,
    picture: Option<be::Texture>,
    matte: Option<StageImage>,
    lab_frames: LabFrames,
    pending: Option<i64>,
}

impl Drop for Offscreen {
    fn drop(&mut self) {
        if let Some(epoch) = self.pending
            && let Some(queue) = &self.queue
            && let Err(error) = be::complete_through(queue, epoch)
        {
            eprintln!("splice: could not finish the snapshot GPU work: {error:?}");
        }
        drop(self.renderer.take());
        drop(std::mem::take(&mut self.lab_frames));
        drop(self.matte.take());
        for texture in [self.target.take(), self.picture.take()]
            .into_iter()
            .flatten()
        {
            let _ = be::destroy_texture(&texture);
        }
        if let Some(queue) = &self.queue
            && let Err(error) = be::destroy_queue(queue)
        {
            eprintln!("splice: could not release the snapshot GPU queue: {error:?}");
        }
        if self.owns_device {
            let _ = be::destroy_device(&self.device);
        }
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
        .read_frame(at, &mut pixels)
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
