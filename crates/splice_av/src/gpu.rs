//! The OGPU compositor's frame path (PLAN.md T12.7): evaluates the lowered layer plan at one frame
//! into `splice_gpu` layers over the decoded sources. The viewer, frame grabs and export all draw
//! through `GpuProgram::draw`.
use crate::{
    Composition, Error, TimedText,
    lower::{Plan, Role},
};
use apple_sys::composition::{CursorSample, FrameSource, Player, VideoLayer};
use ogpu_core::backend::{self as b, Commands, Device, PreparedRenderTarget, Texture};
use ogpu_core::types::{Extent, Origin, TextureDescriptor, TextureFormat, TextureUsage};
use splice_eval::{Animate, FramePlan, Look, MaskMode, MaskShape};
use splice_gpu::{Bound, Compositor, Layer, MAX_LAYERS, rgb};
use splice_model::{Frame, ShapeKind, TimelineId};

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
    textures: Vec<Bound>,
    /// Rasterized titles, captions and chips (export and grabs; the viewer draws text in PXUI).
    texts: Vec<(Texture, TimedText)>,
    /// LUT atlases by `Grade::lut` index, with their cube size.
    luts: Vec<(Texture, f32)>,
    /// The evaluated frame (reused: allocation-free per frame).
    frame: FramePlan,
    /// Matte textures bound this frame, by owner.
    mattes: Vec<(TimelineId, Texture)>,
    /// Leave out the black base (alpha exports).
    pub(crate) transparent: bool,
}

fn gpu(error: ogpu_core::error::Error) -> Error {
    Error::Input(ogpu_core::error::to_string(&error))
}

impl GpuProgram {
    /// Prepares the viewer's compositor for `composition` on `device` (text stays in PXUI).
    pub fn new(device: &Device, composition: &Composition) -> Result<Self, Error> {
        Self::with_plan(device, &composition.plan, &[])
    }

    /// Prepares a compositor for `plan`, uploading its cursor images and rasterizing `text`.
    pub(crate) fn with_plan(
        device: &Device,
        plan: &Plan,
        text: &[TimedText],
    ) -> Result<Self, Error> {
        let mtl_device = b::native_leased(device).map_err(gpu)?.mtl_device;
        let upload = |pixels: &[u8], [w, h]: [u32; 2], label: &str| -> Result<Texture, Error> {
            let texture = b::create_texture(
                device,
                TextureDescriptor {
                    label: Some(label.into()),
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
                pixels,
            )
            .map_err(gpu)?;
            Ok(texture)
        };
        let mut texts = Vec::with_capacity(text.len());
        for cue in text {
            let [_, _, w, h] = cue.layer.bounds;
            let size = [w.ceil().max(1.0) as u32, h.ceil().max(1.0) as u32];
            let pixels = apple_sys::composition::text_rgba(&cue.layer, size)?;
            texts.push((upload(&pixels, size, "Splice text")?, cue.clone()));
        }
        let mut luts = Vec::with_capacity(plan.luts.len());
        for path in &plan.luts {
            let (pixels, n) = crate::lut::atlas(path)?;
            luts.push((upload(&pixels, [n * n, n], "Splice LUT")?, n as f32));
        }
        let mut cursors = Vec::with_capacity(plan.cursors.len());
        for (_, images, _) in &plan.cursors {
            let mut table = Vec::with_capacity(images.len());
            for image in images {
                let (pixels, [w, h]) = apple_sys::image::decode_rgba(&image.path)?;
                let texture = upload(&pixels, [w, h], "Splice cursor")?;
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
            texts,
            luts,
            frame: plan
                .evaluator
                .as_ref()
                .map(|e| e.buffer())
                .unwrap_or_default(),
            mattes: Vec::with_capacity(4),
            transparent: false,
        })
    }

    /// The `id<MTLDevice>` decoded frames must be imported on.
    pub(crate) fn mtl_device(&self) -> *mut std::ffi::c_void {
        self.mtl_device
    }

    /// Where a reader or grab writes the frame's sources before `draw`.
    pub(crate) fn sources_mut(&mut self) -> &mut [FrameSource] {
        &mut self.sources
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
        let drawn = self.draw(
            &composition.plan,
            composition.size,
            at,
            count,
            native_commands,
            commands,
            target,
            false,
            false,
        )?;
        Ok(drawn.then_some(at))
    }

    /// Draws frame `at` from the `count` sources in `sources_mut`: copies them in
    /// `native_commands` (the `id<MTLCommandBuffer>` behind `commands`), then encodes the layers.
    /// `text` adds the title, caption and chip layers; `bgra` writes BGRA bytes for an encoder
    /// buffer. False when `at` has no instruction.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        plan: &Plan,
        size: [u32; 2],
        at: Frame,
        count: usize,
        native_commands: *mut std::ffi::c_void,
        commands: &Commands,
        target: &mut PreparedRenderTarget,
        text: bool,
        bgra: bool,
    ) -> Result<bool, Error> {
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
        let intervals = &plan.intervals;
        let index = intervals.partition_point(|(range, _)| range[1] <= at);
        let Some((range, layers)) = intervals.get(index) else {
            return Ok(false);
        };
        if layers.len() != count {
            return Err(Error::Input(format!(
                "Frame {at} has {count} sources for {} layers.",
                layers.len()
            )));
        }
        if let Some(evaluator) = &plan.evaluator {
            evaluator.frame(at, &mut self.frame);
        }
        let roles = plan.roles.get(index).map_or(&[][..], Vec::as_slice);
        self.layers.clear();
        self.textures.clear();
        self.mattes.clear();
        let canvas = size.map(f64::from);
        for (i, layer) in layers.iter().enumerate() {
            let source = self.sources[i];
            let role = roles.get(i).copied().unwrap_or(Role::Plain);
            let fill = plan
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
            if self.transparent && role == Role::Plain && layer.track == plan.base && i == 0 {
                continue;
            }
            if let Role::Matte(owner) = role {
                if let Some(texture) = texture {
                    self.mattes.push((owner, texture));
                }
                continue;
            }
            let look = match role {
                Role::Picture(id) | Role::Backing(id) => self.look(id),
                _ => Look::default(),
            };
            let orient = plan
                .orients
                .iter()
                .find_map(|&(track, o)| (track == layer.track).then_some(o))
                .unwrap_or(0);
            self.layer(
                plan, layer, *range, at, canvas, fill, source, texture, &look, role, orient,
            )?;
        }
        self.shapes(canvas);
        if text {
            self.text(at, canvas);
        }
        if bgra {
            for layer in &mut self.layers {
                layer.flags |= Layer::OUT_BGRA;
            }
        }
        if self.layers.len() > MAX_LAYERS {
            self.layers.truncate(MAX_LAYERS);
            self.textures.truncate(MAX_LAYERS);
        }
        self.compositor
            .encode(commands, target, size, &self.layers, &self.textures)
            .map_err(gpu)?;
        Ok(true)
    }

