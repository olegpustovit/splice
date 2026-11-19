//! The portable compositor (PLAN.md D1, T12.7): draws a frame's layers into an OGPU texture.
//! One quad per layer; a source texture, a solid or gradient fill, or a click ring; rounded
//! corners and the drop shadow from a signed distance in the same pass. The caller owns the device
//! (shared with PXUI) and the textures; steady-state frames allocate nothing.
#![deny(missing_docs)]

use ogpu_core::backend::{
    self as b, Buffer, ColorAttachment, Commands, Device, Pipeline, PreparedRenderTarget,
    RenderTarget, Sampler, ShaderStage, Texture,
};
use ogpu_core::binding::{self, LayoutEntry};
use ogpu_core::error::Error;
use ogpu_core::pipeline::{ColorFormat, DepthFormat, RenderDescriptor};
use ogpu_core::render_pass::{Load, Primitive, Rect, Store};
use ogpu_core::shader::{self as sh, Binding, BindingKind, EntryPoint, Stage};
use ogpu_core::types::{
    AddressMode, BufferDescriptor, BufferUsage, MipFilter, SamplerDescriptor, SamplerFilter,
    TextureDescriptor, TextureFormat, TextureUsage,
};

const MSL: &str = include_str!("../shaders/layer.metal");
/// Constant-buffer offsets are 256-byte aligned on every Metal GPU.
const STRIDE: usize = 512;
/// Most layers one frame can draw.
pub const MAX_LAYERS: usize = 64;
const PAGES: usize = 3;

/// Textures a layer samples: its source, LUT atlas and person matte (`Rc` handles: cloning
/// into a reused `Vec` allocates nothing).
#[derive(Clone, Default)]
pub struct Bound {
    /// The source picture.
    pub source: Option<Texture>,
    /// LUT atlas (`n² × n`) when `Layer::LUT` is set.
    pub lut: Option<Texture>,
    /// Person matte when `Layer::CUTOUT` is set.
    pub matte: Option<Texture>,
}

/// One mask on a texture layer (mirrors `Mask` in layer.metal).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mask {
    /// x, y, w, h normalized to the layer's destination.
    pub rect: [f32; 4],
    /// Shape (0 rect, 1 rounded, 2 ellipse), mode (0 blur, 1 pixelate, 2 cover, 3 spotlight),
    /// amount, feather.
    pub params: [f32; 4],
    /// Cover RGB, and invert (1) in the last slot.
    pub color: [f32; 4],
}

/// One quad's parameters (mirrors `Layer` in layer.metal, 512 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layer {
    /// x, y, w, h in target pixels before rotation.
    pub dest: [f32; 4],
    /// u, v, w, h normalized to the oriented source.
    pub crop: [f32; 4],
    /// Fill, ring or shape color.
    pub color0: [f32; 4],
    /// Gradient end color.
    pub color1: [f32; 4],
    /// Target size in pixels.
    pub target: [f32; 2],
    /// Opacity.
    pub opacity: f32,
    /// Corner radius in pixels.
    pub corner: f32,
    /// Drop shadow 0…1.
    pub shadow: f32,
    /// Ring width, or a shape's stroke width, in pixels.
    pub ring: f32,
    /// One of the `Layer::*` kinds.
    pub kind: u32,
    /// `BGRA`, `OUT_BGRA`, `GRADE`, `LUT`, `KEY`, `CUTOUT`, `CUTOUT_BLUR`, `CLIP` bits.
    pub flags: u32,
    /// Rotation pivot in target pixels.
    pub pivot: [f32; 2],
    /// Clockwise radians.
    pub rotation: f32,
    /// Source orientation: 0 none, 1 90°, 2 180°, 3 270°.
    pub orient: u32,
    /// Exposure, contrast, saturation, vibrance.
    pub grade0: [f32; 4],
    /// Temperature, tint, highlights, shadows.
    pub grade1: [f32; 4],
    /// LUT amount, cube size, cutout amount.
    pub lut: [f32; 4],
    /// Key RGB and tolerance; for shapes, the stroke RGB.
    pub key0: [f32; 4],
    /// Key softness and spill.
    pub key1: [f32; 4],
    /// Visible region in target pixels when `CLIP` is set.
    pub clip: [f32; 4],
    /// Masks in order; `mask_count` are live.
    pub masks: [Mask; 4],
    /// 0 normal, 1 multiply, 2 screen, 3 overlay, 4 add, 5 darken, 6 lighten.
    pub blend: u32,
    /// Live masks.
    pub mask_count: u32,
    /// Padding to the shader's 16-byte alignment.
    pub pad: [u32; 2],
}

