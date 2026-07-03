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

use boyko_rhi::enums::{AddressMode, BarrierAccess, BarrierStage, DescriptorKind, Filter};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferUsage, CompareOp, ComputePipelineDesc, CullMode, DepthBias, Format,
    GraphicsPipelineDesc, ImageAspect, ImageBarrierDesc, ImageLayout, ImageSubresourceRange,
    ImageUsage, MemoryLocation, MipMode, PrimitiveTopology, RhiCommandEncoder, RhiDevice,
    RhiQueue, SamplerDesc, ShaderStage, TextureDesc, TextureDimension, VertexAttribute,
    VertexBufferLayout, VertexFormat,
};
use boyko_rhi_vulkan::brick_atlas::BrickClipmap;
use boyko_rhi_vulkan::compute::{
    B5_CAMERA_UBO_BYTES_M4, COMPOSITE_PUSH_CONSTANT_BYTES, CoarseMode, EDITLIST_BUFFER_WORDS,
    INTERP_INSTANCES_PUSH_BYTES, LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS, LOCAL_SIZE_X,
    TILE_BOUND_BYTES, csm_depth_fs_spirv, csm_depth_vs_spirv, deferred_pbr_spirv, encode_edit_list,
    fullscreen_sample_fs_spirv, fullscreen_sample_vs_spirv, gbuffer_mrt_fs_spirv,
    gbuffer_mrt_vs_spirv, interp_instances_spirv, punctual_depth_fs_spirv, punctual_depth_vs_spirv,
    sdf_gbuffer_composite_spirv, tile_grid_extent,
};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::rhi_impl::{
    ComputePipeline, VulkanBindGroup, VulkanBindGroupLayout, VulkanGraphicsPipeline,
    VulkanSampler, VulkanShaderModule,
};
use boyko_rhi_vulkan::swapchain::{
    CsmDepthActivation, FRAMES_IN_FLIGHT, GBUFFER_INSTANCE_MODEL_BYTES, GBUFFER_PUSH_BYTES,
    GBufferMeshDraw, GBufferScene, InterpActivation,
};
use boyko_rhi_vulkan::texture::VulkanTexture;
use boyko_sdf_math::SdfEdit;

use boyko_render::{
    GPU_LIGHT_BYTES, GPU_LIGHT_WORDS, GPU_TRANSFORM3D_BYTES, GpuLight, LIGHT_HEADER_BASE_WORDS,
    LIGHT_HEADER_BYTES, LightHeaderGpu, LightingConfig, M_SLOTS, MAX_LIGHTS, MaterialGpu,
    RESOLVED_CSM_BYTES, RESOLVED_SHADOW_ATLAS_BYTES, ResolvedCsm, SHADOW_DIM, Vertex,
};

/// The boot instance budget: the per-slot instance-model SSBO holds this many
/// 48-byte `InstanceModelCol` records. A gather beyond it is a hard panic in
/// `upload_instance_models` (buffer-overflow guard); dynamic growth is host
/// plan R7.
pub(crate) const INSTANCE_CAPACITY: usize = 1024;

/// The mesh-raster G-buffer color format — MUST equal the recorder's
/// `GBUFFER_FORMAT` (`R8G8B8A8_UNORM`), the same pin the showcase carries.
const RASTER_COLOR_FORMAT: Format = Format::R8G8B8A8Unorm;

/// The MARCHER's cast-shadow direction (`L`, direction TO the light) — the A1
/// analytic SDF soft-shadow march lane of the marcher push. The host's SDF edit
/// list is EMPTY in v1 (no pixel takes the SDF path), so this lane is
/// bound-but-inert; it mirrors the showcase's `SHOWCASE_SUN_DIR` so a future
/// SDF instance path (host plan R7) starts from the familiar sun. The RESOLVE's
/// lighting is ECS-owned since host plan R4 (the light table uploads from
/// `LightTableStaging`); this constant no longer seeds any light-table row.
const DEFAULT_SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// The full staged-light-table capacity (`[LightHeaderGpu || GpuLight[MAX_LIGHTS]]`)
/// — the size of the device light table AND each staging ring slot, so ANY table
/// `collect_lights` stages (its scratch is preallocated to exactly this) fits the
/// recorded staging→table copy.
const LIGHT_TABLE_CAPACITY: u64 =
    (LIGHT_HEADER_BYTES + (MAX_LIGHTS as usize) * GPU_LIGHT_BYTES) as u64;

