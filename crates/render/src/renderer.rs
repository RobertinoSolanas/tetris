//! The GPU side of the presentation context: [`Scene`] in, pixels out.
//!
//! Frame graph, once per frame:
//!
//! ```text
//!   Scene ──(instanced quads)──► scene texture (full res)
//!                                   │
//!                                   ├──(bright pass, ½ res)──► bloom A
//!                                   │                            │
//!                                   │                    blur H/V, twice
//!                                   │                            │
//!                                   └──────────► composite ◄─────┘
//!                                                  │
//!                                 CRT distortion · scanlines · vignette · flash
//!                                                  ▼
//!                                 swapchain image or snapshot texture
//! ```
//!
//! A renderer is bound to one output format: either a window's swapchain ([`Renderer::for_window`])
//! or an offscreen `Rgba8UnormSrgb` texture ([`Renderer::render_to_image`], used by `--snapshot` and
//! by the headless GPU tests). It deliberately does not depend on winit: the app hands over anything
//! that can become a [`wgpu::SurfaceTarget`'].

use bytemuck::{Pod, Zeroable};

use crate::font::Font;
use crate::scene::{Layer, Quad, QuadKind, Scene};

const QUAD_SHADER: &str = include_str!("../shaders/quad.wgsl");
const POST_SHADER: &str = include_str!("../shaders/post.wgsl");

/// Format of the offscreen snapshot target.
pub const SNAPSHOT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Upper bound on instances per frame (playfield + HUD + a couple of thousand particles fits twice).
pub const MAX_INSTANCES: usize = 16384;

/// Anything that stops the renderer from starting up.
#[derive(Debug)]
pub struct RenderError {
    pub message: String,
}

impl RenderError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "render error: {}", self.message)
    }
}

impl std::error::Error for RenderError {}

/// GPU mirror of `struct Instance` in `shaders/quad.wgsl` (64 bytes, 16-byte aligned).
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct Instance {
    pos: [f32; 2],
    size: [f32; 2],
    color: [f32; 4],
    uv_min: [f32; 2],
    uv_max: [f32; 2],
    /// `x`: corner radius, `y`: glow, `z`: kind, `w`: pattern.
    params: [f32; 4],
}

impl Instance {
    fn from_quad(q: &Quad, offset: [f32; 2]) -> Self {
        Self {
            pos: [q.x + offset[0], q.y + offset[1]],
            size: [q.w.max(0.0), q.h.max(0.0)],
            color: [
                q.color[0].max(0.0),
                q.color[1].max(0.0),
                q.color[2].max(0.0),
                q.color[3].clamp(0.0, 1.0),
            ],
            uv_min: [q.uv[0], q.uv[1]],
            uv_max: [q.uv[2], q.uv[3]],
            params: [q.radius.max(0.0), q.glow, kind_to_f32(q.kind), q.pattern],
        }
    }

    /// Top-left position in pixels (test accessor).
    pub fn pos(&self) -> [f32; 2] {
        self.pos
    }

    /// RGBA as sent to the GPU (test accessor).
    pub fn color(&self) -> [f32; 4] {
        self.color
    }

    /// Kind tag as sent to the GPU (test accessor).
    pub fn kind(&self) -> f32 {
        self.params[2]
    }

    /// Glow as sent to the GPU (test accessor).
    pub fn glow(&self) -> f32 {
        self.params[1]
    }
}

