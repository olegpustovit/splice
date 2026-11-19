//! The OGPU compositor's frame path (PLAN.md T12.7): evaluates the lowered layer plan at one frame
//! into `splice_gpu` layers over decoded sources imported without copies. The per-interval math
//! matches the interim Core Image compositor in `apple_sys/native/composition.m`.
use crate::{Composition, Error};
use apple_sys::composition::{CursorSample, FrameSource, Player, VideoLayer};
use ogpu_core::backend::{self as b, Commands, Device, PreparedRenderTarget, Texture};
use ogpu_core::types::{Extent, Origin, TextureDescriptor, TextureFormat, TextureUsage};
use splice_gpu::{Compositor, Layer, MAX_LAYERS, rgb};
use splice_model::Frame;

const RING_SECONDS: f64 = 0.45;

/// Per-viewer GPU state: the compositor, per-layer source copies and cursor textures.
pub struct GpuProgram {
    compositor: Compositor,
    device: Device,
    mtl_device: *mut std::ffi::c_void,
    /// One BGRA texture per source layer, refilled each frame by a GPU copy of the decoded buffer.
    /// ponytail: one copy per source per frame, because OGPU has no allocation-free way to point a
    /// wrapper at the next decoded IOSurface. Upgrade: an OGPU `rewrap_texture` (PXUI) makes it
    /// zero-copy.
    copies: Vec<Option<(Texture, [u32; 2], usize)>>,
    blit_from: Vec<*mut std::ffi::c_void>,
    blit_to: Vec<*mut std::ffi::c_void>,
    /// Per cursor table, per image: texture and its size in pixels.
    cursors: Vec<Vec<(Texture, [f64; 2])>>,
    sources: Vec<FrameSource>,
    layers: Vec<Layer>,
    textures: Vec<Option<Texture>>,
}

fn gpu(error: ogpu_core::error::Error) -> Error {
    Error::Input(ogpu_core::error::to_string(&error))
}

impl GpuProgram {
    /// Prepares the compositor for `composition` on `device`, uploading its cursor images.
    pub fn new(device: &Device, composition: &Composition) -> Result<Self, Error> {
        let mtl_device = b::native_leased(device).map_err(gpu)?.mtl_device;
        let mut cursors = Vec::with_capacity(composition.plan.cursors.len());
        for (_, images, _) in &composition.plan.cursors {
            let mut table = Vec::with_capacity(images.len());
            for image in images {
                let (pixels, [w, h]) = apple_sys::image::decode_rgba(&image.path)?;
                let texture = b::create_texture(
                    device,
                    TextureDescriptor {
                        label: Some("Splice cursor".into()),
                        width: i64::from(w),
                        height: i64::from(h),
                        depth: 1,
                        mip_levels: 1,
                        sample_count: 1,
                        format: TextureFormat::Rgba8Unorm,
                        usage: vec![TextureUsage::TextureBinding, TextureUsage::TextureCopyDst],
                    },
                )
                .map_err(gpu)?;
                b::write_texture(
                    &texture,
                    None,
                    Origin { x: 0, y: 0, z: 0 },
                    Extent {
                        width: i64::from(w),
                        height: i64::from(h),
                        depth: 1,
                    },
                    i64::from(w) * 4,
                    &pixels,
                )
                .map_err(gpu)?;
                table.push((texture, [f64::from(w), f64::from(h)]));
            }
            cursors.push(table);
        }
        Ok(Self {
            compositor: Compositor::new(device).map_err(gpu)?,
            device: device.clone(),
            mtl_device,
            copies: Vec::with_capacity(MAX_LAYERS),
            blit_from: Vec::with_capacity(MAX_LAYERS),
            blit_to: Vec::with_capacity(MAX_LAYERS),
            cursors,
            sources: vec![FrameSource::default(); MAX_LAYERS],
            layers: Vec::with_capacity(MAX_LAYERS),
            textures: Vec::with_capacity(MAX_LAYERS),
        })
    }

