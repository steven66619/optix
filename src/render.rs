use std::collections::HashMap;
use std::path::Path;

use bytemuck::{Pod, Zeroable};
use cosmic_text::CacheKey;
use image::GenericImageView;
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

use crate::color::Rgba;
use crate::fonts::Fonts;

const SPRITE_SHADER: &str = r#"
struct Uniforms {
    screen: vec2<f32>,
    corner: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VSIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) is_color: f32,
};

struct VSOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) is_color: f32,
};

struct FSIn {
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) is_color: f32,
};

@vertex
fn vs_main(in: VSIn) -> VSOut {
    var out: VSOut;
    out.position = vec4<f32>(
        in.pos.x / uniforms.screen.x * 2.0 - 1.0,
        1.0 - in.pos.y / uniforms.screen.y * 2.0,
        0.0,
        1.0,
    );
    out.uv = in.uv;
    out.color = in.color;
    out.is_color = in.is_color;
    return out;
}

@fragment
fn fs_main(in: FSIn, @builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, in.uv);

    var rgb: vec3<f32> = in.color.rgb;
    var alpha: f32 = in.color.a;
    if (in.is_color > 0.5) {
        rgb = t.rgb;
        alpha = in.color.a * t.a;
    } else if (in.is_color < -0.5) {
        // mask glyph: texture holds alpha
        alpha = in.color.a * t.a;
    } else {
        // solid quad: texture is white
    }

    // Rounded-corner mask across the whole window.
    let r = uniforms.corner;
    let half = uniforms.screen * 0.5;
    let q = abs(frag.xy - half) - (half - vec2<f32>(r));
    let dist = length(max(q, vec2<f32>(0.0))) - r;
    let mask = 1.0 - smoothstep(0.0, 1.0, dist);

    return vec4<f32>(rgb, alpha * mask);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    is_color: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    screen: [f32; 2],
    corner: f32,
    _pad: f32,
}

/// A single colored rectangle.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: Rgba,
}

/// A glyph instance drawn from the atlas, tinted by `color`.
#[derive(Clone, Copy, Debug)]
pub struct GlyphInstance {
    pub x: f32,
    pub y: f32,
    pub cache_key: CacheKey,
    pub color: Rgba,
}

/// Full-window background image quad.
#[derive(Clone, Copy, Debug)]
pub struct ImageQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Source UV rectangle `[u0, v0, u1, v1]`.
    pub uv: [f32; 4],
}

/// Accumulated draw commands for one frame.
#[derive(Default)]
pub struct Frame {
    pub rects: Vec<Rect>,
    pub glyphs: Vec<GlyphInstance>,
    pub image: Option<ImageQuad>,
}

impl Frame {
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Rgba) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.rects.push(Rect { x, y, w, h, color });
    }
    pub fn glyph(&mut self, x: f32, y: f32, cache_key: CacheKey, color: Rgba) {
        self.glyphs.push(GlyphInstance { x, y, cache_key, color });
    }
    pub fn image_quad(&mut self, x: f32, y: f32, w: f32, h: f32, uv: [f32; 4]) {
        self.image = Some(ImageQuad { x, y, w, h, uv });
    }
}

#[derive(Clone)]
struct AtlasEntry {
    u: f32,
    v: f32,
    w: f32,
    h: f32,
    /// Swash placement offset of the glyph within the bitmap (relative to the pen origin).
    left: f32,
    top: f32,
    is_color: bool,
    pixels: Vec<u8>,
}

/// Simple bump-allocated glyph atlas texture.
struct GlyphAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: u32,
    next_x: u32,
    next_y: u32,
    row_h: u32,
    entries: HashMap<CacheKey, AtlasEntry>,
}

