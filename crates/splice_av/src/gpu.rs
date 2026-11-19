//! The OGPU compositor's frame path (PLAN.md T12.7): evaluates the lowered layer plan at one frame
//! into `splice_gpu` layers over the decoded sources. The viewer, frame grabs and export all draw
//! through `GpuProgram::draw`.
use crate::{
    Composition, Error, TimedText,
    lower::{Plan, Role, Slot},
};
use apple_sys::composition::{CursorSample, FrameSource, Player};
use ogpu_core::backend::{self as b, Commands, Device, PreparedRenderTarget, Texture};
use ogpu_core::types::{Extent, Origin, TextureDescriptor, TextureFormat, TextureUsage};
use splice_eval::{Animate, FramePlan, Look, MaskMode, MaskShape};
use splice_gpu::{Bound, Compositor, Layer, MAX_LAYERS, rgb};
use splice_model::{CameraMode, Frame, ShapeKind};

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
    /// Matte textures bound this frame, by the role of the layer they belong to.
    mattes: Vec<(Role, Texture)>,
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
        for (_, images, ..) in &plan.cursors {
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
        let Some((_, layers)) = intervals.get(index) else {
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
        self.layers.clear();
        self.textures.clear();
        self.mattes.clear();
        let canvas = size.map(f64::from);
        for (i, slot) in layers.iter().enumerate() {
            let source = self.sources[i];
            let texture = match self.copies.get(i) {
                Some(Some((texture, ..))) if !source.texture.is_null() => Some(texture.clone()),
                _ => None,
            };
            // Mattes are bound to the layer they cut out, never drawn.
            let owner = match slot.role {
                Role::Matte(id) => Some(Role::Picture(id)),
                Role::CameraMatte(clip) => Some(Role::Camera(clip)),
                _ => None,
            };
            if let Some(owner) = owner {
                if let Some(texture) = texture {
                    self.mattes.push((owner, texture));
                }
                continue;
            }
            let Some(evaluated) = self.frame.layers.get(slot.depth as usize).copied() else {
                // The black base: left out of transparent exports.
                if !self.transparent {
                    let mut base = Layer::new(
                        Layer::SOLID,
                        [0.0, 0.0, canvas[0] as f32, canvas[1] as f32],
                        canvas.map(|v| v as f32),
                    );
                    base.color0 = [0.0, 0.0, 0.0, 1.0];
                    self.push(base, Bound::default());
                }
                continue;
            };
            let orient = plan
                .orients
                .iter()
                .find_map(|&(track, o)| (track == slot.track).then_some(o))
                .unwrap_or(0);
            self.layer(plan, slot, evaluated, canvas, source, texture, orient)?;
        }
        self.shapes(canvas);
        if text {
            self.text(at, canvas);
        }
        // Nothing under an opaque full-canvas fill shows (the black base under a window
        // background): skip it, ~2 ms per covered layer at 4K on an M1.
        let [cw, ch] = canvas;
        let covers = |l: &Layer| {
            matches!(l.kind, Layer::SOLID | Layer::GRADIENT)
                && l.opacity >= 1.0
                && l.color0[3] >= 1.0
                && l.color1[3] >= 1.0
                && l.blend == 0
                && l.rotation == 0.0
                && l.corner == 0.0
                && l.flags & Layer::CLIP == 0
                && l.dest[0] <= 0.0
                && l.dest[1] <= 0.0
                && f64::from(l.dest[0] + l.dest[2]) >= cw
                && f64::from(l.dest[1] + l.dest[3]) >= ch
        };
        if let Some(top) = self.layers.iter().rposition(covers)
            && top > 0
            && self.textures.len() == self.layers.len()
        {
            self.layers.drain(..top);
            self.textures.drain(..top);
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
            .encode(
                commands,
                target,
                size,
                &self.layers,
                &self.textures,
                !self.transparent,
            )
            .map_err(gpu)?;
        Ok(true)
    }

    /// Builds the GPU layer(s) for one slot from its evaluated layer: every value is this
    /// frame's, so nothing is interpolated between interval ends.
    fn layer(
        &mut self,
        plan: &Plan,
        slot: &Slot,
        evaluated: splice_eval::Layer,
        [cw, ch]: [f64; 2],
        source: FrameSource,
        texture: Option<Texture>,
        orient: u32,
    ) -> Result<(), Error> {
        let target = [cw as f32, ch as f32];
        match evaluated {
            // A backdrop is the backing's own evaluated opacity; it never borrows the picture's
            // look or camera (H7, D14).
            splice_eval::Layer::Background {
                bounds,
                fill,
                opacity,
            } => {
                let [a, b] = match fill {
                    splice_eval::Background::None => return Ok(()),
                    splice_eval::Background::Solid(c) => [c, c],
                    splice_eval::Background::Gradient(a, b) => [a, b],
                };
                let kind = if a == b {
                    Layer::SOLID
                } else {
                    Layer::GRADIENT
                };
                let mut out = Layer::new(kind, rect(bounds), target);
                (out.color0, out.color1, out.opacity) = (rgb(a), rgb(b), opacity as f32);
                self.push(out, Bound::default());
            }
            splice_eval::Layer::Fade { opacity, color } => {
                let mut out = Layer::new(Layer::SOLID, [0.0, 0.0, target[0], target[1]], target);
                (out.color0, out.opacity) = (rgb(color), opacity as f32);
                self.push(out, Bound::default());
            }
            splice_eval::Layer::Camera {
                bounds,
                radius,
                shadow,
                background,
                ..
            } => {
                let Some(texture) = texture else {
                    return Ok(());
                };
                let look = camera_look(background);
                let dest = [bounds.x, bounds.y, bounds.w, bounds.h];
                let crop = center_crop(source, orient, [0.0, 0.0, 1.0, 1.0], dest);
                let mut camera = Layer::new(Layer::TEXTURE, rect(bounds), target);
                camera.crop = crop.map(|v| v as f32);
                camera.corner = radius as f32;
                camera.shadow = shadow.clamp(0.0, 1.0) as f32;
                camera.flags = Layer::BGRA;
                camera.orient = orient;
                let mut bound = Bound {
                    source: Some(texture),
                    ..Bound::default()
                };
                self.cutout(&mut camera, &mut bound, &look, slot.role);
                self.push(camera, bound);
            }
            splice_eval::Layer::Picture(picture) => {
                if let Some(texture) = texture {
                    self.picture(plan, slot, &picture, target, source, texture, orient);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// A clip or overlay picture with its look, camera view, window corners and cursor.
    #[allow(clippy::too_many_arguments)]
    fn picture(
        &mut self,
        plan: &Plan,
        slot: &Slot,
        p: &splice_eval::Picture,
        target: [f32; 2],
        source: FrameSource,
        texture: Texture,
        orient: u32,
    ) {
        let look = &p.look;
        let [cw, ch] = target.map(f64::from);
        // Crop math runs in the displayed orientation; the shader maps back to decoded pixels.
        let (w, h) = if orient % 2 == 1 {
            (f64::from(source.height), f64::from(source.width))
        } else {
            (f64::from(source.width), f64::from(source.height))
        };
        let d = p.destination;
        let [dx, dy, dw, dh] = [d.x, d.y, d.w, d.h];
        let base = look
            .crop
            .map_or([p.source.x, p.source.y, p.source.w, p.source.h], |c| c);
        let [mut cx, mut cy, mut cwid, mut chei] =
            [base[0] * w, base[1] * h, base[2] * w, base[3] * h];
        if look.crop.is_some() {
            let ratio = dw / dh;
            let width = cwid.min(chei * ratio);
            let height = chei.min(cwid / ratio);
            cx += (cwid - width) / 2.0;
            cy += (chei - height) / 2.0;
            (cwid, chei) = (width, height);
        }
        // The timeline camera (D12, D13): content mode narrows the crop to the viewport inside
        // a fixed frame; frame mode scales the card so the viewport fills the card's place.
        let view = p.view;
        let scale = view.amount.max(1.0);
        let [vx, vy, vw, vh] = view.viewport();
        let content = match view.mode {
            CameraMode::Content => true,
            CameraMode::Frame => false,
            CameraMode::Auto => ch > cw,
        };
        let zoomed = if content || scale <= 1.0 {
            (cx, cy, cwid, chei) = (cx + vx * cwid, cy + vy * chei, cwid * vw, chei * vh);
            [dx, dy, dw, dh]
        } else {
            [
                dx - vx * scale * dw,
                dy - vy * scale * dh,
                dw * scale,
                dh * scale,
            ]
        };
        let card = if content { 1.0 } else { scale };
        let (dest, pivot) = transform(zoomed, look);
        let placed = dest.map(f64::from);
        let opacity = (p.opacity * look.opacity) as f32;
        let mut layer = Layer::new(Layer::TEXTURE, dest, target);
        layer.crop = [cx / w, cy / h, cwid / w, chei / h].map(|v| v as f32);
        layer.opacity = opacity;
        if let Some(window) = p.window {
            layer.corner = (window.corner * dw.min(dh) * card * look.scale) as f32;
            layer.shadow = window.shadow.clamp(0.0, 1.0) as f32;
        }
        layer.flags = Layer::BGRA;
        layer.orient = orient;
        layer.pivot = pivot;
        layer.rotation = look.rotation.to_radians() as f32;
        layer.blend = look.blend.index();
        let mut bound = Bound {
            source: Some(texture),
            ..Bound::default()
        };
        if let Some(g) = look.color {
            layer.flags |= Layer::GRADE;
            layer.grade0 = [g.exposure, g.contrast, g.saturation, g.vibrance].map(|v| v as f32);
            layer.grade1 = [g.temperature, g.tint, g.highlights, g.shadows].map(|v| v as f32);
            if let Some((lut, n)) = g.lut.and_then(|i| self.luts.get(usize::from(i))) {
                layer.flags |= Layer::LUT;
                layer.lut[0] = g.lut_amount as f32;
                layer.lut[1] = *n;
                bound.lut = Some(lut.clone());
            }
        }
        for (mask_slot, mask) in layer.masks.iter_mut().zip(look.masks.iter().flatten()) {
            mask_slot.rect = mask.rect.map(|v| v as f32);
            mask_slot.params = [
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
            mask_slot.color = [r, g, b, if mask.invert { 1.0 } else { 0.0 }];
            layer.mask_count += 1;
        }
        if let Some(key) = look.key {
            layer.flags |= Layer::KEY;
            let [r, g, b, _] = rgb(key.color);
            layer.key0 = [r, g, b, key.tolerance as f32];
            layer.key1 = [key.softness as f32, key.spill as f32, 0.0, 0.0];
        }
        self.cutout(&mut layer, &mut bound, look, slot.role);
        clip(&mut layer, look);
        self.push(layer, bound);
        // The recorded cursor, on the take's clock (the evaluated source frame, so speed ramps
        // and freezes are exact).
        let (Some(cursor), Some((samples, images, points, fps))) = (
            p.cursor,
            usize::try_from(slot.cursor)
                .ok()
                .and_then(|i| plan.cursors.get(i)),
        ) else {
            return;
        };
        let st = p.source_frame / fps.max(1.0);
        let per_px = placed[2] / cwid; // canvas pixels per source pixel
        let px_per_point = w / points;
        let to_canvas = |x: f64, y: f64| {
            [
                placed[0] + (x * w - cx) * per_px,
                placed[1] + (y * h - cy) * per_px,
            ]
        };
        let rotation = look.rotation.to_radians() as f32;
        if cursor.click_rings {
            for sample in samples.iter().filter(|s| s.click == 1) {
                let age = st - sample.t;
                if !(0.0..=RING_SECONDS).contains(&age) {
                    continue;
                }
                let [x, y] = to_canvas(sample.x, sample.y);
                let radius =
                    (10.0 + 34.0 * age / RING_SECONDS) * px_per_point * per_px * cursor.scale;
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
        let table = slot.cursor as usize;
        let sprite = cursor_at(samples, st).and_then(|(sample, at)| {
            let image = usize::try_from(sample.image).ok()?;
            let (texture, size) = self.cursors[table].get(image)?;
            Some((images.get(image)?, texture.clone(), *size, at))
        });
        if let Some((meta, texture, [pw, ph], [x, y])) = sprite {
            let k = cursor.scale * px_per_point * per_px / meta.scale; // canvas px per PNG px
            let [x, y] = to_canvas(x, y);
            let mut sprite = Layer::new(
                Layer::TEXTURE,
                [x - meta.hot[0] * k, y - meta.hot[1] * k, pw * k, ph * k].map(|v| v as f32),
                target,
            );
            sprite.opacity = opacity;
            (sprite.pivot, sprite.rotation) = (pivot, rotation);
            self.push(
                sprite,
                Bound {
                    source: Some(texture),
                    ..Bound::default()
                },
            );
        }
    }

    /// Binds a person matte to a picture or camera bubble with a cutout look.
    fn cutout(&self, layer: &mut Layer, bound: &mut Bound, look: &Look, role: Role) {
        if let Some(cutout) = look.cutout
            && let Some((_, matte)) = self.mattes.iter().find(|(owner, _)| *owner == role)
        {
            layer.flags |= Layer::CUTOUT;
            if cutout.blur {
                layer.flags |= Layer::CUTOUT_BLUR;
            }
            layer.lut[2] = cutout.amount as f32;
            bound.matte = Some(matte.clone());
        }
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

/// A camera bubble's background treatment (0 none, 1 blur, 2 remove) as a cutout look.
fn camera_look(background: u8) -> Look {
    Look {
        cutout: match background {
            1 => Some(splice_eval::Cutout {
                blur: true,
                amount: 1.0,
            }),
            2 => Some(splice_eval::Cutout {
                blur: false,
                amount: 1.0,
            }),
            _ => None,
        },
        ..Look::default()
    }
}

fn rect(r: splice_model::Rect) -> [f32; 4] {
    [r.x, r.y, r.w, r.h].map(|v| v as f32)
}

/// The crop (normalized to the oriented source) that center-fills `dest` from `base`.
fn center_crop(
    source: FrameSource,
    orient: u32,
    base: [f64; 4],
    [_, _, dw, dh]: [f64; 4],
) -> [f64; 4] {
    let (w, h) = if orient % 2 == 1 {
        (f64::from(source.height), f64::from(source.width))
    } else {
        (f64::from(source.width), f64::from(source.height))
    };
    let [bx, by, bw, bh] = [base[0] * w, base[1] * h, base[2] * w, base[3] * h];
    let ratio = dw / dh.max(1e-9);
    let width = bw.min(bh * ratio);
    let height = bh.min(bw / ratio);
    [
        (bx + (bw - width) / 2.0) / w.max(1.0),
        (by + (bh - height) / 2.0) / h.max(1.0),
        width / w.max(1.0),
        height / h.max(1.0),
    ]
}