    /// Composites the player's newest displayed frame into `target`. The frame's sources are
    /// copied in `native_commands` (the frame's `id<MTLCommandBuffer>`), then the layers are
    /// encoded in `commands` (the same buffer, adopted). `slot` (0…2) must be a frame slot whose
    /// GPU work has completed. Returns the frame drawn.
    pub fn render(
        &mut self,
        composition: &Composition,
        player: &mut Player,
        slot: u32,
        native_commands: *mut std::ffi::c_void,
        commands: &Commands,
        target: &mut PreparedRenderTarget,
    ) -> Result<Option<Frame>, Error> {
        let Some((at, count)) = player.sources(self.mtl_device, slot, &mut self.sources)? else {
            return Ok(None);
        };
        self.blit_from.clear();
        self.blit_to.clear();
        for i in 0..count {
            let source = self.sources[i];
            if source.texture.is_null() {
                continue;
            }
            if self.copies.len() <= i {
                self.copies.resize_with(i + 1, || None);
            }
            let size = [source.width, source.height];
            if self.copies[i]
                .as_ref()
                .is_none_or(|(_, known, _)| *known != size)
            {
                let texture = b::create_texture(
                    &self.device,
                    TextureDescriptor {
                        label: Some("Splice source".into()),
                        width: i64::from(size[0]),
                        height: i64::from(size[1]),
                        depth: 1,
                        mip_levels: 1,
                        sample_count: 1,
                        format: TextureFormat::Rgba8Unorm,
                        usage: vec![TextureUsage::TextureBinding, TextureUsage::TextureCopyDst],
                    },
                )
                .map_err(gpu)?;
                let native = b::native_texture(&texture).map_err(gpu)?.mtl_texture as usize;
                self.copies[i] = Some((texture, size, native));
            }
            let (_, _, native) = self.copies[i].as_ref().expect("source copy");
            self.blit_from.push(source.texture);
            self.blit_to.push(*native as *mut std::ffi::c_void);
        }
        apple_sys::composition::blit(native_commands, &self.blit_from, &self.blit_to)?;
        let intervals = &composition.plan.intervals;
        let index = intervals.partition_point(|(range, _)| range[1] <= at);
        let Some((range, layers)) = intervals.get(index) else {
            return Ok(None);
        };
        if layers.len() != count {
            return Err(Error::Input(format!(
                "Frame {at} has {count} sources for {} layers.",
                layers.len()
            )));
        }
        self.layers.clear();
        self.textures.clear();
        let size = composition.size.map(f64::from);
        for (i, layer) in layers.iter().enumerate() {
            let source = self.sources[i];
            let fill = composition
                .plan
                .fills
                .iter()
                .find(|(track, _)| *track == layer.track)
                .map(|(_, colors)| *colors);
            let texture = match (&fill, self.copies.get(i)) {
                (None, Some(Some((texture, ..)))) if !source.texture.is_null() => {
                    Some(texture.clone())
                }
                _ => None,
            };
            self.layer(composition, layer, *range, at, size, fill, source, texture)?;
        }
        if self.layers.len() > MAX_LAYERS {
            self.layers.truncate(MAX_LAYERS);
            self.textures.truncate(MAX_LAYERS);
        }
        self.compositor
            .encode(
                commands,
                target,
                composition.size,
                &self.layers,
                &self.textures,
            )
            .map_err(gpu)?;
        Ok(Some(at))
    }

