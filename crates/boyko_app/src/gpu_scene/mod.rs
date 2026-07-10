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
#[cfg(feature = "hwrt")]
use boyko_rhi::SpecConstant;
use boyko_rhi_vulkan::brick_atlas::BrickClipmap;
use boyko_rhi_vulkan::ddgi::DdgiAtlas;
use boyko_rhi_vulkan::compute::{
    B5_CAMERA_UBO_BYTES_M4, COMPOSITE_PUSH_CONSTANT_BYTES, CoarseMode, EDITLIST_BUFFER_WORDS,
    INTERP_INSTANCES_PUSH_BYTES, LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS,
    LOCAL_SIZE_X, TILE_BOUND_BYTES, csm_depth_fs_spirv, csm_depth_vs_spirv, deferred_pbr_spirv,
    encode_edit_list, fullscreen_sample_fs_spirv, fullscreen_sample_vs_spirv, gbuffer_mrt_fs_spirv,
    gbuffer_mrt_pm_fs_spirv, gbuffer_mrt_pm_vs_spirv, gbuffer_mrt_vs_spirv, interp_instances_spirv,
    punctual_depth_fs_spirv, punctual_depth_vs_spirv, sdf_gbuffer_composite_spirv,
    sdf_probe_update_spirv, tile_grid_extent,
};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::rhi_impl::{
    ComputePipeline, VulkanBindGroup, VulkanBindGroupLayout, VulkanGraphicsPipeline,
    VulkanSampler, VulkanShaderModule, rebind_storage_buffer,
};
use boyko_rhi_vulkan::swapchain::{
    CsmDepthActivation, DdgiUpdateActivation, FRAMES_IN_FLIGHT, FrameWriteToken,
    GBUFFER_INSTANCE_MODEL_BYTES, GBUFFER_PUSH_BYTES, GBufferMeshDraw, GBufferScene,
    InterpActivation, PunctualDepthActivation,
};
#[cfg(feature = "hwrt")]
use boyko_rhi_vulkan::accel::BoundAccelStruct;
#[cfg(feature = "hwrt")]
use boyko_rhi_vulkan::accel_build::{
    PersistentTlas, buffer_device_address, create_persistent_tlas, destroy_persistent_tlas,
};
#[cfg(feature = "hwrt")]
use boyko_rhi_vulkan::compute::{
    BUILD_TLAS_INSTANCES_PUSH_BYTES, build_tlas_instances_spirv, deferred_pbr_denoised_spirv,
    deferred_pbr_hwrt_spirv, deferred_pbr_vis_mv_spirv, deferred_pbr_vis_spirv,
    gbuffer_mrt_mv_fs_spirv, gbuffer_mrt_mv_vs_spirv, gbuffer_mrt_mvpm_fs_spirv,
    gbuffer_mrt_mvpm_vs_spirv, shadow_atrous_spirv, shadow_temporal_spirv,
};
#[cfg(feature = "hwrt")]
use boyko_rhi_vulkan::swapchain::{ShadowVisActivation, TlasBuildActivation};
use boyko_rhi_vulkan::texture::VulkanTexture;
use boyko_sdf_math::SdfEdit;

use boyko_render::{
    DDGI_UPDATE_UBO_BYTES, DdgiConfig, DdgiUpdateConfig, DdgiUpdateUbo, GI_MAX_RAYS, GPU_LIGHT_BYTES,
    GPU_LIGHT_WORDS, GPU_TRANSFORM3D_BYTES, GpuLight, LIGHT_HEADER_BASE_WORDS, LIGHT_HEADER_BYTES,
    LightHeaderGpu, LightingConfig, M_SLOTS, MAX_LIGHTS, MaterialTable, PER_INSTANCE_MATERIAL_BYTES,
    RESOLVED_CSM_BYTES, RESOLVED_DDGI_BYTES, RESOLVED_SHADOW_ATLAS_BYTES, RETIRE_DELAY, ResolvedCsm,
    ResolvedShadowAtlas, RetiredGpuBuffers, SHADOW_DIM, Vertex, ddgi_update_dispatch_groups,
    fill_fibonacci_ray_table, pack_ddgi_update_ubo, resolve_ddgi,
};
#[cfg(feature = "hwrt")]
use boyko_ecs::ecs::core::asset::Assets;
#[cfg(feature = "hwrt")]
use boyko_render::{MeshAssetsExt, MeshGpu};
#[cfg(feature = "hwrt")]
use boyko_render::MOTION_CAM_UBO_BYTES;
#[cfg(feature = "hwrt")]
use boyko_render::{RESOLVED_RAY_SHADOW_BYTES, RayShadowConfig};
#[cfg(feature = "hwrt")]
use boyko_scene::render_caps::MeshHandle;

// ── Self-contained resource bundles, split out of this boot god-file (a
// behaviour-preserving module split). Each bundle owns its `create`/`destroy`
// and takes its dependencies as `create` parameters; the boot orchestrator +
// per-frame `scene()` assembler stay here in the core. The submodules pull the
// shared imports/helpers/constants above via `use super::*`. `MotionVecResources`
// is NOT split out — its creation is inlined into (and interleaved with) `boot`.
mod csm;
mod interp;
#[cfg(feature = "hwrt")]
mod tlas;

use csm::CsmResources;
use interp::InterpGpuProd;
#[cfg(feature = "hwrt")]
use tlas::TlasResources;

/// The boot instance budget: the per-slot instance-model SSBO holds this many
/// 48-byte `InstanceModelCol` records. A gather beyond it is a hard panic in
/// `upload_instance_models` (buffer-overflow guard); dynamic growth is host
/// plan R7.
pub(crate) const INSTANCE_CAPACITY: usize = 1024;

/// Asset-streaming plan F7 Q2: a sane upper bound on the non-RT instance family's
/// grown capacity — mirrors `MESH_ADDR_CAP`'s role for the BLAS-address table.
/// `debug_assert`-only (not a hard cap like `boyko_render::MaterialTable`'s
/// `MAX_MATERIAL_ROWS` on the material side — there is no addressing-width limit
/// here, only a runaway-leak sanity net): catches a leaking `MeshRenderScratch::ring`
/// in dev without a release cost.
pub(crate) const MAX_INSTANCE_CAP: usize = 1 << 22;

/// HW-RT rung R2a-3: the per-mesh BLAS-address table capacity (the max distinct meshes the
/// host's TLAS packer can reference). The table is a tiny host-visible `u64` column indexed by
/// `MeshHandle.0`; a registration beyond it is a hard `debug_assert` (consistent with
/// [`INSTANCE_CAPACITY`]'s overflow discipline). Frame-invariant (BLASes never move), rewritten
/// only when [`MeshAssetsExt::blas_generation`](boyko_render::MeshAssetsExt::blas_generation)
/// advances.
#[cfg(feature = "hwrt")]
pub(crate) const MESH_ADDR_CAP: usize = 256;

/// HW-RT rung R2a-3: bytes of one `VkAccelerationStructureInstanceKHR` record the packer writes
/// (must equal the R2a-1 `size_of::<VkAccelerationStructureInstanceKHR>()`).
#[cfg(feature = "hwrt")]
const TLAS_INSTANCE_BYTES: usize = 64;

// The M3 instance-ring record is byte-identical to the packer's `InstanceModelCol` input (48 B).
#[cfg(feature = "hwrt")]
const _: () = assert!(
    GBUFFER_INSTANCE_MODEL_BYTES == 48,
    "invariant: InstanceModelCol is 48 bytes (the R2a-3 packer reads it verbatim)"
);

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
/// HW-RT rung 1b/3b: byte size of one HWRT shadow-params-UBO ring slot — the resolved
/// [`RESOLVED_RAY_SHADOW_BYTES`] mirror (cone/tmax/tmin/bias, 16 B) PLUS the runner-injected
/// rung-3b `SHADOW_FRAME_SEED` at offset 16 (4 B, see `upload_ray_shadow_ring`), rounded up to
/// the HLSL `RayShadowUbo` cbuffer's 32-byte std140 block (two vec4 slots; the trailing 12 B is
/// bound-but-unread pad). The +16 over the bare resolved size is negligible (×2 FIF ring).
#[cfg(feature = "hwrt")]
const RAY_SHADOW_UBO_BYTES: u64 = RESOLVED_RAY_SHADOW_BYTES as u64 + 16;
/// HW-RT rung 3a: the à-trous filter's push-constant size — a single `{ uint step }` (4 B). The
/// recorder pushes `step = 1 << level` per dispatch.
#[cfg(feature = "hwrt")]
const SHADOW_ATROUS_PUSH_BYTES: u32 = 4;
/// Sparse spot/point shadow-atlas resolution — `boyko_render`'s [`SHADOW_DIM`].
const SPOT_SHADOW_DIM: u32 = SHADOW_DIM;
/// Atlas layer budget — `boyko_render`'s [`M_SLOTS`] (the atlas texture's
/// `array_layers` and the `ResolvedShadowAtlas` face count).
const SPOT_ATLAS_SLOTS: u32 = M_SLOTS as u32;
/// Byte size of the host atlas UBO — `size_of::<ResolvedShadowAtlas>()` via
/// [`RESOLVED_SHADOW_ATLAS_BYTES`] (the resolve's binding-15 shape).
const SPOT_ATLAS_UBO_BYTES: u64 = RESOLVED_SHADOW_ATLAS_BYTES as u64;
/// Byte size of the SDFDDGI grid UBO — `size_of::<ResolvedDdgi>()` via
/// [`RESOLVED_DDGI_BYTES`] (48 B, the resolve's binding-18 shape). A SINGLE buffer (the grid is
/// world-fixed — Decision D1), NOT a per-FIF ring. Zero-seeded (bound-but-unread on the OFF path).
const DDGI_UBO_BYTES: u64 = RESOLVED_DDGI_BYTES as u64;
/// Byte size of the SDFDDGI I2 probe-update UBO — `size_of::<DdgiUpdateUbo>()` via
/// [`DDGI_UPDATE_UBO_BYTES`] (48 B, the update set's b6 shape). A SINGLE buffer (identity
/// ray-rotation at I2 → static UBO, no per-FIF ring). Zero-seeded (bound-but-unread on the OFF path).
const DDGI_UPDATE_UBO_SIZE: u64 = DDGI_UPDATE_UBO_BYTES as u64;
/// Byte size of the SDFDDGI I2 Fibonacci ray-table STORAGE buffer — `GI_MAX_RAYS` `float4`s (16 B
/// each). Boot-filled ONCE with the spherical-Fibonacci directions; non-ringed (static, world-fixed).
const DDGI_RAY_TABLE_BYTES: u64 = (GI_MAX_RAYS as u64) * 16;

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

