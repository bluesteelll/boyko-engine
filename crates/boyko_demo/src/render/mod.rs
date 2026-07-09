//! wgpu rendering behind egui's `egui_wgpu::CallbackTrait` (plan §5 / D6 / D8).
//!
//! The render path follows egui's `custom3d_wgpu` example: GPU resources
//! (pipeline + buffers + bind group) live in egui's per-renderer type-map
//! (`callback_resources`), created once at app start. Each frame a lightweight
//! [`RenderCallback`] is registered into the egui paint stream; its `prepare`
//! uploads per-frame data (camera + instances) and its `paint` records the single
//! instanced draw call.
//!
//! ## Lifetime shape (D6 / D8 / plan H4)
//! egui paint callbacks are `'static`: the callback may not borrow app or ECS
//! state. All cross-data travels through the GPU buffers via `prepare`.
//! [`RenderCallback`] therefore carries only owned/`Copy` values (a viewport size
//! and an owned instance snapshot), never a borrow into [`crate::app::DemoApp`].
//! Wave 3 (ECS wiring) must respect this constraint — see the [`RenderCallback`]
//! docs for the concrete handoff.

pub mod camera;
pub mod instance;

use std::sync::Arc;

use eframe::egui::PaintCallbackInfo;
use eframe::egui_wgpu::wgpu;
use eframe::egui_wgpu::wgpu::util::DeviceExt;
use eframe::egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use camera::{CAMERA_UNIFORM_SIZE, CameraUniform};
use instance::GPU_INSTANCE_SIZE;

/// Maximum instances the instance buffer is sized for (plan §5.2 / D6). The
/// buffer is allocated once at this cap; per frame only the live prefix is
/// uploaded and drawn.
pub const MAX_INSTANCES: u64 = 1_048_576;

/// Half-extent of the simulated world on each axis, in world units (plan §5.3).
/// The camera fits this rectangle into the viewport.
pub const WORLD_HALF_EXTENT: f32 = 100.0;

/// Unit-quad corners centered at the origin, `[-0.5, 0.5]^2` (plan §5.2). Slot 0,
/// per-vertex. Indexed by [`QUAD_INDICES`].
const QUAD_VERTICES: [[f32; 2]; 4] = [
    [-0.5, -0.5], // 0: bottom-left
    [0.5, -0.5],  // 1: bottom-right
    [-0.5, 0.5],  // 2: top-left
    [0.5, 0.5],   // 3: top-right
];

/// Two-triangle index list for the quad (plan §5.2): (0,1,2) and (2,1,3).
const QUAD_INDICES: [u16; 6] = [0, 1, 2, 2, 1, 3];

/// GPU resources stored in egui's `callback_resources` type-map. Created once at
/// app start ([`RenderResources::new`]); reused every frame.
pub struct RenderResources {
    pipeline: wgpu::RenderPipeline,
    quad_vertex_buffer: wgpu::Buffer,
    quad_index_buffer: wgpu::Buffer,
    /// Per-instance buffer, sized at [`MAX_INSTANCES`]. Shared (`Arc`) with the
    /// app: from Wave 3 the zero-copy upload happens in `App::update` (which
    /// holds `&mut world`), since the `'static` paint callback cannot borrow the
    /// world (plan H4). `paint` reads the same buffer through this handle.
    instance_buffer: Arc<wgpu::Buffer>,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
}