impl GlyphAtlas {
    fn new(device: &wgpu::Device, size: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph atlas"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            size,
            next_x: 1,
            next_y: 1,
            row_h: 0,
            entries: HashMap::new(),
        }
    }

    fn uv(&self, u: f32, v: f32, w: f32, h: f32) -> [f32; 4] {
        let s = self.size as f32;
        [u / s, v / s, (u + w) / s, (v + h) / s]
    }

    /// Insert or fetch an entry for a glyph image; returns UV rect + is_color.
    fn insert(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, key: CacheKey, img: &cosmic_text::SwashImage) -> Option<(bool, [f32; 4])> {
        if let Some(entry) = self.entries.get(&key) {
            return Some((entry.is_color, self.uv(entry.u, entry.v, entry.w, entry.h)));
        }

        let placement = &img.placement;
        let w = placement.width;
        let h = placement.height;
        if w == 0 || h == 0 {
            return None;
        }
        // Convert to 4 bytes per pixel (straight RGBA).
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        let is_color = match &img.content {
            cosmic_text::SwashContent::Mask => {
                for &a in &img.data {
                    pixels.extend_from_slice(&[a, a, a, a]);
                }
                false
            },
            cosmic_text::SwashContent::Color => {
                pixels.extend_from_slice(&img.data);
                true
            },
            _ => {
                return None;
            },
        };

        // Bump allocate a slot (with 1px padding).
        let pad = 1u32;
        let slot_w = w + pad * 2;
        let slot_h = h + pad * 2;
        if self.next_x + slot_w + 1 > self.size {
            self.next_x = 1;
            self.next_y += self.row_h + 1;
            self.row_h = 0;
        }
        if self.next_y + slot_h + 1 > self.size {
            self.grow(device, queue, self.size * 2);
        }
        if self.next_y + slot_h + 1 > self.size {
            return None;
        }

        let x = self.next_x;
        let y = self.next_y;
        self.next_x += slot_w;
        self.row_h = self.row_h.max(slot_h);

        // Upload into the padded slot.
        let bytes_per_row = slot_w * 4;
        let mut padded = vec![0u8; (slot_w * slot_h * 4) as usize];
        for row in 0..h {
            let src = &pixels[(row * w * 4) as usize..((row + 1) * w * 4) as usize];
            let dst_start = ((row + pad) * slot_w * 4) as usize + (pad * 4) as usize;
            padded[dst_start..dst_start + src.len()].copy_from_slice(src);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
            &padded,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes_per_row), rows_per_image: Some(slot_h) },
            wgpu::Extent3d { width: slot_w, height: slot_h, depth_or_array_layers: 1 },
        );

        self.entries.insert(key, AtlasEntry { u: x as f32, v: y as f32, w: slot_w as f32, h: slot_h as f32, left: placement.left as f32, top: placement.top as f32, is_color, pixels: padded });
        Some((is_color, self.uv(x as f32, y as f32, slot_w as f32, slot_h as f32)))
    }

    fn grow(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, new_size: u32) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph atlas (grown)"),
            size: wgpu::Extent3d { width: new_size, height: new_size, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.texture = texture;
        self.view = self.texture.create_view(&Default::default());
        self.size = new_size;
        // Re-upload all existing entries at their current positions.
        for entry in self.entries.values() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: 0, origin: wgpu::Origin3d { x: entry.u as u32, y: entry.v as u32, z: 0 }, aspect: wgpu::TextureAspect::All },
                &entry.pixels,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some((entry.w as u32) * 4), rows_per_image: Some(entry.h as u32) },
                wgpu::Extent3d { width: entry.w as u32, height: entry.h as u32, depth_or_array_layers: 1 },
            );
        }
    }
}

/// GPU renderer: surface, pipelines, glyph atlas, and a single-batch frame builder.
pub struct Renderer {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pub size: PhysicalSize<u32>,
    pub dpi_scale: f32,

    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group_layout: wgpu::BindGroupLayout,
    white_bg: wgpu::BindGroup,
    atlas_bg: wgpu::BindGroup,

    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    swash: cosmic_text::SwashCache,
    atlas: GlyphAtlas,

    image_bg: Option<wgpu::BindGroup>,
    image_uv: [f32; 4],
    image_tex_size: (u32, u32),
    corner_radius: f32,
    pub transparent: bool,
}

const MAX_VERTS: usize = 1 << 17;