const _: () = assert!(std::mem::size_of::<Layer>() == 416 && STRIDE >= 416);

impl Layer {
    /// Samples the bound source texture.
    pub const TEXTURE: u32 = 0;
    /// Solid fill.
    pub const SOLID: u32 = 1;
    /// Two-color diagonal gradient.
    pub const GRADIENT: u32 = 2;
    /// Click ring.
    pub const RING: u32 = 3;
    /// Ellipse shape.
    pub const ELLIPSE: u32 = 4;
    /// Line from the top-left to the bottom-right of `dest`.
    pub const LINE: u32 = 5;
    /// Line with an arrowhead at the end.
    pub const ARROW: u32 = 6;
    /// Rounded box with a pointer.
    pub const CALLOUT: u32 = 7;
    /// Rounded box with an outline (a stroked rectangle shape).
    pub const BOX: u32 = 8;
    /// The source texture holds BGRA bytes.
    pub const BGRA: u32 = 1;
    /// Write BGRA (for an encoder buffer).
    pub const OUT_BGRA: u32 = 2;
    /// Apply `grade0`/`grade1`.
    pub const GRADE: u32 = 4;
    /// Apply the bound LUT.
    pub const LUT: u32 = 8;
    /// Chroma key.
    pub const KEY: u32 = 16;
    /// Multiply alpha by the bound matte.
    pub const CUTOUT: u32 = 32;
    /// Blur the background through the matte instead of removing it.
    pub const CUTOUT_BLUR: u32 = 64;
    /// Limit drawing to `clip`.
    pub const CLIP: u32 = 128;

    /// A layer of `kind` covering `dest` in a `target`-sized canvas.
    pub fn new(kind: u32, dest: [f32; 4], target: [f32; 2]) -> Self {
        Self {
            dest,
            crop: [0.0, 0.0, 1.0, 1.0],
            target,
            opacity: 1.0,
            kind,
            pivot: [dest[0] + dest[2] / 2.0, dest[1] + dest[3] / 2.0],
            grade0: [0.0, 1.0, 1.0, 0.0],
            ..Self::default()
        }
    }
}

/// Converts a 0xRRGGBB color to straight-alpha floats.
pub fn rgb(value: u32) -> [f32; 4] {
    [
        ((value >> 16) & 255) as f32 / 255.0,
        ((value >> 8) & 255) as f32 / 255.0,
        (value & 255) as f32 / 255.0,
        1.0,
    ]
}

/// Pipeline, sampler and parameter pages for one device.
pub struct Compositor {
    pipeline: Pipeline,
    sampler: Sampler,
    blank: Texture,
    pages: [Buffer; PAGES],
    next: usize,
    scratch: Vec<u8>,
}