// ── CSM / shadow-atlas constants. The UBO byte sizes come from the OWNING
// `boyko_render` mirror structs (`ResolvedCsm` / `ResolvedShadowAtlas` — the
// exact shapes R4 uploads into these buffers), NOT hand copies; the dim/slot
// values likewise reuse the render crate's exports where they exist. ──

/// Cascade shadow-map resolution — the boot-fixed size of the host's cascade
/// array texture AND the depth-pass viewport (`CsmDepthActivation::shadow_dim`).
/// Matches `CsmConfig`'s default `resolution` (2048), and the runner
/// debug-asserts the owner keeps them equal when arming (a diverging owner-set
/// resolution would skew the fit's `texel_size` against the real map).
pub(crate) const CSM_SHADOW_DIM: u32 = 2048;
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
/// reads at binding 6 — the PRODUCTION `boyko_render` types. Since host plan R4
/// this only seeds the EMPTY (count-0) boot placeholder; the live table uploads
/// from `LightTableStaging` through the generation protocol.
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
/// Since host plan R4 the CASCADE side is live: the runner memcpys the frame's
/// `ResolvedCsm` into `ubo[token.slot()]` and `scene()` arms the depth pass when
/// the ECS predicate holds (zero-seeded UBOs = the boot OFF state). Both depth
/// maps are one-shot BOOT-TRANSITIONED to `SHADER_READ_ONLY_OPTIMAL` (review
/// R4-W1 — see [`Self::seed_boot_layouts`]), so a resolve that reaches them
/// under a stale header gate before any depth pass ever recorded samples a
/// DEFINED layout. The punctual ATLAS side stays OFF (`atlas_punctual == None`,
/// `mode_word == 0` ⇒ bound-but-unread) — the shadowed-punctual composition is
/// a later rung.
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

        // ── One-time BOOT LAYOUT SEED (review R4-W1): transition the cascade array
        // + the shadow atlas from their created UNDEFINED layout to
        // SHADER_READ_ONLY_OPTIMAL, fence-waited, before any frame is recorded.
        // The resolve set binds both as combined image+samplers whose descriptors
        // expect SHADER_READ_ONLY_OPTIMAL; on the armed path the depth pass
        // transitions them every frame, but the header CSM gate can lag the arming
        // predicate by a frame (cross-plugin ordering is unconstrained), so a
        // multi-coincidence exists where the resolve samples the cascade on a frame
        // stream where the depth pass NEVER ran — this seed closes that
        // never-rendered class categorically (a sample then reads undefined VALUES
        // at a DEFINED layout: a benign 1–2 frame shadow artifact, never an invalid
        // access). The graph's armed-frame transition is unaffected: its seeded
        // model uses `oldLayout = UNDEFINED` (content re-rendered, discard-legal
        // from ANY actual layout).
        Self::seed_boot_layouts(device, &cascade, &atlas);

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

    /// Records + submits the one-shot boot transition of the cascade array +
    /// shadow atlas to `SHADER_READ_ONLY_OPTIMAL` (see the call-site comment in
    /// [`Self::create`]), fence-waited; the encoder + fence are setup-class
    /// transients torn down here (the `BrickClipmap::upload_region` boot-submit
    /// shape). Panics on any RHI failure — a setup-stage failure by design.
    fn seed_boot_layouts(device: &VulkanContext, cascade: &VulkanTexture, atlas: &VulkanTexture) {
        let mut encoder = RhiDevice::create_command_encoder(device)
            .expect("invariant: CSM boot-layout command encoder create");
        let fence = RhiDevice::create_fence(device, false)
            .expect("invariant: CSM boot-layout fence create");
        encoder.begin().expect("invariant: CSM boot-layout encoder begin");
        for (texture, layer_count) in [(cascade, 4u32), (atlas, SPOT_ATLAS_SLOTS)] {
            encoder.image_barrier(&ImageBarrierDesc {
                texture,
                src_stage: BarrierStage::TOP_OF_PIPE,
                dst_stage: BarrierStage::COMPUTE_SHADER,
                src_access: BarrierAccess::NONE,
                dst_access: BarrierAccess::SHADER_READ,
                old_layout: ImageLayout::Undefined,
                new_layout: ImageLayout::ShaderReadOnlyOptimal,
                range: ImageSubresourceRange {
                    aspect: ImageAspect::DEPTH,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count,
                },
            });
        }
        encoder.end().expect("invariant: CSM boot-layout encoder end");
        device
            .rhi_queue()
            .submit(&encoder, &fence)
            .expect("invariant: CSM boot-layout submit");
        RhiDevice::wait_fence(device, &fence, u64::MAX)
            .expect("invariant: CSM boot-layout fence wait");
        // SAFETY: `encoder` and `fence` were created on `device` above; the
        // encoder's ONLY submission completed (the fence wait just returned), so
        // no GPU work references either; each is moved by value ⇒ destroyed
        // exactly once. Boot-stage: no other submission is in flight (the scene
        // boot runs before the first frame, and the only earlier boot submit —
        // the brick clip-map bake — is itself fence-waited).
        unsafe {
            RhiDevice::destroy_command_encoder(device, encoder);
            RhiDevice::destroy_fence(device, fence);
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

/// The productionized B3 interpolation pre-pass GPU resources (host plan D7/R5,
/// refined-B): the interp compute pipeline + its 3-binding set layout, and the
/// `FRAMES_IN_FLIGHT`-ringed pair / out-slot SSBOs (frame-private, like the G-buffer
/// ring). The COMPUTE output target is the SHARED instance ring (`instance_rings`),
/// NOT a private draw ring — refined-B retires the private draw/draw_bg rings so
/// static and interpolated instances share ONE draw-ordered buffer.
///
/// The host writes this frame's `pairs[fi]` + `out_slot[fi]` (from the World's
/// `MeshRenderScratch` `pair_ring` / `pair_out_slot` lanes — the source of truth
/// stays in the World, Principle 0: the host only memcpys them) and CPU-scatters the
/// static rows into `instance_rings[fi]`; the interp compute reads `pairs[fi]` +
/// `out_slot[fi]` and OVERWRITES ONLY the dynamic slots of `instance_rings[fi]`; the
/// raster / CSM / atlas VS read that SAME shared ring via
/// `GBufferScene::instance_bind_group` = `instance_bind_groups[fi]` (unchanged — the
/// recorder binds one instance set, raster+CSM+atlas 3-pass reuse). Owned by
/// [`GpuSceneBundles`], created at boot, destroyed in the explicit reverse-order
/// teardown.
///
/// # Sizing
///
/// Both the pair ring and the out-slot ring are sized to [`INSTANCE_CAPACITY`] (the
/// same budget as the affine instance ring), so any per-frame gather up to the budget
/// fits. The PER-FRAME dynamic count (which may be `< INSTANCE_CAPACITY`) is the
/// activation's `instance_count` — the dispatch bound and the shader's loop guard;
/// the built `capacity` only bounds the SSBOs.
struct InterpGpuProd {
    /// The B2 interp compute pipeline (`interp_instances.comp`).
    pipeline: ComputePipeline,
    /// The 3-binding interp set layout { pairs @0 (read), out_slot @1 (read),
    /// model_out @2 (write) }, all COMPUTE.
    layout: VulkanBindGroupLayout,
    /// FIF ring of pair SSBOs (host-written, COMPUTE-read); [`INSTANCE_CAPACITY`] × 96 B.
    pairs: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// FIF ring of out-slot SSBOs (host-written, COMPUTE-read); [`INSTANCE_CAPACITY`] × 4 B.
    /// `out_slot[fi][d]` is dynamic instance `d`'s offset into the shared instance ring
    /// (the gather's `pair_out_slot` lane — the shader's `OutSlot` binding).
    out_slot: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// FIF ring of interp bind groups { pairs[fi] @0, out_slot[fi] @1,
    /// instance_rings[fi] @2 } on [`Self::layout`] — the model_out target is the SHARED
    /// instance ring, so the compute writes what the raster VS reads (no private ring).
    interp_bg: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// The built SSBO capacity in instances ([`INSTANCE_CAPACITY`]) — the SSBO bound,
    /// NOT the per-frame dispatch count (that arrives via the activation).
    capacity: u32,
}

impl InterpGpuProd {
    /// Builds the interp pipeline + the FIF-ringed pair/out-slot SSBOs + their bind
    /// groups (whose model_out target is the SHARED `instance_rings`) sized to
    /// `capacity` instances. `instance_rings` is the gbuffer set-0 instance ring the
    /// raster VS reads — the compute writes it directly (refined-B). `capacity` must
    /// be ≥ 1.
    ///
    /// # Panics
    /// Panics (`expect("invariant: ...")`) on any RHI create failure — a setup-stage
    /// device failure by design (the `GpuSceneBundles::boot` contract).
    fn create(
        device: &VulkanContext,
        instance_rings: &[BoundBuffer; FRAMES_IN_FLIGHT],
        capacity: u32,
    ) -> Self {
        debug_assert!(capacity >= 1, "invariant: the interp pass needs at least one instance slot");
        let cs = RhiDevice::create_shader_module(device, interp_instances_spirv())
            .expect("invariant: B3 interp compute shader module create");
        let layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                ],
            },
        )
        .expect("invariant: B3 interp bind-group layout create");
        let pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &cs,
                entry: c"main",
                push_constant_bytes: INTERP_INSTANCES_PUSH_BYTES,
                bind_group_layout: Some(&layout),
            },
        )
        .expect("invariant: B3 interp compute pipeline create");

        let pair_bytes = capacity as u64 * GPU_TRANSFORM3D_BYTES as u64;
        // The out-slot lane is one u32 per dynamic instance (the shader's `OutSlot`).
        let out_slot_bytes = capacity as u64 * 4;
        let make_buf = |size: u64, what: &str| {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc { size, usage: BufferUsage::STORAGE, location: MemoryLocation::HostVisibleCoherent },
            )
            .unwrap_or_else(|e| panic!("invariant: B3 interp {what} SSBO create: {e:?}"));
            // Zero-seed: a fresh sub-allocation carries prior bytes, and a first
            // frame binds the ring before its first host write.
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .unwrap_or_else(|| panic!("invariant: host-visible B3 interp {what} SSBO is mapped"));
            zero_fill(mapped, size as usize);
            b
        };
        // Each ring slot is a distinct frame-private SSBO (host-coherent so the pair /
        // out-slot writes need no explicit flush).
        let pairs: [BoundBuffer; FRAMES_IN_FLIGHT] =
            core::array::from_fn(|_| make_buf(pair_bytes, "pairs"));
        let out_slot: [BoundBuffer; FRAMES_IN_FLIGHT] =
            core::array::from_fn(|_| make_buf(out_slot_bytes, "out_slot"));
        // interp_bg binds { pairs[fi] @0, out_slot[fi] @1, instance_rings[fi] @2 } —
        // the model_out target is the SHARED instance ring (refined-B): the compute
        // writes the dynamic slots the raster VS then reads, on the SAME buffer.
        let interp_bg: [VulkanBindGroup; FRAMES_IN_FLIGHT] = core::array::from_fn(|fi| {
            RhiDevice::create_bind_group(
                device,
                &BindGroupDesc {
                    layout: &layout,
                    entries: &[
                        BindGroupEntry::StorageBuffer { buffer: &pairs[fi] },
                        BindGroupEntry::StorageBuffer { buffer: &out_slot[fi] },
                        BindGroupEntry::StorageBuffer { buffer: &instance_rings[fi] },
                    ],
                },
            )
            .expect("invariant: B3 interp bind group create")
        });

        // The shader module is consumed by pipeline creation; destroy it now
        // (the ComputePipeline owns the compiled state — mirrors the boot's
        // post-create module teardown).
        // SAFETY: `cs` was created on `device` above and is no longer needed once
        // the pipeline exists; it is destroyed exactly once; no GPU work has been
        // submitted yet (boot stage).
        unsafe {
            RhiDevice::destroy_shader_module(device, cs);
        }

        Self { pipeline, layout, pairs, out_slot, interp_bg, capacity }
    }

    /// The [`InterpActivation`] for this frame slot `fi` and this frame's overstep
    /// `alpha`: the interp set (pairs@0 read + out_slot@1 read + model_out@2 write for
    /// slot `fi`), the pair / out-slot / model-out slot buffers, the per-frame dynamic
    /// `instance_count`, and `alpha`. `model_out_buffer` is the SHARED instance ring
    /// slot (`instance_rings[fi]`), which the caller ALSO binds as
    /// `GBufferScene::instance_bind_group` for the raster read — the ring contract.
    ///
    /// `instance_count` is THIS frame's gathered DYNAMIC count (the dispatch bound +
    /// the push count) — the caller passes the gather's `dynamic_count()`, which the
    /// pair-ring upload already hard-asserts fits `capacity`.
    #[inline]
    fn activation<'a>(
        &'a self,
        fi: usize,
        model_out: &'a BoundBuffer,
        instance_count: u32,
        alpha: f32,
    ) -> InterpActivation<'a> {
        debug_assert!(
            instance_count <= self.capacity,
            "invariant: the per-frame interp instance count fits the built SSBO capacity"
        );
        InterpActivation {
            pipeline: &self.pipeline,
            interp_set: &self.interp_bg[fi],
            pair_buffer: &self.pairs[fi],
            out_slot_buffer: &self.out_slot[fi],
            model_out_buffer: model_out,
            instance_count,
            alpha,
        }
    }

    /// Tears every owned resource down in reverse dependency order (interp_bg →
    /// out_slot → pairs → pipeline → layout). The SHARED instance rings are owned by
    /// `GpuSceneBundles`, not this struct, so they are NOT destroyed here.
    ///
    /// # Safety
    /// The device is idle (the caller's renderer drop waited) so no submission
    /// references these; each is destroyed exactly once (by-value `self`); `device`
    /// is the live context they were created on.
    unsafe fn destroy(self, device: &VulkanContext) {
        // SAFETY: per the contract the device is idle + live; reverse creation order.
        unsafe {
            for bg in self.interp_bg {
                RhiDevice::destroy_bind_group(device, bg);
            }
            for b in self.out_slot {
                RhiDevice::destroy_buffer(device, b);
            }
            for b in self.pairs {
                RhiDevice::destroy_buffer(device, b);
            }
            RhiDevice::destroy_compute_pipeline(device, self.pipeline);
            RhiDevice::destroy_bind_group_layout(device, self.layout);
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
    /// The B3 interpolation pre-pass resources (host plan R5, refined-B): the
    /// FIF-ringed pair / out-slot SSBOs + bind groups + the interp compute pipeline.
    /// The runner writes this frame's pairs into `interp.pairs[slot]` + out-slots into
    /// `interp.out_slot[slot]`, CPU-scatters the static rows into
    /// `instance_rings[slot]`, and arms `scene.interp` — whose model_out target is that
    /// SAME `instance_rings[slot]` (the compute overwrites the dynamic slots). The
    /// raster `instance_bind_group` stays `instance_bind_groups[slot]` (no bind swap).
    interp: InterpGpuProd,
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
    /// The device light table (resolve binding 6), [`LIGHT_TABLE_CAPACITY`] bytes.
    /// The recorder copies `light_upload_bytes` from the fenced slot's staging into
    /// it on a dirty frame (the rung L0-r0 async re-upload).
    light_table: BoundBuffer,
    /// The per-in-flight-slot light STAGING ring (host plan R4 — see the boot
    /// comment for the race pin). The runner writes slot `token.slot()` through
    /// `boyko_render::upload_light_table` iff its uploaded generation lags.
    pub(crate) light_staging: [BoundBuffer; FRAMES_IN_FLIGHT],
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

        // ── The light table (resolve binding 6) + its per-slot STAGING RING
        // (host plan R4): ECS owns lighting — the generation protocol uploads
        // the reconciled `LightTableStaging` bytes into staging slot
        // `token.slot()` and the recorder copies staging→table on dirty frames.
        // Both sides are sized at the FULL `[header || MAX_LIGHTS]` capacity
        // (the exact preallocation `collect_lights`' scratch carries), so any
        // staged table fits. Seeded with the EMPTY header (count 0 —
        // byte-identical to `LightTableStaging::default()`'s seed, the golden
        // empty-table anchor: zero lights, all word-7 gates 0): visible only to
        // a world with no lights, since the host's `light_uploaded_gen`
        // (`u64::MAX`) forces a first-frame upload of the real ECS table.
        //
        // The staging is a RING, not a single instance (the R4 race pin): both
        // in-flight slots re-upload on the two consecutive frames after a
        // change, and rewriting a SINGLE staging on frame N+1 would race frame
        // N's still-in-flight recorded copy (host-write-vs-GPU-transfer-read).
        // Slot `s`'s staging is only read by frames occupying slot `s`, whose
        // fence the write token proves waited.
        let empty_light_words =
            pack_light_table(&LightHeaderGpu::new(0, 0, &LightingConfig::default()), &[]);
        let light_table = RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: LIGHT_TABLE_CAPACITY,
                usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("invariant: light table storage buffer create");
        {
            let mapped = RhiDevice::buffer_mapped_ptr(device, &light_table)
                .expect("invariant: host-visible light table is mapped");
            zero_fill(mapped, LIGHT_TABLE_CAPACITY as usize);
            write_words(mapped, &empty_light_words);
        }
        let light_staging: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: LIGHT_TABLE_CAPACITY,
                    usage: BufferUsage::TRANSFER_SRC,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: light staging ring slot create");
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .expect("invariant: host-visible light staging is mapped");
            zero_fill(mapped, LIGHT_TABLE_CAPACITY as usize);
            write_words(mapped, &empty_light_words);
            b
        });

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

        // ── The B3 interpolation pre-pass (host plan R5, refined-B): the pair /
        // out-slot SSBO rings + bind groups + compute pipeline, sized to the same
        // INSTANCE_CAPACITY as the affine instance ring. Its model_out target is the
        // SHARED `instance_rings` (the compute writes the dynamic slots the raster VS
        // reads), so an armed interp frame keeps `scene.instance_bind_group` at
        // `instance_bind_groups[slot]` — no bind swap.
        let interp = InterpGpuProd::create(device, &instance_rings, INSTANCE_CAPACITY as u32);

        let dispatch_group_count_x = (cw * ch).div_ceil(LOCAL_SIZE_X);

        Self {
            raster_pipeline,
            instance_layout,
            instance_rings,
            instance_bind_groups,
            interp,
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
    /// R4 wiring: SDF empty, brick/coarse/SSAO/atlas/interp OFF (their
    /// always-bound resources are valid placeholders); lighting is ECS-owned —
    /// `light_upload` is `Some(staged_bytes)` on a frame whose staging slot was
    /// just rewritten (the recorder then records the staging→table copy), and
    /// `csm` is `Some(resolved)` when the runner's arming predicate holds (a
    /// fitted sun AND live caster batches — the SAME predicate
    /// `sync_csm_light_gate` drives the light-header gate with, so the resolve
    /// samples the cascades only on frame streams where this depth pass runs).
    ///
    /// # The O1 single-matrix pin
    ///
    /// The depth-pass push matrices built here are byte-images of the SAME
    /// `resolved.cascades[c].view_proj` floats `upload_csm_ring` memcpys into
    /// the slot's cascade UBO — one fit, two byte-identical consumers.
    ///
    /// # Interpolation arming (host plan R5)
    ///
    /// `interp_count` is THIS frame's gathered interpolation-pair count
    /// (`MeshRenderScratch::pair_ring.len()`) and `overstep` is
    /// `FixedTime::overstep_fraction()` (the lerp `alpha`, refreshed EVERY frame via
    /// the 8-byte push even when the pairs are not re-uploaded). When
    /// `interp_count > 0` the interp pass is armed: the raster VS still binds the
    /// SAME shared instance ring (`instance_bind_groups[slot]` — NO bind swap under
    /// refined-B), the interp compute overwrites that ring's dynamic slots in place
    /// before the raster pass, and `scene.interp` carries the activation. When
    /// `interp_count == 0` the path is byte-identical to pre-R5 (the same shared
    /// instance bind group, `interp: None`) — the recorder records no interp dispatch
    /// and no COMPUTE→VERTEX barrier.
    // The per-frame scene assembler: each argument is a distinct per-frame INPUT the
    // stack-built `GBufferScene` (a ~50-field POD borrow bundle) needs — the push,
    // the fenced slot, the draw list, and the four independent arming inputs (light,
    // csm, interp count, overstep). Grouping them into a struct would only relocate
    // the same fields behind an extra indirection with no other caller to share it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scene<'a>(
        &'a self,
        mvp: [u8; GBUFFER_PUSH_BYTES],
        slot: usize,
        mesh_draw: &'a [GBufferMeshDraw<'a>],
        light_upload: Option<u64>,
        csm: Option<&ResolvedCsm>,
        interp_count: u32,
        overstep: f32,
    ) -> GBufferScene<'a> {
        debug_assert!(
            light_upload.unwrap_or(0) <= LIGHT_TABLE_CAPACITY,
            "invariant: the staged light table fits the device table capacity"
        );
        // Interp arming (refined-B): the raster VS ALWAYS reads the shared instance
        // ring (`instance_bind_groups[slot]`) — no bind swap. When the gather produced
        // DYNAMIC instances, the interp compute overwrites that ring's dynamic slots
        // in place (its model_out target IS `instance_rings[slot]`) before the raster
        // pass; a COMPUTE→VERTEX barrier on that shared ring is derived by the graph.
        // `interp_count` is the DYNAMIC count (`dynamic_count()`); `0` records no
        // dispatch (byte-identical to the pre-R5 path).
        let interp_armed = interp_count > 0;
        let instance_bind_group = &self.instance_bind_groups[slot];
        let interp = interp_armed.then(|| {
            self.interp
                .activation(slot, &self.instance_rings[slot], interp_count, overstep)
        });
        GBufferScene {
            raster_pipeline: &self.raster_pipeline,
            vertex_buffer: &self.vertex_buffer,
            vertex_count: 6,
            mvp,
            instance_bind_group,
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
            // The FENCED slot's staging (the ring the R4 race pin demands).
            light_staging: &self.light_staging[slot],
            light_upload_bytes: light_upload.unwrap_or(0),
            light_dirty: light_upload.is_some(),
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
            // The cascade depth-pass activation (host plan R4): built from the
            // SAME ResolvedCsm the runner memcpys into this slot's cascade UBO
            // (the O1 single-matrix pin — LE float bytes == the in-memory f32s
            // the UBO copy carries on x86_64).
            csm: csm.map(|resolved| {
                debug_assert!(
                    resolved.csm_mode_word == 1 && resolved.active_count > 0,
                    "invariant: the csm arming predicate passes a fitted selection"
                );
                let count = (resolved.active_count as usize).min(resolved.cascades.len());
                let mut cascade_view_proj = [[0u8; 64]; 4];
                for (dst, src) in
                    cascade_view_proj.iter_mut().zip(resolved.cascades.iter()).take(count)
                {
                    for (col, column) in src.view_proj.iter().enumerate() {
                        for (row, f) in column.iter().enumerate() {
                            let at = col * 16 + row * 4;
                            dst[at..at + 4].copy_from_slice(&f.to_le_bytes());
                        }
                    }
                }
                // The 88-byte depth-pass push TEMPLATE: `use_model_matrix == 1`
                // (@84). The recorder overwrites `view_proj` (@0..64) per
                // cascade + `base_instance` (@80) per caster batch.
                let mut push = [0u8; GBUFFER_PUSH_BYTES];
                push[84..88].copy_from_slice(&1u32.to_le_bytes());
                CsmDepthActivation {
                    pipeline: &self.csm.depth_pipeline,
                    push,
                    cascade_view_proj,
                    active_count: count as u32,
                    shadow_dim: CSM_SHADOW_DIM,
                }
            }),
            shadow_atlas_texture: &self.csm.atlas,
            shadow_atlas_sampler: &self.csm.atlas_sampler,
            shadow_atlas_ubo: &self.csm.atlas_ubo,
            atlas_punctual: None,
            // The B3 interp activation (host plan R5, refined-B): Some(_) on a frame
            // the gather produced DYNAMIC instances. Its model_out target is the SHARED
            // instance ring the raster VS reads (instance_bind_group ==
            // instance_bind_groups[slot], unchanged) — the compute overwrites the
            // dynamic slots in place before the raster pass.
            interp,
        }
    }

    /// The FENCED slot's interpolation-pair SSBO — the write target of the runner's
    /// per-frame [`upload_pair_ring`](boyko_render::upload_pair_ring) (the interp
    /// compute reads the same slot at binding 0). The sibling in-flight frame binds
    /// the OTHER slot — the lock-free write-after-read discipline the ring exists for.
    #[inline]
    pub(crate) fn interp_pair_slot(&self, slot: usize) -> &BoundBuffer {
        &self.interp.pairs[slot]
    }

    /// The FENCED slot's interpolation OUT-SLOT SSBO (refined-B) — the write target of
    /// the runner's per-frame [`upload_pair_out_slot`](boyko_render::upload_pair_out_slot)
    /// (the interp compute reads the same slot at binding 1 as its `OutSlot` lane). The
    /// sibling in-flight frame binds the OTHER slot — the same lock-free discipline as
    /// the pair slot.
    #[inline]
    pub(crate) fn interp_out_slot_slot(&self, slot: usize) -> &BoundBuffer {
        &self.interp.out_slot[slot]
    }

    /// The FENCED slot's cascade-UBO ring buffer — the write target of the
    /// runner's per-frame `boyko_render::upload_csm_ring` (the resolve binds the
    /// same slot at binding 13, so the sibling in-flight frame reads the OTHER
    /// slot — the lock-free write-after-read discipline the ring exists for).
    #[inline]
    pub(crate) fn csm_ubo_slot(&self, slot: usize) -> &BoundBuffer {
        &self.csm.ubo[slot]
    }

    /// The marcher's binding-0 edit-list SSBO — the write target of the runner's
    /// ONE-SHOT boot-static `boyko_render::upload_sdf_edit_list` (host plan R7).
    /// Unlike the cascade UBO this is a SINGLE shared buffer, not a per-slot ring:
    /// in v1 the edit list is boot-static (written once, before the first marcher
    /// dispatch reads it), so no in-flight race exists (see `upload_sdf_edit_list`).
    #[inline]
    pub(crate) fn edit_list(&self) -> &BoundBuffer {
        &self.edit_list
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
            // The B3 interp cluster (host plan R5, refined-B): its `interp_bg` binds
            // the SHARED `instance_rings` (the model-out target) plus the pair +
            // out-slot rings; it is torn down here (before the shared instance
            // bind groups/rings below), reverse creation order internally.
            self.interp.destroy(ctx);
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
            for slot in self.light_staging {
                RhiDevice::destroy_buffer(ctx, slot);
            }
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
