//! [`GpuSceneBundles`] — the static GPU resource boot for the production
//! G-buffer path (host plan R3).
//!
//! This is a LIFT, not a redesign: the resource set mirrors, creation call for
//! creation call, the minimal (SSAO-off / brick-off / CSM-off / interp-off)
//! configuration of `boyko_rhi_vulkan`'s `window_present_gbuffer::run_showcase_dump`
//! boot (its ~7460..8060 region), packaged as a library builder with an explicit
//! reverse-order teardown. Per-frame VARIABLE data (camera, instance models) is
//! NOT here — it lives in the World and is uploaded by the runner through
//! `boyko_render`'s token-typed upload fns (the D1 what/when split).
//!
//! Bound-but-unread discipline: several marcher/resolve bindings are STATICALLY
//! referenced by the committed SPIR-V past their runtime gates (brick 9..=14,
//! mesh-SDF 15, CSM 12/13, shadow atlas 14/15), so VALID resources must be
//! bound even on the OFF paths — exactly the placeholders the showcase creates.

use core::ptr::NonNull;

use boyko_rhi::enums::{AddressMode, DescriptorKind, Filter};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferUsage, CompareOp, ComputePipelineDesc, CullMode, DepthBias, Format,
    GraphicsPipelineDesc, ImageUsage, MemoryLocation, MipMode, PrimitiveTopology, RhiDevice,
    SamplerDesc, ShaderStage, TextureDesc, TextureDimension, VertexAttribute, VertexBufferLayout,
    VertexFormat,
};
use boyko_rhi_vulkan::brick_atlas::BrickClipmap;
use boyko_rhi_vulkan::compute::{
    B5_CAMERA_UBO_BYTES_M4, COMPOSITE_PUSH_CONSTANT_BYTES, CoarseMode, EDITLIST_BUFFER_WORDS,
    LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS, LOCAL_SIZE_X, TILE_BOUND_BYTES, csm_depth_fs_spirv,
    csm_depth_vs_spirv, deferred_pbr_spirv, encode_edit_list, fullscreen_sample_fs_spirv,
    fullscreen_sample_vs_spirv, gbuffer_mrt_fs_spirv, gbuffer_mrt_vs_spirv,
    punctual_depth_fs_spirv, punctual_depth_vs_spirv, sdf_gbuffer_composite_spirv,
    tile_grid_extent,
};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::rhi_impl::{
    ComputePipeline, VulkanBindGroup, VulkanBindGroupLayout, VulkanGraphicsPipeline,
    VulkanSampler, VulkanShaderModule,
};
use boyko_rhi_vulkan::swapchain::{
    FRAMES_IN_FLIGHT, GBUFFER_INSTANCE_MODEL_BYTES, GBUFFER_PUSH_BYTES, GBufferMeshDraw,
    GBufferScene,
};
use boyko_rhi_vulkan::texture::VulkanTexture;
use boyko_sdf_math::SdfEdit;

use boyko_render::{
    DirectionalLight, GPU_LIGHT_WORDS, GpuLight, LIGHT_HEADER_BASE_WORDS, LightHeaderGpu,
    LightingConfig, M_SLOTS, MaterialGpu, RESOLVED_CSM_BYTES, RESOLVED_SHADOW_ATLAS_BYTES,
    SHADOW_DIM, SkyLight, Vertex,
};

/// The boot instance budget: the per-slot instance-model SSBO holds this many
/// 48-byte `InstanceModelCol` records. A gather beyond it is a hard panic in
/// `upload_instance_models` (buffer-overflow guard); dynamic growth is host
/// plan R7.
pub(crate) const INSTANCE_CAPACITY: usize = 1024;

/// The mesh-raster G-buffer color format — MUST equal the recorder's
/// `GBUFFER_FORMAT` (`R8G8B8A8_UNORM`), the same pin the showcase carries.
const RASTER_COLOR_FORMAT: Format = Format::R8G8B8A8Unorm;

/// The default engine sun (direction TO the light) — element 0 of the boot
/// light table AND the marcher's cast-shadow direction (one source, no drift).
/// Mirrors the showcase's `SHOWCASE_SUN_DIR`. Replaced by the ECS light path
/// in host plan R4.
const DEFAULT_SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

// ── CSM / shadow-atlas constants. The UBO byte sizes come from the OWNING
// `boyko_render` mirror structs (`ResolvedCsm` / `ResolvedShadowAtlas` — the
// exact shapes R4 uploads into these buffers), NOT hand copies; the dim/slot
// values likewise reuse the render crate's exports where they exist. ──