impl Renderer {
    pub fn new(
        window: &Window,
        size: PhysicalSize<u32>,
        dpi_scale: f32,
        corner_radius: f32,
    ) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        // SAFETY: the window is owned by the application alongside this renderer,
        // so the raw handles remain valid for the surface's entire lifetime.
        let surface = unsafe {
            let window_handle = window
                .window_handle()
                .map_err(|e| format!("failed to get window handle: {e}"))?;
            let display_handle = window
                .display_handle()
                .map_err(|e| format!("failed to get display handle: {e}"))?;
            instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(display_handle.as_raw()),
                    raw_window_handle: window_handle.as_raw(),
                })
        }
        .map_err(|e| format!("failed to create surface: {e}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("no compatible GPU adapter found: {e}"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("oterminal device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            },
        ))
        .map_err(|e| format!("failed to create device: {e}"))?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PostMultiplied) {
            wgpu::CompositeAlphaMode::PostMultiplied
        } else {
            wgpu::CompositeAlphaMode::Auto
        };
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::AutoVsync) {
            wgpu::PresentMode::AutoVsync
        } else {
            caps.present_modes[0]
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            desired_maximum_frame_latency: 2,
            present_mode,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let transparent = alpha_mode != wgpu::CompositeAlphaMode::Auto
            && alpha_mode != wgpu::CompositeAlphaMode::Opaque;

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sprite uniforms"),
            contents: bytemuck::bytes_of(&Uniforms { screen: [size.width.max(1) as f32, size.height.max(1) as f32], corner: corner_radius, _pad: 0.0 }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // 1x1 white texture for solid quads.
        let white_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("white"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &white_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let white_view = white_texture.create_view(&Default::default());
        let white_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("white bg"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&white_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        let atlas = GlyphAtlas::new(&device, 2048);
        let atlas_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas bg"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&atlas.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite shader"),
            source: wgpu::ShaderSource::Wgsl(SPRITE_SHADER.into()),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4,
                        3 => Float32,
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
                        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vertices"),
            size: (MAX_VERTS * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("indices"),
            size: (MAX_VERTS * std::mem::size_of::<u32>() * 6) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            dpi_scale,
            pipeline,
            uniform_buffer,
            bind_group_layout,
            white_bg,
            atlas_bg,
            vertex_buffer,
            index_buffer,
            swash: cosmic_text::SwashCache::new(),
            atlas,
            image_bg: None,
            image_uv: [0.0, 0.0, 1.0, 1.0],
            image_tex_size: (0, 0),
            corner_radius,
            transparent,
        })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>, dpi_scale: f32) {
        self.size = size;
        self.dpi_scale = dpi_scale;
        self.config.width = size.width.max(1);
        self.config.height = size.height.max(1);
        self.surface.configure(&self.device, &self.config);
        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&Uniforms { screen: [self.config.width as f32, self.config.height as f32], corner: self.corner_radius, _pad: 0.0 }),
        );
    }

    pub fn set_corner_radius(&mut self, radius: f32) {
        self.corner_radius = radius;
    }

    /// Whether a background image is loaded.
    pub fn has_background_image(&self) -> bool {
        self.image_bg.is_some()
    }

    /// Cover UV rect of the loaded background image.
    pub fn background_uv(&self) -> [f32; 4] {
        self.image_uv
    }

    /// Load a background image (PNG) into a texture, covering the window.
    pub fn load_background_image(&mut self, path: &Path) -> Result<(), String> {
        let img = image::open(path).map_err(|e| format!("failed to load image {path:?}: {e}"))?;
        let (iw, ih) = img.dimensions();
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("background image"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );

        let view = texture.create_view(&Default::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bg image sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let image_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bg image bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        // Compute a "cover" UV rect so the image fills the window without distortion.
        let (sw, sh) = (self.config.width as f32, self.config.height as f32);
        let scale = (sw / iw as f32).max(sh / ih as f32);
        let cw = sw / scale;
        let ch = sh / scale;
        let u0 = ((iw as f32 - cw) * 0.5 / iw as f32).clamp(0.0, 1.0);
        let u1 = 1.0 - u0;
        let v0 = ((ih as f32 - ch) * 0.5 / ih as f32).clamp(0.0, 1.0);
        let v1 = 1.0 - v0;

        self.image_bg = Some(image_bg);
        self.image_uv = [u0, v0, u1, v1];
        self.image_tex_size = (w, h);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn push_quad(verts: &mut Vec<Vertex>, indices: &mut Vec<u32>, x: f32, y: f32, w: f32, h: f32, uv: [f32; 4], color: Rgba, is_color: f32) {
        let base = verts.len() as u32;
        let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);
        let color = [color.r, color.g, color.b, color.a];
        let mk = |x: f32, y: f32, u: f32, v: f32| Vertex { pos: [x, y], uv: [u, v], color, is_color };
        verts.push(mk(x, y, u0, v0));
        verts.push(mk(x + w, y, u1, v0));
        verts.push(mk(x + w, y + h, u1, v1));
        verts.push(mk(x, y + h, u0, v1));
        for v in [base, base + 1, base + 2, base, base + 2, base + 3] {
            indices.push(v);
        }
    }

    /// Render one frame from the accumulated draw commands.
    pub fn render(&mut self, frame: &Frame, fonts: &mut Fonts) -> Result<(), String> {
        // Rasterize any glyphs that are new to the atlas.
        for g in &frame.glyphs {
            if !self.atlas.entries.contains_key(&g.cache_key) {
                if let Some(img) = self
                    .swash
                    .get_image(&mut fonts.font_system, g.cache_key)
                    .as_ref()
                {
                    self.atlas.insert(&self.device, &self.queue, g.cache_key, img);
                } else {
                    log::warn!("glyph {:?}: no swash image", g.cache_key);
                }
            }
        }

        let mut verts: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();

        // Segment 0: background image (image bind group), full window.
        let mut seg_image_end = 0usize;
        if self.image_bg.is_some() {
            if let Some(img) = frame.image {
                let uv = if self.image_tex_size.0 > 0 { self.image_uv } else { img.uv };
                Self::push_quad(&mut verts, &mut indices, img.x, img.y, img.w, img.h, uv, Rgba::rgb(1.0, 1.0, 1.0), 0.0);
                seg_image_end = indices.len();
            }
        }

        // Segment 1: solid rects (white bind group).
        for r in &frame.rects {
            Self::push_quad(&mut verts, &mut indices, r.x, r.y, r.w, r.h, [0.0, 0.0, 1.0, 1.0], r.color, 0.0);
        }
        let seg_white_end = indices.len();

        // Segment 2: glyphs (atlas bind group).
        for g in &frame.glyphs {
            if let Some(entry) = self.atlas.entries.get(&g.cache_key) {
                // Skip the 1px transparent padding around the glyph in the atlas.
                let u0 = (entry.u + 1.0) / self.atlas.size as f32;
                let v0 = (entry.v + 1.0) / self.atlas.size as f32;
                let u1 = (entry.u + entry.w - 1.0) / self.atlas.size as f32;
                let v1 = (entry.v + entry.h - 1.0) / self.atlas.size as f32;
                let is_color = if entry.is_color { 1.0 } else { -1.0 };
                let x = g.x + entry.left;
                let y = g.y - entry.top;
                Self::push_quad(&mut verts, &mut indices, x, y, entry.w - 2.0, entry.h - 2.0, [u0, v0, u1, v1], g.color, is_color);
            }
        }
        let seg_atlas_end = indices.len();

        if verts.is_empty() {
            return Ok(());
        }

        self.queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&verts));
        self.queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&indices));

        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Validation => return Ok(()),
        };
        let view = texture.texture.create_view(&Default::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

            if seg_image_end > 0 {
                pass.set_bind_group(0, self.image_bg.as_ref().unwrap(), &[]);
                pass.draw_indexed(0..seg_image_end as u32, 0, 0..1);
            }
            if seg_white_end > seg_image_end {
                pass.set_bind_group(0, &self.white_bg, &[]);
                pass.draw_indexed(seg_image_end as u32..seg_white_end as u32, 0, 0..1);
            }
            if seg_atlas_end > seg_white_end {
                pass.set_bind_group(0, &self.atlas_bg, &[]);
                pass.draw_indexed(seg_white_end as u32..seg_atlas_end as u32, 0, 0..1);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(texture);
        Ok(())
    }
}
