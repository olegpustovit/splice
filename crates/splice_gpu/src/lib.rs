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
use ogpu_core::pipeline::{Blend, ColorFormat, DepthFormat, RenderDescriptor};
use ogpu_core::render_pass::{Load, Primitive, Rect, Store};
use ogpu_core::shader::{self as sh, Binding, BindingKind, EntryPoint, Stage};
use ogpu_core::types::{
    AddressMode, BufferDescriptor, BufferUsage, MipFilter, SamplerDescriptor, SamplerFilter,
    TextureDescriptor, TextureFormat, TextureUsage,
};

const MSL: &str = include_str!("../shaders/layer.metal");
/// Constant-buffer offsets are 256-byte aligned on every Metal GPU.
const STRIDE: usize = 256;
/// Layers in one frame. A take with its window, cursor, rings and camera uses about ten.
pub const MAX_LAYERS: usize = 64;
/// Parameter pages in flight; the caller keeps at most this many frames uncompleted.
const PAGES: usize = 3;

/// One drawn quad. Mirrors `Layer` in `shaders/layer.metal`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layer {
    /// x, y, width, height in target pixels, top-left origin.
    pub dest: [f32; 4],
    /// u, v, width, height normalized to the source texture.
    pub crop: [f32; 4],
    /// Fill or ring color, straight alpha.
    pub color0: [f32; 4],
    /// Gradient end color (top-left `color0` to bottom-right `color1`).
    pub color1: [f32; 4],
    /// Target size in pixels.
    pub target: [f32; 2],
    /// 0…1.
    pub opacity: f32,
    /// Corner radius in pixels.
    pub corner: f32,
    /// Drop shadow, 0…1.
    pub shadow: f32,
    /// Ring width in pixels, for `RING`.
    pub ring: f32,
    /// `TEXTURE`, `SOLID`, `GRADIENT` or `RING`.
    pub kind: u32,
    /// `BGRA` when the texture holds BGRA bytes (decoded video viewed as RGBA).
    pub flags: u32,
}

const _: () = assert!(std::mem::size_of::<Layer>() == 96 && STRIDE >= 96);

impl Layer {
    /// Samples the layer's texture through `crop`.
    pub const TEXTURE: u32 = 0;
    /// Fills with `color0`.
    pub const SOLID: u32 = 1;
    /// Fills with a diagonal gradient from `color0` to `color1`.
    pub const GRADIENT: u32 = 2;
    /// A ring inscribed in `dest`, `ring` pixels wide, in `color0`.
    pub const RING: u32 = 3;
    /// Flag: swap red and blue when sampling.
    pub const BGRA: u32 = 1;

    /// An opaque, square-cornered layer of `kind` filling `dest`.
    pub fn new(kind: u32, dest: [f32; 4], target: [f32; 2]) -> Self {
        Self {
            dest,
            crop: [0.0, 0.0, 1.0, 1.0],
            target,
            opacity: 1.0,
            kind,
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
            Some(Blend::Alpha),
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
        textures: &[Option<Texture>],
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
            let bytes = layer_bytes(layer);
            self.scratch[index * STRIDE..index * STRIDE + bytes.len()].copy_from_slice(&bytes);
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
                let texture = match (layer.kind, &textures[index]) {
                    (Layer::TEXTURE, Some(texture)) => texture,
                    _ => &self.blank,
                };
                b::set_stage_texture(&render, ShaderStage::Fragment, 1, texture)?;
                b::draw(&render, Primitive::TriangleList, 0, 6, None)?;
            }
            Ok(())
        })();
        let ended = b::end_render(&render);
        drawn.and(ended)
    }
}

fn layer_bytes(layer: &Layer) -> [u8; 96] {
    let mut out = [0u8; 96];
    let tail = [layer.opacity, layer.corner, layer.shadow, layer.ring];
    let floats = layer
        .dest
        .iter()
        .chain(&layer.crop)
        .chain(&layer.color0)
        .chain(&layer.color1)
        .chain(&layer.target)
        .chain(&tail);
    for (index, value) in floats.enumerate() {
        out[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    out[88..92].copy_from_slice(&layer.kind.to_le_bytes());
    out[92..96].copy_from_slice(&layer.flags.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_bytes_match_the_shader_layout() {
        let mut layer = Layer::new(Layer::RING, [1.0, 2.0, 3.0, 4.0], [100.0, 50.0]);
        layer.ring = 6.0;
        let bytes = layer_bytes(&layer);
        let float = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
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