fn kind_to_f32(kind: QuadKind) -> f32 {
    match kind {
        QuadKind::Solid => 0.0,
        QuadKind::Rounded => 1.0,
        QuadKind::Block => 2.0,
        QuadKind::Outline => 3.0,
        QuadKind::Gradient => 4.0,
        QuadKind::Ring => 5.0,
        QuadKind::Glyph => 6.0,
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Uniforms {
    resolution: [f32; 2],
    time: f32,
    _pad: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct PostUniforms {
    /// 1 / source size in pixels.
    texel: [f32; 2],
    /// Source size in pixels.
    source_size: [f32; 2],
    /// threshold, knee, strength, time
    params0: [f32; 4],
    /// direction.x, direction.y, flash, unused
    params1: [f32; 4],
    /// scanlines, curvature, vignette, aberration
    fx: [f32; 4],
}

/// Compile-time guard: the WGSL structs above must stay 64/… bytes.
const _: () = assert!(std::mem::size_of::<Instance>() == 64);
const _: () = assert!(std::mem::size_of::<Uniforms>() == 16);
const _: () = assert!(std::mem::size_of::<PostUniforms>().is_multiple_of(16));

/// Uniform slots in the post buffer. Each pass gets its own slot: a single buffer rewritten
/// several times *before* `submit` would leave every pass reading the last value written, which is
/// exactly how a blur pass silently turns into a copy.
mod slot {
    pub const BRIGHT: u32 = 0;
    pub const BLUR_1H: u32 = 1;
    pub const BLUR_1V: u32 = 2;
    pub const BLUR_2H: u32 = 3;
    pub const BLUR_2V: u32 = 4;
    pub const COMPOSITE: u32 = 5;
    pub const COUNT: u64 = 6;
}
/// Stride between slots: `min_uniform_buffer_offset_alignment` is 256 on every backend we target.
const POST_SLOT_STRIDE: u64 = 256;

#[derive(Clone)]
struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl Target {
    fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
        label: &str,
    ) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width,
            height,
        }
    }
}

/// Offscreen targets plus the bind groups that point at them (rebuilt on resize).
struct FrameTargets {
    scene: Target,
    bloom_a: Target,
    bloom_b: Target,
    bright_bind: wgpu::BindGroup,
    blur_from_a: wgpu::BindGroup,
    blur_from_b: wgpu::BindGroup,
    composite_bind: wgpu::BindGroup,
}

enum Output {
    Surface {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    Offscreen(Target),
}

/// Turn a scene into the instance list the GPU draws.
///
/// A free function on purpose: no device, no window, so the "did the shake reach the right layers"
/// class of bugs is a unit test rather than a screenshot review.
pub fn build_instances(scene: &Scene, font: &Font) -> Vec<Instance> {
    let counts = scene.counts();
    let mut out = Vec::with_capacity(counts.quads + counts.particles + 256);

    // Shake moves the world but never the HUD or the menus on top of it.
    let shake = if scene.shake.abs() < 0.01 {
        [0.0f32; 2]
    } else {
        [(scene.shake * 0.6).round(), (scene.shake * 0.35).round()]
    };

    for layer in Layer::ALL {
        let offset = match layer {
            Layer::Background | Layer::Playfield | Layer::Pieces | Layer::Effects => shake,
            Layer::Hud | Layer::Overlay => [0.0, 0.0],
        };
        for quad in scene.layer(layer) {
            out.push(Instance::from_quad(quad, offset));
        }
    }

    for text in &scene.text {
        for quad in font.layout(
            &text.content,
            text.x,
            text.y,
            text.scale,
            text.tracking,
            text.align,
            text.color,
            text.glow,
        ) {
            out.push(Instance::from_quad(&quad, [0.0, 0.0]));
        }
    }

    for p in &scene.particles {
        let quad = Quad::rounded(
            p.x - p.size * 0.5,
            p.y - p.size * 0.5,
            p.size,
            p.size,
            p.size * 0.4,
            p.color,
        )
        .with_glow(p.glow);
        out.push(Instance::from_quad(&quad, shake));
    }

    out.truncate(MAX_INSTANCES);
    out
}

/// The renderer.
pub struct Renderer {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,

    scene_format: wgpu::TextureFormat,
    output_format: wgpu::TextureFormat,

    font: Font,
    _font_texture: wgpu::Texture,
    font_view: wgpu::TextureView,
    sampler: wgpu::Sampler,

    quad_bind_layout: wgpu::BindGroupLayout,
    post_layout_simple: wgpu::BindGroupLayout,
    post_layout_composite: wgpu::BindGroupLayout,

    quad_pipeline: wgpu::RenderPipeline,
    bright_pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,

    uniform: wgpu::Buffer,
    post_uniform: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    quad_bind: wgpu::BindGroup,

    /// `None` only during construction: both public constructors set it immediately.
    output: Option<Output>,
    /// Largest texture the adapter will make; offscreen targets are clamped to this.
    max_dimension: u32,
    targets: FrameTargets,
}

/// Pick a scene/bloom format: 16-bit float when the adapter will filter *and* attach it, else 8-bit.
fn pick_scene_format(adapter: &wgpu::Adapter) -> wgpu::TextureFormat {
    for format in [
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        let features = adapter.get_texture_format_features(format);
        let ok = features
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
            && features
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE);
        if ok {
            return format;
        }
    }
    wgpu::TextureFormat::Bgra8UnormSrgb
}