/// HW-RT Rung 3b step 5a: the MESH motion-vector raster resources — the `gbuffer_mrt_mv`
/// pipeline variant (a 4th MRT writing screen-space Δuv) plus the per-FIF prev-instance ring +
/// motion-cam UBO ring the MV vertex shader reads at set 0.
///
/// Built at boot ONLY on an RT device (`ray_query_enabled` + `shadow_denoise_storage_ok`), the
/// SAME capability gate the à-trous / temporal denoise stack lives on; `None` otherwise. Bound by
/// the recorder ONLY when the per-frame temporal gate opens (`temporal_enabled`); on every other
/// frame — and in a non-hwrt build (the whole struct is `#[cfg(feature = "hwrt")]`) — the base
/// 3-MRT raster pipeline draws and these resources are unbound (byte-identical OFF path).
#[cfg(feature = "hwrt")]
pub(crate) struct MotionVecResources {
    /// The `gbuffer_mrt_mv.{vs,fs}` graphics pipeline: identical to the base raster pipeline
    /// (40-byte vertex layout, D32 depth, 88-byte VERTEX push, `CullMode::None`, no blend/bias)
    /// EXCEPT for a 4th color format `R16G16Sfloat` (the `motion_vec` Δuv attachment) and its set-0
    /// layout ([`Self::layout`]).
    pipeline: VulkanGraphicsPipeline,
    /// The 3-binding set-0 layout the MV pipeline declares: binding 0 = the current instance SSBO
    /// (VERTEX), binding 1 = the prev-instance SSBO (VERTEX), binding 2 = the motion-cam UBO
    /// (VERTEX). A SEPARATE layout from the base raster's single-binding instance layout.
    layout: VulkanBindGroupLayout,
    /// HW-RT Rung 3b step 5b: the SDF motion-vector VIS-variant resolve pipeline
    /// (`deferred_pbr_hwrt_vis_mv.comp` / [`deferred_pbr_vis_mv_spirv`]) — identical to the base VIS
    /// resolve (`deferred_pbr_hwrt_vis.comp`, writes `gShadowVis` @21) EXCEPT it ALSO writes each SDF
    /// pixel's camera-only motion vector `Δuv` to the `motion_vec` STORAGE image @23, reprojecting the
    /// reconstructed surface `P` through the `MotionCam` UBO @22. Bound instead of the base VIS
    /// pipeline (in the VIS pass) ONLY when the temporal denoiser is active (`sdf_mv_active()`).
    vis_mv_pipeline: ComputePipeline,
    /// HW-RT Rung 3b step 5b: the 24-binding VIS-MV resolve layout — the 22-binding VIS/DENOISED
    /// layout (0..=21, incl. `gShadowVis` @21) PLUS the `MotionCam` UNIFORM buffer @22 + the
    /// `motion_vec` STORAGE image @23 (both COMPUTE). Threaded (as `scene.vis_mv_layout`) into
    /// [`GBufferTargets::build_shadow_vis_mv_resolve_set`] so the per-FIF VIS-MV set is written once
    /// per extent, decoupled from the per-frame gate.
    vis_mv_layout: VulkanBindGroupLayout,
    /// The per-slot PREVIOUS-frame instance-model SSBO ring ([`INSTANCE_CAPACITY`] × 48 B,
    /// zero-seeded) — identical in shape to `instance_rings`. The runner uploads the gathered
    /// `prev_ring` into slot `token.slot()` when temporal is on (via
    /// [`upload_prev_instance_models`](boyko_render::upload_prev_instance_models)).
    pub(crate) prev_instance_rings: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// The per-slot motion-cam UBO ring ([`MOTION_CAM_UBO_BYTES`] each, zero-seeded): the runner
    /// memcpys `MotionCam` (cur + prev view-proj) into slot `token.slot()` when temporal is on
    /// (via [`upload_motion_cam_ring`](boyko_render::upload_motion_cam_ring)).
    pub(crate) motion_cam_ubo: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// Per-FIF bind groups against [`Self::layout`]: slot `i` binds `{ instance_rings[i],
    /// prev_instance_rings[i], motion_cam_ubo[i] }`. The recorder binds slot `s` at set 0 when the
    /// temporal gate opens.
    bind_groups: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// F8-mv: the combined `gbuffer_mrt_mvpm.{vs,fs}` graphics pipeline — identical to
    /// [`Self::pipeline`] EXCEPT its set-0 layout ([`Self::mvpm_layout`]) also declares the
    /// per-instance material SSBO at binding 3 (the nested `#if defined(MOTION_VECTORS)`
    /// branch in the VS moves it there to dodge the binding-1 collision with
    /// `prev_instances`). Selected instead of [`Self::pipeline`] when a temporal frame ALSO
    /// carries a non-default material (F8-mv; MV+PM combined).
    mvpm_pipeline: VulkanGraphicsPipeline,
    /// F8-mv: the 4-binding set-0 layout the mvpm pipeline declares: binding 0 = the current
    /// instance SSBO (VERTEX), binding 1 = the prev-instance SSBO (VERTEX), binding 2 = the
    /// motion-cam UBO (VERTEX), binding 3 = the per-instance material SSBO (VERTEX). A
    /// SEPARATE layout from [`Self::layout`] (3-binding) and
    /// [`GpuSceneBundles::pm_instance_material_layout`] (2-binding).
    mvpm_layout: VulkanBindGroupLayout,
    /// F8-mv: per-FIF bind groups against [`Self::mvpm_layout`]: slot `i` binds
    /// `{ instance_rings[i], prev_instance_rings[i], motion_cam_ubo[i],
    /// pm_instance_material_rings[i] }`. The recorder binds slot `s` at set 0 when both the
    /// temporal gate opens AND a non-default material is present this frame.
    mvpm_bind_groups: [VulkanBindGroup; FRAMES_IN_FLIGHT],
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
    /// HW-RT rung R2a-3: the GPU-resident per-frame TLAS resources (the packer pipeline +
    /// FIF-ringed mesh-id / instance-array SSBOs + persistent per-slot TLASes + the frame-
    /// invariant BLAS-address table). Built at boot ONLY on an RT device (`ray_query_enabled`);
    /// `None` on a non-RT device (the byte-identical OFF path — `scene.tlas` stays `None`).
    #[cfg(feature = "hwrt")]
    tlas: Option<TlasResources>,
    /// HW-RT Rung 3b step 5a: the MESH motion-vector raster resources (the `gbuffer_mrt_mv`
    /// pipeline variant + the prev-instance ring + motion-cam UBO ring + per-FIF bind groups).
    /// Built at boot ONLY on an RT device (`ray_query_enabled && shadow_denoise_storage_ok`);
    /// `None` otherwise. Bound by the recorder ONLY when the per-frame temporal gate opens — on
    /// every other frame the base 3-MRT raster pipeline draws (byte-identical OFF path).
    #[cfg(feature = "hwrt")]
    mv: Option<MotionVecResources>,
    /// Asset-streaming plan F7 §7.3: PER-FIF-SLOT current capacity of the non-RT instance
    /// family (`instance_rings[s]` + `interp.pairs[s]` + `interp.out_slot[s]`), starting at
    /// [`INSTANCE_CAPACITY`]. Slots grow INDEPENDENTLY — one fenced slot at a time, in
    /// lockstep across the three co-sized buffers — so this is a per-slot array, not a
    /// single scalar (mirrors [`InterpGpuProd`]'s own per-slot `capacity`). On an RT device
    /// (`tlas`/`mv` both `Some`) this stays pinned at [`INSTANCE_CAPACITY`] forever — the W3
    /// runtime gate in [`Self::grow_instance_family_if_needed`] never grows it there.
    instance_capacity: [u32; FRAMES_IN_FLIGHT],
    /// Asset-streaming plan F8: the PER_INSTANCE_MATERIAL gbuffer producer pipeline
    /// (`gbuffer_mrt_pm.{vs,fs}` — the base pair recompiled with `-D
    /// PER_INSTANCE_MATERIAL=1`). Built UNCONDITIONALLY at boot (materials are not
    /// RT-specific — unlike `mv`, this is NOT `#[cfg(feature = "hwrt")]`). Bound instead
    /// of `raster_pipeline` ONLY on a frame with `any_non_default_material` (and no MV —
    /// MV takes priority, F8 §2.3). Its own 2-binding set-0 layout
    /// ([`Self::pm_instance_material_layout`]): instances @0 (VERTEX) + instance_materials
    /// @1 (VERTEX).
    raster_pipeline_pm: VulkanGraphicsPipeline,
    /// Asset-streaming plan F8: the 2-binding set-0 layout [`Self::raster_pipeline_pm`]
    /// declares. A SEPARATE layout from [`Self::instance_layout`] (which is
    /// single-binding). Both bindings VERTEX stage.
    pm_instance_material_layout: VulkanBindGroupLayout,
    /// Asset-streaming plan F8+ (owner: material-drives-albedo-too): the per-slot
    /// instance-material SSBO ring ([`INSTANCE_CAPACITY`]
    /// [`PerInstanceMaterial`](boyko_render::PerInstanceMaterial)s = 32 B each,
    /// zero-seeded). The runner uploads `scratch.material_ids` into slot
    /// `token.slot()` ONLY on an `any_non_default_material` frame (Principle 1 — no
    /// OFF-path upload cost). Grows in
    /// LOCKSTEP with [`Self::instance_rings`] via
    /// [`Self::grow_instance_family_if_needed`] on the non-RT leg; HARD-CAPPED at
    /// [`INSTANCE_CAPACITY`] on an RT device (the F7 W3 gate) — its index space is
    /// IDENTICAL to `instance_rings`, so the two MUST share capacity at all times (a
    /// divergent capacity would OOB the instant the instance ring grows).
    pub(crate) pm_instance_material_rings: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// Asset-streaming plan F8: per-FIF bind groups against
    /// [`Self::pm_instance_material_layout`]: slot `i` binds `{ instance_rings[i] @0,
    /// pm_instance_material_rings[i] @1 }`. The recorder binds slot `s` at set 0 when the
    /// PM pipeline is selected.
    pm_bind_groups: [VulkanBindGroup; FRAMES_IN_FLIGHT],
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
    /// HW-RT rung R2a-4b: the HWRT-variant deferred resolve pipeline (`deferred_pbr_hwrt.comp`)
    /// paired with its 20-binding layout (the 19 software bindings plus binding 19
    /// `AccelerationStructure`). Built at boot ONLY on an RT device (`ray_query_enabled`) under
    /// `feature = "hwrt"`, the same capability gate as [`Self::tlas`]; `None` otherwise (the
    /// byte-identical software path). Its mesh-shadow term traces the per-FIF TLAS with `rayQuery`
    /// instead of sampling the CSM map.
    #[cfg(feature = "hwrt")]
    resolve_pipeline_hwrt: Option<(ComputePipeline, VulkanBindGroupLayout)>,
    /// HW-RT rung 3a: the spatial-denoise VIS + DENOISED resolve pipelines + their SHARED 22-binding
    /// layout (the 21-binding RESOLVE_INLINE-hwrt layout + `gShadowVis` STORAGE image @21). `.0` =
    /// the VIS pipeline (`deferred_pbr_hwrt_vis.comp`, writes `gShadowVis`), `.1` = the DENOISED
    /// pipeline (`deferred_pbr_hwrt_denoised.comp`, reads it), `.2` = the shared layout. Built at boot
    /// ONLY on an RT device (`ray_query_enabled`) under `feature = "hwrt"`, the same gate as
    /// [`Self::resolve_pipeline_hwrt`]; `None` otherwise. Bound only when a frame wires
    /// `scene.shadow = Some(..)` (the step-7 gate; kept `None` this rung, so unbound ⇒ byte-identical).
    #[cfg(feature = "hwrt")]
    shadow_denoise_pipelines:
        Option<(ComputePipeline, ComputePipeline, VulkanBindGroupLayout)>,
    /// HW-RT rung 3a: the à-trous spatial-denoise filter pipeline (`shadow_atrous.comp`) + its
    /// DEDICATED 6-binding layout { `gVisIn` @0, `gVisOut` @1, `gNormal` @2, `gViewT` @3, the
    /// `ResolvedShadowDenoise` UBO @4, the camera UBO @5 } + a 4-byte `{ uint step; }` push. Built at
    /// boot under the SAME gate as [`Self::shadow_denoise_pipelines`]; `None` otherwise.
    #[cfg(feature = "hwrt")]
    shadow_atrous_pipeline: Option<(ComputePipeline, VulkanBindGroupLayout)>,
    /// HW-RT Rung 3b step 6: the temporal reproject compute pipeline (`shadow_temporal.comp`) + its
    /// DEDICATED 8-binding layout { `gVisIn` @0, `gMotionVec` @1, `gViewT` @2, `gHistIn` @3, `gHistOut`
    /// @4, `gTemporalOut` @5 STORAGE images, the `ResolvedTemporalShadow` UBO @6, the camera UBO @7 } +
    /// a 4-byte declared-but-unread COMPUTE push (the RHI rejects a 0-byte compute range; the shader
    /// reads no push — the DDGI-update precedent). Built at boot under the SAME `ray_query_enabled`
    /// gate as [`Self::shadow_atrous_pipeline`]; `None` otherwise. Bound only when a frame's mode is
    /// temporal (kept `None`/`Spatial` ⇒ unbound ⇒ byte-identical).
    #[cfg(feature = "hwrt")]
    shadow_temporal_pipeline: Option<(ComputePipeline, VulkanBindGroupLayout)>,
    /// HW-RT rung 1b: the HWRT soft-shadow-params UBO RING (one host-coherent slot per in-flight
    /// frame, [`RAY_SHADOW_UBO_BYTES`] each, zero-seeded). Slot `i` is bound into slot `i`'s HWRT
    /// resolve set at binding 20 (the tunable cone/tmax/tmin/bias) — each in-flight frame reads its
    /// OWN slot; the runner memcpys `ResolvedRayShadow` into the fenced slot every HWRT frame via
    /// [`upload_ray_shadow_ring`](boyko_render::upload_ray_shadow_ring). RINGED like the CSM cascade
    /// UBO (the fit is author-config-dependent — a retune must reach the resolve without a WAR
    /// hazard). `Some` only on an RT device (`ray_query_enabled`) under `feature = "hwrt"`, the same
    /// gate as [`Self::resolve_pipeline_hwrt`]; `None` otherwise (never bound — the software resolve
    /// set has no binding 20).
    #[cfg(feature = "hwrt")]
    ray_shadow_ubo: Option<[BoundBuffer; FRAMES_IN_FLIGHT]>,
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
    /// error (the `MeshAssetsExt::register_mesh` precedent). The window / WSI
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