    /// The look of a picture (or its backing) in the evaluated frame.
    fn look(&self, id: TimelineId) -> Look {
        self.frame
            .layers
            .iter()
            .find_map(|layer| match layer {
                splice_eval::Layer::Picture(p) if p.item == id => Some(p.look),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[allow(clippy::too_many_arguments)]
    fn layer(
        &mut self,
        plan: &Plan,
        layer: &VideoLayer,
        [from, to]: [Frame; 2],
        at: Frame,
        [cw, ch]: [f64; 2],
        fill: Option<[u32; 2]>,
        source: FrameSource,
        texture: Option<Texture>,
        look: &Look,
        role: Role,
        orient: u32,
    ) -> Result<(), Error> {
        let span = f64::from((to - from).max(1));
        let linear = (f64::from(at - from) / span).clamp(0.0, 1.0);
        let t = if layer.flags & VideoLayer::EASE != 0 {
            linear * linear * (3.0 - 2.0 * linear)
        } else {
            linear
        };
        let lerp = |a: f64, b: f64, k: f64| a + (b - a) * k;
        let opacity = (lerp(layer.opacity[0], layer.opacity[1], linear) * look.opacity) as f32;
        let target = [cw as f32, ch as f32];
        let [dx, dy, dw, dh] = layer.destination;
        if let Some([a, b]) = fill {
            let kind = if a == b {
                Layer::SOLID
            } else {
                Layer::GRADIENT
            };
            let (dest, pivot) = transform([dx, dy, dw, dh], look);
            let mut out = Layer::new(kind, dest, target);
            (out.color0, out.color1, out.opacity) = (rgb(a), rgb(b), opacity);
            out.pivot = pivot;
            out.rotation = look.rotation.to_radians() as f32;
            if matches!(role, Role::Backing(_)) {
                clip(&mut out, look);
            }
            self.push(out, Bound::default());
            return Ok(());
        }
        let Some(texture) = texture else {
            return Ok(());
        };
        // Crop math runs in the displayed orientation; the shader maps back to decoded pixels.
        let (w, h) = if orient % 2 == 1 {
            (f64::from(source.height), f64::from(source.width))
        } else {
            (f64::from(source.width), f64::from(source.height))
        };
        let crop = look.crop.unwrap_or(layer.crop);
        let [mut cx, mut cy, mut cwid, mut chei] =
            [crop[0] * w, crop[1] * h, crop[2] * w, crop[3] * h];
        if layer.center_crop != 0 || look.crop.is_some() {
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
        let zoomed = [
            fx + (dx - fx) * scale,
            fy + (dy - fy) * scale,
            dw * scale,
            dh * scale,
        ];
        let (dest, pivot) = transform(zoomed, look);
        let placed = dest.map(f64::from);
        let mut picture = Layer::new(Layer::TEXTURE, dest, target);
        picture.crop = [cx / w, cy / h, cwid / w, chei / h].map(|v| v as f32);
        picture.opacity = opacity;
        picture.corner = (layer.corner * scale * look.scale) as f32;
        picture.shadow = layer.shadow as f32;
        picture.flags = Layer::BGRA;
        picture.orient = orient;
        picture.pivot = pivot;
        picture.rotation = look.rotation.to_radians() as f32;
        picture.blend = look.blend.index();
        let mut bound = Bound {
            source: Some(texture),
            ..Bound::default()
        };
        if let Some(g) = look.color {
            picture.flags |= Layer::GRADE;
            picture.grade0 = [g.exposure, g.contrast, g.saturation, g.vibrance].map(|v| v as f32);
            picture.grade1 = [g.temperature, g.tint, g.highlights, g.shadows].map(|v| v as f32);
            if let Some((lut, n)) = g.lut.and_then(|i| self.luts.get(usize::from(i))) {
                picture.flags |= Layer::LUT;
                picture.lut[0] = g.lut_amount as f32;
                picture.lut[1] = *n;
                bound.lut = Some(lut.clone());
            }
        }
        for (slot, mask) in picture.masks.iter_mut().zip(look.masks.iter().flatten()) {
            slot.rect = mask.rect.map(|v| v as f32);
            slot.params = [
                match mask.shape {
                    MaskShape::Rect => 0.0,
                    MaskShape::RoundRect => 1.0,
                    MaskShape::Ellipse => 2.0,
                },
                match mask.mode {
                    MaskMode::Blur => 0.0,
                    MaskMode::Pixelate => 1.0,
                    MaskMode::Cover => 2.0,
                    MaskMode::Spotlight => 3.0,
                },
                mask.amount as f32,
                mask.feather as f32,
            ];
            let [r, g, b, _] = rgb(mask.color);
            slot.color = [r, g, b, if mask.invert { 1.0 } else { 0.0 }];
            picture.mask_count += 1;
        }
        if let Some(key) = look.key {
            picture.flags |= Layer::KEY;
            let [r, g, b, _] = rgb(key.color);
            picture.key0 = [r, g, b, key.tolerance as f32];
            picture.key1 = [key.softness as f32, key.spill as f32, 0.0, 0.0];
        }
        if let (Some(cutout), Role::Picture(id)) = (look.cutout, role)
            && let Some((_, matte)) = self.mattes.iter().find(|(owner, _)| *owner == id)
        {
            picture.flags |= Layer::CUTOUT;
            if cutout.blur {
                picture.flags |= Layer::CUTOUT_BLUR;
            }
            picture.lut[2] = cutout.amount as f32;
            bound.matte = Some(matte.clone());
        }
        clip(&mut picture, look);
        self.push(picture, bound);
        let Some((samples, _, points)) = usize::try_from(layer.cursor)
            .ok()
            .and_then(|i| plan.cursors.get(i))
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
        let rotation = look.rotation.to_radians() as f32;
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
                (ring.pivot, ring.rotation) = (pivot, rotation);
                self.push(ring, Bound::default());
            }
        }
        let table = usize::try_from(layer.cursor).unwrap_or(0);
        let images = &plan.cursors[table].1;
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
            (cursor.pivot, cursor.rotation) = (pivot, rotation);
            self.push(
                cursor,
                Bound {
                    source: Some(texture),
                    ..Bound::default()
                },
            );
        }
        Ok(())
    }

    /// Generated shapes from the evaluated frame.
    fn shapes(&mut self, [cw, ch]: [f64; 2]) {
        let target = [cw as f32, ch as f32];
        for index in 0..self.frame.layers.len() {
            let splice_eval::Layer::Shape {
                shape,
                bounds,
                fill,
                fill2,
                stroke,
                stroke_width,
                corner,
                opacity,
                look,
                ..
            } = self.frame.layers[index]
            else {
                continue;
            };
            let kind = match shape {
                ShapeKind::Solid | ShapeKind::Rect => {
                    if fill == fill2 {
                        Layer::SOLID
                    } else {
                        Layer::GRADIENT
                    }
                }
                ShapeKind::Gradient => Layer::GRADIENT,
                ShapeKind::Ellipse => Layer::ELLIPSE,
                ShapeKind::Line => Layer::LINE,
                ShapeKind::Arrow => Layer::ARROW,
                ShapeKind::Callout => Layer::CALLOUT,
            };
            let (dest, pivot) = transform([bounds.x, bounds.y, bounds.w, bounds.h], &look);
            let mut out = Layer::new(kind, dest, target);
            out.color0 = rgb(fill);
            out.color1 = rgb(fill2);
            let [r, g, b, _] = rgb(stroke);
            out.key0 = [r, g, b, 0.0];
            out.ring = (stroke_width * look.scale) as f32;
            out.corner = (corner * look.scale) as f32;
            out.opacity = (opacity * look.opacity) as f32;
            out.pivot = pivot;
            out.rotation = look.rotation.to_radians() as f32;
            out.blend = look.blend.index();
            if shape == ShapeKind::Rect && stroke_width > 0.0 {
                out.kind = Layer::BOX;
            }
            self.push(out, Bound::default());
        }
    }

    /// Titles, captions and chips: per-frame fade and look from the evaluator, animators.
    fn text(&mut self, at: Frame, [cw, ch]: [f64; 2]) {
        let target = [cw as f32, ch as f32];
        for index in 0..self.texts.len() {
            let (texture, cue) = &self.texts[index];
            if at < cue.range[0] || at >= cue.range[1] {
                continue;
            }
            let (fade, look) = match cue.item {
                Some(item) => match self.frame.layers.iter().find_map(|layer| match *layer {
                    splice_eval::Layer::Title {
                        item: id,
                        opacity,
                        look,
                        ..
                    }
                    | splice_eval::Layer::Caption {
                        item: id,
                        opacity,
                        look,
                        ..
                    } if id == item => Some((opacity, look)),
                    _ => None,
                }) {
                    Some(found) => found,
                    None => continue,
                },
                None => (f64::from(cue.opacity(at)), Look::default()),
            };
            let [x, y, w, h] = cue.layer.bounds;
            let mut bounds = [x, y, w.ceil().max(1.0), h.ceil().max(1.0)];
            let mut alpha = fade;
            if let Some((start, duration, animate)) = cue.reveal {
                let p = ((f64::from(at - start) + 1.0) / duration).clamp(0.0, 1.0);
                let ease = 1.0 - (1.0 - p) * (1.0 - p);
                match animate {
                    Animate::Fade | Animate::Highlight => alpha *= p,
                    Animate::Rise => {
                        alpha *= p;
                        bounds[1] += (1.0 - ease) * h * 0.35;
                    }
                    Animate::Slide => {
                        alpha *= p;
                        bounds[0] -= (1.0 - ease) * w * 0.08;
                    }
                    Animate::Pop | Animate::Bounce => {
                        let s = if animate == Animate::Bounce {
                            1.0 + 0.25 * (std::f64::consts::PI * p).sin() * (1.0 - p)
                                - (1.0 - ease) * 0.4
                        } else {
                            0.6 + 0.4 * ease
                        };
                        alpha *= p.min(1.0);
                        bounds = [
                            x + w * (1.0 - s) / 2.0,
                            y + h * (1.0 - s) / 2.0,
                            w * s,
                            h * s,
                        ];
                    }
                    Animate::Typewriter => {
                        if at < start {
                            alpha = 0.0;
                        }
                    }
                    Animate::None => {}
                }
            }
            let (dest, pivot) = transform(bounds, &look);
            let mut layer = Layer::new(Layer::TEXTURE, dest, target);
            layer.opacity = (alpha * look.opacity) as f32;
            layer.pivot = pivot;
            layer.rotation = look.rotation.to_radians() as f32;
            layer.blend = look.blend.index();
            let texture = texture.clone();
            self.push(
                layer,
                Bound {
                    source: Some(texture),
                    ..Bound::default()
                },
            );
        }
    }

    fn push(&mut self, layer: Layer, bound: Bound) {
        self.layers.push(layer);
        self.textures.push(bound);
    }
}

/// Applies a look's scale (about its anchor) and offset to a destination; returns the new
/// destination and the rotation pivot in canvas pixels.
fn transform([x, y, w, h]: [f64; 4], look: &Look) -> ([f32; 4], [f32; 2]) {
    let pivot = [x + look.anchor[0] * w, y + look.anchor[1] * h];
    let s = look.scale;
    let dest = [
        pivot[0] + (x - pivot[0]) * s + look.offset[0],
        pivot[1] + (y - pivot[1]) * s + look.offset[1],
        w * s,
        h * s,
    ];
    (
        dest.map(|v| v as f32),
        [
            (pivot[0] + look.offset[0]) as f32,
            (pivot[1] + look.offset[1]) as f32,
        ],
    )
}

/// Limits a layer to a transition wipe's region.
fn clip(layer: &mut Layer, look: &Look) {
    if let Some(region) = look.clip {
        layer.flags |= Layer::CLIP;
        layer.clip = region.map(|v| v as f32);
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