    #[allow(clippy::too_many_arguments)]
    fn layer(
        &mut self,
        composition: &Composition,
        layer: &VideoLayer,
        [from, to]: [Frame; 2],
        at: Frame,
        [cw, ch]: [f64; 2],
        fill: Option<[u32; 2]>,
        source: FrameSource,
        texture: Option<Texture>,
    ) -> Result<(), Error> {
        let span = f64::from((to - from).max(1));
        let linear = (f64::from(at - from) / span).clamp(0.0, 1.0);
        let t = if layer.flags & VideoLayer::EASE != 0 {
            linear * linear * (3.0 - 2.0 * linear)
        } else {
            linear
        };
        let lerp = |a: f64, b: f64, k: f64| a + (b - a) * k;
        let opacity = lerp(layer.opacity[0], layer.opacity[1], linear) as f32;
        let target = [cw as f32, ch as f32];
        let [dx, dy, dw, dh] = layer.destination;
        if let Some([a, b]) = fill {
            let mut out = Layer::new(
                if a == b {
                    Layer::SOLID
                } else {
                    Layer::GRADIENT
                },
                [dx, dy, dw, dh].map(|v| v as f32),
                target,
            );
            (out.color0, out.color1, out.opacity) = (rgb(a), rgb(b), opacity);
            self.push(out, None);
            return Ok(());
        }
        let Some(texture) = texture else {
            return Ok(());
        };
        // ponytail: sources are drawn in decoded orientation. Rotated phone video needs a UV
        // transform in `Layer`; add it when a rotated import is reported.
        let [w, h] = [f64::from(source.width), f64::from(source.height)];
        let [mut cx, mut cy, mut cwid, mut chei] = [
            layer.crop[0] * w,
            layer.crop[1] * h,
            layer.crop[2] * w,
            layer.crop[3] * h,
        ];
        if layer.center_crop != 0 {
            let ratio = dw / dh;
            let width = cwid.min(chei * ratio);
            let height = chei.min(cwid / ratio);
            cx += (cwid - width) / 2.0;
            cy += (chei - height) / 2.0;
            (cwid, chei) = (width, height);
        }
        let scale = lerp(layer.scale[0], layer.scale[1], t);
        let fx = lerp(layer.focus[0], layer.focus[2], t);
        let fy = lerp(layer.focus[1], layer.focus[3], t);
        let placed = [
            fx + (dx - fx) * scale,
            fy + (dy - fy) * scale,
            dw * scale,
            dh * scale,
        ];
        let mut picture = Layer::new(Layer::TEXTURE, placed.map(|v| v as f32), target);
        picture.crop = [cx / w, cy / h, cwid / w, chei / h].map(|v| v as f32);
        picture.opacity = opacity;
        picture.corner = (layer.corner * scale) as f32;
        picture.shadow = layer.shadow as f32;
        picture.flags = Layer::BGRA;
        self.push(picture, Some(texture));
        let Some((samples, _, points)) = usize::try_from(layer.cursor)
            .ok()
            .and_then(|i| composition.plan.cursors.get(i))
        else {
            return Ok(());
        };
        let st = lerp(layer.source[0], layer.source[1], linear);
        let per_px = placed[2] / cwid; // canvas pixels per source pixel
        let px_per_point = w / points;
        let to_canvas = |x: f64, y: f64| {
            [
                placed[0] + (x * w - cx) * per_px,
                placed[1] + (y * h - cy) * per_px,
            ]
        };
        if layer.flags & VideoLayer::CLICK_RINGS != 0 {
            for sample in samples.iter().filter(|s| s.click == 1) {
                let age = st - sample.t;
                if !(0.0..=RING_SECONDS).contains(&age) {
                    continue;
                }
                let [x, y] = to_canvas(sample.x, sample.y);
                let radius =
                    (10.0 + 34.0 * age / RING_SECONDS) * px_per_point * per_px * layer.cursor_scale;
                let mut ring = Layer::new(
                    Layer::RING,
                    [x - radius, y - radius, 2.0 * radius, 2.0 * radius].map(|v| v as f32),
                    target,
                );
                ring.ring = (radius * 0.45) as f32;
                ring.color0 = [1.0, 1.0, 1.0, (0.6 * (1.0 - age / RING_SECONDS)) as f32];
                ring.opacity = opacity;
                self.push(ring, None);
            }
        }
        let table = usize::try_from(layer.cursor).unwrap_or(0);
        let images = &composition.plan.cursors[table].1;
        let sprite = cursor_at(samples, st).and_then(|(sample, at)| {
            let image = usize::try_from(sample.image).ok()?;
            let (texture, size) = self.cursors[table].get(image)?;
            Some((images.get(image)?, texture.clone(), *size, at))
        });
        if let Some((meta, texture, [pw, ph], [x, y])) = sprite {
            let k = layer.cursor_scale * px_per_point * per_px / meta.scale; // canvas px per PNG px
            let [x, y] = to_canvas(x, y);
            let mut cursor = Layer::new(
                Layer::TEXTURE,
                [x - meta.hot[0] * k, y - meta.hot[1] * k, pw * k, ph * k].map(|v| v as f32),
                target,
            );
            cursor.opacity = opacity;
            self.push(cursor, Some(texture));
        }
        Ok(())
    }

    fn push(&mut self, layer: Layer, texture: Option<Texture>) {
        self.layers.push(layer);
        self.textures.push(texture);
    }
}

/// The cursor sample in effect at `t`, gliding across gaps under 0.25 s (as the native one does).
fn cursor_at(samples: &[CursorSample], t: f64) -> Option<(CursorSample, [f64; 2])> {
    let next = samples.partition_point(|s| s.t <= t);
    let a = *samples.get(next.checked_sub(1)?)?;
    let mut at = [a.x, a.y];
    if let Some(b) = samples.get(next)
        && b.t - a.t < 0.25
        && b.t > a.t
    {
        let k = (t - a.t) / (b.t - a.t);
        at = [a.x + (b.x - a.x) * k, a.y + (b.y - a.y) * k];
    }
    Some((a, at))
}