        // ── The PBR material table (vocab binding 7 + resolve binding 4) is now
        // `Assets<MaterialGpu>`/`MaterialTable`-owned (asset-system rung A1):
        // `boyko_app::runner` boot-seeds `MaterialTable` (World NonSend) AFTER user
        // `setup` and BEFORE the first frame's `sync_gbuffer` binds it, so `scene()`
        // reads `material_table.table()` directly — no buffer is created here anymore.

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

        // ── Asset-streaming plan F8: the PER_INSTANCE_MATERIAL gbuffer producer
        // pipeline — built UNCONDITIONALLY (materials are device-agnostic, unlike
        // `mv`). Its own 2-binding set-0 layout: instances @0 (VERTEX, the SAME shared
        // `instance_rings`) + instance_materials @1 (VERTEX, its own per-slot ring).
        let pm_instance_material_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry {
                        binding: 0,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::VERTEX,
                    },
                    BindGroupLayoutEntry {
                        binding: 1,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::VERTEX,
                    },
                ],
            },
        )
        .expect("invariant: PM instance-material bind-group layout create");
        let pm_vs = RhiDevice::create_shader_module(device, gbuffer_mrt_pm_vs_spirv())
            .expect("invariant: PM mesh-MRT vertex shader module create");
        let pm_fs = RhiDevice::create_shader_module(device, gbuffer_mrt_pm_fs_spirv())
            .expect("invariant: PM mesh-MRT fragment shader module create");
        let raster_pipeline_pm = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &pm_vs,
                vertex_entry: c"main",
                fragment_module: &pm_fs,
                fragment_entry: c"main",
                color_formats: &[RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT],
                depth_format: Some(Format::D32Sfloat),
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: Some(VertexBufferLayout { stride: 40, attributes: &attributes }),
                push_constant_bytes: GBUFFER_PUSH_BYTES as u32,
                bind_group_layout: Some(&pm_instance_material_layout),
                blend: None,
                cull_mode: CullMode::None,
                depth_bias: None,
            },
        )
        .expect("invariant: PM mesh-MRT graphics pipeline create");
        // SAFETY: both modules were created on `device` and are consumed by the pipeline
        // create; each is destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, pm_fs);
            RhiDevice::destroy_shader_module(device, pm_vs);
        }
        let pm_material_ring_bytes = (INSTANCE_CAPACITY * PER_INSTANCE_MATERIAL_BYTES) as u64;
        let pm_instance_material_rings: [BoundBuffer; FRAMES_IN_FLIGHT] =
            core::array::from_fn(|_| {
                let b = RhiDevice::create_buffer(
                    device,
                    &BufferDesc {
                        size: pm_material_ring_bytes,
                        usage: BufferUsage::STORAGE,
                        location: MemoryLocation::HostVisibleCoherent,
                    },
                )
                .expect("invariant: PM instance-material SSBO ring slot create");
                let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                    .expect("invariant: host-visible PM instance-material SSBO is mapped");
                zero_fill(mapped, pm_material_ring_bytes as usize);
                b
            });
        let pm_bind_groups: [VulkanBindGroup; FRAMES_IN_FLIGHT] = core::array::from_fn(|i| {
            RhiDevice::create_bind_group(
                device,
                &BindGroupDesc {
                    layout: &pm_instance_material_layout,
                    entries: &[
                        BindGroupEntry::StorageBuffer { buffer: &instance_rings[i] },
                        BindGroupEntry::StorageBuffer { buffer: &pm_instance_material_rings[i] },
                    ],
                },
            )
            .expect("invariant: PM instance-material bind group create")
        });

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
                spec_constants: &[],
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
            // SDFDDGI I0: the DDGI probe-irradiance combined image @16 + depth combined image @17 +
            // the `ResolvedDdgi` grid UBO @18 (bound-but-unread; the resolve `.spv` statically
            // references all three). The set is now EXACT-FILL at 19/19 (== MAX_BIND_GROUP_BINDINGS).
            BindGroupLayoutEntry { binding: 16, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 17, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 18, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
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
                spec_constants: &[],
            },
        )
        .expect("invariant: deferred resolve compute pipeline create");

        // ── HW-RT rung R2a-4b: the HWRT-variant resolve pipeline + its 20-binding layout.
        // Built ONLY on an RT device (`ray_query_enabled`) under `feature = "hwrt"` — the SAME
        // capability gate the TLAS resources use (`RayBackendConfig` resolves the mesh-shadow cell
        // to `HardwareTri` on exactly this device tier, so presence == the routing decision).
        // `None` on the software path ⇒ the render binds the software pipeline ⇒ byte-identical. The
        // layout is the 19 software bindings + binding 19 (`AccelerationStructure`) the
        // `deferred_pbr_hwrt.comp` `rayQuery` mesh-shadow trace reads.
        #[cfg(feature = "hwrt")]
        let resolve_pipeline_hwrt = ctx.ray_query_enabled().then(|| {
            let hwrt_cs = RhiDevice::create_shader_module(device, deferred_pbr_hwrt_spirv())
                .expect("invariant: HWRT deferred resolve compute shader module create");
            let mut hwrt_entries = resolve_entries.to_vec();
            hwrt_entries.push(BindGroupLayoutEntry {
                binding: 19,
                count: 1,
                kind: DescriptorKind::AccelerationStructure,
                stage: ShaderStage::COMPUTE,
            });
            // Rung 1b: binding 20 = the tunable soft-shadow-params UBO (`ResolvedRayShadow`,
            // cone/tmax/tmin/bias). Declared ONLY on the HWRT layout (the software resolve
            // layout still fills exactly 19 bindings — byte-neutral).
            hwrt_entries.push(BindGroupLayoutEntry {
                binding: 20,
                count: 1,
                kind: DescriptorKind::UniformBuffer,
                stage: ShaderStage::COMPUTE,
            });
            let hwrt_layout = RhiDevice::create_bind_group_layout(
                device,
                &BindGroupLayoutDesc { entries: &hwrt_entries },
            )
            .expect("invariant: HWRT deferred resolve bind-group layout create");
            // Rung 1b: bake the ray COUNT into spec-const id 0 (the Vogel-disk loop unrolls
            // against it — a retune is a relaunch, Decision 5). `RayShadowConfig` is NOT
            // reachable at this boot site (`boot` takes no `World`), so the count bakes the
            // DEFAULT (16 — the R2a-4b const); a later rung threads the world config here to
            // bake an author retune at boot. `.max(1)` guards the `occ/0` NaN-visibility a `0`
            // count would bake (the `resolve_ray_shadow` debug-assert's build-side counterpart).
            let ray_count = RayShadowConfig::default().ray_count.max(1);
            let hwrt_pipeline = RhiDevice::create_compute_pipeline(
                device,
                &ComputePipelineDesc {
                    module: &hwrt_cs,
                    entry: c"main",
                    push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
                    bind_group_layout: Some(&hwrt_layout),
                    spec_constants: &[SpecConstant { id: 0, value: ray_count }],
                },
            )
            .expect("invariant: HWRT deferred resolve compute pipeline create");
            (hwrt_pipeline, hwrt_layout)
        });

        // ── HW-RT rung 3a: the spatial-denoise VIS + DENOISED resolve pipelines (their SHARED
        // 22-binding layout = the RESOLVE_INLINE-hwrt 21 bindings + `gShadowVis` STORAGE image @21) +
        // the à-trous filter pipeline (its own 6-binding layout + a 4-byte `{ uint step }` push).
        // Built under the SAME `ray_query_enabled` gate as `resolve_pipeline_hwrt` (the à-trous stack
        // lives on exactly this RT tier). `None` on the software path ⇒ the scene never wires
        // `scene.shadow` ⇒ the resolve stays RESOLVE_INLINE-hwrt ⇒ byte-identical.
        #[cfg(feature = "hwrt")]
        let shadow_denoise_pipelines: Option<(
            ComputePipeline,
            ComputePipeline,
            VulkanBindGroupLayout,
        )> = ctx.ray_query_enabled().then(|| {
            // The 22-binding VIS/DENOISED layout: the 19 software resolve bindings (0..=18),
            // binding 19 (`AccelerationStructure` — the VIS trace target), binding 20 (the rung-1b
            // soft-shadow-params UBO), binding 21 (`gShadowVis` STORAGE image). Rebuilt here (the
            // `hwrt_entries` above lives inside its own closure).
            let mut denoise_entries = resolve_entries.to_vec();
            denoise_entries.push(BindGroupLayoutEntry {
                binding: 19,
                count: 1,
                kind: DescriptorKind::AccelerationStructure,
                stage: ShaderStage::COMPUTE,
            });
            denoise_entries.push(BindGroupLayoutEntry {
                binding: 20,
                count: 1,
                kind: DescriptorKind::UniformBuffer,
                stage: ShaderStage::COMPUTE,
            });
            denoise_entries.push(BindGroupLayoutEntry {
                binding: 21,
                count: 1,
                kind: DescriptorKind::StorageImage,
                stage: ShaderStage::COMPUTE,
            });
            let denoise_layout = RhiDevice::create_bind_group_layout(
                device,
                &BindGroupLayoutDesc { entries: &denoise_entries },
            )
            .expect("invariant: rung-3a VIS/DENOISED resolve bind-group layout create");

            // The VIS variant traces — bake the SAME `SHADOW_RAY_COUNT` spec-const (id 0) as the
            // RESOLVE_INLINE resolve so `mesh_vis` is bit-identical. The DENOISED variant does NOT
            // trace (it reads `gShadowVis`), so it declares no spec-const (`spec_constants: &[]`).
            let ray_count = RayShadowConfig::default().ray_count.max(1);
            let vis_cs = RhiDevice::create_shader_module(device, deferred_pbr_vis_spirv())
                .expect("invariant: rung-3a VIS resolve compute shader module create");
            let vis_pipeline = RhiDevice::create_compute_pipeline(
                device,
                &ComputePipelineDesc {
                    module: &vis_cs,
                    entry: c"main",
                    push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
                    bind_group_layout: Some(&denoise_layout),
                    spec_constants: &[SpecConstant { id: 0, value: ray_count }],
                },
            )
            .expect("invariant: rung-3a VIS resolve compute pipeline create");
            let denoised_cs =
                RhiDevice::create_shader_module(device, deferred_pbr_denoised_spirv())
                    .expect("invariant: rung-3a DENOISED resolve compute shader module create");
            let denoised_pipeline = RhiDevice::create_compute_pipeline(
                device,
                &ComputePipelineDesc {
                    module: &denoised_cs,
                    entry: c"main",
                    push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
                    bind_group_layout: Some(&denoise_layout),
                    spec_constants: &[],
                },
            )
            .expect("invariant: rung-3a DENOISED resolve compute pipeline create");
            // SAFETY: both modules were created on `device` and are consumed by their pipeline
            // create; destroy each once; no GPU work is in flight yet.
            unsafe {
                RhiDevice::destroy_shader_module(device, denoised_cs);
                RhiDevice::destroy_shader_module(device, vis_cs);
            }
            (vis_pipeline, denoised_pipeline, denoise_layout)
        });

        // ── HW-RT rung 3a: the à-trous filter pipeline + its 6-binding layout { `gVisIn` @0,
        // `gVisOut` @1, `gNormal` @2, `gViewT` @3, the `ResolvedShadowDenoise` UBO @4, the camera
        // UBO @5 } + a 4-byte `{ uint step }` COMPUTE push. Same `ray_query_enabled` gate.
        #[cfg(feature = "hwrt")]
        let shadow_atrous_pipeline: Option<(ComputePipeline, VulkanBindGroupLayout)> =
            ctx.ray_query_enabled().then(|| {
                let atrous_layout = RhiDevice::create_bind_group_layout(
                    device,
                    &BindGroupLayoutDesc {
                        entries: &[
                            BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                        ],
                    },
                )
                .expect("invariant: rung-3a à-trous bind-group layout create");
                let atrous_cs = RhiDevice::create_shader_module(device, shadow_atrous_spirv())
                    .expect("invariant: rung-3a à-trous compute shader module create");
                let atrous_pipeline = RhiDevice::create_compute_pipeline(
                    device,
                    &ComputePipelineDesc {
                        module: &atrous_cs,
                        entry: c"main",
                        push_constant_bytes: SHADOW_ATROUS_PUSH_BYTES,
                        bind_group_layout: Some(&atrous_layout),
                        spec_constants: &[],
                    },
                )
                .expect("invariant: rung-3a à-trous compute pipeline create");
                // SAFETY: the module was created on `device` and is consumed by the pipeline create;
                // destroy it once; no GPU work is in flight yet.
                unsafe { RhiDevice::destroy_shader_module(device, atrous_cs) };
                (atrous_pipeline, atrous_layout)
            });

        // ── HW-RT Rung 3b step 6: the temporal reproject pipeline + its 8-binding layout { `gVisIn`
        // @0, `gMotionVec` @1, `gViewT` @2, `gHistIn` @3, `gHistOut` @4, `gTemporalOut` @5 STORAGE
        // images, the `ResolvedTemporalShadow` UBO @6, the camera UBO @7 }. The shader declares NO push
        // constant, but the RHI's `create_compute_pipeline` REJECTS a 0-byte range — so create it with
        // `push_constant_bytes: 4` (a declared-but-unread COMPUTE range; the DDGI-update precedent),
        // matching the RHI's minimum. Same `ray_query_enabled` gate as `shadow_atrous_pipeline`.
        #[cfg(feature = "hwrt")]
        let shadow_temporal_pipeline: Option<(ComputePipeline, VulkanBindGroupLayout)> =
            ctx.ray_query_enabled().then(|| {
                let temporal_layout = RhiDevice::create_bind_group_layout(
                    device,
                    &BindGroupLayoutDesc {
                        entries: &[
                            BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                        ],
                    },
                )
                .expect("invariant: rung-3b temporal reproject bind-group layout create");
                let temporal_cs = RhiDevice::create_shader_module(device, shadow_temporal_spirv())
                    .expect("invariant: rung-3b temporal reproject compute shader module create");
                let temporal_pipeline = RhiDevice::create_compute_pipeline(
                    device,
                    &ComputePipelineDesc {
                        module: &temporal_cs,
                        entry: c"main",
                        // The shader reads no push; the RHI rejects a 0-byte compute range, so declare
                        // the minimum 4-byte range (bound-but-unread — the DDGI-update precedent).
                        push_constant_bytes: 4,
                        bind_group_layout: Some(&temporal_layout),
                        spec_constants: &[],
                    },
                )
                .expect("invariant: rung-3b temporal reproject compute pipeline create");
                // SAFETY: the module was created on `device` and is consumed by the pipeline create;
                // destroy it once; no GPU work is in flight yet.
                unsafe { RhiDevice::destroy_shader_module(device, temporal_cs) };
                (temporal_pipeline, temporal_layout)
            });

        // Rung 1b: the HWRT soft-shadow-params UBO ring — minted ONLY on an RT device
        // (`ray_query_enabled`), the SAME gate that builds `resolve_pipeline_hwrt`. `None` on the
        // software path (the resolve set has no binding 20 there). Zero-seeded (the runner memcpys
        // `ResolvedRayShadow` into the fenced slot every HWRT frame).
        #[cfg(feature = "hwrt")]
        let ray_shadow_ubo: Option<[BoundBuffer; FRAMES_IN_FLIGHT]> =
            ctx.ray_query_enabled().then(|| {
                core::array::from_fn(|_| {
                    let b = RhiDevice::create_buffer(
                        device,
                        &BufferDesc {
                            size: RAY_SHADOW_UBO_BYTES,
                            usage: BufferUsage::UNIFORM,
                            location: MemoryLocation::HostVisibleCoherent,
                        },
                    )
                    .expect("invariant: HWRT shadow-params UBO create");
                    let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                        .expect("invariant: host-visible HWRT shadow-params UBO is mapped");
                    zero_fill(mapped, RAY_SHADOW_UBO_BYTES as usize);
                    b
                })
            });

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

        // ── HW-RT rung R2a-3: the GPU-resident per-frame TLAS resources — built ONLY on an RT
        // device (`ray_query_enabled`). Its packer reads the SHARED `instance_rings` at @0 (the
        // same ring the raster VS reads), sized to the same INSTANCE_CAPACITY. `None` on a non-RT
        // device → `scene.tlas` stays `None` (the byte-identical OFF path).
        #[cfg(feature = "hwrt")]
        let tlas = ctx
            .ray_query_enabled()
            .then(|| TlasResources::create(device, &instance_rings, INSTANCE_CAPACITY as u32));

        // ── HW-RT Rung 3b step 5a: the MESH motion-vector raster resources — built ONLY on an RT
        // device that also supports the denoise storage targets (`ray_query_enabled` +
        // `shadow_denoise_storage_ok`), the SAME capability gate the à-trous / temporal stack lives
        // on. `None` otherwise → the recorder never binds the MV pipeline (the base 3-MRT raster
        // draws) → byte-identical. Bound at record time ONLY when the per-frame temporal gate opens.
        #[cfg(feature = "hwrt")]
        let mv = (ctx.ray_query_enabled() && ctx.device_caps().shadow_denoise_storage_ok()).then(
            || {
                // The 3-binding set-0 layout: current instances @0 (VERTEX), prev instances @1
                // (VERTEX), motion-cam UBO @2 (VERTEX) — the exact binding order the
                // `gbuffer_mrt_mv` VS declares.
                let mv_layout = RhiDevice::create_bind_group_layout(
                    device,
                    &BindGroupLayoutDesc {
                        entries: &[
                            BindGroupLayoutEntry {
                                binding: 0,
                                count: 1,
                                kind: DescriptorKind::StorageBuffer,
                                stage: ShaderStage::VERTEX,
                            },
                            BindGroupLayoutEntry {
                                binding: 1,
                                count: 1,
                                kind: DescriptorKind::StorageBuffer,
                                stage: ShaderStage::VERTEX,
                            },
                            BindGroupLayoutEntry {
                                binding: 2,
                                count: 1,
                                kind: DescriptorKind::UniformBuffer,
                                stage: ShaderStage::VERTEX,
                            },
                        ],
                    },
                )
                .expect("invariant: motion-vector bind-group layout create");

                // The MV pipeline: identical to `raster_pipeline` (same 40-byte vertex layout, D32
                // depth, 88-byte VERTEX push, CullMode::None, no blend/bias) EXCEPT a 4th color
                // format `R16G16Sfloat` (the motion_vec Δuv attachment) + the 3-binding MV layout.
                let mv_vs = RhiDevice::create_shader_module(device, gbuffer_mrt_mv_vs_spirv())
                    .expect("invariant: motion-vector vertex shader module create");
                let mv_fs = RhiDevice::create_shader_module(device, gbuffer_mrt_mv_fs_spirv())
                    .expect("invariant: motion-vector fragment shader module create");
                let mv_attributes = [
                    VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
                    VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
                    VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
                ];
                let mv_pipeline = RhiDevice::create_graphics_pipeline(
                    device,
                    &GraphicsPipelineDesc {
                        vertex_module: &mv_vs,
                        vertex_entry: c"main",
                        fragment_module: &mv_fs,
                        fragment_entry: c"main",
                        color_formats: &[
                            RASTER_COLOR_FORMAT,
                            RASTER_COLOR_FORMAT,
                            RASTER_COLOR_FORMAT,
                            Format::R16G16Sfloat,
                        ],
                        depth_format: Some(Format::D32Sfloat),
                        topology: PrimitiveTopology::TriangleList,
                        vertex_layout: Some(VertexBufferLayout {
                            stride: 40,
                            attributes: &mv_attributes,
                        }),
                        push_constant_bytes: GBUFFER_PUSH_BYTES as u32,
                        bind_group_layout: Some(&mv_layout),
                        blend: None,
                        cull_mode: CullMode::None,
                        depth_bias: None,
                    },
                )
                .expect("invariant: motion-vector graphics pipeline create");
                // SAFETY: both modules were created on `device` and are consumed by the pipeline
                // create; each is destroyed once; no GPU work is in flight yet.
                unsafe {
                    RhiDevice::destroy_shader_module(device, mv_fs);
                    RhiDevice::destroy_shader_module(device, mv_vs);
                }

                // ── HW-RT Rung 3b step 5b: the SDF motion-vector VIS-variant resolve pipeline +
                // its 24-binding layout. The layout = the 22-binding VIS/DENOISED entries (the 19
                // software resolve bindings + TLAS @19 + soft-shadow UBO @20 + `gShadowVis` @21,
                // rebuilt here from `resolve_entries`) PLUS the `MotionCam` UBO @22 + the
                // `motion_vec` STORAGE image @23 (both COMPUTE). Built under the SAME `mv` gate
                // (`ray_query_enabled && shadow_denoise_storage_ok`); bound only when temporal is on.
                let mut vis_mv_entries = resolve_entries.to_vec();
                vis_mv_entries.push(BindGroupLayoutEntry {
                    binding: 19,
                    count: 1,
                    kind: DescriptorKind::AccelerationStructure,
                    stage: ShaderStage::COMPUTE,
                });
                vis_mv_entries.push(BindGroupLayoutEntry {
                    binding: 20,
                    count: 1,
                    kind: DescriptorKind::UniformBuffer,
                    stage: ShaderStage::COMPUTE,
                });
                vis_mv_entries.push(BindGroupLayoutEntry {
                    binding: 21,
                    count: 1,
                    kind: DescriptorKind::StorageImage,
                    stage: ShaderStage::COMPUTE,
                });
                vis_mv_entries.push(BindGroupLayoutEntry {
                    binding: 22,
                    count: 1,
                    kind: DescriptorKind::UniformBuffer,
                    stage: ShaderStage::COMPUTE,
                });
                vis_mv_entries.push(BindGroupLayoutEntry {
                    binding: 23,
                    count: 1,
                    kind: DescriptorKind::StorageImage,
                    stage: ShaderStage::COMPUTE,
                });
                let vis_mv_layout = RhiDevice::create_bind_group_layout(
                    device,
                    &BindGroupLayoutDesc { entries: &vis_mv_entries },
                )
                .expect("invariant: rung-3b VIS-MV resolve bind-group layout create");
                // The VIS-MV variant TRACES (it writes `gShadowVis` @21 like the base VIS), so bake
                // the SAME `SHADOW_RAY_COUNT` spec-const (id 0) as the VIS / RESOLVE_INLINE resolve
                // so `mesh_vis` stays bit-identical.
                let vis_mv_ray_count = RayShadowConfig::default().ray_count.max(1);
                let vis_mv_cs =
                    RhiDevice::create_shader_module(device, deferred_pbr_vis_mv_spirv())
                        .expect("invariant: rung-3b VIS-MV resolve compute shader module create");
                let vis_mv_pipeline = RhiDevice::create_compute_pipeline(
                    device,
                    &ComputePipelineDesc {
                        module: &vis_mv_cs,
                        entry: c"main",
                        push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
                        bind_group_layout: Some(&vis_mv_layout),
                        spec_constants: &[SpecConstant { id: 0, value: vis_mv_ray_count }],
                    },
                )
                .expect("invariant: rung-3b VIS-MV resolve compute pipeline create");
                // SAFETY: the module was created on `device` and is consumed by the pipeline create;
                // destroy it once; no GPU work is in flight yet.
                unsafe { RhiDevice::destroy_shader_module(device, vis_mv_cs) };

                // The prev-instance ring: identical shape to `instance_rings` (same
                // `instance_ring_bytes`, STORAGE, HostVisibleCoherent, zero-seeded).
                let prev_instance_rings: [BoundBuffer; FRAMES_IN_FLIGHT] =
                    core::array::from_fn(|_| {
                        let b = RhiDevice::create_buffer(
                            device,
                            &BufferDesc {
                                size: instance_ring_bytes,
                                usage: BufferUsage::STORAGE,
                                location: MemoryLocation::HostVisibleCoherent,
                            },
                        )
                        .expect("invariant: prev-instance-model SSBO ring slot create");
                        let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                            .expect("invariant: host-visible prev-instance SSBO is mapped");
                        zero_fill(mapped, instance_ring_bytes as usize);
                        b
                    });

                // The motion-cam UBO ring: one 128-byte slot per in-flight frame, zero-seeded
                // (mirrors the CSM cascade UBO ring).
                let motion_cam_ubo: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
                    let b = RhiDevice::create_buffer(
                        device,
                        &BufferDesc {
                            size: MOTION_CAM_UBO_BYTES as u64,
                            usage: BufferUsage::UNIFORM,
                            location: MemoryLocation::HostVisibleCoherent,
                        },
                    )
                    .expect("invariant: motion-cam UBO ring slot create");
                    let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                        .expect("invariant: host-visible motion-cam UBO is mapped");
                    zero_fill(mapped, MOTION_CAM_UBO_BYTES);
                    b
                });

                // Per-FIF bind groups: slot `i` binds { instances[i], prev[i], motion_cam[i] }.
                let bind_groups: [VulkanBindGroup; FRAMES_IN_FLIGHT] = core::array::from_fn(|i| {
                    RhiDevice::create_bind_group(
                        device,
                        &BindGroupDesc {
                            layout: &mv_layout,
                            entries: &[
                                BindGroupEntry::StorageBuffer { buffer: &instance_rings[i] },
                                BindGroupEntry::StorageBuffer { buffer: &prev_instance_rings[i] },
                                BindGroupEntry::UniformBuffer { buffer: &motion_cam_ubo[i] },
                            ],
                        },
                    )
                    .expect("invariant: motion-vector bind group create")
                });

                // F8-mv: the combined MV+PM 4-binding set-0 layout — the 3-binding MV layout
                // (current @0 / prev @1 / motion-cam @2) PLUS the per-instance material SSBO
                // @3 (VERTEX; the VS's nested `#if defined(MOTION_VECTORS)` branch moves
                // `instance_materials` here to dodge the binding-1 collision with
                // `prev_instances`).
                let mvpm_layout = RhiDevice::create_bind_group_layout(
                    device,
                    &BindGroupLayoutDesc {
                        entries: &[
                            BindGroupLayoutEntry {
                                binding: 0,
                                count: 1,
                                kind: DescriptorKind::StorageBuffer,
                                stage: ShaderStage::VERTEX,
                            },
                            BindGroupLayoutEntry {
                                binding: 1,
                                count: 1,
                                kind: DescriptorKind::StorageBuffer,
                                stage: ShaderStage::VERTEX,
                            },
                            BindGroupLayoutEntry {
                                binding: 2,
                                count: 1,
                                kind: DescriptorKind::UniformBuffer,
                                stage: ShaderStage::VERTEX,
                            },
                            BindGroupLayoutEntry {
                                binding: 3,
                                count: 1,
                                kind: DescriptorKind::StorageBuffer,
                                stage: ShaderStage::VERTEX,
                            },
                        ],
                    },
                )
                .expect("invariant: mvpm bind-group layout create");

                // F8-mv: the combined pipeline — identical to `mv_pipeline` (same 40-byte
                // vertex layout, D32 depth, 88-byte VERTEX push, CullMode::None, no
                // blend/bias, the same 4 color formats) EXCEPT the 4-binding `mvpm_layout`.
                let mvpm_vs = RhiDevice::create_shader_module(device, gbuffer_mrt_mvpm_vs_spirv())
                    .expect("invariant: mvpm vertex shader module create");
                let mvpm_fs = RhiDevice::create_shader_module(device, gbuffer_mrt_mvpm_fs_spirv())
                    .expect("invariant: mvpm fragment shader module create");
                let mvpm_pipeline = RhiDevice::create_graphics_pipeline(
                    device,
                    &GraphicsPipelineDesc {
                        vertex_module: &mvpm_vs,
                        vertex_entry: c"main",
                        fragment_module: &mvpm_fs,
                        fragment_entry: c"main",
                        color_formats: &[
                            RASTER_COLOR_FORMAT,
                            RASTER_COLOR_FORMAT,
                            RASTER_COLOR_FORMAT,
                            Format::R16G16Sfloat,
                        ],
                        depth_format: Some(Format::D32Sfloat),
                        topology: PrimitiveTopology::TriangleList,
                        vertex_layout: Some(VertexBufferLayout {
                            stride: 40,
                            attributes: &mv_attributes,
                        }),
                        push_constant_bytes: GBUFFER_PUSH_BYTES as u32,
                        bind_group_layout: Some(&mvpm_layout),
                        blend: None,
                        cull_mode: CullMode::None,
                        depth_bias: None,
                    },
                )
                .expect("invariant: mvpm graphics pipeline create");
                // SAFETY: both modules were created on `device` and are consumed by the
                // pipeline create; each is destroyed once; no GPU work is in flight yet.
                unsafe {
                    RhiDevice::destroy_shader_module(device, mvpm_fs);
                    RhiDevice::destroy_shader_module(device, mvpm_vs);
                }

                // Per-FIF bind groups: slot `i` binds { instances[i], prev[i], motion_cam[i],
                // pm_instance_material_rings[i] }.
                let mvpm_bind_groups: [VulkanBindGroup; FRAMES_IN_FLIGHT] =
                    core::array::from_fn(|i| {
                        RhiDevice::create_bind_group(
                            device,
                            &BindGroupDesc {
                                layout: &mvpm_layout,
                                entries: &[
                                    BindGroupEntry::StorageBuffer { buffer: &instance_rings[i] },
                                    BindGroupEntry::StorageBuffer {
                                        buffer: &prev_instance_rings[i],
                                    },
                                    BindGroupEntry::UniformBuffer { buffer: &motion_cam_ubo[i] },
                                    BindGroupEntry::StorageBuffer {
                                        buffer: &pm_instance_material_rings[i],
                                    },
                                ],
                            },
                        )
                        .expect("invariant: mvpm bind group create")
                    });

                MotionVecResources {
                    pipeline: mv_pipeline,
                    layout: mv_layout,
                    vis_mv_pipeline,
                    vis_mv_layout,
                    prev_instance_rings,
                    motion_cam_ubo,
                    bind_groups,
                    mvpm_pipeline,
                    mvpm_layout,
                    mvpm_bind_groups,
                }
            },
        );

        let dispatch_group_count_x = (cw * ch).div_ceil(LOCAL_SIZE_X);

        Self {
            raster_pipeline,
            instance_layout,
            instance_rings,
            instance_bind_groups,
            interp,
            #[cfg(feature = "hwrt")]
            tlas,
            #[cfg(feature = "hwrt")]
            mv,
            instance_capacity: [INSTANCE_CAPACITY as u32; FRAMES_IN_FLIGHT],
            raster_pipeline_pm,
            pm_instance_material_layout,
            pm_instance_material_rings,
            pm_bind_groups,
            vertex_buffer,
            marcher,
            vocab_layout,
            edit_list,
            camera_ring,
            tiles_buffer,
            clipmap,
            resolve_pipeline,
            resolve_layout,
            #[cfg(feature = "hwrt")]
            resolve_pipeline_hwrt,
            #[cfg(feature = "hwrt")]
            shadow_denoise_pipelines,
            #[cfg(feature = "hwrt")]
            shadow_atrous_pipeline,
            #[cfg(feature = "hwrt")]
            shadow_temporal_pipeline,
            #[cfg(feature = "hwrt")]
            ray_shadow_ubo,
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

    /// Cheap, allocation-free steady-state check (asset-streaming plan F7 review W1):
    /// `true` iff `needed` exceeds slot `slot`'s current instance-family capacity.
    /// Touches no World resource at all (`host.gpu` is a plain `WindowHost` field) —
    /// call this BEFORE paying for the (rare) [`Self::grow_instance_family_if_needed`]
    /// path's NonSend `RetiredGpuBuffers` take-out. On an RT device this naturally
    /// tracks the W3 hard cap too: `instance_capacity[slot]` never advances past
    /// `INSTANCE_CAPACITY` there, so an RT overflow still reports `true` here (the
    /// grow call itself then no-ops via the W3 gate and the caller's next upload trips
    /// the documented hard `assert!`).
    #[inline]
    pub(crate) fn needs_instance_grow(&self, needed: u32, slot: usize) -> bool {
        needed > self.instance_capacity[slot]
    }

    /// Asset-streaming plan F7 §7.3: grows the FENCED slot's non-RT instance family
    /// (`instance_rings[s]` + `interp.pairs[s]` + `interp.out_slot[s]`, in lockstep) to
    /// `next_pow2(needed)` iff `needed` exceeds that slot's current capacity — called
    /// BEFORE `upload_instance_models` fills the (possibly grown) ring this frame.
    ///
    /// # W3 runtime gate (E1 Option B)
    ///
    /// An RT device (`self.tlas.is_some() || self.mv.is_some()`) hard-caps this family at
    /// [`INSTANCE_CAPACITY`] and NEVER grows it here: the TLAS packer's `instance_arrays`
    /// / backing / scratch (`tlas.rs`) are all sized ONCE for `INSTANCE_CAPACITY`, and
    /// growing the ring without regrowing those would let the packer read a
    /// `count > INSTANCE_CAPACITY` ring while writing a still-`INSTANCE_CAPACITY`-sized
    /// `instance_arrays[fi]` — an out-of-bounds device write (device-lost). This is a
    /// RUNTIME check (`tlas`/`mv` are `None` on a non-RT device even in an `hwrt` build),
    /// never `#[cfg(feature = "hwrt")]` — a non-hwrt build has neither field at all, so
    /// growth is unconditional there. A > `INSTANCE_CAPACITY` gather on an RT device trips
    /// the LIVE hard `assert!` in `upload_mesh_ids`/`upload_prev_instance_models`
    /// (`boyko_render::upload`) — documented, not silently grown.
    ///
    /// # Steady-state cost
    ///
    /// The caller is expected to have already consulted [`Self::needs_instance_grow`]
    /// before paying for the NonSend take-out this call requires; the internal `needed
    /// <= self.instance_capacity[s]` re-check below is a defensive belt-and-suspenders,
    /// not the steady-state gate anymore.
    ///
    /// # No seed
    ///
    /// The reallocated buffers are `write_bytes(0)`-cleared only — `upload_instance_models`
    /// (the caller's very next step) rewrites the whole ring this frame, and
    /// `upload_pair_ring`/`upload_pair_out_slot` do the same for the interp pair/out-slot
    /// lanes, so no device-side re-seed is needed (unlike the material table, which has no
    /// per-frame full-rewrite guarantee).
    ///
    /// # Safety
    ///
    /// The caller guarantees `token` proves THIS frame's fence wait for slot `token.slot()`
    /// — every descriptor set this fn repoints (`instance_bind_groups[s]`, `interp`'s
    /// `interp_bg[s]`) is therefore non-command-buffer-pending.
    pub(crate) unsafe fn grow_instance_family_if_needed(
        &mut self,
        needed: u32,
        ctx: &VulkanContext,
        token: &FrameWriteToken,
        retired: &mut RetiredGpuBuffers,
        epoch: u64,
    ) {
        #[cfg(feature = "hwrt")]
        if self.tlas.is_some() || self.mv.is_some() {
            return;
        }
        // F8-mv (C2): `self.mv`/`self.tlas` are hwrt-only fields, so this reachability check
        // must itself be hwrt-gated — an unguarded assert would not compile on the default
        // (non-hwrt) build (no such field exists there).
        #[cfg(feature = "hwrt")]
        debug_assert!(
            self.mv.is_none(),
            "invariant: past the W3 early-return, so mv/mvpm resources are unreachable — \
             the grow/rebind path below is the non-RT leg only"
        );

        let s = token.slot();
        if needed <= self.instance_capacity[s] {
            return;
        }
        let new_cap = needed.next_power_of_two();
        debug_assert!(new_cap.is_power_of_two());
        debug_assert!(new_cap >= needed);
        debug_assert!(
            new_cap as usize <= MAX_INSTANCE_CAP,
            "invariant: the grown instance-family capacity ({new_cap}) exceeds the sane \
             MAX_INSTANCE_CAP bound ({MAX_INSTANCE_CAP}) — a likely gather leak"
        );

        let new_bytes = new_cap as u64 * GBUFFER_INSTANCE_MODEL_BYTES as u64;
        let new_ring = RhiDevice::create_buffer(
            ctx,
            &BufferDesc {
                size: new_bytes,
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("invariant: grown instance-model SSBO ring slot create");
        let mapped = RhiDevice::buffer_mapped_ptr(ctx, &new_ring)
            .expect("invariant: host-visible grown instance SSBO is mapped");
        // No seed (see this fn's doc): `upload_instance_models` rewrites the whole ring
        // this frame; zero-fill only covers the gap until that write lands.
        zero_fill(mapped, new_bytes as usize);

        let old_ring = core::mem::replace(&mut self.instance_rings[s], new_ring);
        retired.push(old_ring, epoch + RETIRE_DELAY);

        // SAFETY: slot `s == token.slot()`'s fence was waited this frame (this fn's
        // caller contract above) — `instance_bind_groups[s]`'s set is non-pending, so
        // rewriting its binding in place is sound.
        unsafe {
            rebind_storage_buffer(ctx, &self.instance_bind_groups[s], 0, &self.instance_rings[s]);
        }

        // Asset-streaming plan F8 §1.2/§7i: the PM instance-material ring shares the SAME
        // index space as `instance_rings` (`instance_materials[i]` names the SAME instance
        // `instances[i]` does), so it MUST grow in lockstep — a divergent capacity would
        // OOB the instant the instance ring grows. Rebind BOTH of `pm_bind_groups[s]`'s
        // bindings (0 = the just-grown `instance_rings[s]`, 1 = the just-grown material
        // ring): forgetting either leaves the PM set pointing at a freed/undersized buffer.
        //
        // WARNING RT-LEG-GROWTH COUPLING (task#11): this rebind runs ONLY on the non-RT leg
        // (the W3 cap above early-returns on an RT device). If RT-leg instance-family growth
        // is ever enabled, EVERY descriptor set aliasing instance_rings/prev_instance_rings/
        // motion_cam_ubo/pm_instance_material_rings MUST also be rebound here, or it dangles
        // (an F7-C1-class UAF):
        //   instance_bind_groups[s], pm_bind_groups[s] (@0 + @1), interp.interp_bg[s],
        //   mv.bind_groups[s] (@0/@1/@2), mv.mvpm_bind_groups[s] (@0/@1/@2/@3).
        let new_pm_bytes = new_cap as u64 * PER_INSTANCE_MATERIAL_BYTES as u64;
        let new_pm_ring = RhiDevice::create_buffer(
            ctx,
            &BufferDesc {
                size: new_pm_bytes,
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("invariant: grown PM instance-material SSBO ring slot create");
        let pm_mapped = RhiDevice::buffer_mapped_ptr(ctx, &new_pm_ring)
            .expect("invariant: host-visible grown PM instance-material SSBO is mapped");
        // No seed (see this fn's doc): the runner's `upload_instance_materials` (gated on
        // `any_non_default_material`) rewrites the whole lane this frame.
        zero_fill(pm_mapped, new_pm_bytes as usize);
        let old_pm_ring = core::mem::replace(&mut self.pm_instance_material_rings[s], new_pm_ring);
        retired.push(old_pm_ring, epoch + RETIRE_DELAY);
        // SAFETY: slot `s == token.slot()`'s fence was waited this frame (this fn's caller
        // contract above) — `pm_bind_groups[s]`'s set is non-pending, so rewriting BOTH its
        // bindings in place is sound. Binding 0 repoints to the SAME grown `instance_rings[s]`
        // rebound above; binding 1 to the just-grown material ring.
        unsafe {
            rebind_storage_buffer(ctx, &self.pm_bind_groups[s], 0, &self.instance_rings[s]);
            rebind_storage_buffer(ctx, &self.pm_bind_groups[s], 1, &self.pm_instance_material_rings[s]);
        }

        // SAFETY: same fence contract as above — `interp.grow_slot`'s own precondition
        // (slot `s`'s fence was waited this frame) — reallocates `pairs[s]`/`out_slot[s]`
        // in lockstep and repoints ALL THREE of `interp_bg[s]`'s bindings, including
        // `model_out`@2 against the just-grown `instance_rings[s]` passed in.
        unsafe {
            self.interp.grow_slot(
                ctx,
                s,
                new_cap,
                &self.instance_rings[s],
                retired,
                epoch + RETIRE_DELAY,
            );
        }

        self.instance_capacity[s] = new_cap;
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
        atlas: Option<&ResolvedShadowAtlas>,
        interp_count: u32,
        overstep: f32,
        ddgi_enabled: bool,
        frame_index: u32,
        #[cfg(feature = "hwrt")] tlas_enabled: bool,
        // HW-RT rung 3a/3b step 7: the DENOISE-ARMED gate — `spatial_enabled() || temporal_enabled()`
        // (the runner's `denoise_armed`). Rung 3a used the spatial-only predicate; Rung 3b widened it
        // so Temporal-only (spatial off, temporal on) still arms the VIS pass + the temporal sets. One
        // of the `scene.shadow` gate conditions; the others (`backend == HardwareTri`, `tlas_nonempty`)
        // are folded into `tlas_enabled`, and (`has_primary_directional`) into `csm.is_some()`.
        #[cfg(feature = "hwrt")] denoise_armed: bool,
        // HW-RT rung 3a/3b step 7: the à-trous iteration count — `spatial ? clamped_levels() (>=1) : 0`
        // (the runner threads `0` for Temporal-only, so the VIS pass feeds the temporal reproject its
        // RAW output). Threaded so `activation.atrous_levels` + `final_is_vis2 = atrous_levels % 2 == 1`
        // use the SAME parity the record + graph sites assume (W1 consistency). Only read when armed.
        #[cfg(feature = "hwrt")] atrous_levels: u32,
        // HW-RT Rung 3b step 5a/6: `ShadowDenoiseConfig.mode ∈ {Temporal, Both}` (the runner reads
        // `ShadowDenoiseConfig::temporal_enabled()`). When `true` AND the MV resources exist, the
        // raster pass swaps to the MESH motion-vector pipeline (a 4th MRT writing Δuv) and the
        // temporal reproject pass runs after the à-trous chain. `false` (the default) ⇒ the base 3-MRT
        // raster + no temporal pass ⇒ byte-identical.
        #[cfg(feature = "hwrt")] temporal_enabled: bool,
        // Asset-system rung A1: the GPU mirror of the World-owned `Assets<MaterialGpu>`
        // (boot-seeded by `boyko_app::runner` after user `setup`, before the first
        // `sync_gbuffer` binds it). `material_table.table()` replaces the old
        // boot-owned 1-slot stub; only slot 0 is ever registered this rung, so the
        // bound bytes are byte-identical to that stub.
        material_table: &'a MaterialTable,
        // Asset-streaming plan F8 §2.2: `true` iff THIS gather scattered any non-default
        // material id (`MeshRenderScratch::any_non_default_material`, read by the runner
        // AFTER the gather). Gates `raster_pipeline_pm`/`pm_bind_group` below — `false` on
        // every all-default scene (the goldens), so the recorder binds the FROZEN base
        // pipeline (byte-identity by construction, F8 §2.4).
        any_non_default_material: bool,
        device: &VulkanContext,
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

        // HW-RT rung R2a-3: arm the GPU-resident per-frame TLAS pack + build. `tlas_enabled` is
        // the runner's `cfg!(hwrt) && ray_query && instance_count() > 0` gate; `self.tlas` is
        // `Some` only on an RT device. The drawable `count` (== the shared instance ring length ==
        // Σ batch.instance_count) is the pack dispatch bound + the build's `primitive_count`. When
        // disabled or absent → `None` (the byte-identical OFF path — no pack, no build, no barrier).
        #[cfg(feature = "hwrt")]
        let tlas = match (tlas_enabled, self.tlas.as_ref()) {
            (true, Some(res)) => {
                let count: u32 = mesh_draw.iter().map(|d| d.instance_count).sum();
                (count > 0).then(|| res.activation(slot, count))
            }
            _ => None,
        };

        // SDFDDGI I2 (the ARM rung): when GI is enabled, pack the b6 update UBO for THIS frame and
        // arm the probe-update pass. The render stays BYTE-IDENTICAL — I3 has not wired the resolve
        // sample yet, so the atlas is written-but-unread; this rung validates the LIVE RDG-integrated
        // dispatch (`record_graph_pass` path). When disabled → `None` (the GI-OFF 0%-gate, default).
        //
        // The grid is world-fixed (Decision D1) → a single enabled `ResolvedDdgi` from the
        // owner-locked default `DdgiConfig` (the host does not run the `DdgiPlugin` resolve, so it
        // builds the carrier inline). The UBO write is host-coherent into the SINGLE (non-ringed)
        // `ddgi_update_ubo` before the dispatch reads it (identity ray-rotation → static UBO).
        // `light_count` drives the shader's per-ray shade loop; the host light table is bound at t5.
        let ddgi_update = ddgi_enabled.then(|| {
            let config = DdgiUpdateConfig::default();
            let resolved = resolve_ddgi(&DdgiConfig { ddgi_indirect: true, ..DdgiConfig::default() });
            // The shade loop iterates `light_count` lights from the bound light table. The host's
            // fold caps at `MAX_LIGHTS`; a conservative full-table count keeps the dispatch
            // representative (the resolve does not sample the atlas this rung, so the exact count
            // does not perturb byte-identity — only the write cost).
            let light_count = MAX_LIGHTS;
            let ubo = pack_ddgi_update_ubo(&resolved, &config, frame_index, light_count);
            let bytes = ubo.as_bytes();
            let mapped = RhiDevice::buffer_mapped_ptr(device, &self.csm.ddgi_update_ubo)
                .expect("invariant: host-visible DDGI update UBO is mapped");
            // SAFETY: `mapped` points at `DDGI_UPDATE_UBO_SIZE` (48) host-coherent bytes; `bytes` is
            // exactly that length and copied in full, in-bounds. The UBO is non-ringed and written
            // here before this frame's update dispatch reads it (the update pass is recorded after
            // the scene is assembled); no in-flight submission references it concurrently (the caller
            // fence-waited this frame's slot before assembling the scene).
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
            }
            DdgiUpdateActivation {
                pipeline: &self.csm.ddgi_update_pipeline,
                layout: &self.csm.ddgi_update_layout,
                dispatch_group_count_x: ddgi_update_dispatch_groups(config.subset_n),
            }
        });

        // HW-RT rung 3a/3b step 7: the per-frame denoise gate. `scene.shadow = Some(..)` IFF ALL hold:
        //   (1) the denoise is armed (`spatial || temporal`) → `denoise_armed` (the world read).
        //   (2) the mesh-shadow backend is `HardwareTri` → folded into `tlas_enabled`.
        //   (3) a primary directional light exists     → `csm.is_some()` (the CSM arm is the
        //       primary-directional signal: `csm_armed = csm_mode_word == 1 && caster_count > 0`,
        //       and the VIS trace only writes for that light).
        //   (4) the TLAS is non-empty this frame       → folded into `tlas_enabled`
        //       (`ray_query_enabled() && backend_hw && instance_count() > 0`).
        // When ANY condition is false ⇒ `None` ⇒ RESOLVE_INLINE-hwrt ⇒ byte-identical. The boot built
        // `shadow_denoise_pipelines` + `shadow_atrous_pipeline` (+ `shadow_temporal_pipeline`) only on
        // an RT device, so conditions (2)/(4) via `tlas_enabled` already imply all are `Some` — the
        // `.zip` below is defensive but keeps the activation total. `atrous_levels`/`temporal`/
        // `final_is_vis2 = atrous_levels % 2 == 1` are threaded from the runner (W1: the SAME parity
        // the record + graph sites assume). `temporal_pipeline` is the SEPARATE Option (built on the
        // SAME gate, so `Some` under the arm) the recorder binds for the temporal dispatch.
        #[cfg(feature = "hwrt")]
        let shadow = (denoise_armed && tlas_enabled && csm.is_some())
            .then(|| {
                self.shadow_denoise_pipelines.as_ref().zip(self.shadow_atrous_pipeline.as_ref())
            })
            .flatten()
            .map(|((vis_pipeline, denoised_pipeline, resolve_layout), (atrous_pipeline, atrous_layout))| {
                ShadowVisActivation {
                    vis_pipeline,
                    denoised_pipeline,
                    resolve_layout,
                    atrous_pipeline,
                    atrous_layout,
                    atrous_levels,
                    final_is_vis2: atrous_levels % 2 == 1,
                    temporal: temporal_enabled,
                    temporal_pipeline: self.shadow_temporal_pipeline.as_ref().map(|(p, _)| p),
                }
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
            material_table: material_table.table(),
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
            // R2a-4b: the HWRT resolve pipeline+layout+per-FIF TLAS triple — `Some` only when the
            // boot built the HWRT resources (RT device + hwrt) AND the TLAS ring exists. All three
            // are `Some`/`None` in lock-step, so the record-site picks a consistent triple. `None`
            // ⇒ the software resolve ⇒ byte-identical.
            #[cfg(feature = "hwrt")]
            resolve_pipeline_hwrt: self.resolve_pipeline_hwrt.as_ref().map(|(p, _)| p),
            #[cfg(feature = "hwrt")]
            resolve_layout_hwrt: self.resolve_pipeline_hwrt.as_ref().map(|(_, l)| l),
            #[cfg(feature = "hwrt")]
            resolve_tlas_hwrt: self.tlas.as_ref().map(|t| t.resolve_accels()),
            // Rung 1b: the HWRT shadow-params UBO ring. `Some` on an RT device (the ring is minted
            // under the same `ray_query_enabled` gate as the HWRT resolve set that binds
            // `ray_shadow_ubo[frame_index]` @20 per FIF slot); on the software path the HWRT resolve
            // set is never built, so this is bound by NO set — a benign valid placeholder (the
            // whole CSM cascade UBO ring, always minted + host-coherent + >= 16 B, same
            // `[BoundBuffer; FRAMES_IN_FLIGHT]` shape) satisfies the field type without ever being
            // read. Pass the WHOLE ring (like `csm_cascade_ring`): the resolve-set builder writes
            // each slot into its own HWRT set, so each in-flight frame reads its own slot.
            #[cfg(feature = "hwrt")]
            ray_shadow_ubo: self.ray_shadow_ubo.as_ref().unwrap_or(&self.csm.ubo),
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
            // The FENCED slot's atlas-UBO ring buffer — the resolve binds the SAME slot the runner
            // memcpys `ResolvedShadowAtlas` into via `upload_atlas_ring` (binding 15). The sibling
            // in-flight frame binds the OTHER slot (the lock-free WAR discipline the ring exists for).
            shadow_atlas_ubo: &self.csm.atlas_ubo[slot],
            // SDFDDGI I1: the 3 DDGI resolve bindings (@16/@17/@18) now bind the REAL probe atlas.
            // The GI gate is OFF by default (`DdgiConfig::ddgi_indirect == false` → LightBuf word-7
            // bit 4 == 0), so the resolve's probe-irradiance sample never runs and all three are
            // bound-but-unread (the 0%-gate — byte-identical pixels). I1 severs the I0a dummy: the
            // irradiance/depth atlases are the dedicated `B10G11R11_UFLOAT`/`R16G16_SFLOAT`
            // `Texture2DArray`s, each sampled with a dedicated LINEAR (non-comparison) sampler —
            // closing the VUID trap (a non-Dref SampleLevel with the old CSM COMPARISON sampler was
            // UB). The grid UBO is the dedicated zeroed `ddgi_ubo` (single buffer — world-fixed grid,
            // no ring).
            ddgi_irr_texture: self.csm.ddgi_atlas.irradiance(),
            ddgi_irr_sampler: self.csm.ddgi_atlas.sampler(),
            ddgi_depth_texture: self.csm.ddgi_atlas.depth(),
            ddgi_depth_sampler: self.csm.ddgi_atlas.sampler(),
            ddgi_grid_ubo: &self.csm.ddgi_ubo,
            // SDFDDGI I2 (the ARM rung): `ddgi_update` is `Some(...)` when GI is enabled (the packed
            // activation computed above) → the update RDG pass is recorded + dispatched in the LIVE
            // frame; `None` on the default GI-OFF path (byte-identical 0%-gate — no pass recorded).
            // Even when armed the render stays byte-identical this rung: I3 has not wired the resolve
            // sample, so the atlas is written-but-unread. The classification / ray-table / update-UBO
            // handles are ALWAYS supplied so the RDG sink can resolve them.
            ddgi_update,
            ddgi_classification: self.csm.ddgi_atlas.classification(),
            ddgi_ray_table: &self.csm.ddgi_ray_table,
            ddgi_update_ubo: &self.csm.ddgi_update_ubo,
            // The punctual (spot/point) depth-pass activation (the punctual host rung): built from
            // the SAME ResolvedShadowAtlas the runner memcpys into this slot's atlas UBO (the O1
            // single-matrix pin — LE float bytes == the in-memory f32s the UBO copy carries on
            // x86_64). `face_is_point[s]` is fed DIRECTLY from `r.face_point_mask` (the resolve's
            // single source of truth — W4), NOT re-derived from `inv_range` or contiguity.
            atlas_punctual: atlas.map(|r| {
                debug_assert!(
                    r.mode_word == 1 && r.active_layers > 0,
                    "invariant: the punctual arming predicate passes a fitted selection"
                );
                // Cross-crate const pins: the host atlas texture + UBO were created at these
                // dimensions/slot budget, which MUST equal the render crate's fit shape.
                debug_assert_eq!(
                    SPOT_SHADOW_DIM, SHADOW_DIM,
                    "invariant: host atlas dim == boyko_render::SHADOW_DIM"
                );
                debug_assert_eq!(
                    SPOT_ATLAS_SLOTS as usize, M_SLOTS,
                    "invariant: host atlas slots == boyko_render::M_SLOTS"
                );
                let count = (r.active_layers as usize).min(r.faces.len());
                let mut face_view_proj = [[0u8; 64]; M_SLOTS];
                let mut face_is_point = [false; M_SLOTS];
                let mut face_light = [[0u8; 16]; M_SLOTS];
                for s in 0..count {
                    let face = &r.faces[s];
                    // COLUMN-MAJOR LE bytes (the O1 single-matrix pin — the depth VS + resolve read
                    // the SAME per-slot matrix; on x86_64 these bytes == the f32s the UBO copy holds).
                    for (col, column) in face.view_proj.iter().enumerate() {
                        for (row, f) in column.iter().enumerate() {
                            let at = col * 16 + row * 4;
                            face_view_proj[s][at..at + 4].copy_from_slice(&f.to_le_bytes());
                        }
                    }
                    // W4 single source of truth: bit `s` of `face_point_mask` ⇒ a POINT cube face.
                    face_is_point[s] = (r.face_point_mask >> s) & 1 != 0;
                    // The POINT-face `cam_eye@64` push lane: `light_pos.xyz` (12 B) + `inv_range` (4 B).
                    // Unused (but harmless) for SPOT faces.
                    face_light[s][0..4].copy_from_slice(&face.light_pos[0].to_le_bytes());
                    face_light[s][4..8].copy_from_slice(&face.light_pos[1].to_le_bytes());
                    face_light[s][8..12].copy_from_slice(&face.light_pos[2].to_le_bytes());
                    face_light[s][12..16].copy_from_slice(&face.inv_range.to_le_bytes());
                }
                // The 88-byte depth-pass push TEMPLATE: `use_model_matrix == 1` (@84). The recorder
                // overwrites `view_proj` (@0..64) per slot, `cam_eye` (@64..80) per POINT slot, and
                // `base_instance` (@80) per caster batch.
                let mut push = [0u8; GBUFFER_PUSH_BYTES];
                push[84..88].copy_from_slice(&1u32.to_le_bytes());
                PunctualDepthActivation {
                    pipeline: &self.csm.depth_pipeline,
                    point_pipeline: &self.csm.point_depth_pipeline,
                    push,
                    face_view_proj,
                    face_is_point,
                    face_light,
                    active_layers: count as u32,
                    shadow_dim: SPOT_SHADOW_DIM,
                }
            }),
            // The B3 interp activation (host plan R5, refined-B): Some(_) on a frame
            // the gather produced DYNAMIC instances. Its model_out target is the SHARED
            // instance ring the raster VS reads (instance_bind_group ==
            // instance_bind_groups[slot], unchanged) — the compute overwrites the
            // dynamic slots in place before the raster pass.
            interp,
            // HW-RT rung R0: GPU timestamp instrumentation OFF on every host frame (byte-
            // identical command stream — the offline `software_ray_baseline_cost` harness is
            // the only `Some` caller).
            gpu_timing: None,
            // HW-RT rung R2a-3: the GPU-resident per-frame TLAS pack + build activation, armed
            // above from `tlas_enabled` + `self.tlas`. `None` on every non-RT / OFF frame (the
            // byte-identical path — the TLAS is built + barriered but never traced this rung).
            #[cfg(feature = "hwrt")]
            tlas,
            // HW-RT rung 3a step 7: the spatial (à-trous) RT soft-shadow denoise activation,
            // computed above from the per-frame gate `mode == Spatial && backend == HardwareTri
            // && has_primary_directional && tlas_nonempty`. `None` (the DEFAULT — any gate
            // condition false) ⇒ NO VIS / à-trous passes recorded + the resolve stays
            // RESOLVE_INLINE-hwrt ⇒ the render is BYTE-IDENTICAL. `Some(_)` records the VIS
            // pre-pass + the `levels` à-trous passes and binds the DENOISED resolve.
            #[cfg(feature = "hwrt")]
            shadow,
            // HW-RT rung 3a: the STABLE denoise-set-build signals. Populated from the boot
            // resources REGARDLESS of the per-frame `shadow` gate above, so the resolve/à-trous
            // sets are written ONCE per extent at `create` (where `shadow` is still `None`) — the
            // decoupling that removes the record-time `None`-set panic. `resolve_layout_denoise_hwrt`
            // / `atrous_layout_denoise_hwrt` are `Some` iff the boot pipelines exist (an RT + hwrt
            // device); `shadow_denoise_enabled` gates the actual build (so a mode-`None` world still
            // builds NO sets → byte-identical); `shadow_denoise_final_is_vis2` uses the SAME
            // `clamped_levels() % 2 == 1` parity the record + graph + activation use (W1).
            #[cfg(feature = "hwrt")]
            resolve_layout_denoise_hwrt: self
                .shadow_denoise_pipelines
                .as_ref()
                .map(|(_, _, layout)| layout),
            #[cfg(feature = "hwrt")]
            atrous_layout_denoise_hwrt: self
                .shadow_atrous_pipeline
                .as_ref()
                .map(|(_, layout)| layout),
            #[cfg(feature = "hwrt")]
            shadow_denoise_enabled: denoise_armed,
            #[cfg(feature = "hwrt")]
            shadow_denoise_final_is_vis2: atrous_levels % 2 == 1,
            // HW-RT Rung 3b step 5a: the MESH motion-vector gate + refs. `temporal_enabled` is the
            // author's `mode ∈ {Temporal, Both}`; the pipeline/bind-group refs are `Some` only when
            // `self.mv` exists (an RT + storage device) AND temporal is on — so a temporal-OFF frame
            // (or a device without the MV resources) passes `None` and the recorder takes the base
            // 3-MRT raster pipeline (byte-identical). This frame's bind group is `bind_groups[slot]`
            // (the FENCED slot — its instance/prev/motion-cam rings the runner just wrote).
            #[cfg(feature = "hwrt")]
            temporal_enabled,
            #[cfg(feature = "hwrt")]
            raster_pipeline_mv: (temporal_enabled)
                .then(|| self.mv.as_ref().map(|m| &m.pipeline))
                .flatten(),
            #[cfg(feature = "hwrt")]
            mv_bind_group: (temporal_enabled)
                .then(|| self.mv.as_ref().map(|m| &m.bind_groups[slot]))
                .flatten(),
            // F8-mv: the combined MV+PM refs. `Some` iff BOTH temporal AND a non-default
            // material are active this frame (a strict superset of `mesh_mvpm_active()`'s
            // other conditions) AND `self.mv` exists (an RT + storage device) — mirrors the
            // pure-MV shape above (`.then(...).flatten()`, not `.then_some`, so
            // `self.mv.is_none()` yields `None`).
            #[cfg(feature = "hwrt")]
            raster_pipeline_mvpm: (temporal_enabled && any_non_default_material)
                .then(|| self.mv.as_ref().map(|m| &m.mvpm_pipeline))
                .flatten(),
            #[cfg(feature = "hwrt")]
            mvpm_bind_group: (temporal_enabled && any_non_default_material)
                .then(|| self.mv.as_ref().map(|m| &m.mvpm_bind_groups[slot]))
                .flatten(),
            // HW-RT Rung 3b step 5b: the SDF motion-vector VIS resolve refs. The pipeline is the
            // recorder's ref — gated on `temporal_enabled` (mirrors `raster_pipeline_mv`) so it is
            // `Some` only on a temporal frame with the MV resources. The layout + the motion-cam UBO
            // ring are STABLE build-time refs (mirror `resolve_layout_denoise_hwrt`): populated
            // whenever `self.mv` exists (an RT + storage device), REGARDLESS of the per-frame gate,
            // so `GBufferTargets::build_shadow_vis_mv_resolve_set` can write the per-FIF VIS-MV set
            // once per extent. `sdf_mv_active()` folds `temporal_enabled` back in. `None` (temporal
            // off / non-storage device) ⇒ the recorder takes the base VIS path (byte-identical).
            #[cfg(feature = "hwrt")]
            vis_mv_pipeline: (temporal_enabled)
                .then(|| self.mv.as_ref().map(|m| &m.vis_mv_pipeline))
                .flatten(),
            #[cfg(feature = "hwrt")]
            vis_mv_layout: self.mv.as_ref().map(|m| &m.vis_mv_layout),
            #[cfg(feature = "hwrt")]
            motion_cam_ubo_ring: self.mv.as_ref().map(|m| &m.motion_cam_ubo),
            // HW-RT Rung 3b step 6: the STABLE 8-binding temporal reproject layout (from the boot
            // temporal pipeline, REGARDLESS of the per-frame gate — mirrors `resolve_layout_denoise_hwrt`),
            // threaded so `GBufferTargets::build_shadow_temporal_sets` writes the per-FIF temporal set
            // once per extent. `None` on a non-RT device.
            #[cfg(feature = "hwrt")]
            temporal_layout: self.shadow_temporal_pipeline.as_ref().map(|(_, l)| l),
            // Asset-streaming plan F8: the PER_INSTANCE_MATERIAL gate + refs. `pm_enabled` is
            // the per-frame `any_non_default_material` read; the pipeline/bind-group refs are
            // `Some` iff `pm_enabled` (belt-and-suspenders — the PM pipeline/rings are ALWAYS
            // built at boot, unlike `mv`). `false`/`None` on every all-default scene (the
            // goldens) ⇒ the recorder binds the FROZEN base pipeline (byte-identity by
            // construction, F8 §2.4). This frame's bind group is `pm_bind_groups[slot]` (the
            // FENCED slot — its instance/material rings the runner just wrote/grew).
            pm_enabled: any_non_default_material,
            raster_pipeline_pm: any_non_default_material.then_some(&self.raster_pipeline_pm),
            pm_bind_group: any_non_default_material.then(|| &self.pm_bind_groups[slot]),
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

    /// HW-RT rung R2a-3: the FENCED slot's mesh-id SSBO — the write target of the runner's
    /// per-frame [`upload_mesh_ids`](boyko_render::upload_mesh_ids) (the TLAS packer reads the
    /// same slot at binding 1). Returns `None` on a non-RT device (`self.tlas` absent). The
    /// sibling in-flight frame binds the OTHER slot — the same lock-free discipline as the ring.
    #[cfg(feature = "hwrt")]
    #[inline]
    pub(crate) fn mesh_id_slot(&self, slot: usize) -> Option<&BoundBuffer> {
        self.tlas.as_ref().map(|t| t.mesh_id_slot(slot))
    }

    /// HW-RT rung R2a-3: rewrites the frame-invariant BLAS-address table from `mesh_assets` IFF its
    /// `install_epoch` advanced (asset-streaming plan F6 — a no-op on the steady, never-retiring
    /// per-frame path; see [`TlasResources::sync_blas_addr`]'s doc for why row-count growth alone
    /// cannot gate this). No-op on a non-RT device (`self.tlas` absent). Called by the runner
    /// before `scene()` on an RT device.
    #[cfg(feature = "hwrt")]
    #[inline]
    pub(crate) fn sync_tlas_blas_addr(&self, device: &VulkanContext, mesh_assets: &Assets<MeshGpu>) {
        if let Some(t) = self.tlas.as_ref() {
            t.sync_blas_addr(device, mesh_assets);
        }
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

    /// The FENCED slot's shadow-atlas-UBO ring buffer — the write target of the runner's
    /// per-frame [`upload_atlas_ring`](boyko_render::upload_atlas_ring) (the resolve binds the
    /// same slot at binding 15, so the sibling in-flight frame reads the OTHER slot — the
    /// lock-free write-after-read discipline the ring exists for).
    #[inline]
    pub(crate) fn atlas_ubo_slot(&self, slot: usize) -> &BoundBuffer {
        &self.csm.atlas_ubo[slot]
    }

    /// HW-RT rung 1b: the FENCED slot's HWRT shadow-params-UBO ring buffer — the write target of
    /// the runner's per-frame [`upload_ray_shadow_ring`](boyko_render::upload_ray_shadow_ring)
    /// (the HWRT resolve binds the same slot at binding 20, so the sibling in-flight frame reads
    /// the OTHER slot — the lock-free write-after-read discipline the ring exists for). Callable
    /// ONLY on an RT device (the ring is `Some` under `ray_query_enabled`); the runner gates the
    /// call on the SAME `feature = "hwrt"` + `ray_query_enabled()` condition, so the `expect`
    /// never fires (an unminted-ring call would be a runner bug).
    #[cfg(feature = "hwrt")]
    #[inline]
    pub(crate) fn ray_shadow_ubo_slot(&self, slot: usize) -> &BoundBuffer {
        &self
            .ray_shadow_ubo
            .as_ref()
            .expect("invariant: ray_shadow_ubo ring is minted on an RT device (ray_query_enabled)")
            [slot]
    }

    /// HW-RT Rung 3b step 5a: the FENCED slot's PREV-instance ring + motion-cam UBO — the write
    /// targets of the runner's per-frame
    /// [`upload_prev_instance_models`](boyko_render::upload_prev_instance_models) +
    /// [`upload_motion_cam_ring`](boyko_render::upload_motion_cam_ring) when temporal is on. Returns
    /// `None` on a device without the MV resources (`self.mv` absent — non-RT / non-storage), so the
    /// runner skips the writes. The MV bind group binds these SAME slots @1/@2; the sibling in-flight
    /// frame binds the OTHER slot — the same lock-free discipline as the instance ring.
    #[cfg(feature = "hwrt")]
    #[inline]
    pub(crate) fn motion_vec_slots(&self, slot: usize) -> Option<(&BoundBuffer, &BoundBuffer)> {
        self.mv
            .as_ref()
            .map(|m| (&m.prev_instance_rings[slot], &m.motion_cam_ubo[slot]))
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
            // HW-RT rung R2a-4b: the HWRT resolve pipeline + its 21-binding layout, `Option`-guarded
            // (present only on an RT device under `feature = "hwrt"`). Pipeline before layout.
            #[cfg(feature = "hwrt")]
            if let Some((pipeline, layout)) = self.resolve_pipeline_hwrt {
                RhiDevice::destroy_compute_pipeline(ctx, pipeline);
                RhiDevice::destroy_bind_group_layout(ctx, layout);
            }
            // HW-RT rung 3a: the VIS + DENOISED resolve pipelines + their shared 22-binding layout,
            // and the à-trous filter pipeline + its 6-binding layout. `Option`-guarded (present only
            // on an RT device). Pipelines before their layout.
            #[cfg(feature = "hwrt")]
            if let Some((vis, denoised, layout)) = self.shadow_denoise_pipelines {
                RhiDevice::destroy_compute_pipeline(ctx, vis);
                RhiDevice::destroy_compute_pipeline(ctx, denoised);
                RhiDevice::destroy_bind_group_layout(ctx, layout);
            }
            #[cfg(feature = "hwrt")]
            if let Some((pipeline, layout)) = self.shadow_atrous_pipeline {
                RhiDevice::destroy_compute_pipeline(ctx, pipeline);
                RhiDevice::destroy_bind_group_layout(ctx, layout);
            }
            // HW-RT Rung 3b step 6: the temporal reproject pipeline + its 8-binding layout,
            // `Option`-guarded (present only on an RT device). Pipeline before layout.
            #[cfg(feature = "hwrt")]
            if let Some((pipeline, layout)) = self.shadow_temporal_pipeline {
                RhiDevice::destroy_compute_pipeline(ctx, pipeline);
                RhiDevice::destroy_bind_group_layout(ctx, layout);
            }
            // HW-RT rung 1b: the HWRT soft-shadow-params UBO ring, `Option`-guarded (minted only on
            // an RT device). Each slot is a plain host-coherent buffer (no dependents).
            #[cfg(feature = "hwrt")]
            if let Some(ring) = self.ray_shadow_ubo {
                for b in ring {
                    RhiDevice::destroy_buffer(ctx, b);
                }
            }
            RhiDevice::destroy_compute_pipeline(ctx, self.marcher);
            RhiDevice::destroy_bind_group_layout(ctx, self.vocab_layout);
            RhiDevice::destroy_graphics_pipeline(ctx, self.raster_pipeline);
            // Asset-streaming plan F8: the PM resources bind the SHARED `instance_rings` (@0)
            // plus their own instance-material ring (@1); torn down BEFORE the shared instance
            // bind groups/rings below (bind groups first, then the pipeline/layout, then the
            // owned buffers) — mirrors the MV teardown ordering. NOT `#[cfg(feature = "hwrt")]`
            // (built unconditionally at boot).
            for bg in self.pm_bind_groups {
                RhiDevice::destroy_bind_group(ctx, bg);
            }
            RhiDevice::destroy_graphics_pipeline(ctx, self.raster_pipeline_pm);
            RhiDevice::destroy_bind_group_layout(ctx, self.pm_instance_material_layout);
            for b in self.pm_instance_material_rings {
                RhiDevice::destroy_buffer(ctx, b);
            }
            // HW-RT Rung 3b step 5a: the MESH motion-vector resources bind the SHARED
            // `instance_rings` (@0) plus their own prev-instance + motion-cam UBO rings; torn down
            // BEFORE the shared instance bind groups/rings below (bind groups first, then the
            // pipeline/layout, then the owned buffers). `None` on a non-RT / non-storage device
            // (no-op).
            #[cfg(feature = "hwrt")]
            if let Some(mv) = self.mv {
                // F8-mv: the combined mvpm bind groups/pipeline/layout, torn down BEFORE the
                // pure-MV cluster below (bind groups first, then the pipeline/layout) — mirrors
                // this whole block's ordering discipline.
                for bg in mv.mvpm_bind_groups {
                    RhiDevice::destroy_bind_group(ctx, bg);
                }
                RhiDevice::destroy_graphics_pipeline(ctx, mv.mvpm_pipeline);
                RhiDevice::destroy_bind_group_layout(ctx, mv.mvpm_layout);
                for bg in mv.bind_groups {
                    RhiDevice::destroy_bind_group(ctx, bg);
                }
                RhiDevice::destroy_graphics_pipeline(ctx, mv.pipeline);
                RhiDevice::destroy_bind_group_layout(ctx, mv.layout);
                // step 5b: the SDF motion-vector VIS resolve pipeline + its 24-binding layout
                // (pipeline before layout; no bind groups — the VIS-MV set lives in `GBufferTargets`).
                RhiDevice::destroy_compute_pipeline(ctx, mv.vis_mv_pipeline);
                RhiDevice::destroy_bind_group_layout(ctx, mv.vis_mv_layout);
                for b in mv.motion_cam_ubo {
                    RhiDevice::destroy_buffer(ctx, b);
                }
                for b in mv.prev_instance_rings {
                    RhiDevice::destroy_buffer(ctx, b);
                }
            }
            // HW-RT rung R2a-3: the TLAS cluster binds the SHARED `instance_rings` at @0 (plus
            // its own mesh-id / instance-array / blas-addr buffers + the persistent per-slot
            // TLASes); torn down FIRST (before the interp cluster + the shared rings below), the
            // AS-before-backing order internal to `destroy`. `None` on a non-RT device (no-op).
            #[cfg(feature = "hwrt")]
            if let Some(t) = self.tlas {
                t.destroy(ctx);
            }
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
            // Asset-system rung A1: the material table is now World-owned
            // (`MaterialTable`); `boyko_app::runner`'s teardown destroys it
            // separately, AFTER this fn returns (mirrors `Assets<MeshGpu>`'s teardown
            // slot) — destroying it here too would double-free.
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