fn texture_binding() -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

fn uniform_binding() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn dynamic_uniform_binding() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: true,
        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<PostUniforms>() as u64),
    }
}

impl Renderer {
    /// Render into a window. `window` is anything wgpu can make a surface from — a winit
    /// `Arc<Window>` does the job (see `tetris-app`).
    pub fn for_window(
        window: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window)
            .map_err(|e| RenderError::new(format!("create_surface: {e}")))?;
        let adapter = request_adapter(&instance, Some(&surface))?;
        let caps = surface.get_capabilities(&adapter);
        let output_format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let mut renderer = Self::with_adapter(adapter, output_format, width, height)?;
        let mut config = surface
            .get_default_config(&renderer.adapter, width.max(1), height.max(1))
            .ok_or_else(|| RenderError::new("surface has no default configuration"))?;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&renderer.device, &config);
        renderer.targets = renderer.make_targets(config.width, config.height);
        renderer.output = Some(Output::Surface { surface, config });
        Ok(renderer)
    }

    /// Headless renderer with an offscreen `SNAPSHOT_FORMAT` target.
    pub fn headless(width: u32, height: u32) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = request_adapter(&instance, None)?;
        let mut renderer = Self::with_adapter(adapter, SNAPSHOT_FORMAT, width, height)?;
        renderer.output = Some(Output::Offscreen(renderer.new_output_target(width, height)));
        Ok(renderer)
    }

    fn new_output_target(&self, width: u32, height: u32) -> Target {
        Target::new(
            &self.device,
            width,
            height,
            SNAPSHOT_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            "snapshot-output",
        )
    }

    fn with_adapter(
        adapter: wgpu::Adapter,
        output_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let backend = adapter.get_info().backend;
        // Ask the adapter what it can actually do: requesting downlevel limits here would cap the
        // offscreen targets to 2048 px and silently letterbox a 4K window.
        let limits = adapter.limits();
        let max_dimension = limits.max_texture_dimension_2d;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("tetris-device"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            ..Default::default()
        }))
        .map_err(|e| RenderError::new(format!("request_device on {backend:?}: {e}")))?;

        let scene_format = pick_scene_format(&adapter);
        let font = Font::build();

        let font_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("font-atlas"),
            size: wgpu::Extent3d {
                width: font.width(),
                height: font.height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Unorm (not sRGB): the atlas is a mask, and we want the alpha read verbatim.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            font_texture.as_image_copy(),
            font.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(font.width() * 4),
                rows_per_image: Some(font.height()),
            },
            wgpu::Extent3d {
                width: font.width(),
                height: font.height(),
                depth_or_array_layers: 1,
            },
        );
        let font_view = font_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let quad_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("quad-bind-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: uniform_binding(),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: texture_binding(),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let post_layout_simple =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("post-bind-simple"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: dynamic_uniform_binding(),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: texture_binding(),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let post_layout_composite =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("post-bind-composite"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: dynamic_uniform_binding(),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: texture_binding(),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: texture_binding(),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let quad_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad-layout"),
            bind_group_layouts: &[Some(&quad_bind_layout)],
            immediate_size: 0,
        });
        let post_simple_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post-layout-simple"),
            bind_group_layouts: &[Some(&post_layout_simple)],
            immediate_size: 0,
        });
        let post_composite_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("post-layout-composite"),
                bind_group_layouts: &[Some(&post_layout_composite)],
                immediate_size: 0,
            });

        let quad_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad.wgsl"),
            source: wgpu::ShaderSource::Wgsl(QUAD_SHADER.into()),
        });
        let post_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post.wgsl"),
            source: wgpu::ShaderSource::Wgsl(POST_SHADER.into()),
        });

        let quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quad-pipeline"),
            layout: Some(&quad_layout),
            multiview_mask: Default::default(),
            vertex: wgpu::VertexState {
                module: &quad_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                // Quads are wound differently once the y axis is flipped; culling buys nothing in 2D.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &quad_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: scene_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            cache: None,
        });

        let post_pipeline = |label: &str,
                             entry_point: &'static str,
                             layout: &wgpu::PipelineLayout,
                             format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                multiview_mask: Default::default(),
                vertex: wgpu::VertexState {
                    module: &post_module,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &post_module,
                    entry_point: Some(entry_point),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                cache: None,
            })
        };

        let bright_pipeline = post_pipeline(
            "bright-pipeline",
            "fs_bright",
            &post_simple_layout,
            scene_format,
        );
        let blur_pipeline = post_pipeline(
            "blur-pipeline",
            "fs_blur",
            &post_simple_layout,
            scene_format,
        );
        let composite_pipeline = post_pipeline(
            "composite-pipeline",
            "fs_composite",
            &post_composite_layout,
            output_format,
        );

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let post_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post-uniforms"),
            size: POST_SLOT_STRIDE * slot::COUNT,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let instance_capacity = 2048;
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (instance_capacity * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let quad_bind = make_quad_bind(
            &device,
            &quad_bind_layout,
            &uniform,
            &instance_buffer,
            &font_view,
            &sampler,
        );

        let targets = build_targets(
            &device,
            &post_layout_simple,
            &post_layout_composite,
            &post_uniform,
            &sampler,
            scene_format,
            width,
            height,
        );

        Ok(Self {
            adapter,
            device,
            queue,
            scene_format,
            output_format,
            font,
            _font_texture: font_texture,
            font_view,
            sampler,
            quad_bind_layout,
            post_layout_simple,
            post_layout_composite,
            quad_pipeline,
            bright_pipeline,
            blur_pipeline,
            composite_pipeline,
            uniform,
            post_uniform,
            instance_buffer,
            instance_capacity,
            quad_bind,
            output: None,
            max_dimension: max_dimension.max(256),
            targets,
        })
    }

    fn make_targets(&self, width: u32, height: u32) -> FrameTargets {
        // A window bigger than the adapter's texture limit still renders: the scene target is
        // clamped and the composite upsamples it. Slightly softer, never a validation error.
        let clamp = |v: u32| v.clamp(1, self.max_dimension);
        build_targets(
            &self.device,
            &self.post_layout_simple,
            &self.post_layout_composite,
            &self.post_uniform,
            &self.sampler,
            self.scene_format,
            clamp(width),
            clamp(height),
        )
    }

    /// Largest offscreen texture this adapter will create.
    pub fn max_texture_dimension(&self) -> u32 {
        self.max_dimension
    }

    /// Name of the active backend, for the title bar and the debug overlay.
    pub fn backend(&self) -> String {
        format!("{:?}", self.adapter.get_info().backend)
    }

    /// Details of the adapter, for logging at startup.
    pub fn adapter_info(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?})", info.name, info.backend)
    }

    pub fn font(&self) -> &Font {
        &self.font
    }

    pub fn scene_format(&self) -> wgpu::TextureFormat {
        self.scene_format
    }

    pub fn output_format(&self) -> wgpu::TextureFormat {
        self.output_format
    }

    /// Current output size (swapchain or offscreen target).
    pub fn size(&self) -> (u32, u32) {
        match &self.output {
            Some(Output::Surface { config, .. }) => (config.width, config.height),
            Some(Output::Offscreen(t)) => (t.width, t.height),
            None => (self.targets.scene.width, self.targets.scene.height),
        }
    }

    /// Call after the window is resized.
    pub fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        match &mut self.output {
            Some(Output::Surface { surface, config }) => {
                config.width = width;
                config.height = height;
                surface.configure(&self.device, config);
            }
            Some(Output::Offscreen(_)) | None => {
                self.output = Some(Output::Offscreen(self.new_output_target(width, height)));
            }
        }
        self.targets = self.make_targets(width, height);
    }

    /// The swapchain surface, when this renderer owns one.
    pub fn surface(&self) -> Option<&wgpu::Surface<'static>> {
        match &self.output {
            Some(Output::Surface { surface, .. }) => Some(surface),
            _ => None,
        }
    }

    /// Present a texture obtained from [`Renderer::surface`].
    pub fn present(&self, texture: wgpu::SurfaceTexture) {
        self.queue.present(texture);
    }

    /// Draw one frame to the window. `view` is the current swapchain image's view.
    pub fn render(&mut self, scene: &Scene, time: f32, view: &wgpu::TextureView) {
        self.encode(scene, time, view);
    }

    /// Draw offscreen and read the result back as RGBA8, top row first.
    pub fn render_to_image(&mut self, scene: &Scene, time: f32) -> (u32, u32, Vec<u8>) {
        let view = match &self.output {
            Some(Output::Offscreen(t)) => t.view.clone(),
            _ => panic!("render_to_image needs a headless renderer (Renderer::headless)"),
        };
        self.encode(scene, time, &view);

        let (width, height) = self.size();
        let bpp = 4usize;
        let unpadded = width as usize * bpp;
        let bytes_per_row = (unpadded as u64).next_multiple_of(256) as usize;
        let total = bytes_per_row * height as usize;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: total as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let source = match &self.output {
            Some(Output::Offscreen(t)) => t.texture.clone(),
            _ => panic!("render_to_image needs a headless renderer (Renderer::headless)"),
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback"),
            });
        encoder.copy_texture_to_buffer(
            source.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row as u32),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = readback.slice(0..total as u64);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll while waiting for readback");
        receiver
            .recv()
            .expect("map callback never fired")
            .expect("readback buffer failed to map");

        let data = slice.get_mapped_range().expect("mapped readback range");
        let mut out = vec![0u8; unpadded * height as usize];
        for row in 0..height as usize {
            let src = &data[row * bytes_per_row..row * bytes_per_row + unpadded];
            out[row * unpadded..(row + 1) * unpadded].copy_from_slice(src);
        }
        drop(data);
        readback.unmap();
        (width, height, out)
    }

    fn encode(&mut self, scene: &Scene, time: f32, target_view: &wgpu::TextureView) {
        let instances = build_instances(scene, &self.font);
        let count = instances.len();
        self.ensure_instance_capacity(count);
        if count > 0 {
            self.queue
                .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        }

        let (width, height) = (self.targets.scene.width, self.targets.scene.height);
        self.queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::cast_slice(&[Uniforms {
                resolution: [width as f32, height as f32],
                time,
                _pad: 0.0,
            }]),
        );

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        // 1. Scene pass: clear to the scene's background, then draw every quad.
        {
            let clear = scene.clear_color;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.scene.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0] as f64,
                            g: clear[1] as f64,
                            b: clear[2] as f64,
                            a: clear[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: Default::default(),
            });
            pass.set_pipeline(&self.quad_pipeline);
            pass.set_bind_group(0, &self.quad_bind, &[]);
            if count > 0 {
                pass.draw(0..4, 0..count as u32);
            }
        }

        // 2. Post uniforms: one block per pass, each in its own 256-byte slot.
        let bloom = scene.post.bloom;
        let (aw, ah) = (self.targets.bloom_a.width, self.targets.bloom_a.height);
        let blocks = [
            // bright: full-res scene → half-res bloom A
            Self::post_block(scene, time, [0.0, 0.0], bloom, (width, height), scene.flash),
            // blur round 1 (narrow), then round 2 (wide, so the glow carries)
            Self::post_block(scene, time, [2.0, 0.0], 1.0, (aw, ah), 0.0),
            Self::post_block(scene, time, [0.0, 2.0], 1.0, (aw, ah), 0.0),
            Self::post_block(scene, time, [3.0, 0.0], 1.0, (aw, ah), 0.0),
            Self::post_block(scene, time, [0.0, 3.0], 1.0, (aw, ah), 0.0),
            // composite: samples the full-res scene, so texel is the scene's
            Self::post_block(scene, time, [0.0, 0.0], bloom, (width, height), scene.flash),
        ];
        for (i, block) in blocks.iter().enumerate() {
            self.queue.write_buffer(
                &self.post_uniform,
                i as u64 * POST_SLOT_STRIDE,
                bytemuck::bytes_of(block),
            );
        }

        fullscreen_pass(
            &mut encoder,
            &self.bright_pipeline,
            &self.targets.bright_bind,
            &self.targets.bloom_a.view,
            slot::BRIGHT * POST_SLOT_STRIDE as u32,
        );

        for (horizontal, round) in [(true, 0), (false, 0), (true, 1), (false, 1)] {
            let (bind, dst) = if horizontal {
                (&self.targets.blur_from_a, &self.targets.bloom_b)
            } else {
                (&self.targets.blur_from_b, &self.targets.bloom_a)
            };
            let offset = if round == 0 {
                if horizontal {
                    slot::BLUR_1H
                } else {
                    slot::BLUR_1V
                }
            } else if horizontal {
                slot::BLUR_2H
            } else {
                slot::BLUR_2V
            };
            fullscreen_pass(
                &mut encoder,
                &self.blur_pipeline,
                bind,
                &dst.view,
                offset * POST_SLOT_STRIDE as u32,
            );
        }

        // 4. Composite: scene + bloom + CRT-isms → output.
        fullscreen_pass(
            &mut encoder,
            &self.composite_pipeline,
            &self.targets.composite_bind,
            target_view,
            slot::COMPOSITE * POST_SLOT_STRIDE as u32,
        );

        self.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Write the post-processing uniform block for one pass.
    ///
    /// `source` is the texture the pass samples from (its size defines `texel`), which for the blur
    /// passes is the half-resolution bloom target, not the scene.
    /// One `PostUniforms` block for one pass; see [`slot`].
    fn post_block(
        scene: &Scene,
        time: f32,
        direction: [f32; 2],
        strength: f32,
        source: (u32, u32),
        flash: f32,
    ) -> PostUniforms {
        let (w, h) = (source.0.max(1), source.1.max(1));
        PostUniforms {
            texel: [1.0 / w as f32, 1.0 / h as f32],
            source_size: [w as f32, h as f32],
            params0: [scene.post.bloom_threshold, 0.15, strength, time],
            params1: [direction[0], direction[1], flash, 0.0],
            fx: [
                scene.post.scanlines,
                scene.post.curvature,
                scene.post.vignette,
                scene.post.aberration,
            ],
        }
    }

    fn ensure_instance_capacity(&mut self, needed: usize) {
        if needed <= self.instance_capacity {
            return;
        }
        let capacity = needed.clamp(1, MAX_INSTANCES).next_power_of_two();
        self.instance_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (capacity * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_capacity = capacity;
        self.quad_bind = make_quad_bind(
            &self.device,
            &self.quad_bind_layout,
            &self.uniform,
            &self.instance_buffer,
            &self.font_view,
            &self.sampler,
        );
    }
}