/// Cascade shadow-map resolution (the `CsmConfig` research default; no
/// exported constant exists for it yet — R4's config path owns the real knob).
const CSM_SHADOW_DIM: u32 = 2048;
/// Byte size of one host cascade-UBO ring slot — `size_of::<ResolvedCsm>()`
/// via [`RESOLVED_CSM_BYTES`] (the resolve's binding-13 shape).
const CSM_UBO_BYTES: u64 = RESOLVED_CSM_BYTES as u64;
/// Sparse spot/point shadow-atlas resolution — `boyko_render`'s [`SHADOW_DIM`].
const SPOT_SHADOW_DIM: u32 = SHADOW_DIM;
/// Atlas layer budget — `boyko_render`'s [`M_SLOTS`] (the atlas texture's
/// `array_layers` and the `ResolvedShadowAtlas` face count).
const SPOT_ATLAS_SLOTS: u32 = M_SLOTS as u32;
/// Byte size of the host atlas UBO — `size_of::<ResolvedShadowAtlas>()` via
/// [`RESOLVED_SHADOW_ATLAS_BYTES`] (the resolve's binding-15 shape).
const SPOT_ATLAS_UBO_BYTES: u64 = RESOLVED_SHADOW_ATLAS_BYTES as u64;

/// Copies a `u32` word stream into a mapped host-coherent buffer.
///
/// # Safety-in-context
/// Callers pass a `base` obtained from `RhiDevice::buffer_mapped_ptr` on a
/// buffer of at least `words.len() * 4` bytes, before any GPU work references
/// it (boot-time seeding).
fn write_words(base: NonNull<u8>, words: &[u32]) {
    // SAFETY: per the fn contract `base` points to >= `words.len() * 4` mapped
    // host-coherent bytes; `words` is a distinct host slice (no overlap with the
    // fresh device allocation); the copy completes before any submit.
    unsafe {
        core::ptr::copy_nonoverlapping(
            words.as_ptr().cast::<u8>(),
            base.as_ptr(),
            words.len() * 4,
        );
    }
}

/// Zero-fills the first `len` bytes of a mapped host-coherent buffer
/// (deterministic boot seeding — a fresh sub-allocation carries prior bytes).
fn zero_fill(base: NonNull<u8>, len: usize) {
    // SAFETY: per the call sites `base` points to >= `len` mapped host-coherent
    // bytes of a just-created buffer; no GPU work references it yet.
    unsafe {
        core::ptr::write_bytes(base.as_ptr(), 0, len);
    }
}

/// Packs a [`MaterialGpu`] into the 12-word (`3 × vec4` std430) table element
/// the marcher/resolve reads (the layout the fingerprint const-asserts in
/// `boyko_render::material` pin).
fn pack_material(m: &MaterialGpu) -> [u32; 12] {
    let mut w = [0u32; 12];
    for c in 0..4 {
        w[c] = m.base_color[c].to_bits();
        w[4 + c] = m.mrr[c].to_bits();
        w[8 + c] = m.emissive[c].to_bits();
    }
    w
}

/// Packs a header + light list into the std430 light-table word stream
/// (`[LightHeaderGpu (16 words) || GpuLight[] (12 words each)]`) the resolve
/// reads at binding 6 — the PRODUCTION `boyko_render` types, boot-seeded once.
/// R4 replaces this seed with the ECS light reconcile path.
fn pack_light_table(header: &LightHeaderGpu, lights: &[GpuLight]) -> Vec<u32> {
    let mut words = vec![0u32; LIGHT_HEADER_BASE_WORDS + lights.len() * GPU_LIGHT_WORDS];
    let lanes = [
        header.counts_exposure,
        header.sky_diffuse,
        header.sky_spec,
        header.cluster_params,
    ];
    for (li, lane) in lanes.iter().enumerate() {
        for (c, &v) in lane.iter().enumerate() {
            words[li * 4 + c] = v.to_bits();
        }
    }
    for (i, l) in lights.iter().enumerate() {
        let base = LIGHT_HEADER_BASE_WORDS + i * GPU_LIGHT_WORDS;
        for (c, &v) in l.dir_kind.iter().enumerate() {
            words[base + c] = v.to_bits();
        }
        for (c, &v) in l.pos_range.iter().enumerate() {
            words[base + 4 + c] = v.to_bits();
        }
        for (c, &v) in l.color_cone.iter().enumerate() {
            words[base + 8 + c] = v.to_bits();
        }
    }
    words
}

/// The CSM + shadow-atlas trio (host lift of the showcase's `CsmSceneResources`):
/// ALWAYS created so the resolve set can bind @12/@13 (cascade map + UBO) and
/// @14/@15 (atlas map + UBO) — the resolve SPIR-V statically references them.
/// R3 keeps both depth passes OFF (`GBufferScene::csm == None`,
/// `atlas_punctual == None`) and both UBOs ZERO-seeded (`csm_mode` /
/// `mode_word` == 0 ⇒ bound-but-unread); R4 arms them from the ECS light path.
struct CsmResources {
    cascade: VulkanTexture,
    sampler: VulkanSampler,
    /// The cascade UBO RING (one host-coherent slot per in-flight frame),
    /// zero-seeded — bound-but-unread while the depth pass is OFF.
    ubo: [BoundBuffer; FRAMES_IN_FLIGHT],
    depth_pipeline: VulkanGraphicsPipeline,
    depth_vs: VulkanShaderModule,
    depth_fs: VulkanShaderModule,
    atlas: VulkanTexture,
    atlas_sampler: VulkanSampler,
    atlas_ubo: BoundBuffer,
    point_depth_pipeline: VulkanGraphicsPipeline,
    point_depth_vs: VulkanShaderModule,
    point_depth_fs: VulkanShaderModule,
}