impl Compositor {
    /// Builds the layer pipeline on `device`. Metal only until the Vulkan SPIR-V lands.
    pub fn new(device: &Device) -> Result<Self, Error> {
        let backend = ogpu_core::impl_::backend()?;
        if backend != "metal" {
            // ponytail: Metal shading language only. Add the SPIR-V pair when Linux gets a decoder.
            return Err(ogpu_core::error::make(
                "Splice.Compositor",
                ogpu_core::error::ErrorKind::Unsupported,
                "the compositor ships MSL only",
            ));
        }
        let shader = |bindings: Vec<Binding>| {
            sh::create(sh::Descriptor {
                backend: backend.clone(),
                label: Some("Splice layer".into()),
                bytes: MSL.as_bytes().to_vec(),
                entry_points: vec![
                    EntryPoint {
                        name: "layer_vertex".into(),
                        stage: Stage::Vertex,
                    },
                    EntryPoint {
                        name: "layer_fragment".into(),
                        stage: Stage::Fragment,
                    },
                ],
                bindings,
            })
        };
        let params = |stages: Vec<Stage>| Binding {
            group: 0,
            binding: 0,
            kind: BindingKind::UniformBuffer,
            visibility: stages,
        };
        let vertex = shader(vec![params(vec![Stage::Vertex])])?;
        let fragment = shader(vec![
            params(vec![Stage::Fragment]),
            Binding {
                group: 0,
                binding: 1,
                kind: BindingKind::SampledTexture,
                visibility: vec![Stage::Fragment],
            },
            Binding {
                group: 0,
                binding: 2,
                kind: BindingKind::Sampler,
                visibility: vec![Stage::Fragment],
            },
            Binding {
                group: 0,
                binding: 3,
                kind: BindingKind::SampledTexture,
                visibility: vec![Stage::Fragment],
            },
            Binding {
                group: 0,
                binding: 4,
                kind: BindingKind::SampledTexture,
                visibility: vec![Stage::Fragment],
            },
        ])?;
        let both = vec![binding::Stage::Vertex, binding::Stage::Fragment];
        let fragment_only = vec![binding::Stage::Fragment];
        let layout = binding::create_layout(&[
            LayoutEntry {
                binding: 0,
                kind: binding::Kind::Buffer,
                visibility: both,
            },
            // OGPU numbers bindings uniquely and Metal checks them against its slots: buffer 0,
            // texture 1, sampler 2 in the MSL and the `set_stage_*` calls.
            LayoutEntry {
                binding: 1,
                kind: binding::Kind::Texture,
                visibility: fragment_only.clone(),
            },
            LayoutEntry {
                binding: 2,
                kind: binding::Kind::Sampler,
                visibility: fragment_only.clone(),
            },
            // LUT atlas (3) and person matte (4).
            LayoutEntry {
                binding: 3,
                kind: binding::Kind::Texture,
                visibility: fragment_only.clone(),
            },
            LayoutEntry {
                binding: 4,
                kind: binding::Kind::Texture,
                visibility: fragment_only,
            },
        ])?;
        let layout = binding::create_pipeline_layout(
            b::device_handle(device),
            &b::capabilities(device),
            &[(0, layout)],
        )?;
        let pipeline = b::create_render_pipeline(
            device,
            &RenderDescriptor {
                backend,
                label: Some("Splice layer".into()),
                layout,
                vertex,
                vertex_entry: "layer_vertex".into(),
                fragment: Some(fragment),
                fragment_entry: Some("layer_fragment".into()),
                color_format: ColorFormat::Rgba8Unorm,
                depth_format: DepthFormat::NoDepth,
                sample_count: 1,
            },
            // The shader blends against `[[color(0)]]` (every blend mode, one pipeline).
            None,
            Some(Primitive::TriangleList),
            Some(false),
        )?;
        let sampler = b::create_sampler(
            device,
            SamplerDescriptor {
                label: Some("Splice layer".into()),
                min_filter: SamplerFilter::Linear,
                mag_filter: SamplerFilter::Linear,
                mip_filter: MipFilter::NoMip,
                address_u: AddressMode::ClampToEdge,
                address_v: AddressMode::ClampToEdge,
                lod_min: 0.0,
                lod_max: 0.0,
                max_anisotropy: 1,
            },
        )?;
        // Fills still bind a texture: the fragment function declares one.
        let blank = b::create_texture(
            device,
            TextureDescriptor {
                label: Some("Splice blank".into()),
                width: 1,
                height: 1,
                depth: 1,
                mip_levels: 1,
                sample_count: 1,
                format: TextureFormat::Rgba8Unorm,
                usage: vec![TextureUsage::TextureBinding],
            },
        )?;
        let page = || {
            b::create_buffer(
                device,
                BufferDescriptor {
                    label: Some("Splice layers".into()),
                    size: (STRIDE * MAX_LAYERS) as i64,
                    usage: vec![BufferUsage::Uniform],
                },
                None,
            )
        };
        Ok(Self {
            pipeline,
            sampler,
            blank,
            pages: [page()?, page()?, page()?],
            next: 0,
            scratch: vec![0; STRIDE * MAX_LAYERS],
        })
    }

    /// A render target for `texture`, cleared to `clear` (straight RGBA). Build it once per
    /// texture and reuse it: a prepared target restarts its pass without allocating.
    pub fn target(
        device: &Device,
        texture: &Texture,
        clear: [f64; 4],
    ) -> Result<PreparedRenderTarget, Error> {
        b::prepare_render_target(
            device,
            RenderTarget {
                colors: vec![ColorAttachment {
                    texture: texture.clone(),
                    resolve: None,
                    load: Load::Clear,
                    store: Store::Store,
                    clear: (clear[0], clear[1], clear[2], clear[3]),
                }],
                depth: None,
            },
        )
    }