/// wgpu needs an adapter that can present to the surface (or any adapter, headless).
fn request_adapter(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'static>>,
) -> Result<wgpu::Adapter, RenderError> {
    let options = wgpu::RequestAdapterOptions {
        compatible_surface: surface,
        ..Default::default()
    };
    pollster::block_on(instance.request_adapter(&options)).map_err(|e| {
        RenderError::new(format!(
            "no compatible GPU adapter ({e}); pi-tetris needs a Vulkan driver - see the README \
             section on running without a GPU"
        ))
    })
}

fn make_quad_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    instances: &wgpu::Buffer,
    font_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("quad-bind"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: instances.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(font_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// Shared construction of the offscreen targets and their bind groups.
#[allow(clippy::too_many_arguments)]
fn build_targets(
    device: &wgpu::Device,
    simple: &wgpu::BindGroupLayout,
    composite: &wgpu::BindGroupLayout,
    post_uniform: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> FrameTargets {
    let attach = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
    let half = |v: u32| (v / 2).max(1);
    let scene = Target::new(device, width, height, format, attach, "scene");
    let bloom_a = Target::new(device, half(width), half(height), format, attach, "bloom-a");
    let bloom_b = Target::new(device, half(width), half(height), format, attach, "bloom-b");

    let simple_bind = |label: &str, source: &Target| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: simple,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    // Bind exactly one slot so the per-pass dynamic offset can walk the buffer.
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: post_uniform,
                        offset: 0,
                        size: wgpu::BufferSize::new(std::mem::size_of::<PostUniforms>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&source.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    };

    let bright_bind = simple_bind("bright-bind", &scene);
    let blur_from_a = simple_bind("blur-from-a", &bloom_a);
    let blur_from_b = simple_bind("blur-from-b", &bloom_b);
    let composite_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("composite-bind"),
        layout: composite,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: post_uniform,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<PostUniforms>() as u64),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&scene.view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&bloom_a.view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });

    FrameTargets {
        scene,
        bloom_a,
        bloom_b,
        bright_bind,
        blur_from_a,
        blur_from_b,
        composite_bind,
    }
}

/// Draw one full-screen triangle into `view`.
fn fullscreen_pass(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
    view: &wgpu::TextureView,
    uniform_offset: u32,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("fullscreen"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: Default::default(),
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[uniform_offset]);
    pass.draw(0..3, 0..1);
}