impl CsmResources {
    /// Creates the cascade trio + atlas trio + both depth-only pipelines
    /// (mirrors `CsmSceneResources::create`). `instance_layout` is the SAME
    /// set-0 instance-SSBO layout the gbuffer raster pipeline uses.
    fn create(device: &VulkanContext, instance_layout: &VulkanBindGroupLayout) -> Self {
        let cascade = RhiDevice::create_texture(
            device,
            &TextureDesc {
                width: CSM_SHADOW_DIM,
                height: CSM_SHADOW_DIM,
                depth: 1,
                format: Format::D32Sfloat,
                dimension: TextureDimension::D2,
                usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
                array_layers: 4,
            },
        )
        .expect("invariant: CSM cascade array texture create (setup stage)");
        let sampler = RhiDevice::create_sampler(
            device,
            &SamplerDesc {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: AddressMode::ClampToEdge,
                mip: MipMode::None,
                compare: Some(CompareOp::LessOrEqual),
            },
        )
        .expect("invariant: CSM PCF comparison sampler create");
        let ubo: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: CSM_UBO_BYTES,
                    usage: BufferUsage::UNIFORM,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: CSM cascade UBO create");
            // Zero seed: csm_mode_word == 0 (bound-but-unread on the OFF path).
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .expect("invariant: host-visible CSM UBO is mapped");
            zero_fill(mapped, CSM_UBO_BYTES as usize);
            b
        });

        let depth_vs = RhiDevice::create_shader_module(device, csm_depth_vs_spirv())
            .expect("invariant: CSM depth VS module create");
        let depth_fs = RhiDevice::create_shader_module(device, csm_depth_fs_spirv())
            .expect("invariant: CSM depth FS module create");
        let attributes = [
            VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
            VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
            VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
        ];
        let depth_bias = Some(DepthBias {
            constant_factor: 0.0015,
            slope_factor: 1.5,
            clamp: 0.0,
        });
        let depth_pipeline = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &depth_vs,
                vertex_entry: c"main",
                fragment_module: &depth_fs,
                fragment_entry: c"main",
                color_formats: &[],
                depth_format: Some(Format::D32Sfloat),
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: Some(VertexBufferLayout { stride: 40, attributes: &attributes }),
                push_constant_bytes: GBUFFER_PUSH_BYTES as u32,
                bind_group_layout: Some(instance_layout),
                blend: None,
                cull_mode: CullMode::Front,
                depth_bias,
            },
        )
        .expect("invariant: CSM depth-only graphics pipeline create");

        let atlas = RhiDevice::create_texture(
            device,
            &TextureDesc {
                width: SPOT_SHADOW_DIM,
                height: SPOT_SHADOW_DIM,
                depth: 1,
                format: Format::D32Sfloat,
                dimension: TextureDimension::D2,
                usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
                array_layers: SPOT_ATLAS_SLOTS,
            },
        )
        .expect("invariant: shadow-atlas array texture create");
        let atlas_sampler = RhiDevice::create_sampler(
            device,
            &SamplerDesc {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: AddressMode::ClampToEdge,
                mip: MipMode::None,
                compare: Some(CompareOp::LessOrEqual),
            },
        )
        .expect("invariant: shadow-atlas PCF comparison sampler create");
        let atlas_ubo = RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: SPOT_ATLAS_UBO_BYTES,
                usage: BufferUsage::UNIFORM,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("invariant: shadow-atlas UBO create");
        {
            // Zero seed: mode_word == 0 (bound-but-unread on the OFF path).
            let mapped = RhiDevice::buffer_mapped_ptr(device, &atlas_ubo)
                .expect("invariant: host-visible atlas UBO is mapped");
            zero_fill(mapped, SPOT_ATLAS_UBO_BYTES as usize);
        }

        let point_depth_vs = RhiDevice::create_shader_module(device, punctual_depth_vs_spirv())
            .expect("invariant: punctual point depth VS module create");
        let point_depth_fs = RhiDevice::create_shader_module(device, punctual_depth_fs_spirv())
            .expect("invariant: punctual point depth FS module create");
        let point_depth_pipeline = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &point_depth_vs,
                vertex_entry: c"main",
                fragment_module: &point_depth_fs,
                fragment_entry: c"main",
                color_formats: &[],
                depth_format: Some(Format::D32Sfloat),
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: Some(VertexBufferLayout { stride: 40, attributes: &attributes }),
                push_constant_bytes: GBUFFER_PUSH_BYTES as u32,
                bind_group_layout: Some(instance_layout),
                blend: None,
                cull_mode: CullMode::Front,
                depth_bias,
            },
        )
        .expect("invariant: punctual point depth-write graphics pipeline create");

        Self {
            cascade,
            sampler,
            ubo,
            depth_pipeline,
            depth_vs,
            depth_fs,
            atlas,
            atlas_sampler,
            atlas_ubo,
            point_depth_pipeline,
            point_depth_vs,
            point_depth_fs,
        }
    }

    /// Tears the trio down in reverse creation order (mirrors
    /// `CsmSceneResources::destroy`).
    ///
    /// # Safety
    /// Each resource was created on `device`, the device is idle (the caller's
    /// renderer drop waited), and each is destroyed exactly once (by-value).
    unsafe fn destroy(self, device: &VulkanContext) {
        // SAFETY: per the contract `device` is live + idle and nothing
        // references these resources; reverse creation order.
        unsafe {
            RhiDevice::destroy_graphics_pipeline(device, self.point_depth_pipeline);
            RhiDevice::destroy_shader_module(device, self.point_depth_fs);
            RhiDevice::destroy_shader_module(device, self.point_depth_vs);
            RhiDevice::destroy_buffer(device, self.atlas_ubo);
            RhiDevice::destroy_sampler(device, self.atlas_sampler);
            RhiDevice::destroy_texture(device, self.atlas);
            RhiDevice::destroy_graphics_pipeline(device, self.depth_pipeline);
            RhiDevice::destroy_shader_module(device, self.depth_fs);
            RhiDevice::destroy_shader_module(device, self.depth_vs);
            for slot in self.ubo {
                RhiDevice::destroy_buffer(device, slot);
            }
            RhiDevice::destroy_sampler(device, self.sampler);
            RhiDevice::destroy_texture(device, self.cascade);
        }
    }
}