    /// Encodes `layers`, back to front, into `target` within `commands`. `textures[i]` is layer
    /// i's source for `Layer::TEXTURE` (handles are `Rc` clones, so a reused `Vec` of them costs no
    /// allocation). At most `MAX_LAYERS`; extra layers are refused.
    pub fn encode(
        &mut self,
        commands: &Commands,
        target: &mut PreparedRenderTarget,
        size: [u32; 2],
        layers: &[Layer],
        textures: &[Bound],
    ) -> Result<(), Error> {
        if layers.len() > MAX_LAYERS || textures.len() != layers.len() {
            return Err(ogpu_core::error::make(
                "Splice.Compositor.encode",
                ogpu_core::error::ErrorKind::InvalidArgument,
                "one texture slot per layer, at most MAX_LAYERS",
            ));
        }
        let page = &self.pages[self.next];
        self.next = (self.next + 1) % PAGES;
        for (index, layer) in layers.iter().enumerate() {
            layer_bytes(
                layer,
                &mut self.scratch[index * STRIDE..index * STRIDE + LAYER_BYTES],
            );
        }
        if !layers.is_empty() {
            b::write_buffer(page, 0, &self.scratch[..layers.len() * STRIDE])?;
        }
        let render = b::render_prepared(commands, target)?;
        let drawn = (|| {
            b::set_render_pipeline(&render, &self.pipeline)?;
            b::set_viewport(
                &render,
                Rect {
                    x: 0,
                    y: 0,
                    width: i64::from(size[0]),
                    height: i64::from(size[1]),
                },
            )?;
            b::set_stage_sampler(&render, ShaderStage::Fragment, 2, &self.sampler)?;
            for (index, layer) in layers.iter().enumerate() {
                let offset = Some((index * STRIDE) as i64);
                b::set_stage_buffer(&render, ShaderStage::Vertex, 0, offset, page)?;
                b::set_stage_buffer(&render, ShaderStage::Fragment, 0, offset, page)?;
                let bound = &textures[index];
                let texture = match (layer.kind, &bound.source) {
                    (Layer::TEXTURE, Some(texture)) => texture,
                    _ => &self.blank,
                };
                b::set_stage_texture(&render, ShaderStage::Fragment, 1, texture)?;
                let lut = bound.lut.as_ref().unwrap_or(&self.blank);
                b::set_stage_texture(&render, ShaderStage::Fragment, 3, lut)?;
                let matte = bound.matte.as_ref().unwrap_or(&self.blank);
                b::set_stage_texture(&render, ShaderStage::Fragment, 4, matte)?;
                b::draw(&render, Primitive::TriangleList, 0, 6, None)?;
            }
            Ok(())
        })();
        let ended = b::end_render(&render);
        drawn.and(ended)
    }
}

/// Serializes a layer in the shader's field order (no `unsafe` transmute).
fn layer_bytes(layer: &Layer, out: &mut [u8]) {
    let mut at = 0;
    let mut put = |bytes: [u8; 4]| {
        out[at..at + 4].copy_from_slice(&bytes);
        at += 4;
    };
    let floats = |put: &mut dyn FnMut([u8; 4]), values: &[f32]| {
        values.iter().for_each(|v| put(v.to_le_bytes()));
    };
    floats(&mut put, &layer.dest);
    floats(&mut put, &layer.crop);
    floats(&mut put, &layer.color0);
    floats(&mut put, &layer.color1);
    floats(&mut put, &layer.target);
    floats(
        &mut put,
        &[layer.opacity, layer.corner, layer.shadow, layer.ring],
    );
    put(layer.kind.to_le_bytes());
    put(layer.flags.to_le_bytes());
    floats(&mut put, &layer.pivot);
    put(layer.rotation.to_le_bytes());
    put(layer.orient.to_le_bytes());
    for block in [
        layer.grade0,
        layer.grade1,
        layer.lut,
        layer.key0,
        layer.key1,
        layer.clip,
    ] {
        floats(&mut put, &block);
    }
    for mask in &layer.masks {
        floats(&mut put, &mask.rect);
        floats(&mut put, &mask.params);
        floats(&mut put, &mask.color);
    }
    for value in [layer.blend, layer.mask_count, layer.pad[0], layer.pad[1]] {
        put(value.to_le_bytes());
    }
    debug_assert_eq!(at, LAYER_BYTES);
}

const LAYER_BYTES: usize = std::mem::size_of::<Layer>();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_bytes_match_the_shader_layout() {
        let mut layer = Layer::new(Layer::RING, [1.0, 2.0, 3.0, 4.0], [100.0, 50.0]);
        layer.ring = 6.0;
        layer.blend = 3;
        layer.masks[3].color[3] = 1.0;
        let mut bytes = [0u8; LAYER_BYTES];
        layer_bytes(&layer, &mut bytes);
        let float = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let uint = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(uint(400), 3, "blend after the four masks");
        assert_eq!(float(400 - 4), 1.0, "last mask's invert slot");
        assert_eq!(
            float(116),
            1.0,
            "grade0 contrast follows pivot, rotation, orient"
        );
        assert_eq!([float(0), float(12)], [1.0, 4.0]);
        assert_eq!([float(64), float(68)], [100.0, 50.0]);
        assert_eq!(float(72), 1.0);
        assert_eq!(float(84), 6.0);
        assert_eq!(
            u32::from_le_bytes(bytes[88..92].try_into().unwrap()),
            Layer::RING
        );
    }
}