impl RenderResources {
    /// Builds all GPU resources for the instanced-quad pipeline.
    ///
    /// `target_format` is egui's surface format
    /// (`egui_wgpu::RenderState::target_format`); the fragment output must match
    /// it or wgpu rejects the pipeline.
    ///
    /// Returns the resources (to be stored in egui's `callback_resources`) plus a
    /// shared handle to the instance buffer, so the app can upload into it each
    /// frame while holding `&mut world` (plan H4).
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
    ) -> (Self, Arc<wgpu::Buffer>) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("boyko_demo.quad_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("boyko_demo.camera_bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(CAMERA_UNIFORM_SIZE as u64),
                    },
                    count: None,
                }],
            });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("boyko_demo.camera_uniform"),
            contents: bytemuck::bytes_of(&CameraUniform::identity()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("boyko_demo.camera_bg"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("boyko_demo.pipeline_layout"),
            // wgpu 29: `bind_group_layouts` is `&[Option<&BindGroupLayout>]`, and
            // push-constant ranges were replaced by a flat `immediate_size` (0 = none).
            bind_group_layouts: &[Some(&camera_bind_group_layout)],
            immediate_size: 0,
        });

        // Slot 0: per-vertex unit quad. shader_location 0.
        let quad_attrs = [wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 0,
            shader_location: 0,
        }];
        // Slot 1: per-instance GpuInstance. shader_locations 2/3/4/5 (location 1
        // is left unused so the instance block has a distinct attribute base).
        // Phase 20.1 D4: prev_pos is APPENDED at offset 16, so locations 2/3/4
        // keep their byte offsets.
        let instance_attrs = [
            // pos: [f32; 2] at offset 0
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 2,
            },
            // scale: f32 at offset 8
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: 8,
                shader_location: 3,
            },
            // color: u32 (packed RGBA8) at offset 12
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: 12,
                shader_location: 4,
            },
            // prev_pos: [f32; 2] at offset 16 (the GPU lerp's other endpoint)
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 16,
                shader_location: 5,
            },
        ];

        let vertex_buffers = [
            wgpu::VertexBufferLayout {
                array_stride: size_of::<[f32; 2]>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &quad_attrs,
            },
            wgpu::VertexBufferLayout {
                array_stride: GPU_INSTANCE_SIZE as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attrs,
            },
        ];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("boyko_demo.quad_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &vertex_buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // Quads are double-sided sprites; no culling.
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            // wgpu 29 renamed `multiview` to `multiview_mask` (`Option<NonZeroU32>`);
            // `None` = single-view rendering.
            multiview_mask: None,
            cache: None,
        });

        let quad_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("boyko_demo.quad_vertices"),
            contents: bytemuck::cast_slice(&QUAD_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let quad_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("boyko_demo.quad_indices"),
            contents: bytemuck::cast_slice(&QUAD_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        // 24 B × 1 M = 24 MiB device-local (Phase 20.1 D4 — was 16 MiB at the
        // 16 B stride).
        let instance_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("boyko_demo.instances"),
            size: MAX_INSTANCES * GPU_INSTANCE_SIZE as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));

        let resources = Self {
            pipeline,
            quad_vertex_buffer,
            quad_index_buffer,
            instance_buffer: Arc::clone(&instance_buffer),
            camera_buffer,
            camera_bind_group,
        };
        (resources, instance_buffer)
    }
}

/// Per-frame paint callback. Carries only owned/`Copy` data so it satisfies the
/// `'static` bound on egui paint callbacks (D6 / D8 / plan H4) — never a borrow
/// into app or ECS state.
///
/// ## Wave 3 handoff (plan H4 / OQ4 — resolved)
/// The egui callback is `'static`, so it cannot borrow `&world`; the zero-copy
/// `for_each_chunk` upload therefore happens in `App::update` (which holds
/// `&mut world`) directly into the shared instance buffer, BEFORE this callback
/// is registered. The callback then carries only the resulting `instance_count`
/// and issues the single instanced draw. `prepare` still rebuilds the camera
/// uniform from the viewport (the only per-frame GPU write it owns).
pub struct RenderCallback {
    /// Viewport size in physical pixels, captured at registration time (a value
    /// copy — no borrow). Drives the camera projection rebuild in `prepare`.
    pub viewport_px: [f32; 2],
    /// Number of live instances to draw, already uploaded into the shared
    /// instance buffer by `App::update` this frame (or cached from the last
    /// upload on a skipped frame, Phase 20.1 D5).
    pub instance_count: u32,
    /// Interpolation alpha ∈ [0, 1), sampled from
    /// `FixedTime::overstep_fraction()` AFTER the fixed loop in `App::ui`
    /// (Phase 20.1 D7). A `Copy` value, so the `'static` callback bound holds.
    /// `prepare` writes it into the camera uniform; the vertex shader lerps
    /// `mix(prev_pos, pos, alpha)`.
    pub alpha: f32,
}

impl CallbackTrait for RenderCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(resources) = callback_resources.get::<RenderResources>() else {
            // Resources are inserted at app start; absence is a setup bug, but a
            // missing-resource frame should be a no-op, not a panic.
            return Vec::new();
        };

        // Rebuild the world->NDC projection from this frame's viewport (M4
        // resize) and carry this frame's interpolation alpha (Phase 20.1 D7) —
        // the SAME single 80 B write_buffer the camera already issued, zero
        // additional GPU writes.
        let camera = CameraUniform::ortho_fit(
            self.viewport_px[0],
            self.viewport_px[1],
            WORLD_HALF_EXTENT,
            WORLD_HALF_EXTENT,
            self.alpha,
        );
        queue.write_buffer(&resources.camera_buffer, 0, bytemuck::bytes_of(&camera));

        Vec::new()
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<RenderResources>() else {
            return;
        };

        // Clamp against the buffer capacity so an oversized count can never
        // index past the GPU buffer (plan D6).
        let count = self.instance_count.min(MAX_INSTANCES as u32);
        if count == 0 {
            return;
        }

        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, &resources.camera_bind_group, &[]);
        render_pass.set_vertex_buffer(0, resources.quad_vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, resources.instance_buffer.slice(..));
        render_pass.set_index_buffer(
            resources.quad_index_buffer.slice(..),
            wgpu::IndexFormat::Uint16,
        );
        // One instanced draw call for the whole scene (plan D4).
        render_pass.draw_indexed(0..QUAD_INDICES.len() as u32, 0, 0..count);
    }
}