/// The static-resource half of the windowed G-buffer scene (host plan R3):
/// every pipeline / layout / sampler / seeded buffer `render_gbuffer_frame`
/// needs beyond the swapchain + the extent-dependent `GBufferFrame` targets.
///
/// Owned by the host (`WindowHost.gpu`), created once at boot, destroyed
/// explicitly (device idle) in [`destroy`](Self::destroy). The per-frame
/// VARIABLE slots (`camera_ring[s]`, `instance_rings[s]`) are written by the
/// runner through `boyko_render`'s token-typed upload fns.
pub(crate) struct GpuSceneBundles {
    // ── Raster (pass A) ──────────────────────────────────────────────────────
    raster_pipeline: VulkanGraphicsPipeline,
    instance_layout: VulkanBindGroupLayout,
    /// The per-slot instance-model SSBO ring ([`INSTANCE_CAPACITY`] × 48 B,
    /// zero-seeded): the runner uploads the gathered ring into slot
    /// `token.slot()` every frame (plan D5 — unconditional).
    pub(crate) instance_rings: [BoundBuffer; FRAMES_IN_FLIGHT],
    instance_bind_groups: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// The DEGENERATE legacy vertex buffer (6 identical vertices ⇒ zero-area ⇒
    /// no fragments): pass A's legacy draw target on empty-gather frames —
    /// mirrors the showcase's `showcase_quad_vertices` discipline.
    vertex_buffer: BoundBuffer,
    // ── Marcher (pass B) ─────────────────────────────────────────────────────
    marcher: ComputePipeline,
    vocab_layout: VulkanBindGroupLayout,
    /// The SDF edit-list SSBO, seeded EMPTY (`edit_count == 0`) — the marcher
    /// no-ops the field cleanly (every pixel falls to mesh/background).
    edit_list: BoundBuffer,
    /// The b5 camera UBO ring (224 B per slot, zero-seeded): the runner writes
    /// the 80-byte camera block into slot `token.slot()` every frame.
    pub(crate) camera_ring: [BoundBuffer; FRAMES_IN_FLIGHT],
    tiles_buffer: BoundBuffer,
    /// The brick clip-map baked from the EMPTY edit field — the valid
    /// bound-but-unread placeholders for vocab bindings 9..=15 (brick OFF).
    clipmap: BrickClipmap,
    // ── Resolve (pass C) ─────────────────────────────────────────────────────
    resolve_pipeline: ComputePipeline,
    resolve_layout: VulkanBindGroupLayout,
    material_table: BoundBuffer,
    light_table: BoundBuffer,
    light_staging: BoundBuffer,
    light_table_bytes: u64,
    light_dir: [f32; 3],
    // ── Present (pass D) ─────────────────────────────────────────────────────
    present_pipeline: VulkanGraphicsPipeline,
    present_layout: VulkanBindGroupLayout,
    present_sampler: VulkanSampler,
    depth_sampler: VulkanSampler,
    // ── CSM / atlas (bound-but-unread trios; depth passes OFF in R3) ─────────
    csm: CsmResources,
    /// `ceil(composite pixels / LOCAL_SIZE_X)` — the marcher + resolve dispatch
    /// width, boot-fixed to the composite extent (plan D7).
    dispatch_group_count_x: u32,
}

impl GpuSceneBundles {
    /// Boots the full static resource set at the boot-fixed `composite` extent
    /// (plan D7). `swap_format` is the swapchain's color format (the present
    /// pipeline's W2-b contract).
    ///
    /// # Panics
    /// Panics (`expect("invariant: ...")`) on any RHI create failure — a device
    /// OOM at scene-boot time is a setup failure, not a recoverable per-frame
    /// error (the `MeshRegistry::register_mesh` precedent). The window / WSI
    /// links that can legitimately fail on end-user machines are handled by
    /// `WindowHost::boot`'s typed error BEFORE this runs.
    pub(crate) fn boot(ctx: &VulkanContext, composite: (u32, u32), swap_format: Format) -> Self {
        let (cw, ch) = composite;
        debug_assert!(cw > 0 && ch > 0, "invariant: boot composite extent is non-zero");
        let device = ctx;

        // ── The edit-list SSBO (vocab binding 0), seeded EMPTY (count == 0):
        // the marcher's field loop runs zero edits — a clean no-op (mirrors the
        // showcase's seed with an empty edit slice).
        let edit_list = RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: (EDITLIST_BUFFER_WORDS as u64) * 4,
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("invariant: edit-list storage buffer create");
        {
            let empty: [SdfEdit; 0] = [];
            let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
            encode_edit_list(&mut header, &empty);
            let mapped = RhiDevice::buffer_mapped_ptr(device, &edit_list)
                .expect("invariant: host-visible edit-list buffer is mapped");
            write_words(mapped, &header);
        }

        // ── The b5 camera UBO ring (binding 5), one 224-byte slot per in-flight
        // frame, ZERO-seeded (the runner camera-writes each slot before its
        // first bind; the M4 tail stays zero — brick OFF, bound-but-unread).
        let camera_ring: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: B5_CAMERA_UBO_BYTES_M4 as u64,
                    usage: BufferUsage::UNIFORM,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: camera uniform ring slot create");
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .expect("invariant: host-visible camera UBO is mapped");
            zero_fill(mapped, B5_CAMERA_UBO_BYTES_M4);
            b
        });

        // ── The P4b coarse-cull tile buffer (vocab binding 6), bound-but-unread
        // (the coarse cull is gated OFF).
        let (tw, th) = tile_grid_extent(cw, ch);
        let tiles_buffer = RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: (tw as u64) * (th as u64) * (TILE_BOUND_BYTES as u64),
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("invariant: coarse-cull tile-bound storage buffer create");

        // ── The PBR material table (vocab binding 7 + resolve binding 4): ONE
        // slot — the engine default mid-gray dielectric (every mesh pixel picks
        // material 0 in R3).
        let mat_words = pack_material(&MaterialGpu::default());
        let material_table = RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: (mat_words.len() as u64) * 4,
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("invariant: PBR material table create");
        {
            let mapped = RhiDevice::buffer_mapped_ptr(device, &material_table)
                .expect("invariant: host-visible material table is mapped");
            write_words(mapped, &mat_words);
        }

        // ── The brick clip-map placeholders (vocab bindings 9..=15): the
        // marcher SPIR-V statically references them past the runtime gates, so
        // VALID resources must be bound even with brick + mesh-SDF OFF. Baked
        // from the SAME (empty) edit authority the edit list carries.
        let field = {
            use boyko_sdf_math::SdfEditField;
            let mut f = SdfEditField::new();
            f.bump_gen();
            f
        };
        let clipmap = BrickClipmap::create(ctx, &field, [0.0, 0.0, 0.0])
            .expect("invariant: brick clip-map (empty field) create + bake + upload");

        // ── The light table (resolve binding 6) + staging: ONE warm directional
        // sun + a cool sky fill so meshes are lit (R4 replaces this with the ECS
        // light path). All header mode gates (shadow/csm/punctual/ssao) stay 0.
        let light_header = LightHeaderGpu::new(2, 0, &LightingConfig::default());
        let lights = [
            GpuLight::from_directional(&DirectionalLight::new(
                DEFAULT_SUN_DIR,
                [1.0, 0.96, 0.90],
                2.8,
            )),
            GpuLight::from_sky(&SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10])),
        ];
        let light_words = pack_light_table(&light_header, &lights);
        let light_table_bytes = (light_words.len() as u64) * 4;
        let light_table = RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: light_table_bytes,
                usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("invariant: light table storage buffer create");
        {
            let mapped = RhiDevice::buffer_mapped_ptr(device, &light_table)
                .expect("invariant: host-visible light table is mapped");
            write_words(mapped, &light_words);
        }
        let light_staging = RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: light_table_bytes,
                usage: BufferUsage::TRANSFER_SRC,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("invariant: light staging buffer create");
        {
            let mapped = RhiDevice::buffer_mapped_ptr(device, &light_staging)
                .expect("invariant: host-visible light staging is mapped");
            write_words(mapped, &light_words);
        }

        // ── The DEGENERATE legacy vertex buffer (6 identical vertices ⇒ two
        // zero-area triangles ⇒ no fragments): pass A's target on empty-gather
        // frames — the raster pass then only clears depth to far.
        let degenerate = Vertex {
            position: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            color: [1.0, 1.0, 1.0, 1.0],
        };
        let legacy_vertices = [degenerate; 6];
        let vertex_bytes = core::mem::size_of_val(&legacy_vertices) as u64;
        let vertex_buffer = RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: vertex_bytes,
                usage: BufferUsage::VERTEX,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("invariant: legacy vertex buffer create");
        {
            let mapped = RhiDevice::buffer_mapped_ptr(device, &vertex_buffer)
                .expect("invariant: host-visible legacy vertex buffer is mapped");
            // SAFETY: `mapped` points to `vertex_bytes` mapped host-coherent
            // bytes; `legacy_vertices` is a distinct stack array of exactly that
            // size (`Vertex` is `#[repr(C)]`, tightly packed); no GPU work is in
            // flight yet (boot-time seeding).
            unsafe {
                core::ptr::copy_nonoverlapping(
                    legacy_vertices.as_ptr().cast::<u8>(),
                    mapped.as_ptr(),
                    vertex_bytes as usize,
                );
            }
        }

        // ── Samplers.
        let depth_sampler = RhiDevice::create_sampler(device, &SamplerDesc::default())
            .expect("invariant: depth sampler create");
        let present_sampler = RhiDevice::create_sampler(
            device,
            &SamplerDesc {
                mag_filter: Filter::Nearest,
                min_filter: Filter::Nearest,
                address_mode: AddressMode::ClampToEdge,
                mip: MipMode::None,
                compare: None,
            },
        )
        .expect("invariant: present nearest/clamp sampler create");

        // ── The set-0 instance-model layout + the FIF-ringed instance SSBOs +
        // bind groups. Unlike the showcase's single static SSBO, the ring is
        // per-slot: the runner rewrites slot `s` every frame (plan D5), so the
        // sibling in-flight frame must read its OWN slot.
        let instance_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[BindGroupLayoutEntry {
                    binding: 0,
                    count: 1,
                    kind: DescriptorKind::StorageBuffer,
                    stage: ShaderStage::VERTEX,
                }],
            },
        )
        .expect("invariant: instance-model bind-group layout create");
        let instance_ring_bytes = (INSTANCE_CAPACITY * GBUFFER_INSTANCE_MODEL_BYTES) as u64;
        let instance_rings: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: instance_ring_bytes,
                    usage: BufferUsage::STORAGE,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: instance-model SSBO ring slot create");
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .expect("invariant: host-visible instance SSBO is mapped");
            zero_fill(mapped, instance_ring_bytes as usize);
            b
        });
        let instance_bind_groups: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
            core::array::from_fn(|i| {
                RhiDevice::create_bind_group(
                    device,
                    &BindGroupDesc {
                        layout: &instance_layout,
                        entries: &[BindGroupEntry::StorageBuffer {
                            buffer: &instance_rings[i],
                        }],
                    },
                )
                .expect("invariant: instance-model bind group create")
            });

        // ── The mesh-MRT G-buffer producer graphics pipeline (pass A).
        let vs = RhiDevice::create_shader_module(device, gbuffer_mrt_vs_spirv())
            .expect("invariant: mesh-MRT vertex shader module create");
        let fs = RhiDevice::create_shader_module(device, gbuffer_mrt_fs_spirv())
            .expect("invariant: mesh-MRT fragment shader module create");
        let attributes = [
            VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
            VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
            VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
        ];
        let raster_pipeline = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &vs,
                vertex_entry: c"main",
                fragment_module: &fs,
                fragment_entry: c"main",
                color_formats: &[RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT],
                depth_format: Some(Format::D32Sfloat),
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: Some(VertexBufferLayout { stride: 40, attributes: &attributes }),
                push_constant_bytes: GBUFFER_PUSH_BYTES as u32,
                bind_group_layout: Some(&instance_layout),
                blend: None,
                cull_mode: CullMode::None,
                depth_bias: None,
            },
        )
        .expect("invariant: mesh-MRT graphics pipeline create");

        // ── The marcher: the 16-entry vocabulary layout + the compute pipeline
        // (bindings 0..=15, mirroring the showcase's `vocab_entries`).
        let cs = RhiDevice::create_shader_module(device, sdf_gbuffer_composite_spirv())
            .expect("invariant: G-buffer marcher compute shader module create");
        let vocab_entries = [
            BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 12, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 13, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 14, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 15, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        ];
        let vocab_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc { entries: &vocab_entries },
        )
        .expect("invariant: marcher vocabulary bind-group layout create");
        let marcher = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &cs,
                entry: c"main",
                push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
                bind_group_layout: Some(&vocab_layout),
            },
        )
        .expect("invariant: G-buffer marcher compute pipeline create");

        // ── The deferred resolve: the 16-entry layout + the compute pipeline
        // (mirroring the showcase's `resolve_entries`).
        let resolve_cs = RhiDevice::create_shader_module(device, deferred_pbr_spirv())
            .expect("invariant: deferred resolve compute shader module create");
        let resolve_entries = [
            BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 12, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 13, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 14, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 15, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        ];
        let resolve_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc { entries: &resolve_entries },
        )
        .expect("invariant: deferred resolve bind-group layout create");
        let resolve_pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &resolve_cs,
                entry: c"main",
                push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
                bind_group_layout: Some(&resolve_layout),
            },
        )
        .expect("invariant: deferred resolve compute pipeline create");

        // ── The present-blit pipeline (`color_formats[0]` == the swapchain
        // format — W2-b).
        let present_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[BindGroupLayoutEntry {
                    binding: 0,
                    count: 1,
                    kind: DescriptorKind::CombinedImageSampler,
                    stage: ShaderStage::FRAGMENT,
                }],
            },
        )
        .expect("invariant: present-blit bind-group layout create");
        let sample_vs = RhiDevice::create_shader_module(device, fullscreen_sample_vs_spirv())
            .expect("invariant: fullscreen vertex shader module create");
        let sample_fs = RhiDevice::create_shader_module(device, fullscreen_sample_fs_spirv())
            .expect("invariant: fullscreen fragment shader module create");
        let present_pipeline = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &sample_vs,
                vertex_entry: c"main",
                fragment_module: &sample_fs,
                fragment_entry: c"main",
                color_formats: &[swap_format],
                depth_format: None,
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: None,
                push_constant_bytes: 0,
                bind_group_layout: Some(&present_layout),
                blend: None,
                cull_mode: CullMode::None,
                depth_bias: None,
            },
        )
        .expect("invariant: present-blit graphics pipeline create");

        // The shader modules are consumed by pipeline creation; destroy them now
        // (mirrors the showcase's post-create module teardown).
        // SAFETY: every module was created on `ctx` above and is no longer
        // needed once its pipeline exists; each is destroyed exactly once; no
        // GPU work has been submitted yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, sample_fs);
            RhiDevice::destroy_shader_module(device, sample_vs);
            RhiDevice::destroy_shader_module(device, resolve_cs);
            RhiDevice::destroy_shader_module(device, cs);
            RhiDevice::destroy_shader_module(device, fs);
            RhiDevice::destroy_shader_module(device, vs);
        }

        // ── CSM + shadow-atlas trios: ALWAYS created (resolve @12..=15 need
        // valid descriptors); both depth passes stay OFF in R3.
        let csm = CsmResources::create(device, &instance_layout);

        let dispatch_group_count_x = (cw * ch).div_ceil(LOCAL_SIZE_X);

        Self {
            raster_pipeline,
            instance_layout,
            instance_rings,
            instance_bind_groups,
            vertex_buffer,
            marcher,
            vocab_layout,
            edit_list,
            camera_ring,
            tiles_buffer,
            clipmap,
            resolve_pipeline,
            resolve_layout,
            material_table,
            light_table,
            light_staging,
            light_table_bytes,
            light_dir: DEFAULT_SUN_DIR,
            present_pipeline,
            present_layout,
            present_sampler,
            depth_sampler,
            csm,
            dispatch_group_count_x,
        }
    }

    /// Assembles this frame's [`GBufferScene`] ON THE STACK (plan D7 — POD +
    /// refs, zero alloc): the static bundles + this frame's `mvp` push, the
    /// fenced slot's instance bind group, and the gathered draw batch list.
    ///
    /// R3 wiring: SDF empty, brick/coarse/SSAO/CSM/atlas/interp all OFF (their
    /// always-bound resources are valid placeholders); `light_dirty == false`
    /// (the table was seeded at boot — no per-frame copy until the R4 ECS light
    /// path).
    pub(crate) fn scene<'a>(
        &'a self,
        mvp: [u8; GBUFFER_PUSH_BYTES],
        slot: usize,
        mesh_draw: &'a [GBufferMeshDraw<'a>],
    ) -> GBufferScene<'a> {
        GBufferScene {
            raster_pipeline: &self.raster_pipeline,
            vertex_buffer: &self.vertex_buffer,
            vertex_count: 6,
            mvp,
            instance_bind_group: &self.instance_bind_groups[slot],
            marcher: &self.marcher,
            vocab_layout: &self.vocab_layout,
            edit_list: &self.edit_list,
            camera_ring: &self.camera_ring,
            tiles_buffer: &self.tiles_buffer,
            pointer_grid: self.clipmap.grid_buffer(0),
            atlas: self.clipmap.atlas(0).texture(),
            atlas_sampler: self.clipmap.sampler(0),
            level_grids: [self.clipmap.grid_buffer(1), self.clipmap.grid_buffer(2)],
            level_atlases: [
                self.clipmap.atlas(1).texture(),
                self.clipmap.atlas(2).texture(),
            ],
            level_atlas_samplers: [self.clipmap.sampler(1), self.clipmap.sampler(2)],
            // Mesh-SDF OFF: the brick atlas is the benign binding-15 placeholder.
            mesh_sdf: self.clipmap.atlas(0).texture(),
            mesh_sdf_sampler: self.clipmap.sampler(0),
            mesh_sdf_enabled: false,
            depth_sampler: &self.depth_sampler,
            present_pipeline: &self.present_pipeline,
            present_layout: &self.present_layout,
            present_sampler: &self.present_sampler,
            material_table: &self.material_table,
            light_table: &self.light_table,
            light_staging: &self.light_staging,
            light_upload_bytes: self.light_table_bytes,
            light_dirty: false,
            cluster_cull: None,
            cull_layout: None,
            cluster_grid: None,
            light_index: None,
            light_index_alloc: None,
            cluster_cull_push: [0u8; 16],
            cluster_count: 0,
            resolve_pipeline: &self.resolve_pipeline,
            resolve_layout: &self.resolve_layout,
            dispatch_group_count_x: self.dispatch_group_count_x,
            brick: None,
            coarse: None,
            coarse_mode: CoarseMode::EmptySkipOnly,
            lighting_flags: LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO,
            light_dir: self.light_dir,
            ssao: None,
            mesh_draw,
            csm_cascade_texture: &self.csm.cascade,
            csm_compare_sampler: &self.csm.sampler,
            csm_cascade_ring: &self.csm.ubo,
            csm: None,
            shadow_atlas_texture: &self.csm.atlas,
            shadow_atlas_sampler: &self.csm.atlas_sampler,
            shadow_atlas_ubo: &self.csm.atlas_ubo,
            atlas_punctual: None,
            interp: None,
        }
    }

    /// Tears every bundle down in reverse dependency order — the showcase
    /// teardown list, minus the resources R3 does not create (staging, SSAO
    /// pipeline, per-mesh instanced buffers).
    ///
    /// # Safety
    /// The device is idle (the caller dropped the `Renderer`, whose `Drop`
    /// waits idle) so no submission references any of these; each is destroyed
    /// exactly once (the by-value `self` enforces it); `ctx` is the live
    /// context they were created on.
    pub(crate) unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract the device is idle and `ctx` is live; each
        // resource is destroyed exactly once, in reverse dependency order
        // (mirrors the showcase teardown at window_present_gbuffer ~8504..8551).
        unsafe {
            self.csm.destroy(ctx);
            RhiDevice::destroy_graphics_pipeline(ctx, self.present_pipeline);
            RhiDevice::destroy_bind_group_layout(ctx, self.present_layout);
            RhiDevice::destroy_compute_pipeline(ctx, self.resolve_pipeline);
            RhiDevice::destroy_bind_group_layout(ctx, self.resolve_layout);
            RhiDevice::destroy_compute_pipeline(ctx, self.marcher);
            RhiDevice::destroy_bind_group_layout(ctx, self.vocab_layout);
            RhiDevice::destroy_graphics_pipeline(ctx, self.raster_pipeline);
            for bg in self.instance_bind_groups {
                RhiDevice::destroy_bind_group(ctx, bg);
            }
            for buf in self.instance_rings {
                RhiDevice::destroy_buffer(ctx, buf);
            }
            RhiDevice::destroy_bind_group_layout(ctx, self.instance_layout);
            RhiDevice::destroy_sampler(ctx, self.present_sampler);
            RhiDevice::destroy_sampler(ctx, self.depth_sampler);
            RhiDevice::destroy_buffer(ctx, self.vertex_buffer);
            RhiDevice::destroy_buffer(ctx, self.tiles_buffer);
            self.clipmap.destroy(ctx);
            RhiDevice::destroy_buffer(ctx, self.light_staging);
            RhiDevice::destroy_buffer(ctx, self.light_table);
            RhiDevice::destroy_buffer(ctx, self.material_table);
            for slot in self.camera_ring {
                RhiDevice::destroy_buffer(ctx, slot);
            }
            RhiDevice::destroy_buffer(ctx, self.edit_list);
        }
    }
}

/// A reusable allocation for the per-frame `Vec<GBufferMeshDraw<'frame>>` —
/// the ONLY heap the draw-list assembly would otherwise touch. The element
/// type is borrow-parameterized, so the allocation is parked at `'static`
/// while EMPTY between frames and re-viewed at the frame lifetime on take
/// (plan budget: 0 heap allocations per frame after warmup).
pub(crate) struct DrawListScratch {
    /// The parked (always EMPTY) allocation.
    buf: Vec<GBufferMeshDraw<'static>>,
}

impl DrawListScratch {
    /// An empty scratch (no preallocation — the first frame's push warms it).
    pub(crate) fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Takes the parked allocation as a fresh, EMPTY `Vec` at the frame
    /// lifetime. The caller returns it via [`put`](Self::put) after the frame's
    /// render call (which ends every element borrow).
    pub(crate) fn take<'a>(&mut self) -> Vec<GBufferMeshDraw<'a>> {
        let v = core::mem::take(&mut self.buf);
        debug_assert!(v.is_empty(), "invariant: the parked draw list is empty");
        // SAFETY: `v` is EMPTY (parked cleared by `put`, debug-asserted): no
        // element — hence no `'static`-tagged borrow — exists; only the raw
        // (ptr, cap) allocation is reused. `Vec<GBufferMeshDraw<'x>>` has one
        // layout for every `'x` (lifetimes are erased at monomorphization), so
        // the transmute only relabels the unused element lifetime.
        unsafe {
            core::mem::transmute::<Vec<GBufferMeshDraw<'static>>, Vec<GBufferMeshDraw<'a>>>(v)
        }
    }

    /// Parks the frame's draw list, clearing it first (the elements' borrows
    /// are dead — the frame's render call already consumed the list).
    pub(crate) fn put(&mut self, mut v: Vec<GBufferMeshDraw<'_>>) {
        v.clear();
        // SAFETY: `v` was just cleared — no element (borrow) exists; the
        // transmute relabels the unused element lifetime of the raw allocation
        // only (same layout argument as `take`).
        self.buf = unsafe {
            core::mem::transmute::<Vec<GBufferMeshDraw<'_>>, Vec<GBufferMeshDraw<'static>>>(v)
        };
    }
}
