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
    ImageUsage, MemoryLocation, MipMode, PrimitiveTopology, QueryPoolDesc, RhiCommandEncoder,
    RhiDevice, RhiQueue, SamplerDesc, ShaderStage, TextureDesc, TextureDimension, VertexAttribute,
    VertexBufferLayout, VertexFormat,
};
#[cfg(feature = "hwrt")]
use boyko_rhi::SpecConstant;
use boyko_rhi_vulkan::brick_atlas::BrickClipmap;
use boyko_rhi_vulkan::ddgi::DdgiAtlas;
use boyko_rhi_vulkan::compute::{
    B5_CAMERA_UBO_BYTES_M4, CAM_MODE_ORTHO, CAM_MODE_PERSPECTIVE, COMPOSITE_PUSH_CONSTANT_BYTES,
    CLUSTER_CULL_HIER_PUSH_BYTES, CLUSTER_CULL_PUSH_BYTES, ClusterCullHierPush, ClusterCullPush,
    CoarseMode, EDITLIST_BUFFER_WORDS,
    INTERP_INSTANCES_PUSH_BYTES, LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS,
    LOCAL_SIZE_X, M4_LEVEL_PARAMS_BYTES, RCAS_PUSH_BYTES, SDF_FORWARD_MARCH_PUSH_BYTES,
    TILE_BOUND_BYTES, TILE_SIZE,
    cluster_cull_hier_spirv, cluster_cull_spirv,
    csm_depth_fs_spirv, csm_depth_vs_spirv, deferred_pbr_spirv,
    deferred_pbr_wrap_spirv,
    depth_prepass_fs_spirv, depth_prepass_vs_spirv,
    encode_edit_list, forward_opaque_fs_spirv, forward_opaque_froxel_fs_spirv, forward_opaque_vs_spirv,
    forward_sky_fs_spirv, forward_sky_vs_spirv, fullscreen_sample_fs_spirv,
    fullscreen_sample_vs_spirv, fxaa_fs_spirv,
    gbuffer_mrt_fs_spirv,
    gbuffer_mrt_pm_fs_spirv, gbuffer_mrt_pm_vs_spirv, gbuffer_mrt_tex_fs_spirv,
    gbuffer_mrt_tex_vs_spirv, gbuffer_mrt_vs_spirv, interp_instances_spirv, punctual_depth_fs_spirv,
    punctual_depth_vs_spirv, rcas_spirv, sdf_forward_march_spirv, sdf_forward_march_sdfonly_spirv,
    sdf_forward_march_sdfonly_viewt_spirv, sdf_forward_march_viewt_spirv,
    sdf_gbuffer_composite_spirv, sdf_probe_update_spirv,
    sdf_ssao_spirv_variant, smaa_blend_fs_spirv, smaa_edge_fs_spirv, smaa_weight_fs_spirv,
    SSAO_ATROUS_PUSH_BYTES, ssao_atrous_read8_spirv, ssao_atrous_spirv, ssao_atrous_write8_spirv,
    SSAO_QUALITY_COUNT, SSAO_QUALITY_HIGH, SSAO_QUALITY_LOW, SSAO_QUALITY_MEDIUM,
    ssaa_downsample_fs_spirv, taa_resolve_spirv, tile_grid_extent, VIEWT_FROM_DEPTH_PUSH_BYTES,
    VIEWT_FROM_DEPTH_RZ_PUSH_BYTES,
    vb_classify_count_spirv, vb_classify_scan_spirv, vb_classify_scatter_spirv,
    vb_geo_spirv, vb_raster_fs_spirv, vb_raster_vs_spirv, vb_resolve_spirv, vb_resolve_froxel_spirv,
    sdf_ssao_vb_spirv, vb_shade_spirv, vb_shade_froxel_spirv, vb_shade_split_spirv, vb_shade_split_tex_spirv,
    vb_shade_tex_spirv, vb_shade_tex_froxel_spirv, viewt_from_depth_spirv, viewt_from_depth_rz_spirv,
};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::ffi::VkDescriptorSet;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::rhi_impl::{
    ComputePipeline, VulkanBindGroup, VulkanBindGroupLayout, VulkanGraphicsPipeline,
    VulkanQueryPool, VulkanSampler, VulkanShaderModule, rebind_storage_buffer,
};
use boyko_rhi_vulkan::swapchain::{
    AaActivation, ClusterCullHierDispatch, CsmDepthActivation, DdgiUpdateActivation,
    FRAMES_IN_FLIGHT, FrameWriteToken, GBUFFER_INSTANCE_MODEL_BYTES, GBUFFER_PUSH_BYTES,
    GBufferMeshDraw, GBufferScene, InterpActivation, PunctualDepthActivation, RcasActivation,
    ResolvedRenderPathGpu, SV0_PASS_COUNT, SmaaActivation, SsaaActivation, SsaoActivation,
    Sv0TimedPass, Sv0TimestampCollector, TaaActivation, VB_PASS_COUNT, VbTimedPass,
    VbTimestampCollector, ViewtFromDepthActivation, ViewtFromVbDepthActivation,
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
    gbuffer_mrt_mvpm_vs_spirv, shadow_atrous_spirv, shadow_temporal_spirv, vb_geo_mv_spirv,
    vb_shade_split_hwrt_spirv, vb_shade_split_tex_hwrt_spirv, vb_shadow_vis_spirv,
};
#[cfg(feature = "hwrt")]
use boyko_rhi_vulkan::swapchain::{ShadowVisActivation, TlasBuildActivation};
use boyko_rhi_vulkan::texture::VulkanTexture;
use boyko_sdf_math::SdfEdit;

use boyko_render::{
    AREA_TEX_BYTES, AREA_TEX_H, AREA_TEX_W, AaMode, BindlessTextureTable, ClusterConfig,
    DDGI_UPDATE_UBO_BYTES,
    DdgiConfig, DdgiUpdateConfig, DdgiUpdateUbo, GI_MAX_RAYS, GPU_LIGHT_BYTES, GPU_LIGHT_WORDS,
    GPU_TRANSFORM3D_BYTES, GpuLight, LIGHT_HEADER_BASE_WORDS, LIGHT_HEADER_BYTES, LightHeaderGpu,
    LightingConfig, M_SLOTS, MAX_LIGHTS, MaterialTable, MESH_VERTEX_STRIDE,
    PER_INSTANCE_MATERIAL_BYTES, PER_INSTANCE_MATERIAL_TEX_BYTES, RESOLVED_CSM_BYTES,
    RESOLVED_DDGI_BYTES, RESOLVED_SHADOW_ATLAS_BYTES, RETIRE_DELAY, ResolvedCsm,
    ResolvedShadowAtlas, RetiredGpuBuffers, SEARCH_TEX_BYTES, SEARCH_TEX_H, SEARCH_TEX_W,
    SHADOW_DIM, SharpenMode, Vertex, ddgi_update_dispatch_groups, fill_fibonacci_ray_table,
    mesh_view_t_norm, pack_ddgi_update_ubo, resolve_ddgi, upload_texture_2d_raw,
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

/// Textured-PBR T6c (review O3): the boot capacity of
/// [`TexturedResources::tex_instance_material_rings`] — an INDEPENDENT literal (not
/// [`INSTANCE_CAPACITY`] itself) so the const-assert immediately below actually guards
/// drift: the tex ring does NOT participate in F7 growth (see `TexturedResources`'s
/// doc), so the two are pinned EQUAL today by design — a future edit to either literal
/// alone is now a BUILD ERROR, not a silent capacity mismatch.
/// `upload_instance_materials_tex`'s own overflow `assert!` compares against the
/// ACTUAL device buffer's `size`, so this pin is not load-bearing for that check —
/// only for keeping the two boot budgets in sync.
const TEX_INSTANCE_CAPACITY: usize = 1024;
const _: () = assert!(
    TEX_INSTANCE_CAPACITY == INSTANCE_CAPACITY,
    "TEX_INSTANCE_CAPACITY must track INSTANCE_CAPACITY (T6c: the TEXTURED \
     instance-material ring does not participate in F7 growth, so it is pinned to the \
     boot instance budget, not a separately-tunable capacity)"
);

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

/// Textured-PBR T6c: the `gPbr` 4th MRT color format the TEXTURED raster pipeline declares
/// — MUST equal `GBufferTargets`'s `gPbr` ring format (T6a's `R16G16B16A16_SFLOAT`).
const TEX_GPBR_COLOR_FORMAT: Format = Format::R16G16B16A16Sfloat;

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

/// Multi-paradigm render-path plan, rung R1: converts the `boyko_render::ResolvedRenderPath`
/// boot carrier into its plain-POD [`ResolvedRenderPathGpu`] mirror for [`GBufferScene`] —
/// THE `boyko_render` → `boyko_rhi_vulkan` boundary-crossing seam (see
/// [`GpuSceneBundles::scene`]'s `resolved_render_path` param doc for why this is a free fn,
/// not a `From` impl: neither `ResolvedRenderPath` nor `ResolvedRenderPathGpu` is local to
/// this crate, so a `From` impl anywhere in `boyko_app` would violate the orphan rule).
/// Field-by-field, no allocation, no branch beyond the `#[repr(u32)]`/newtype `as`/`.bits()`
/// casts — mirrors how `pack_ddgi_update_ubo` packs a `boyko_render` carrier into its device
/// byte-mirror.
#[inline]
fn to_gpu_resolved_render_path(r: &boyko_render::ResolvedRenderPath) -> ResolvedRenderPathGpu {
    ResolvedRenderPathGpu {
        path: r.path as u32,
        legs: r.legs as u32,
        mesh_leg: r.mesh_leg,
        sdf_leg: r.sdf_leg,
        sdf_forward_marched: r.sdf_forward_marched,
        needs_depth_prepass: r.needs_depth_prepass,
        prepass_writes_motion: r.prepass_writes_motion,
        mesh_geo_shade_split: r.mesh_geo_shade_split,
        sdf_geo_shade_split: r.sdf_geo_shade_split,
        sdf_surface_cache: r.sdf_surface_cache,
        vb_geometry_table: r.vb_geometry_table,
        depth_kind: r.depth_kind as u32,
        thin_aux: r.thin_aux.bits(),
        shadow: r.shadow.bits(),
        froxel_light_cull: r.froxel_light_cull,
    }
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
    /// (64-byte vertex layout, D32 depth, 88-byte VERTEX push, `CullMode::None`, no blend/bias)
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

#[cfg(feature = "hwrt")]
impl MotionVecResources {
    /// Asset-streaming plan F7-hwrt (task#11): grows slot `s`'s `prev_instance_rings[s]`
    /// to `new_cap` instances (48 B × `new_cap`, zero-filled, old buffer deferred) in
    /// lockstep with the caller's ALREADY-grown `instance_rings_s`/`pm_ring_s` — the SAME
    /// buffers [`GpuSceneBundles::grow_shared_instance_rings`] just repointed
    /// `instance_bind_groups[s]`/`pm_bind_groups[s]` against, passed here BY REFERENCE
    /// (this struct never owns or grows them itself). Rebinds `bind_groups[s]` (@0
    /// current / @1 prev — @2 `motion_cam_ubo` untouched) and `mvpm_bind_groups[s]` (@0
    /// current / @1 prev / @3 pm — @2 `motion_cam_ubo` untouched).
    ///
    /// # No seed
    ///
    /// `upload_prev_instance_models` rewrites the whole prev-ring THIS frame when the
    /// temporal gate is armed (mirrors [`GpuSceneBundles::grow_shared_instance_rings`]'s
    /// own no-seed reasoning) — zero-fill only covers the gap until that write lands.
    ///
    /// # Panics
    ///
    /// Panics (`expect`) on an RHI create/map failure — a device OOM on a post-boot grow
    /// is setup-adjacent, not a recoverable per-frame error.
    ///
    /// # Safety
    ///
    /// The caller guarantees slot `s`'s in-flight fence was waited THIS frame (the
    /// `FrameWriteToken` proof [`GpuSceneBundles::grow_instance_family_rt`] holds) —
    /// neither `bind_groups[s]` nor `mvpm_bind_groups[s]` is command-buffer-pending, so
    /// rewriting their bindings in place is sound. `instance_rings_s`/`pm_ring_s` are the
    /// caller's live, already-grown buffers.
    // Review O2 pins `instance_rings_s`/`pm_ring_s` as SEPARATE borrowed params (not
    // grown here, not cloned) so the caller's split-borrow of `&mut self.mv` against
    // `&self.instance_rings[s]`/`&self.pm_instance_material_rings[s]` stays disjoint —
    // grouping them into a struct would only relocate the same two fields behind an
    // extra indirection with no other caller to share it.
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn grow_slot(
        &mut self,
        device: &VulkanContext,
        s: usize,
        new_cap: u32,
        instance_rings_s: &BoundBuffer,
        pm_ring_s: &BoundBuffer,
        retired: &mut RetiredGpuBuffers,
        retire_frame: u64,
    ) {
        let prev_bytes = new_cap as u64 * GBUFFER_INSTANCE_MODEL_BYTES as u64;
        let new_prev = {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: prev_bytes,
                    usage: BufferUsage::STORAGE,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: grown prev-instance-model SSBO ring slot create");
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .expect("invariant: host-visible grown prev-instance SSBO is mapped");
            zero_fill(mapped, prev_bytes as usize);
            b
        };
        let old_prev = core::mem::replace(&mut self.prev_instance_rings[s], new_prev);
        retired.push(old_prev, retire_frame);

        const MV_GROWN_BINDINGS: usize = 5;
        let mut rebound = 0usize;
        // SAFETY: slot `s`'s fence was waited this frame (this fn's caller contract
        // above) — neither set is command-buffer-pending, so rewriting their bindings in
        // place is sound. `motion_cam_ubo[s]` (@2 on both sets) is untouched: it does not
        // share the instance-family index space and never grows.
        unsafe {
            rebind_storage_buffer(device, &self.bind_groups[s], 0, instance_rings_s);
            rebound += 1;
            rebind_storage_buffer(device, &self.bind_groups[s], 1, &self.prev_instance_rings[s]);
            rebound += 1;
            rebind_storage_buffer(device, &self.mvpm_bind_groups[s], 0, instance_rings_s);
            rebound += 1;
            rebind_storage_buffer(device, &self.mvpm_bind_groups[s], 1, &self.prev_instance_rings[s]);
            rebound += 1;
            rebind_storage_buffer(device, &self.mvpm_bind_groups[s], 3, pm_ring_s);
            rebound += 1;
        }
        debug_assert_eq!(
            rebound, MV_GROWN_BINDINGS,
            "invariant: exactly 5 mv/mvpm bindings rebound (bind_groups@0/@1 + \
             mvpm_bind_groups@0/@1/@3; @2 motion_cam_ubo untouched on both)"
        );
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
    /// Asset-streaming plan F7 §7.3: PER-FIF-SLOT current capacity of the instance family
    /// (`instance_rings[s]` + `interp.pairs[s]` + `interp.out_slot[s]`, plus — on the RT
    /// leg — `tlas`'s + `mv`'s co-sized buffers), starting at [`INSTANCE_CAPACITY`]. Slots
    /// grow INDEPENDENTLY — one fenced slot at a time, in lockstep across every co-sized
    /// buffer — so this is a per-slot array, not a single scalar (mirrors
    /// [`InterpGpuProd`]'s own per-slot `capacity`). Asset-streaming plan F7-hwrt
    /// (task#11): the RT leg (`tlas.is_some()`) now ALSO grows this past
    /// [`INSTANCE_CAPACITY`] via [`Self::grow_instance_family_rt`] — the former W3 hard
    /// cap (pinning this forever at boot capacity on an RT device) is REMOVED; both legs
    /// share the SAME [`MAX_INSTANCE_CAP`] ceiling (no separate RT ceiling).
    instance_capacity: [u32; FRAMES_IN_FLIGHT],
    /// Asset-streaming plan F7-hwrt (task#11): `true` iff slot `s`'s [`TlasResources`]
    /// minted a NEW `VkAccelerationStructureKHR` handle this grow (via
    /// [`Self::grow_instance_family_rt`]) whose resolve-family descriptor sets have not
    /// yet been repointed. The runner's per-frame repoint step (mirrors
    /// [`MaterialTable::rebind_pending`](boyko_render::MaterialTable::rebind_pending)'s
    /// FIX-E discipline) `core::mem::take`s this flag and drives
    /// [`GBufferFrame::repoint_tlas_accel`](boyko_rhi_vulkan::present::GBufferFrame::repoint_tlas_accel)
    /// — gated ONLY on this flag, never on "grew this frame", so a slot left lagging by a
    /// prior grow still converges.
    #[cfg(feature = "hwrt")]
    pub(crate) tlas_accel_rebind_pending: [bool; FRAMES_IN_FLIGHT],
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
    /// OFF-path upload cost). Grows in LOCKSTEP with [`Self::instance_rings`] via
    /// [`Self::grow_shared_instance_rings`] on BOTH legs (asset-streaming plan F7-hwrt,
    /// task#11 — the former RT hard cap is removed) — its index space is IDENTICAL to
    /// `instance_rings`, so the two MUST share capacity at all times (a divergent
    /// capacity would OOB the instant the instance ring grows).
    pub(crate) pm_instance_material_rings: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// Asset-streaming plan F8: per-FIF bind groups against
    /// [`Self::pm_instance_material_layout`]: slot `i` binds `{ instance_rings[i] @0,
    /// pm_instance_material_rings[i] @1 }`. The recorder binds slot `s` at set 0 when the
    /// PM pipeline is selected.
    pm_bind_groups: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// Textured-PBR T6c: the TEXTURED gbuffer producer pipeline resources, built LAZILY via
    /// [`Self::build_textured_resources`] (called from `run_windowed` AFTER the bindless
    /// texture-array table exists — its fallible create is deferred past `boot()`/
    /// `finish()`, see `runner.rs`'s boot-order comment). `None` until that call lands (or
    /// permanently, if the bindless table create failed and the runner already tore down
    /// before reaching it); `Some` for the whole remaining process lifetime afterward.
    tex: Option<TexturedResources>,
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
    /// Multi-paradigm render-path plan, rung R-SDFFWD: the `sdf_forward_march` pass's dedicated
    /// BrickLevels UBO (Set-0 binding 11, `BRICK_LEVELS * M4_LEVEL_PARAMS_BYTES` = 144 B) — a
    /// STANDALONE buffer, distinct from `camera_ring`'s own M4Level tail (this pass's Camera @3
    /// stays the plain 80-byte Forward shape). Zero-seeded, single (NOT ringed — never rewritten:
    /// `brick_enabled = brick_trilinear = brick_levels = 0` every frame this rung, an explicit
    /// 0%-gate mirroring the deferred marcher's own first-landed M1/M2/M4 activation).
    brick_levels_ubo: BoundBuffer,
    // ── Resolve (pass C) ─────────────────────────────────────────────────────
    resolve_pipeline: ComputePipeline,
    resolve_layout: VulkanBindGroupLayout,
    /// Render terminator-softening: the SOFTWARE-RESOLVE-ONLY diffuse light-wrap variant
    /// pipeline (`deferred_pbr_wrap.comp`, `-D TERMINATOR_WRAP=1`), built UNCONDITIONALLY at
    /// boot alongside [`Self::resolve_pipeline`] (mirroring that pipeline's own always-built
    /// discipline — the variant is device-agnostic, not RT-gated). Bound to the SAME
    /// [`Self::resolve_layout`] as the base resolve (the variant changes only the diffuse
    /// accumulation math, no descriptor — no separate layout is built). Selected instead of
    /// [`Self::resolve_pipeline`] by [`Self::scene`]'s `terminator_wrap` gate, ONLY when
    /// `LightingConfig::terminator_softening > 0`; every other frame binds the base pipeline
    /// (the byte-identical 0%-gate — `deferred_pbr.hlsl`'s frozen-base discipline).
    resolve_pipeline_wrap: ComputePipeline,
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
    /// Rung R9d: the VB split's DEDICATED shadow-vis gather compute pipeline (`vb_shadow_vis.comp`),
    /// plus its 7-binding layout { `thin_normal` @0, `gViewT` @1 STORAGE images, `LightTable` @2
    /// STORAGE buffer, the camera UBO @3, the TLAS `AccelerationStructure` @4, the
    /// `ResolvedRayShadow` UBO @5 (reuses [`Self::ray_shadow_ubo`]), `gShadowVis` @6 (W) }. Built
    /// at boot under the SAME `ray_query_enabled` gate as [`Self::shadow_denoise_pipelines`]'s own
    /// VIS pipeline; `None` otherwise. Bound instead of the deferred VIS pipeline only when
    /// `GBufferScene::path_vb_hwrt_shadow()`.
    #[cfg(feature = "hwrt")]
    vb_shadow_vis_pipeline: Option<(ComputePipeline, VulkanBindGroupLayout)>,
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
    /// Anti-aliasing Stage 1: the FXAA fullscreen graphics pipeline
    /// (`fullscreen_sample.vs` + `fxaa.fs`), built unconditionally at boot (like
    /// [`Self::present_pipeline`]) so the mode can flip at runtime. `color_formats[0]`
    /// == `R8G8B8A8_UNORM` (`aa_out`'s format), NOT the swapchain format; reuses
    /// [`Self::present_layout`].
    fxaa_pipeline: VulkanGraphicsPipeline,
    /// Anti-aliasing Stage 1: the dedicated LINEAR/ClampToEdge sampler FXAA's sub-texel
    /// tap needs — DISTINCT from the NEAREST [`Self::present_sampler`].
    fxaa_sampler: VulkanSampler,
    /// Anti-aliasing Stage 2: pass 1 (edge detection) fullscreen graphics pipeline
    /// (`fullscreen_sample.vs` + `smaa_edge.fs`), built unconditionally at boot (like
    /// [`Self::fxaa_pipeline`]). `color_formats[0]` == `R8G8_UNORM` (`smaa_edges`' format);
    /// reuses [`Self::present_layout`] (1 CIS: `lit`).
    smaa_edge_pipeline: VulkanGraphicsPipeline,
    /// Anti-aliasing Stage 2: pass 2 (blending-weight calculation) fullscreen graphics
    /// pipeline (`smaa_weight.fs`). `color_formats[0]` == `R8G8B8A8_UNORM` (`smaa_weights`'
    /// format); layout = [`Self::smaa_weight_layout`] (3 CIS).
    smaa_weight_pipeline: VulkanGraphicsPipeline,
    /// Anti-aliasing Stage 2: pass 3 (neighborhood blending) fullscreen graphics pipeline
    /// (`smaa_blend.fs`). `color_formats[0]` == `R8G8B8A8_UNORM` (`aa_out`'s format — the
    /// same target FXAA's single pass writes); layout = [`Self::smaa_blend_layout`] (2 CIS).
    smaa_blend_pipeline: VulkanGraphicsPipeline,
    /// Anti-aliasing Stage 2: the 3-CIS bind-group LAYOUT `{ edges @0, areaTex @1, searchTex
    /// @2 }` [`Self::smaa_weight_pipeline`] declares at set 0.
    smaa_weight_layout: VulkanBindGroupLayout,
    /// Anti-aliasing Stage 2: the 2-CIS bind-group LAYOUT `{ lit @0, weights @1 }`
    /// [`Self::smaa_blend_pipeline`] declares at set 0.
    smaa_blend_layout: VulkanBindGroupLayout,
    /// Anti-aliasing Stage 2: the dedicated LINEAR/ClampToEdge sampler EVERY SMAA tap uses
    /// (`lit`, `edges`, `weights`, `areaTex`, `searchTex`) — a SEPARATE boot object from
    /// [`Self::fxaa_sampler`] (isolation; the FXAA path stays untouched).
    smaa_sampler: VulkanSampler,
    /// Anti-aliasing Stage 2: the boot-resident `AreaTex` LUT (160×560, `R8G8_UNORM`),
    /// uploaded once via `boyko_render::upload_texture_2d_raw` and never touched again.
    smaa_area_tex: VulkanTexture,
    /// Anti-aliasing Stage 2: the boot-resident `SearchTex` LUT (64×16, `R8_UNORM`),
    /// uploaded once via `boyko_render::upload_texture_2d_raw` and never touched again.
    smaa_search_tex: VulkanTexture,
    /// Anti-aliasing Stage 3: the SSAA downsample fullscreen graphics pipeline
    /// (`fullscreen_sample.vs` + `ssaa_downsample.fs`), built unconditionally at boot (like
    /// [`Self::fxaa_pipeline`]) so records nothing until `AaMode::Ssaa` is host-armed.
    /// `color_formats[0]` == `R8G8B8A8_UNORM` (`aa_out`'s format), NO push constants;
    /// reuses [`Self::present_layout`] — the SAME 1-CIS shape [`Self::fxaa_pipeline`] uses.
    /// Reuses [`Self::present_sampler`] (NEAREST — the shader's `.Load` ignores it) as the
    /// SSAA sampler; no dedicated sampler field (unlike `fxaa_sampler`/`smaa_sampler`).
    ssaa_pipeline: VulkanGraphicsPipeline,
    /// Anti-aliasing Stage 4 (TAA W5): the temporal-resolve compute pipeline
    /// (`taa_resolve.comp`), built unconditionally at boot (like [`Self::ssaa_pipeline`]) —
    /// NOT hwrt-gated. Bound + dispatched by `record_taa` when `scene.taa.is_some()`.
    taa_resolve_pipeline: ComputePipeline,
    /// Anti-aliasing Stage 4 (TAA W5): the DEDICATED 8-binding bind-group LAYOUT
    /// [`Self::taa_resolve_pipeline`] declares at set 0 — { `gLit` CIS @0, `gViewT` @1,
    /// `gHistIn` @2, `gHistOut` @3, `gAaOut` @4 STORAGE images, the `ResolvedTaa` UBO @5, the
    /// camera UBO @6, the `MotionCam` UBO @7 }. [`GBufferTargets`] writes a per-FIF
    /// `taa_resolve_set` against it once per extent.
    taa_resolve_layout: VulkanBindGroupLayout,
    /// Anti-aliasing Stage 4 (TAA W5): the dedicated LINEAR/ClampToEdge sampler for the
    /// resolve's `gLit` combined-image-sampler tap — DISTINCT boot object from
    /// [`Self::fxaa_sampler`]/[`Self::smaa_sampler`].
    taa_linear_sampler: VulkanSampler,
    /// TAA rung T3: the post-resolve RCAS sharpen compute pipeline (`rcas.comp`), built
    /// UNCONDITIONALLY at boot (like [`Self::taa_resolve_pipeline`]) so the mode can flip at
    /// runtime. Bound + dispatched by `record_rcas` when `scene.rcas.is_some()`.
    rcas_pipeline: ComputePipeline,
    /// TAA rung T3: the DEDICATED 2-binding bind-group LAYOUT [`Self::rcas_pipeline`] declares
    /// at set 0 — { `gRcasIn` STORAGE @0, `gAaOut` STORAGE @1 }. [`GBufferTargets`] writes a
    /// per-FIF `rcas_set` against it once per extent.
    rcas_layout: VulkanBindGroupLayout,
    // ── Render P7-Q2: SSAO (dormant → live) ───────────────────────────────────
    /// Render P7-Q2: the [`SSAO_QUALITY_COUNT`] pre-compiled SSAO quality-variant compute
    /// pipelines (`sdf_ssao_{low,medium,high}.comp`), built unconditionally at boot (like
    /// [`Self::fxaa_pipeline`]/[`Self::ssaa_pipeline`]/[`Self::taa_resolve_pipeline`]
    /// above) so the owner-resolved quality
    /// ([`boyko_render::ResolvedSsao::variant`]) can select a pipeline with no boot-time
    /// rebuild. All three share [`Self::ssao_layout`] (the SSAO shader interface is
    /// identical across variants — only the baked tap-budget constants differ, Mechanism
    /// C) — indexed by `SSAO_QUALITY_LOW`/`_MEDIUM`/`_HIGH` (0/1/2). Boot-time creation
    /// records no command / samples no pixel — byte-identical to the golden regardless of
    /// this array's existence (`GBufferScene::ssao` stays `None` unless a non-`Off`
    /// `SsaoQuality` is host-resolved). Mirrors the test harness's
    /// (`window_present_gbuffer.rs`) SSAO boot bundle, widened to all 3 variants.
    ssao_pipelines: [ComputePipeline; SSAO_QUALITY_COUNT],
    /// Rung R9b: the `-D VB_THIN=1` SSAO gather pipelines (the VB split's gather — reads
    /// `thin_normal`+`gViewT` instead of `gNormal`/`gMaterial`), indexed like
    /// [`Self::ssao_pipelines`]. Built UNCONDITIONALLY at boot (same rationale: negligible
    /// object cost; `GBufferScene::path_vb_ssao` gates dispatch).
    ssao_vb_pipelines: [ComputePipeline; SSAO_QUALITY_COUNT],
    /// Rung R9b: the VB gather's DEDICATED dense 4-binding LAYOUT { `thin_normal` @0, `gViewT`
    /// @1 STORAGE READ, `ssao` @2 STORAGE WRITE, the camera UBO @3 } — `sdf_ssao`'s `VB_THIN`
    /// table. [`GBufferTargets`] writes a `vb_ssao_set` against it when the split arms.
    vb_ssao_layout: VulkanBindGroupLayout,
    /// Rung R9b: `vb_geo`'s Set-1 aux LAYOUT { `thin_normal` STORAGE @0, `motion` STORAGE @1
    /// (R9d — the software `.spv` never references it, the R2 contract), `MotionCam` UBO @2
    /// (R9d, ditto) }. Built at boot; the `vb_geo` pipeline itself is deferred-built (needs the
    /// geometry Set-2 layout).
    vb_geo_aux_layout: VulkanBindGroupLayout,
    /// Rung R9b: `vb_shade_split`'s Set-1 LAYOUT (9 bindings; 8 on the software leg): @0-3 =
    /// `forward_layout1`'s shadow table kinds verbatim, @4 `gSsao` STORAGE, @5/@6 the DDGI
    /// COMBINED image+sampler pair, @7 `ResolvedDdgi` UBO, @8 cfg(hwrt) `gShadowVis` STORAGE.
    /// A DISTINCT object — `forward_layout1` stays byte-untouched.
    vb_split_layout1: VulkanBindGroupLayout,
    /// Rung R9b: the split pair pipelines — deferred-built by [`Self::build_vb_split_pipelines`]
    /// (the SAME geometry-Set-2 dependency as [`Self::build_vb_resolve_pipeline`]).
    vb_geo_pipeline: Option<ComputePipeline>,
    /// Rung R9b: the split's lit producer (see [`Self::vb_geo_pipeline`]).
    vb_shade_split_pipeline: Option<ComputePipeline>,
    /// Rung R9b: the `-D TEXTURED=1` sibling (also needs the bindless Set-3 —
    /// `build_vb_shade_textured_pipeline`'s own two-dependency reason).
    vb_shade_split_tex_pipeline: Option<ComputePipeline>,
    /// Rung R9d: the `-D MOTION=1` sibling of [`Self::vb_geo_pipeline`] (`vb_geo_mv.comp.hlsl`) —
    /// deferred-built by [`Self::build_vb_split_pipelines`] under the SAME geometry-Set-2
    /// dependency, gated ADDITIONALLY on `ctx.ray_query_enabled()` (an RT-only variant). `None`
    /// on a non-RT device.
    #[cfg(feature = "hwrt")]
    vb_geo_mv_pipeline: Option<ComputePipeline>,
    /// Rung R9d: the `-D HWRT=1` sibling of [`Self::vb_shade_split_pipeline`]. Same deferred-build
    /// and `ray_query_enabled` gate as [`Self::vb_geo_mv_pipeline`].
    #[cfg(feature = "hwrt")]
    vb_shade_split_hwrt_pipeline: Option<ComputePipeline>,
    /// Rung R9d: the `-D TEXTURED=1 -D HWRT=1` sibling of [`Self::vb_shade_split_tex_pipeline`].
    /// Same deferred-build and `ray_query_enabled` gate as [`Self::vb_geo_mv_pipeline`], PLUS the
    /// bindless Set-3 dependency (built only when `bindless` is `Some`).
    #[cfg(feature = "hwrt")]
    vb_shade_split_tex_hwrt_pipeline: Option<ComputePipeline>,
    /// Render P7-Q2: the DEDICATED 5-binding SSAO bind-group LAYOUT { `gNormal` @0,
    /// `gMaterial` @1, `gViewT` @2 STORAGE images READ, the `ssao` out STORAGE image @3
    /// WRITE, the camera UBO @4 } — matching `sdf_ssao.comp`'s set 0, shared by every
    /// entry of [`Self::ssao_pipelines`]. [`GBufferTargets`] writes an `ssao_set` against
    /// it once per extent when [`GBufferScene::ssao`] is armed.
    ssao_layout: VulkanBindGroupLayout,
    /// Multi-paradigm render-path plan, rung R3b (`Deferred × Mesh` — the SDF leg fully off): the
    /// `viewt_from_depth` compute pipeline (`viewt_from_depth.comp.hlsl` /
    /// [`viewt_from_depth_spirv`]), the `gViewT` producer that stands in for the (undispatched)
    /// marcher on a mesh-only frame. Built UNCONDITIONALLY at boot (like
    /// [`Self::ssao_pipelines`] above — the pipeline itself needs no device precondition to
    /// CREATE; [`GBufferScene::path_has_viewt_from_depth`] gates whether it is actually
    /// dispatched, so a `Both`/`Sdf`-resolved boot pays only this one negligible pipeline
    /// object, never a descriptor set/dispatch/VRAM cost).
    viewt_from_depth_pipeline: ComputePipeline,
    /// The DEDICATED 2-binding `viewt_from_depth` bind-group LAYOUT { SAMPLED depth @0, STORAGE
    /// `gViewT` @1 } — matching `viewt_from_depth.comp`'s set 0. [`GBufferTargets`] writes a
    /// `viewt_from_depth_set` against it once per extent when
    /// [`GBufferScene::path_has_viewt_from_depth`] holds.
    viewt_from_depth_layout: VulkanBindGroupLayout,
    /// TAA-under-VB: the `viewt_from_depth_rz` compute pipeline (`viewt_from_depth_rz.comp.hlsl`
    /// / [`viewt_from_depth_rz_spirv`]) — the REVERSE-Z sibling of [`Self::viewt_from_depth_pipeline`],
    /// the `gViewT` producer for `VisibilityBuffer × Mesh`'s TAA seam (see
    /// [`ViewtFromVbDepthActivation`]'s doc). Built UNCONDITIONALLY at boot — the SAME rationale
    /// as [`Self::viewt_from_depth_pipeline`] (no device precondition to CREATE;
    /// [`GBufferScene::viewt_from_vb_depth`] gates whether it is actually dispatched, so a
    /// non-VB or TAA-off boot pays only this one negligible pipeline object).
    viewt_from_vb_depth_pipeline: ComputePipeline,
    /// The DEDICATED 3-binding `viewt_from_depth_rz` bind-group LAYOUT { SAMPLED depth @0,
    /// STORAGE `gViewT` @1, UNIFORM camera @2 } — matching `viewt_from_depth_rz.comp`'s set 0
    /// (one more binding than [`Self::viewt_from_depth_layout`]: the reverse-Z ray
    /// reparameterization needs the camera basis). [`GBufferTargets`] writes a
    /// `viewt_from_vb_depth_set` against it once per extent when
    /// [`GBufferScene::viewt_from_vb_depth`] holds.
    viewt_from_vb_depth_layout: VulkanBindGroupLayout,
    /// The SSAO edge-avoiding à-trous denoise chain: the `level == 0` pipeline variant
    /// (`ssao_atrous_read8.comp` / [`ssao_atrous_read8_spirv`]) — `gAoIn` pinned `r8` (reads the
    /// frozen `gSsao` gather endpoint), `gAoOut` pinned `r16` (writes ring 0). Built UNCONDITIONALLY
    /// at boot (like [`Self::ssao_pipelines`] — the pipeline itself needs no device precondition to
    /// CREATE, only the interior ring IMAGE needs `R16_UNORM` storage, checked separately by
    /// [`GBufferTargets`]'s degrade). Shares [`Self::ssao_atrous_layout`] with the other two
    /// variants below.
    ssao_atrous_read8_pipeline: ComputePipeline,
    /// The SSAO à-trous chain's INTERIOR pipeline variant (`ssao_atrous.comp` /
    /// [`ssao_atrous_spirv`]): both `gAoIn`/`gAoOut` pinned `r16` (the two ping-pong rings). Built
    /// in lock-step with [`Self::ssao_atrous_read8_pipeline`] (same discipline, same doc).
    ssao_atrous_interior_pipeline: ComputePipeline,
    /// The SSAO à-trous chain's LAST-level pipeline variant (`ssao_atrous_write8.comp` /
    /// [`ssao_atrous_write8_spirv`]): `gAoIn` pinned `r16` (reads a ring), `gAoOut` pinned `r8`
    /// (writes BACK into the frozen `gSsao` endpoint the resolve reads — the C1 fix). Built in
    /// lock-step with [`Self::ssao_atrous_read8_pipeline`] (same discipline, same doc).
    ssao_atrous_write8_pipeline: ComputePipeline,
    /// The SSAO à-trous chain's DEDICATED 4-binding bind-group LAYOUT { `gAoIn` STORAGE image @0
    /// (R), `gAoOut` STORAGE image @1 (W), `gViewT` STORAGE image @2 (R), the camera UNIFORM
    /// buffer @3 } — IDENTICAL across all three pipeline variants above (only the bound VIEW + the
    /// `[[vk::image_format]]` pin differ). Shared with `sdf_ssao`'s design ONE step narrower (no
    /// `gMaterial`, no `gNormal` — the à-trous gate is depth-plane-fit only). [`GBufferTargets`]
    /// writes FIVE role-keyed sets against it once per extent
    /// ([`boyko_rhi_vulkan::present::ssao_atrous_step`]'s role→set mapping) when the device
    /// supports `R16_UNORM` STORAGE.
    ssao_atrous_layout: VulkanBindGroupLayout,
    // ── CSM / atlas (bound-but-unread trios; depth passes OFF in R3) ─────────
    csm: CsmResources,
    // ── Multi-paradigm render-path plan, rung R4b-b (Set 0 UNIFIED at rung R5 code-review
    // fix): the Forward FAMILY's mesh raster pipelines + descriptor-set layouts (see `boot`'s
    // doc for why these are built UNCONDITIONALLY, not gated on `ResolvedRenderPath`, and for
    // the layout-compatibility bug the unification fixed). ──────────────────
    /// The plain-`Forward` mesh raster pipeline (`forward_opaque.{vs,fs}.hlsl`, base FS,
    /// `VK_COMPARE_OP_GREATER`, depth-write ON) — a plain 2-set `[Set0, Set1]` layout (no
    /// placeholder — see `boot`'s doc for the boot-panic fix that renumbered the shadow set from
    /// Set 2 to Set 1). Set 0 = [`Self::forward_layout0`] (the UNIFIED 7-binding layout).
    forward_pipeline: VulkanGraphicsPipeline,
    /// Code-review follow-up (rung R4b-b): the Forward-family sky background pipeline
    /// (`forward_sky.{vs,fs}.hlsl`) — reuses [`Self::forward_layout0`], no depth attachment
    /// declared (`boot`'s doc). Shared verbatim by `Forward` and `ForwardPlus`.
    forward_sky_pipeline: VulkanGraphicsPipeline,
    /// The UNIFIED Forward-family Set-0 (core) bind-group layout — 7 bindings: instances @0,
    /// instance_materials @1, Camera @2, LightBuf @3, Materials @4, `ClusterGrid` @5,
    /// `LightIndexList` @6 (`GpuSceneBundles::boot`'s doc). Rung R5 code-review fix: shared by
    /// EVERY Forward-family pipeline (`forward_pipeline`/`forward_sky_pipeline`/
    /// `forward_prepass_pipeline`/`forward_plus_pipeline`) and the per-extent
    /// `ForwardTargets::set0[fi]` descriptor set they are all bound alongside — a SINGLE layout
    /// object is REQUIRED for Vulkan pipeline/descriptor-set compatibility at draw time (two
    /// structurally-identical but DISTINCT `VkDescriptorSetLayout` handles are NOT
    /// interchangeable — an earlier revision's bug).
    forward_layout0: VulkanBindGroupLayout,
    /// The Forward-family Set-1 (shadow) bind-group layout — 4 bindings (CSM + punctual atlas).
    /// Shared verbatim by `Forward` and `ForwardPlus` (UNCHANGED by rung R5).
    forward_layout1: VulkanBindGroupLayout,
    // ── Multi-paradigm render-path plan, rung R5: ForwardPlus's depth prepass + froxel
    // opaque pipeline variant (see `boot`'s doc — same "built UNCONDITIONALLY, cheap"
    // precedent as the Forward v1 trio above; BOTH built against [`Self::forward_layout0`],
    // the unified layout). ─────────────────
    /// The `depth_prepass` pipeline (`depth_prepass.{vs,fs}.hlsl`) — depth-only,
    /// `VK_COMPARE_OP_GREATER`, depth-write ON; Set 0 = [`Self::forward_layout0`] as its only
    /// set (the prepass VS references only the `instances` binding, a subset of that layout).
    /// Shared verbatim by `Forward` and `ForwardPlus` (only `ForwardPlus` ever records it).
    forward_prepass_pipeline: VulkanGraphicsPipeline,
    /// The `forward_opaque` FROXEL pipeline variant (`forward_opaque_froxel.fs.spv` +
    /// `forward_opaque.vs.spv`, the SAME VS) — `VK_COMPARE_OP_EQUAL`, depth-write OFF; Set 0 =
    /// [`Self::forward_layout0`] (the SAME unified layout `forward_pipeline` uses), Set 1 =
    /// [`Self::forward_layout1`] (UNCHANGED, shared verbatim).
    forward_plus_pipeline: VulkanGraphicsPipeline,
    /// Multi-paradigm render-path plan, rung R-SDFFWD: the `sdf_forward_march` `HAS_MESH`
    /// compute pipeline (`sdf_forward_march.comp.hlsl` compiled with `-D HAS_MESH=1`). Built
    /// UNCONDITIONALLY at boot (the Forward v1 trio's own "cheap, no per-frame cost either way"
    /// precedent — `ResolvedRenderPath` does not reach `boot()`'s call site); only a
    /// Forward-family-resolved boot with the SDF leg present ever RECORDS it. Built against
    /// [`Self::sdf_forward_march_layout`] (Set 0) + `forward_layout1` (Set 1, the shadow set —
    /// REUSED VERBATIM, no separate layout).
    sdf_forward_march_pipeline: ComputePipeline,
    /// The `sdf_forward_march` mesh-less compute pipeline (compiled with no `-D`). Built
    /// UNCONDITIONALLY alongside [`Self::sdf_forward_march_pipeline`], against the SAME
    /// [`Self::sdf_forward_march_layout`] (the code-review-fixed "one layout object per pipeline
    /// family" discipline `forward_layout0` already establishes).
    sdf_forward_march_sdfonly_pipeline: ComputePipeline,
    /// TAA-under-VB: the `sdf_forward_march` `HAS_MESH + VIEWT` compute pipeline (`-D HAS_MESH=1
    /// -D VIEWT=1`) — [`Self::sdf_forward_march_pipeline`] plus the `gViewT` binding-13 write.
    /// Selected at record when the scene's `path_sdf_forward_writes_viewt()` predicate holds.
    sdf_forward_march_viewt_pipeline: ComputePipeline,
    /// TAA-under-VB: the `sdf_forward_march` mesh-less `VIEWT` compute pipeline (`-D VIEWT=1`).
    sdf_forward_march_sdfonly_viewt_pipeline: ComputePipeline,
    /// The `sdf_forward_march` pass's dedicated 14-binding Set-0 bind-group LAYOUT — see
    /// `boyko_rhi_vulkan::present::scene_types::GBufferScene::sdf_forward_march_layout`'s doc for
    /// the full binding table this fn's `boot` construction site mirrors.
    sdf_forward_march_layout: VulkanBindGroupLayout,
    /// `ceil(composite pixels / LOCAL_SIZE_X)` — the marcher + resolve dispatch
    /// width, boot-fixed to the composite extent (plan D7).
    dispatch_group_count_x: u32,
    // ── Multi-paradigm render-path plan, rung R8: the VisibilityBuffer v1 FUSED path's own
    // pipelines + Set-0 layout + instance ring (see `boot`'s doc — built UNCONDITIONALLY, the
    // SAME "cheap, no per-frame cost either way" precedent as the Forward v1 trio). ──────
    /// The VB-only Set-0 (core + images) bind-group layout — 7 bindings: `gVbInstances` @0,
    /// `instance_materials` @1, `Camera` @2, `LightBuf` @3, `Materials` @4, `gVbId` @5 (SAMPLED),
    /// `gLit` @6 (STORAGE). Camera/LightBuf at bindings 2/3 (matching `forward_layout0`'s own
    /// numbering) so [`Self::vb_sky_pipeline`] can reuse `forward_sky.{vs,fs}.hlsl`'s compiled
    /// SPIR-V verbatim against a NEW pipeline object built for THIS layout.
    vb_layout0: VulkanBindGroupLayout,
    /// The `vb_raster` mesh id-raster pipeline (`vb_raster.{vs,fs}.hlsl`) — a plain 1-set
    /// pipeline (Set 0 = [`Self::vb_layout0`], its VS reads only `gVbInstances`/the push, a
    /// bound-but-unread subset).
    vb_raster_pipeline: VulkanGraphicsPipeline,
    /// The VB v1 sky background pipeline — REUSES `forward_sky.{vs,fs}.hlsl`'s compiled SPIR-V
    /// verbatim (`GBufferScene::vb_sky_pipeline`'s doc), built as a NEW pipeline object against
    /// [`Self::vb_layout0`].
    vb_sky_pipeline: VulkanGraphicsPipeline,
    /// The `vb_resolve` FUSED compute pipeline (`vb_resolve.comp.hlsl`) — a 3-set pipeline: Set 0
    /// = [`Self::vb_layout0`], Set 1 = [`Self::forward_layout1`] (the shadow set, REUSED
    /// verbatim), Set 2 = the Decision-0 geometry table's own Set. Built LAZILY
    /// ([`Self::build_vb_resolve_pipeline`], mirroring [`Self::tex`]'s deferred-build shape) —
    /// Set 2's layout does not exist at [`Self::boot`]'s call site (the live `MeshGeometryTable`
    /// is a World `NonSendResource`, `boyko_render::mesh_geometry_table::MeshGeometryTableSlot`,
    /// constructed by `boyko_app::runner` only on a `VisibilityBuffer`-resolved boot). `None` on
    /// every OTHER boot (the 0%-gate).
    vb_resolve_pipeline: Option<ComputePipeline>,
    // ── VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2a (dark infra,
    // unwired): the four classify/shade pipelines. Built LAZILY by
    // [`Self::build_vb_classify_pipelines`], the SAME deferred-build shape as
    // [`Self::vb_resolve_pipeline`] (`vb_shade` needs the geometry table's Set-2 layout, which
    // does not exist at [`Self::boot`]'s call site). Nothing declares/records against these
    // this rung — `record_vb`/`declare_vb_graph` are untouched; the fused `vb_resolve` still
    // shades every VB frame. `None` on every boot that never calls that fn. ──────────────────
    /// The `count` classify compute pipeline (`vb_classify_count.comp.hlsl`) — a 1-set
    /// pipeline built against [`Self::vb_layout0`] via the GENERIC
    /// `RhiDevice::create_compute_pipeline` (plan P2-1 — no dedicated `_vb1` helper).
    vb_classify_count_pipeline: Option<ComputePipeline>,
    /// The `scan` classify compute pipeline (`vb_classify_scan.comp.hlsl`) — a 1-set pipeline,
    /// built the SAME way as [`Self::vb_classify_count_pipeline`].
    vb_classify_scan_pipeline: Option<ComputePipeline>,
    /// The `scatter` classify compute pipeline (`vb_classify_scatter.comp.hlsl`) — a 1-set
    /// pipeline, built the SAME way as [`Self::vb_classify_count_pipeline`].
    vb_classify_scatter_pipeline: Option<ComputePipeline>,
    /// The `vb_shade` material-classified shading compute pipeline (`vb_shade.comp.hlsl`) — a
    /// 3-set pipeline built via [`VulkanContext::create_compute_pipeline_vb`], mirroring
    /// [`Self::vb_resolve_pipeline`]'s own pipeline shape (Set 0 = `vb_layout0`, Set 1 =
    /// `forward_layout1`, Set 2 = the geometry table's own Set).
    vb_shade_pipeline: Option<ComputePipeline>,
    /// Textured-PBR rung TV0 (`RENDER-PARITY-PLAN.md` §2.3): the `vb_shade` TEXTURED-variant
    /// shading compute pipeline (`vb_shade.comp.hlsl`, `-D TEXTURED=1`, `vb_shade_tex.comp.spv`)
    /// — a 4-set pipeline built via [`VulkanContext::create_compute_pipeline_vb_textured`]
    /// (Set 0 = `vb_layout0`, Set 1 = `forward_layout1`, Set 2 = the geometry table's own Set,
    /// Set 3 = the shared bindless texture-array table's Set — REUSED verbatim, R5). Built
    /// LAZILY by [`Self::build_vb_shade_textured_pipeline`], the SAME deferred-build shape as
    /// [`Self::vb_shade_pipeline`] widened by ONE more dependency (the bindless table's Set-3
    /// layout, which ALSO does not exist at [`Self::boot`]'s call site). `None` until that fn
    /// runs (a `VisibilityBuffer`-resolved boot with BOTH the geometry table AND the bindless
    /// table armed).
    vb_shade_tex_pipeline: Option<ComputePipeline>,
    /// The per-slot VB instance-model SSBO ring ([`INSTANCE_CAPACITY`] ×
    /// [`boyko_render::instance_model::VB_INSTANCE_ROW_BYTES`] (64 B) each, zero-seeded) — a
    /// DEDICATED ring, distinct from [`Self::instance_rings`] (`InstanceModelCol`, 48 B).
    /// Rung R8 v1 scope cut: FIXED at [`INSTANCE_CAPACITY`], no growth-past-cap support yet
    /// (mirrors the pre-F7 state of `instance_rings` itself — the golden scene's instance count
    /// is far below this cap). Uploaded by `boyko_render::upload::upload_vb_instance_rows` from
    /// `MeshRenderScratch::vb_ring` (`boyko_app::runner`, gated on a `VisibilityBuffer`-resolved
    /// boot).
    ///
    /// Code review P2-2 (documented deviation, not fixed this rung): built UNCONDITIONALLY in
    /// [`Self::boot`] — the SAME "cheap, no per-frame cost either way" precedent
    /// `vb_layout0`/`vb_raster_pipeline`/`vb_sky_pipeline` follow — rather than `Option`-gated
    /// on `ResolvedRenderPath.path == VisibilityBuffer` the way [`Self::vb_resolve_pipeline`]
    /// (which genuinely CANNOT exist before the geometry table does) is. Unlike those three
    /// pipeline objects (a few hundred bytes of driver-side pipeline state each), this ring is
    /// `INSTANCE_CAPACITY * 64` bytes **per in-flight frame** of real HOST-VISIBLE device memory
    /// — a measurable, not merely nominal, cost paid on every non-VB boot (Deferred included),
    /// violating the plan's "zero-cost leg/path toggle" invariant more concretely than the
    /// pipeline objects do. Gating this allocation behind `ResolvedRenderPath.path ==
    /// VisibilityBuffer` (an `Option<[BoundBuffer; FRAMES_IN_FLIGHT]>`, mirroring
    /// `vb_resolve_pipeline`'s own `Option` shape) is a follow-up, not done this rung.
    pub(crate) vb_instance_rings: [BoundBuffer; FRAMES_IN_FLIGHT],

    // ── VB-P1b: the froxel light-cull machinery — built LAZILY by
    // [`Self::build_froxel_light_cull`], gated entirely on `ResolvedRenderPath::froxel_light_cull`
    // at the `boyko_app::runner` call site. That gate is armed iff the booted scene's
    // `LightingConfig::clusters_enabled` is `true` UNDER `RenderPath::VisibilityBuffer` — every
    // field below stays `None`/zeroed on every other boot (unarmed scenes, and every non-VB path,
    // are byte-identical to VB-P1a's 0%-gate). See that fn's doc for the full build.
    // ──────────────────────────────────────────────────
    /// The L1 clustered froxel light-cull compute pipeline (`cluster_cull.comp.hlsl`) — a 1-set
    /// pipeline built against [`Self::cull_layout`]. `None` unless the froxel arm is built.
    cluster_cull_pipeline: Option<ComputePipeline>,
    /// The cull bind-group LAYOUT { camera UBO @0, light table SSBO @1, `ClusterGrid` SSBO @2,
    /// `LightIndexList` SSBO @3, `LightIndexAlloc` SSBO @4 } — matching `cluster_cull.hlsl`'s own
    /// set 0. `None` unless [`Self::cluster_cull_pipeline`] is `Some`.
    cull_layout: Option<VulkanBindGroupLayout>,
    /// The L1 per-froxel `ClusterCell`/`{offset,count}` grid SSBO (`DEVICE_LOCAL`, STORAGE,
    /// Principle 0 — a VM-native `BoundBuffer`, never `std::Vec`), sized `cluster_count * 8 B`.
    /// `None` unless the froxel arm is built.
    cluster_grid: Option<BoundBuffer>,
    /// The L1 flat light-index list SSBO (`DEVICE_LOCAL`, STORAGE), sized `index_list_cap * 4 B`.
    /// `None` unless the froxel arm is built.
    light_index: Option<BoundBuffer>,
    /// The L1 global slice-allocation counter SSBO (one `u32`, `DEVICE_LOCAL`, STORAGE). `None`
    /// unless the froxel arm is built.
    light_index_alloc: Option<BoundBuffer>,
    /// The [`ClusterCullPush`] the cull dispatch pushes (exp-Z near/far + the caps) — meaningless
    /// while [`Self::cluster_cull_pipeline`] is `None` (never read then).
    cluster_cull_push: ClusterCullPush,
    /// The L1 froxel count (`dim_x * dim_y * dim_z`) the cull's 1D dispatch covers — meaningless
    /// while [`Self::cluster_cull_pipeline`] is `None`.
    cluster_count: u32,
    /// VB-P1e D11/H4: `Some` IFF [`Self::cluster_cull_pipeline`] holds the `-D HIER=1` variant
    /// instead of the base 64-wide arm — the group count + the 24-byte push bytes
    /// [`Self::scene`] threads into [`GBufferScene::cluster_cull_hier`]. `None` (the default)
    /// keeps every record site on the base arm, byte-identical to every pre-H4 boot.
    cluster_cull_hier: Option<ClusterCullHierDispatch>,
    /// VB-P1e D11: the BOOT-frozen `ClusterConfig::packed_dims()` snapshot
    /// [`Self::build_froxel_light_cull`] sized every L1 buffer from — meaningless while
    /// [`Self::cluster_cull_pipeline`] is `None`. The per-frame runner compares this against the
    /// LIVE `ClusterConfig` Resource (`runner.rs`'s debug-only boot/live dims assert) to catch an
    /// owner system stomping the Resource after boot; release builds do not pay for it.
    cluster_boot_packed_dims: u32,
    /// The froxel-only Set-0 bind-group LAYOUT — 10 bindings: [`Self::vb_layout0`]'s own 0..7
    /// PLUS `ClusterGrid` @8 + `LightIndexList` @9 — a DISTINCT layout OBJECT from `vb_layout0`
    /// (never widened in place, so `vb_layout0` stays byte-identical). `None` unless the froxel
    /// arm is built.
    vb_layout0_froxel: Option<VulkanBindGroupLayout>,
    /// The `vb_resolve` FROXEL-variant compute pipeline (`vb_resolve.comp.hlsl`, `-D FROXEL=1`)
    /// — the SAME 3-set shape as [`Self::vb_resolve_pipeline`], built against
    /// [`Self::vb_layout0_froxel`]. `None` unless the froxel arm is built.
    vb_resolve_froxel_pipeline: Option<ComputePipeline>,
    /// The `vb_shade` FROXEL-variant compute pipeline (`vb_shade.comp.hlsl`, `-D FROXEL=1`) — the
    /// SAME 3-set shape as [`Self::vb_shade_pipeline`], built against
    /// [`Self::vb_layout0_froxel`]. `None` unless the froxel arm is built.
    vb_shade_froxel_pipeline: Option<ComputePipeline>,
    /// The `vb_shade` TEXTURED+FROXEL-variant compute pipeline (`vb_shade.comp.hlsl`, `-D
    /// TEXTURED=1 -D FROXEL=1`) — the SAME 4-set shape as [`Self::vb_shade_tex_pipeline`], built
    /// against [`Self::vb_layout0_froxel`]. `None` unless the froxel arm is built.
    vb_shade_tex_froxel_pipeline: Option<ComputePipeline>,
    /// VB-P1d: the froxel cull/shade GPU-timestamp bench collector — a per-FIF ring of
    /// `2 * VB_PASS_COUNT`-query TIMESTAMP pools. Built at [`Self::boot`] ONLY when the
    /// `BOYKO_VB_BENCH` env is set AND the device supports timestamps
    /// (`VulkanContext::device_caps().timestamps_usable()`); `None` on every other boot (every
    /// golden/host/interactive run — the DEFAULT), so [`Self::scene`] threads `vb_gpu_timing:
    /// None` into every `GBufferScene` and the `record_vb` command stream stays
    /// byte-identical. See [`Self::vb_bench_armed`] / [`Self::read_vb_bench_ns`].
    vb_bench: Option<VbTimestampCollector>,
    /// VB-SV0 rung S1.5: the DEFERRED fine-marcher GPU-timestamp bench collector — a per-FIF
    /// ring of `2 * SV0_PASS_COUNT`-query TIMESTAMP pools. Built at [`Self::boot`] ONLY when the
    /// `BOYKO_SV0_BENCH` env is set AND the device supports timestamps; `None` on every other
    /// boot (every golden/host/interactive run — the DEFAULT), so [`Self::scene`] threads
    /// `sv0_gpu_timing: None` into every `GBufferScene` and the `record_gbuffer` command stream
    /// stays byte-identical. See [`Self::sv0_bench_armed`] / [`Self::read_sv0_marcher_ticks`].
    sv0_bench: Option<Sv0TimestampCollector>,
}

/// Textured-PBR T6c: the TEXTURED gbuffer producer pipeline resources — see
/// [`GpuSceneBundles::tex`] / [`GpuSceneBundles::build_textured_resources`] for the
/// lazy-build rationale (the bindless texture-array table's descriptor-set LAYOUT, a
/// `create_graphics_pipeline_bindless` input, does not exist at `GpuSceneBundles::boot()`
/// time).
struct TexturedResources {
    /// The 2-SET TEXTURED gbuffer producer pipeline (`gbuffer_mrt_tex.{vs,fs}`) — set 0 =
    /// [`Self::tex_instance_material_layout`] (VERTEX), set 1 = the bindless texture-array
    /// set's layout (FRAGMENT), built via
    /// [`VulkanContext::create_graphics_pipeline_bindless`].
    raster_pipeline_tex: VulkanGraphicsPipeline,
    /// The 2-binding set-0 layout [`Self::raster_pipeline_tex`] declares: instances @0
    /// (VERTEX, the SAME shared `instance_rings`) + instance_materials_tex @1 (VERTEX, its
    /// own per-slot ring). A SEPARATE layout from
    /// [`GpuSceneBundles`]'s `pm_instance_material_layout` (a wider element stride —
    /// `PerInstanceMaterialTex`, 48 B, vs `PerInstanceMaterial`'s 32 B).
    tex_instance_material_layout: VulkanBindGroupLayout,
    /// The per-slot TEXTURED instance-material SSBO ring ([`INSTANCE_CAPACITY`]
    /// [`PerInstanceMaterialTex`](boyko_render::PerInstanceMaterialTex)s = 48 B each,
    /// zero-seeded). The runner uploads `scratch.material_tex` into slot `token.slot()`
    /// ONLY on an `any_textured_material` frame (Principle 1 — no OFF-path upload cost).
    ///
    /// This RING ITSELF does NOT participate in the F7/F7-hwrt lockstep instance-family
    /// grow (a disclosed T6c limitation, see the developer report): a scene whose gathered
    /// instance count grows past [`INSTANCE_CAPACITY`] while using textured materials hits
    /// `upload_instance_materials_tex`'s hard capacity assert rather than silently
    /// corrupting memory. (Its bind-group BINDING — [`Self::tex_bind_groups`]'s binding 1
    /// — is correspondingly never rebound either, since there is no grown buffer for it to
    /// point at.)
    pub(crate) tex_instance_material_rings: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// Per-FIF bind groups against [`Self::tex_instance_material_layout`]: slot `i` binds
    /// `{ instance_rings[i] @0, tex_instance_material_rings[i] @1 }`. The recorder binds
    /// slot `s` at set 0 when the TEXTURED pipeline is selected. Binding 0 points at the
    /// SHARED, growable `instance_rings` — [`GpuSceneBundles::grow_shared_instance_rings`]
    /// rebinds it in lockstep (review W1 fix; mirrors `pm_bind_groups[s]`@0), so a grow
    /// past [`INSTANCE_CAPACITY`] on a LATER non-textured frame cannot leave this pointing
    /// at a freed ring for a STILL-LATER textured frame.
    tex_bind_groups: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// The bindless texture-array descriptor SET, cached by value (a `Copy` FFI handle —
    /// its LAYOUT is baked into [`Self::raster_pipeline_tex`]'s `VkPipelineLayout`, not
    /// retained here). Bound at set 1 by the recorder every TEXTURED frame.
    bindless_set: VkDescriptorSet,
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
        // SSAA (W1): `composite` is ALREADY 2× native when `WindowHost::boot` armed SSAA (it
        // folds the scale into `composite_extent` before calling this fn), so every buffer
        // sized from `(cw, ch)` here scales automatically — no SSAA-specific branch needed in
        // this function. Full enumeration of what DOES vs does NOT key off `(cw, ch)`:
        // - PIXEL-COUNT-DERIVED (scale with SSAA, verified by the `debug_assert!`s below):
        //   `dispatch_group_count_x` (the marcher/resolve dispatch width) and `tiles_buffer`
        //   (the P4b coarse-cull tile grid, via `tile_grid_extent(cw, ch)`).
        // - RESOLUTION-INDEPENDENT (fixed capacity or per-frame CONTENTS, not sized from
        //   `(cw, ch)`): `camera_ring`/`edit_list`/`light_table`/the vertex/instance rings/the
        //   hwrt resources/the DDGI atlas — none of these buffers' byte SIZE is a function of
        //   `(cw, ch)` (their per-frame *contents* may encode the composite dims, but that is
        //   host-written data, not a boot-time allocation size).
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

        // ── Multi-paradigm render-path plan, rung R-SDFFWD: the `sdf_forward_march` pass's OWN
        // dedicated BrickLevels UBO (Set-0 binding 11) — a STANDALONE buffer, distinct from
        // `camera_ring`'s own M4Level tail (this pass's Camera @3 stays the plain 80-byte Forward
        // shape — `boyko_render::view::forward_gbuffer_push_from_view`'s contract, no M4 tail).
        // Single (NOT ringed), zero-seeded, never rewritten: `brick_enabled = brick_trilinear =
        // brick_levels = 0` every frame this rung (the explicit 0%-gate `SdfForwardMarchPush`'s
        // doc documents), so its contents are never read.
        const SDF_FORWARD_BRICK_LEVELS_UBO_BYTES: usize =
            boyko_sdf_math::brick::BRICK_LEVELS * M4_LEVEL_PARAMS_BYTES;
        let brick_levels_ubo = RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: SDF_FORWARD_BRICK_LEVELS_UBO_BYTES as u64,
                usage: BufferUsage::UNIFORM,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("invariant: sdf_forward_march BrickLevels uniform buffer create");
        {
            let mapped = RhiDevice::buffer_mapped_ptr(device, &brick_levels_ubo)
                .expect("invariant: host-visible BrickLevels UBO is mapped");
            zero_fill(mapped, SDF_FORWARD_BRICK_LEVELS_UBO_BYTES);
        }

        // ── The P4b coarse-cull tile buffer (vocab binding 6), bound-but-unread
        // (the coarse cull is gated OFF).
        let (tw, th) = tile_grid_extent(cw, ch);
        // SSAA (W1): the tile grid must cover the FULL composite extent on both axes — the
        // per-axis analogue of the `dispatch_group_count_x` coverage assert below (a future
        // edit that keyed either buffer to `native` instead of `composite` would silently
        // under-cull/under-dispatch at any SSAA scale; this fires immediately in debug).
        debug_assert!(
            (tw as u64) * (TILE_SIZE as u64) >= cw as u64
                && (th as u64) * (TILE_SIZE as u64) >= ch as u64,
            "invariant: the coarse-cull tile grid covers the composite pixel extent at any SSAA scale"
        );
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
        // `Assets<Material>`/`MaterialTable`-owned (asset-system rung A1):
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
        let degenerate = Vertex::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]);
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
                vertex_layout: Some(VertexBufferLayout {
                    stride: MESH_VERTEX_STRIDE as u32,
                    attributes: &attributes,
                }),
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
                vertex_layout: Some(VertexBufferLayout {
                    stride: MESH_VERTEX_STRIDE as u32,
                    attributes: &attributes,
                }),
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
            // references all three). `resolve_entries` itself is EXACT-FILL at 19/19 and is the
            // SHARED derivation base for every HWRT-family layout below (`hwrt_entries`/
            // `denoise_entries`/`vis_mv_entries` all read it directly) — it MUST stay 19 and
            // UNTOUCHED (textured-PBR T6a's C1 fix: bumping it would shift the HWRT TLAS 19→20).
            BindGroupLayoutEntry { binding: 16, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 17, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 18, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        ];
        // Textured-PBR T6a (the critic's C1 fix): binding 19 = `gPbr` (StorageImage) —
        // SOFTWARE-RESOLVE-ONLY. Appended to a SEPARATE vec (mirroring `hwrt_entries`'s idiom
        // below) so `resolve_entries` itself is NEVER mutated — every HWRT-family layout
        // (`hwrt_entries`/`denoise_entries`/`vis_mv_entries`) still derives from the untouched
        // 19-entry base, so TLAS stays @19 and their binding counts (21/22/24) are unaffected.
        let mut resolve_software_layout_entries = resolve_entries.to_vec();
        resolve_software_layout_entries.push(BindGroupLayoutEntry {
            binding: 19,
            count: 1,
            kind: DescriptorKind::StorageImage,
            stage: ShaderStage::COMPUTE,
        });
        debug_assert_eq!(
            resolve_software_layout_entries.len(),
            20,
            "invariant: the software resolve layout is EXACT-FILL at 20 (19 shared + gPbr @19)"
        );
        let resolve_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc { entries: &resolve_software_layout_entries },
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

        // Render terminator-softening: the `-D TERMINATOR_WRAP=1` variant pipeline, built
        // unconditionally (device-agnostic — not RT-gated) alongside `resolve_pipeline`, reusing
        // the SAME `resolve_layout` (the variant adds no binding; see `deferred_pbr_wrap_spirv`'s
        // doc). Selected per-frame by `scene`'s `terminator_wrap` gate.
        let resolve_wrap_cs = RhiDevice::create_shader_module(device, deferred_pbr_wrap_spirv())
            .expect("invariant: terminator-wrap deferred resolve compute shader module create");
        let resolve_pipeline_wrap = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &resolve_wrap_cs,
                entry: c"main",
                push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
                bind_group_layout: Some(&resolve_layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: terminator-wrap deferred resolve compute pipeline create");

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

        // ── Rung R9d: the VB split's DEDICATED shadow-vis gather pipeline + its 7-binding
        // layout { `thin_normal` @0, `gViewT` @1 STORAGE images, `LightTable` @2 STORAGE buffer,
        // the camera UBO @3, the TLAS `AccelerationStructure` @4, the `ResolvedRayShadow` UBO
        // @5 (reuses the SAME `ray_shadow_ubo` ring below), `gShadowVis` @6 (W) }. Same
        // `ray_query_enabled` gate as the deferred VIS pipeline (`shadow_denoise_pipelines`'s
        // own `vis_pipeline`, above) — this is the split's own standalone sibling: it has no
        // fat G-buffer to re-run the resolve front-matter against, so it traces against the
        // split's `thin_normal`/`gViewT` lanes directly.
        #[cfg(feature = "hwrt")]
        let vb_shadow_vis_pipeline: Option<(ComputePipeline, VulkanBindGroupLayout)> =
            ctx.ray_query_enabled().then(|| {
                let layout = RhiDevice::create_bind_group_layout(
                    device,
                    &BindGroupLayoutDesc {
                        entries: &[
                            BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::AccelerationStructure, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                            BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                        ],
                    },
                )
                .expect("invariant: R9d vb_shadow_vis bind-group layout create");
                // The SAME `SHADOW_RAY_COUNT` spec-const (id 0) the deferred VIS pipeline bakes,
                // so `mesh_vis` is bit-identical across both producers.
                let ray_count = RayShadowConfig::default().ray_count.max(1);
                let cs = RhiDevice::create_shader_module(device, vb_shadow_vis_spirv())
                    .expect("invariant: R9d vb_shadow_vis compute shader module create");
                let pipeline = RhiDevice::create_compute_pipeline(
                    device,
                    &ComputePipelineDesc {
                        module: &cs,
                        entry: c"main",
                        // The shader reads no push; the RHI rejects a 0-byte compute range, so
                        // declare the minimum 4-byte range (bound-but-unread — the temporal
                        // reproject pipeline's own precedent, above).
                        push_constant_bytes: 4,
                        bind_group_layout: Some(&layout),
                        spec_constants: &[SpecConstant { id: 0, value: ray_count }],
                    },
                )
                .expect("invariant: R9d vb_shadow_vis compute pipeline create");
                // SAFETY: the module was created on `device` and is consumed by the pipeline
                // create; destroy it once; no GPU work is in flight yet.
                unsafe { RhiDevice::destroy_shader_module(device, cs) };
                (pipeline, layout)
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

        // Anti-aliasing Stage 1: the FXAA fullscreen pipeline + its dedicated LINEAR
        // sampler, built unconditionally here (like `present_pipeline` above) so the mode
        // can flip at runtime without a boot-time rebuild. Reuses `sample_vs` (the
        // fullscreen-triangle VS) and `present_layout` (the same single-
        // CombinedImageSampler shape); `color_formats[0]` is `aa_out`'s format
        // (`R8G8B8A8_UNORM`), NOT the swapchain format. Boot-time creation records no
        // command / writes no pixel — byte-identical to the golden regardless of this
        // pipeline's existence.
        let fxaa_sampler = RhiDevice::create_sampler(
            device,
            &SamplerDesc {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: AddressMode::ClampToEdge,
                mip: MipMode::None,
                compare: None,
            },
        )
        .expect("invariant: FXAA linear/clamp sampler create");
        let fxaa_fs = RhiDevice::create_shader_module(device, fxaa_fs_spirv())
            .expect("invariant: FXAA fragment shader module create");
        let fxaa_pipeline = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &sample_vs,
                vertex_entry: c"main",
                fragment_module: &fxaa_fs,
                fragment_entry: c"main",
                // `aa_out`'s format (== `boyko_rhi_vulkan`'s private `GBUFFER_FORMAT`
                // constant, R8G8B8A8_UNORM) — NOT `swap_format`.
                color_formats: &[Format::R8G8B8A8Unorm],
                depth_format: None,
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: None,
                push_constant_bytes: 8,
                bind_group_layout: Some(&present_layout),
                blend: None,
                cull_mode: CullMode::None,
                depth_bias: None,
            },
        )
        .expect("invariant: FXAA graphics pipeline create");

        // Anti-aliasing Stage 2: the SMAA 1x boot bundle — 2 dedicated layouts, 1 dedicated
        // sampler, 3 fullscreen pipelines, 2 boot-resident LUTs — built UNCONDITIONALLY here
        // (like `fxaa_pipeline` above) so the mode can flip at runtime without a boot-time
        // rebuild. Reuses `sample_vs` (the fullscreen-triangle VS shared by all three SMAA
        // passes) and `present_layout` (the edge pass's 1-CIS shape). Boot-time creation
        // records no command / samples no OFF pixel — byte-identical to the golden
        // regardless of this bundle's existence.
        let smaa_weight_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry {
                        binding: 0,
                        count: 1,
                        kind: DescriptorKind::CombinedImageSampler,
                        stage: ShaderStage::FRAGMENT,
                    },
                    BindGroupLayoutEntry {
                        binding: 1,
                        count: 1,
                        kind: DescriptorKind::CombinedImageSampler,
                        stage: ShaderStage::FRAGMENT,
                    },
                    BindGroupLayoutEntry {
                        binding: 2,
                        count: 1,
                        kind: DescriptorKind::CombinedImageSampler,
                        stage: ShaderStage::FRAGMENT,
                    },
                ],
            },
        )
        .expect("invariant: SMAA weight bind-group layout create");
        let smaa_blend_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry {
                        binding: 0,
                        count: 1,
                        kind: DescriptorKind::CombinedImageSampler,
                        stage: ShaderStage::FRAGMENT,
                    },
                    BindGroupLayoutEntry {
                        binding: 1,
                        count: 1,
                        kind: DescriptorKind::CombinedImageSampler,
                        stage: ShaderStage::FRAGMENT,
                    },
                ],
            },
        )
        .expect("invariant: SMAA blend bind-group layout create");
        let smaa_sampler = RhiDevice::create_sampler(
            device,
            &SamplerDesc {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: AddressMode::ClampToEdge,
                mip: MipMode::None,
                compare: None,
            },
        )
        .expect("invariant: SMAA linear/clamp sampler create");
        let smaa_edge_fs = RhiDevice::create_shader_module(device, smaa_edge_fs_spirv())
            .expect("invariant: SMAA edge fragment shader module create");
        let smaa_edge_pipeline = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &sample_vs,
                vertex_entry: c"main",
                fragment_module: &smaa_edge_fs,
                fragment_entry: c"main",
                // `smaa_edges`' format (R8G8_UNORM) — NOT `swap_format`.
                color_formats: &[Format::R8G8Unorm],
                depth_format: None,
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: None,
                push_constant_bytes: 16,
                bind_group_layout: Some(&present_layout),
                blend: None,
                cull_mode: CullMode::None,
                depth_bias: None,
            },
        )
        .expect("invariant: SMAA edge graphics pipeline create");
        let smaa_weight_fs = RhiDevice::create_shader_module(device, smaa_weight_fs_spirv())
            .expect("invariant: SMAA weight fragment shader module create");
        let smaa_weight_pipeline = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &sample_vs,
                vertex_entry: c"main",
                fragment_module: &smaa_weight_fs,
                fragment_entry: c"main",
                // `smaa_weights`' format (R8G8B8A8_UNORM) — NOT `swap_format`.
                color_formats: &[Format::R8G8B8A8Unorm],
                depth_format: None,
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: None,
                push_constant_bytes: 16,
                bind_group_layout: Some(&smaa_weight_layout),
                blend: None,
                cull_mode: CullMode::None,
                depth_bias: None,
            },
        )
        .expect("invariant: SMAA weight graphics pipeline create");
        let smaa_blend_fs = RhiDevice::create_shader_module(device, smaa_blend_fs_spirv())
            .expect("invariant: SMAA blend fragment shader module create");
        let smaa_blend_pipeline = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &sample_vs,
                vertex_entry: c"main",
                fragment_module: &smaa_blend_fs,
                fragment_entry: c"main",
                // `aa_out`'s format (R8G8B8A8_UNORM) — NOT `swap_format`.
                color_formats: &[Format::R8G8B8A8Unorm],
                depth_format: None,
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: None,
                push_constant_bytes: 16,
                bind_group_layout: Some(&smaa_blend_layout),
                blend: None,
                cull_mode: CullMode::None,
                depth_bias: None,
            },
        )
        .expect("invariant: SMAA blend graphics pipeline create");
        let smaa_area_tex = upload_texture_2d_raw(
            device,
            AREA_TEX_W,
            AREA_TEX_H,
            AREA_TEX_BYTES,
            Format::R8G8Unorm,
        )
        .expect("invariant: SMAA AreaTex LUT upload (boot stage)");
        let smaa_search_tex = upload_texture_2d_raw(
            device,
            SEARCH_TEX_W,
            SEARCH_TEX_H,
            SEARCH_TEX_BYTES,
            Format::R8Unorm,
        )
        .expect("invariant: SMAA SearchTex LUT upload (boot stage)");

        // Anti-aliasing Stage 3: the SSAA downsample fullscreen pipeline, built
        // unconditionally here (like `fxaa_pipeline` above) so the host-armed mode
        // (see `boyko_app::host::WindowHost`) can select it with no boot-time rebuild.
        // Reuses `sample_vs` + `present_layout` (the same single-CombinedImageSampler
        // shape FXAA uses) + `present_sampler` (NEAREST — the shader's `.Load` ignores
        // it, so no dedicated sampler is built). `color_formats[0]` is `aa_out`'s format
        // (`R8G8B8A8_UNORM`); NO push constants (the 2× ratio is compiled into the
        // shader). Boot-time creation records no command / writes no pixel —
        // byte-identical to the golden regardless of this pipeline's existence.
        let ssaa_fs = RhiDevice::create_shader_module(device, ssaa_downsample_fs_spirv())
            .expect("invariant: SSAA downsample fragment shader module create");
        let ssaa_pipeline = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &sample_vs,
                vertex_entry: c"main",
                fragment_module: &ssaa_fs,
                fragment_entry: c"main",
                // `aa_out`'s format (R8G8B8A8_UNORM) — NOT `swap_format`.
                color_formats: &[Format::R8G8B8A8Unorm],
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
        .expect("invariant: SSAA downsample graphics pipeline create");

        // Anti-aliasing Stage 4 (TAA W5): the temporal-resolve compute pipeline + its DEDICATED
        // 8-binding layout { `gLit` COMBINED_IMAGE_SAMPLER @0, `gViewT` STORAGE @1, `gHistIn`
        // STORAGE @2, `gHistOut` STORAGE @3, `gAaOut` STORAGE @4, the `ResolvedTaa` UBO @5, the
        // camera UBO @6 (UNJITTERED — C1 cut), the `MotionCam` UBO @7 } + a 4-byte `{ uint reset;
        // }` push constant (`boyko_render::taa_state::TaaState`) + a DEDICATED LINEAR/ClampToEdge
        // sampler for the `gLit` tap. Built UNCONDITIONALLY here (like `fxaa_pipeline`/
        // `ssaa_pipeline` above) — TAA is NOT hwrt-gated (its motion vector reconstructs from
        // `gViewT`, never a `rayQuery` trace), mirroring `shadow_temporal_pipeline`'s boot-build
        // pattern but unconditional. Boot-time creation records no command / samples no OFF pixel
        // — byte-identical to the golden regardless of this pipeline's existence (`GBufferScene::
        // taa` stays `None` unless `AaMode::Taa` is selected).
        let taa_linear_sampler = RhiDevice::create_sampler(
            device,
            &SamplerDesc {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: AddressMode::ClampToEdge,
                mip: MipMode::None,
                compare: None,
            },
        )
        .expect("invariant: TAA resolve linear/clamp sampler create");
        let taa_resolve_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                ],
            },
        )
        .expect("invariant: TAA resolve bind-group layout create");
        let taa_resolve_cs = RhiDevice::create_shader_module(device, taa_resolve_spirv())
            .expect("invariant: TAA resolve compute shader module create");
        let taa_resolve_pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &taa_resolve_cs,
                entry: c"main",
                // The shader reads the reset flag via a 4-byte `{ uint reset; }` COMPUTE push
                // range (`boyko_render::taa_state::TaaState::advance`'s consumed-this-frame bit).
                push_constant_bytes: 4,
                bind_group_layout: Some(&taa_resolve_layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: TAA resolve compute pipeline create");

        // TAA rung T3: the post-resolve RCAS sharpen compute pipeline + its DEDICATED 2-binding
        // layout { `gRcasIn` STORAGE @0, `gAaOut` STORAGE @1 } + a 16-byte `RcasPush` COMPUTE
        // push range. Built UNCONDITIONALLY here (like `taa_resolve_pipeline` above) — RCAS is
        // NOT hwrt-gated (a pure image-space kernel). Boot-time creation records no command /
        // samples no OFF pixel — byte-identical to the golden regardless of this pipeline's
        // existence (`GBufferScene::rcas` stays `None` unless `SharpenMode::Rcas` is selected
        // AND `AaMode::Taa` is armed).
        let rcas_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                ],
            },
        )
        .expect("invariant: RCAS bind-group layout create");
        let rcas_cs = RhiDevice::create_shader_module(device, rcas_spirv())
            .expect("invariant: RCAS compute shader module create");
        let rcas_pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &rcas_cs,
                entry: c"main",
                push_constant_bytes: RCAS_PUSH_BYTES,
                bind_group_layout: Some(&rcas_layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: RCAS compute pipeline create");

        // Render P7-Q2: the SSAO compute pass — 3 pre-compiled quality-variant pipelines
        // sharing ONE dedicated 5-binding layout, built UNCONDITIONALLY here (like
        // `fxaa_pipeline`/`ssaa_pipeline`/`taa_resolve_pipeline` above) so the
        // owner-resolved quality (`boyko_render::ResolvedSsao::variant`) can bind a
        // variant with no boot-time rebuild. Mirrors the test harness's SSAO boot bundle
        // (`window_present_gbuffer.rs`'s `ssao_layout`/`ssao_pipeline` construction),
        // widened to build all 3 variants instead of one. Boot-time creation records no
        // command / samples no pixel — byte-identical to the golden regardless of this
        // bundle's existence (`GBufferScene::ssao` stays `None` unless a non-`Off`
        // `SsaoQuality` is host-resolved).
        let ssao_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                ],
            },
        )
        .expect("invariant: Render P7 SSAO bind-group layout create");
        let ssao_cs_low = RhiDevice::create_shader_module(device, sdf_ssao_spirv_variant(SSAO_QUALITY_LOW))
            .expect("invariant: SSAO LOW compute shader module create");
        let ssao_pipeline_low = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &ssao_cs_low,
                entry: c"main",
                push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
                bind_group_layout: Some(&ssao_layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: SSAO LOW compute pipeline create");
        let ssao_cs_medium = RhiDevice::create_shader_module(device, sdf_ssao_spirv_variant(SSAO_QUALITY_MEDIUM))
            .expect("invariant: SSAO MEDIUM compute shader module create");
        let ssao_pipeline_medium = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &ssao_cs_medium,
                entry: c"main",
                push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
                bind_group_layout: Some(&ssao_layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: SSAO MEDIUM compute pipeline create");
        let ssao_cs_high = RhiDevice::create_shader_module(device, sdf_ssao_spirv_variant(SSAO_QUALITY_HIGH))
            .expect("invariant: SSAO HIGH compute shader module create");
        let ssao_pipeline_high = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &ssao_cs_high,
                entry: c"main",
                push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
                bind_group_layout: Some(&ssao_layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: SSAO HIGH compute pipeline create");
        // Indexed by `SSAO_QUALITY_LOW`/`_MEDIUM`/`_HIGH` (0/1/2) — `ResolvedSsao::variant`
        // selects directly into this array.
        let ssao_pipelines = [ssao_pipeline_low, ssao_pipeline_medium, ssao_pipeline_high];

        // Rung R9b (docs/R9-VB-SPLIT-PLAN.md §5/§6): the VB split's boot-buildable objects —
        // the `-D VB_THIN` gather trio + its dense 4-binding layout, `vb_geo`'s Set-1 aux
        // layout, and `vb_shade_split`'s Set-1 layout. Pipelines that need the geometry Set-2
        // (`vb_geo`/`vb_shade_split{,_tex}`) are deferred to `build_vb_split_pipelines` (the
        // `build_vb_resolve_pipeline` two-dependency reason).
        let vb_ssao_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    // thin_normal @0 (R), gViewT @1 (R), ssao @2 (W), camera UBO @3 — the
                    // VB_THIN dense table (`sdf_ssao.comp.hlsl`'s own header doc).
                    BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                ],
            },
        )
        .expect("invariant: R9b VB SSAO bind-group layout create");
        let mut ssao_vb_slots: [Option<ComputePipeline>; SSAO_QUALITY_COUNT] =
            [const { None }; SSAO_QUALITY_COUNT];
        for (variant, slot) in ssao_vb_slots.iter_mut().enumerate() {
            let cs = RhiDevice::create_shader_module(device, sdf_ssao_vb_spirv(variant))
                .expect("invariant: R9b VB_THIN SSAO compute shader module create");
            let p = RhiDevice::create_compute_pipeline(
                device,
                &ComputePipelineDesc {
                    module: &cs,
                    entry: c"main",
                    // The SAME push block the base gather compiles (one source, the VB_THIN
                    // define only swaps the binding table).
                    push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
                    bind_group_layout: Some(&vb_ssao_layout),
                    spec_constants: &[],
                },
            )
            .expect("invariant: R9b VB_THIN SSAO compute pipeline create");
            // SAFETY: the module was created on `device` and consumed by the pipeline create;
            // destroyed once; no GPU work is in flight yet.
            unsafe {
                RhiDevice::destroy_shader_module(device, cs);
            }
            *slot = Some(p);
        }
        let ssao_vb_pipelines = ssao_vb_slots
            .map(|s| s.expect("invariant: every VB_THIN SSAO variant pipeline built above"));
        let vb_geo_aux_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    // thin_normal @0 (W). @1/@2 are the R9d MOTION slots — in the layout now
                    // (one layout object for both variant generations, the R2 contract; the
                    // software `vb_geo.comp.spv` never references them).
                    BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                ],
            },
        )
        .expect("invariant: R9b vb_geo aux bind-group layout create");
        // Rung R9d: the @8 `gShadowVis` STORAGE slot joins the layout together with the
        // `-D HWRT` `vb_shade_split` shader variant that references it — the R9b `.spv` is
        // compiled without the define on BOTH legs, so a `not(hwrt)` build's entry would be pure
        // dead surface (the layout stays EXACT-FILL at 8 there).
        let vb_split_layout1_base: [BindGroupLayoutEntry; 8] = [
            // @0-3: forward_layout1's shadow-table kinds VERBATIM (a DISTINCT object —
            // the Forward family's own layout stays byte-untouched).
            BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
            // @4: gSsao (STORAGE, READ by the split shade under the header ssao_mode gate).
            BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
            // @5/@6: the DDGI atlases (COMBINED image+sampler — the deferred t16/s16 idiom).
            BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            // @7: the ResolvedDdgi UBO.
            BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        ];
        #[cfg(not(feature = "hwrt"))]
        let vb_split_layout1_entries = vb_split_layout1_base;
        #[cfg(feature = "hwrt")]
        let vb_split_layout1_entries: [BindGroupLayoutEntry; 9] = {
            let ninth = BindGroupLayoutEntry {
                binding: 8,
                count: 1,
                kind: DescriptorKind::StorageImage,
                stage: ShaderStage::COMPUTE,
            };
            let mut chained = vb_split_layout1_base.into_iter().chain(core::iter::once(ninth));
            core::array::from_fn(|_| chained.next().expect("invariant: exactly 9 entries"))
        };
        let vb_split_layout1 = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc { entries: &vb_split_layout1_entries },
        )
        .expect("invariant: R9b vb_shade_split Set-1 bind-group layout create");

        // The SSAO edge-avoiding à-trous denoise chain (RHI DISPATCH WIRING follow-up to the
        // gather above): the shared 4-binding layout { `gAoIn` @0 (R), `gAoOut` @1 (W) STORAGE
        // images, `gViewT` @2 (R) STORAGE image, the camera UNIFORM buffer @3 } — matching
        // `ssao_atrous.comp.hlsl`'s set 0 — plus the three role-keyed pipeline variants
        // (read8/interior/write8; see that shader's header doc for the R8<->R16 format-pin
        // rationale). Built UNCONDITIONALLY at boot (like `ssao_pipelines` above — the pipeline
        // itself needs NO device precondition to CREATE; only the interior ring IMAGE needs
        // `R16_UNORM` storage, checked separately by `GBufferTargets`'s degrade,
        // `ssao_atrous_storage_ok()`). A 4-byte `{ uint step }` COMPUTE push range
        // (`SSAO_ATROUS_PUSH_BYTES`) — the current à-trous level's hole width.
        let ssao_atrous_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                ],
            },
        )
        .expect("invariant: SSAO à-trous bind-group layout create");
        let ssao_atrous_read8_cs = RhiDevice::create_shader_module(device, ssao_atrous_read8_spirv())
            .expect("invariant: SSAO à-trous read8 compute shader module create");
        let ssao_atrous_read8_pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &ssao_atrous_read8_cs,
                entry: c"main",
                push_constant_bytes: SSAO_ATROUS_PUSH_BYTES,
                bind_group_layout: Some(&ssao_atrous_layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: SSAO à-trous read8 compute pipeline create");
        let ssao_atrous_interior_cs = RhiDevice::create_shader_module(device, ssao_atrous_spirv())
            .expect("invariant: SSAO à-trous interior compute shader module create");
        let ssao_atrous_interior_pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &ssao_atrous_interior_cs,
                entry: c"main",
                push_constant_bytes: SSAO_ATROUS_PUSH_BYTES,
                bind_group_layout: Some(&ssao_atrous_layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: SSAO à-trous interior compute pipeline create");
        let ssao_atrous_write8_cs = RhiDevice::create_shader_module(device, ssao_atrous_write8_spirv())
            .expect("invariant: SSAO à-trous write8 compute shader module create");
        let ssao_atrous_write8_pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &ssao_atrous_write8_cs,
                entry: c"main",
                push_constant_bytes: SSAO_ATROUS_PUSH_BYTES,
                bind_group_layout: Some(&ssao_atrous_layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: SSAO à-trous write8 compute pipeline create");

        // Multi-paradigm render-path plan, rung R3b (`Deferred × Mesh` — the SDF leg fully off):
        // the `viewt_from_depth` `gViewT`-producer pipeline — the dedicated 2-binding layout
        // { SAMPLED depth @0, STORAGE `gViewT` @1 } + the 12-byte `ViewtFromDepthPush` COMPUTE
        // push range. Built UNCONDITIONALLY here (like `ssao_pipelines`/`ssao_atrous_*` above —
        // the pipeline itself needs no device precondition to CREATE); `GBufferScene::
        // path_has_viewt_from_depth` gates whether the pass is DECLARED/RECORDED/dispatched, so a
        // `Both`/`Sdf`-resolved boot pays only this one negligible pipeline object.
        let viewt_from_depth_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                ],
            },
        )
        .expect("invariant: viewt_from_depth bind-group layout create");
        let viewt_from_depth_cs = RhiDevice::create_shader_module(device, viewt_from_depth_spirv())
            .expect("invariant: viewt_from_depth compute shader module create");
        let viewt_from_depth_pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &viewt_from_depth_cs,
                entry: c"main",
                push_constant_bytes: VIEWT_FROM_DEPTH_PUSH_BYTES,
                bind_group_layout: Some(&viewt_from_depth_layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: viewt_from_depth compute pipeline create");

        // TAA-under-VB: the `viewt_from_depth_rz` REVERSE-Z sibling — the dedicated 3-binding
        // layout { SAMPLED depth @0, STORAGE `gViewT` @1, UNIFORM camera @2 } + the 16-byte
        // `ViewtFromDepthRzPush` COMPUTE push range. Built UNCONDITIONALLY here (like
        // `viewt_from_depth_pipeline` above — the pipeline itself needs no device precondition to
        // CREATE); `GBufferScene::viewt_from_vb_depth` gates whether it is actually dispatched, so
        // a non-VB or TAA-off boot pays only this one negligible pipeline object.
        let viewt_from_vb_depth_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                ],
            },
        )
        .expect("invariant: viewt_from_depth_rz bind-group layout create");
        let viewt_from_vb_depth_cs =
            RhiDevice::create_shader_module(device, viewt_from_depth_rz_spirv())
                .expect("invariant: viewt_from_depth_rz compute shader module create");
        let viewt_from_vb_depth_pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &viewt_from_vb_depth_cs,
                entry: c"main",
                push_constant_bytes: VIEWT_FROM_DEPTH_RZ_PUSH_BYTES,
                bind_group_layout: Some(&viewt_from_vb_depth_layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: viewt_from_depth_rz compute pipeline create");

        // The shader modules are consumed by pipeline creation; destroy them now
        // (mirrors the showcase's post-create module teardown).
        // SAFETY: every module was created on `ctx` above and is no longer
        // needed once its pipeline exists; each is destroyed exactly once; no
        // GPU work has been submitted yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, viewt_from_vb_depth_cs);
            RhiDevice::destroy_shader_module(device, viewt_from_depth_cs);
            RhiDevice::destroy_shader_module(device, ssao_atrous_write8_cs);
            RhiDevice::destroy_shader_module(device, ssao_atrous_interior_cs);
            RhiDevice::destroy_shader_module(device, ssao_atrous_read8_cs);
            RhiDevice::destroy_shader_module(device, taa_resolve_cs);
            RhiDevice::destroy_shader_module(device, rcas_cs);
            RhiDevice::destroy_shader_module(device, ssao_cs_high);
            RhiDevice::destroy_shader_module(device, ssao_cs_medium);
            RhiDevice::destroy_shader_module(device, ssao_cs_low);
            RhiDevice::destroy_shader_module(device, ssaa_fs);
            RhiDevice::destroy_shader_module(device, smaa_blend_fs);
            RhiDevice::destroy_shader_module(device, smaa_weight_fs);
            RhiDevice::destroy_shader_module(device, smaa_edge_fs);
            RhiDevice::destroy_shader_module(device, fxaa_fs);
            RhiDevice::destroy_shader_module(device, sample_fs);
            RhiDevice::destroy_shader_module(device, sample_vs);
            RhiDevice::destroy_shader_module(device, resolve_cs);
            RhiDevice::destroy_shader_module(device, resolve_wrap_cs);
            RhiDevice::destroy_shader_module(device, cs);
            RhiDevice::destroy_shader_module(device, fs);
            RhiDevice::destroy_shader_module(device, vs);
        }

        // ── CSM + shadow-atlas trios: ALWAYS created (resolve @12..=15 need
        // valid descriptors); both depth passes stay OFF in R3.
        let csm = CsmResources::create(device, &instance_layout);

        // ── Multi-paradigm render-path plan, rung R4b-b (Set 0 UNIFIED at rung R5 code-review
        // fix): the Forward FAMILY's mesh raster pipeline(s) + Set-0 (core)/Set-1 (shadow)
        // bind-group layouts. Built UNCONDITIONALLY at boot (the `ssao_pipelines` precedent —
        // `ResolvedRenderPath` does not reach `boot()`'s call site, and the pipelines are cheap
        // to create; only a Forward-family-resolved boot ever RECORDS a given pass).
        //
        // Code-review fix (rung R5): an earlier revision built TWO Set-0 layouts — a 5-binding
        // `forward_layout0` for plain `Forward` and a SEPARATE 7-binding `forward_plus_layout0`
        // for `ForwardPlus` — but every pipeline in this family shares ONE per-extent descriptor
        // SET (`ForwardTargets::set0[fi]`, written ONCE against whichever layout the boot path
        // selects). A pipeline created against a DIFFERENT `VkDescriptorSetLayout` object than
        // the one the bound `VkDescriptorSet` was allocated from is INCOMPATIBLE at draw time
        // (`VUID-vkCmdDrawIndexed-None-02699` family) even when the two layouts declare
        // byte-identical binding shapes — Vulkan compares LAYOUT HANDLES, not structural
        // equivalence. With validation disabled this manifested as a SILENT no-op (every Forward
        // draw skipped, the frame showing nothing but the `lit` clear color) whenever a
        // `forward_sky_pipeline`/`forward_prepass_pipeline` built against the 5-binding handle
        // was bound alongside a `ForwardPlus`-resolved `forward.set0[fi]` built against the
        // 7-binding handle. FIX: exactly ONE Set-0 layout for the WHOLE family — `ClusterGrid`/
        // `LightIndexList` @5/@6 are bound-but-unread placeholders (`scene.light_table`, the
        // established idiom) under plain `Forward`; every one of the four pipelines below
        // (`forward_pipeline` GREATER, `forward_sky_pipeline`, `forward_prepass_pipeline`,
        // `forward_plus_pipeline` EQUAL) is built against THIS SAME layout object, and
        // `ForwardTargets::build` writes `forward.set0[fi]` against it UNCONDITIONALLY (7
        // entries every boot — `targets.rs`'s doc). Binding shape: instances @0 (VERTEX),
        // instance_materials @1 (VERTEX), Camera @2 (FRAGMENT), LightBuf @3 (FRAGMENT),
        // Materials @4 (FRAGMENT), ClusterGrid @5 (FRAGMENT), LightIndexList @6 (FRAGMENT) —
        // `forward_opaque.fs.hlsl`'s doc (bindings 5/6 declared only under `-D FROXEL=1`, but a
        // pipeline layout may always be a SUPERSET of what a given shader stage references).
        //
        // Set 1 binding shape: `gCsm`+`gCsmCmp` @0 (FRAGMENT, combined), `CsmCascades` @1
        // (FRAGMENT), `gShadowAtlas`+`gShadowAtlasCmp` @2 (FRAGMENT, combined), `ShadowAtlas`
        // @3 (FRAGMENT) — `forward_opaque.fs.hlsl`'s OWN binding numbers (a DIFFERENT layout
        // than the deferred resolve's single compute set, same underlying resources).
        //
        // Boot-panic fix: renumbered from an original Set 2 design (with an empty Set-1
        // PLACEHOLDER layout in between, for Vulkan's contiguous-set-index rule). A zero-binding
        // `BindGroupLayoutDesc` violates `create_bind_group_layout`'s own `1..=
        // MAX_BIND_GROUP_BINDINGS` invariant (`rhi_impl/device.rs:205`) — a real
        // `GpuSceneBundles::boot` panic. `forward_opaque.fs.hlsl`'s shadow bindings were
        // renumbered to Set 1 instead (that shader's doc), so Forward is a plain 2-set
        // `[Set0, Set1]` pipeline — no placeholder needed.
        let forward_layout1 = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry {
                        binding: 0,
                        count: 1,
                        kind: DescriptorKind::CombinedImageSampler,
                        stage: ShaderStage::FRAGMENT,
                    },
                    BindGroupLayoutEntry {
                        binding: 1,
                        count: 1,
                        kind: DescriptorKind::UniformBuffer,
                        stage: ShaderStage::FRAGMENT,
                    },
                    BindGroupLayoutEntry {
                        binding: 2,
                        count: 1,
                        kind: DescriptorKind::CombinedImageSampler,
                        stage: ShaderStage::FRAGMENT,
                    },
                    BindGroupLayoutEntry {
                        binding: 3,
                        count: 1,
                        kind: DescriptorKind::UniformBuffer,
                        stage: ShaderStage::FRAGMENT,
                    },
                ],
            },
        )
        .expect("invariant: Forward Set-1 bind-group layout create");
        // The UNIFIED Set-0 (core) layout — 7 bindings, shared by EVERY Forward-family pipeline
        // (see the block comment above `forward_layout1` for the code-review fix rationale).
        let forward_layout0 = RhiDevice::create_bind_group_layout(
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
                        stage: ShaderStage::FRAGMENT,
                    },
                    BindGroupLayoutEntry {
                        binding: 3,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::FRAGMENT,
                    },
                    BindGroupLayoutEntry {
                        binding: 4,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::FRAGMENT,
                    },
                    // ClusterGrid @5 (the L1 froxel cell array, `{offset,count}` per froxel) —
                    // bound-but-unread under plain `Forward` (the base FS never declares this
                    // binding; `ForwardTargets::build` fills the slot with `scene.light_table`).
                    BindGroupLayoutEntry {
                        binding: 5,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::FRAGMENT,
                    },
                    // LightIndexList @6 (the per-froxel light-index slices) — same bound-but-
                    // unread discipline as @5 under plain `Forward`.
                    BindGroupLayoutEntry {
                        binding: 6,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::FRAGMENT,
                    },
                ],
            },
        )
        .expect("invariant: Forward Set-0 bind-group layout create");
        let forward_vs = RhiDevice::create_shader_module(device, forward_opaque_vs_spirv())
            .expect("invariant: Forward mesh vertex shader module create");
        let forward_fs = RhiDevice::create_shader_module(device, forward_opaque_fs_spirv())
            .expect("invariant: Forward mesh fragment shader module create");
        let forward_pipeline = ctx
            .create_graphics_pipeline_forward(
                &GraphicsPipelineDesc {
                    vertex_module: &forward_vs,
                    vertex_entry: c"main",
                    fragment_module: &forward_fs,
                    fragment_entry: c"main",
                    // ONE color attachment (`lit`, reused from the Deferred allocation, C5) —
                    // unlike the 3/4-MRT Deferred raster pipelines.
                    color_formats: &[RASTER_COLOR_FORMAT],
                    // Forward's OWN reverse-Z depth image (a SEPARATE allocation from Deferred's
                    // custom-linear `depth` — Decision 4); the format is the SAME `D32Sfloat`.
                    depth_format: Some(Format::D32Sfloat),
                    topology: PrimitiveTopology::TriangleList,
                    vertex_layout: Some(VertexBufferLayout {
                        stride: MESH_VERTEX_STRIDE as u32,
                        attributes: &attributes,
                    }),
                    push_constant_bytes: GBUFFER_PUSH_BYTES as u32,
                    bind_group_layout: Some(&forward_layout0),
                    blend: None,
                    cull_mode: CullMode::None,
                    depth_bias: None,
                },
                forward_layout1.set_layout(),
            )
            .expect("invariant: Forward mesh graphics pipeline create");
        // SAFETY: both modules were created on `device` and are consumed by the pipeline
        // create; each is destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, forward_fs);
            RhiDevice::destroy_shader_module(device, forward_vs);
        }

        // ── Code-review follow-up (rung R4b-b): the Forward v1 sky BACKGROUND pipeline
        // (`forward_sky.{vs,fs}.hlsl`) — replicates the deferred resolve's analytic sky/ground
        // gradient + sun disc for `mask == 0` pixels (`deferred_pbr.hlsl:1369-1414`), drawn FIRST
        // inside `forward_opaque`'s SAME dynamic-rendering scope so opaque geometry then draws
        // over it. REUSES `forward_layout0` (its FS reads only Camera @2 + LightBuf @3, a subset
        // of that layout's 5 bindings — the SAME "bound-but-unread subset" idiom every other
        // pipeline in this fn's set already relies on) via the ORDINARY `create_graphics_pipeline`
        // (a plain 1-set pipeline — the sky FS never reads the shadow Set-1 layout, so
        // `create_graphics_pipeline_forward`'s 2-set shape is unneeded here). `depth_format:
        // None`: no depth attachment declared (Vulkan permits recording a pipeline with
        // `depthAttachmentFormat == UNDEFINED` inside a rendering scope that DOES bind a depth
        // attachment — this pipeline simply neither tests nor writes it), so `record_forward`
        // draws it with depth test/write OFF while `forward_pipeline`'s own real
        // `VK_COMPARE_OP_GREATER` depth-write pass (drawn right after, same scope) is untouched.
        // No vertex buffer (`vertex_layout: None`, `SV_VertexID`-only fullscreen triangle) and no
        // push constants (`push_constant_bytes: 0`).
        let sky_vs = RhiDevice::create_shader_module(device, forward_sky_vs_spirv())
            .expect("invariant: Forward sky vertex shader module create");
        let sky_fs = RhiDevice::create_shader_module(device, forward_sky_fs_spirv())
            .expect("invariant: Forward sky fragment shader module create");
        let forward_sky_pipeline = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &sky_vs,
                vertex_entry: c"main",
                fragment_module: &sky_fs,
                fragment_entry: c"main",
                color_formats: &[RASTER_COLOR_FORMAT],
                depth_format: None,
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: None,
                push_constant_bytes: 0,
                bind_group_layout: Some(&forward_layout0),
                blend: None,
                cull_mode: CullMode::None,
                depth_bias: None,
            },
        )
        .expect("invariant: Forward sky graphics pipeline create");
        // SAFETY: both modules were created on `device` and are consumed by the pipeline
        // create; each is destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, sky_fs);
            RhiDevice::destroy_shader_module(device, sky_vs);
        }

        // ── Multi-paradigm render-path plan, rung R5 (ForwardPlus): the depth PRE-PASS
        // pipeline (Decision 4's EQUAL-depth early-Z contract). Built UNCONDITIONALLY at boot
        // (the SAME "cheap, no per-frame cost either way" precedent as the Forward v1 trio
        // above — `ResolvedRenderPath` does not reach `boot()`'s call site); only a
        // `ForwardPlus`-resolved boot ever RECORDS it. Reuses `forward_layout0` as its ONLY set
        // (the prepass VS references only the `instances` binding, a subset of that layout's 5
        // bindings — the SAME bound-but-unread-subset idiom `forward_sky_pipeline` already
        // establishes, so no new bind-group layout is needed for this pipeline). DEPTH-ONLY:
        // `color_formats: &[]` — the SAME zero-color-attachment shape `build_graphics_pipeline`
        // already builds for the CSM/atlas shadow-map pipelines.
        let prepass_vs = RhiDevice::create_shader_module(device, depth_prepass_vs_spirv())
            .expect("invariant: ForwardPlus depth-prepass vertex shader module create");
        let prepass_fs = RhiDevice::create_shader_module(device, depth_prepass_fs_spirv())
            .expect("invariant: ForwardPlus depth-prepass fragment shader module create");
        let forward_prepass_pipeline = ctx
            .create_graphics_pipeline_forward_prepass(&GraphicsPipelineDesc {
                vertex_module: &prepass_vs,
                vertex_entry: c"main",
                fragment_module: &prepass_fs,
                fragment_entry: c"main",
                color_formats: &[],
                depth_format: Some(Format::D32Sfloat),
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: Some(VertexBufferLayout {
                    stride: MESH_VERTEX_STRIDE as u32,
                    attributes: &attributes,
                }),
                push_constant_bytes: GBUFFER_PUSH_BYTES as u32,
                bind_group_layout: Some(&forward_layout0),
                blend: None,
                cull_mode: CullMode::None,
                depth_bias: None,
            })
            .expect("invariant: ForwardPlus depth-prepass graphics pipeline create");
        // SAFETY: both modules were created on `device` and are consumed by the pipeline
        // create; each is destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, prepass_fs);
            RhiDevice::destroy_shader_module(device, prepass_vs);
        }

        // ── Multi-paradigm render-path plan, rung R5 (ForwardPlus): the `forward_opaque` FROXEL
        // pipeline variant (`VK_COMPARE_OP_EQUAL`, depth-write OFF) — built against the SAME
        // UNIFIED `forward_layout0`/`forward_layout1` every other Forward-family pipeline uses
        // (the code-review fix above; NO separate layout object). Built UNCONDITIONALLY at boot,
        // same precedent as above.
        let forward_plus_vs = RhiDevice::create_shader_module(device, forward_opaque_vs_spirv())
            .expect("invariant: ForwardPlus opaque vertex shader module create");
        let forward_plus_fs =
            RhiDevice::create_shader_module(device, forward_opaque_froxel_fs_spirv())
                .expect("invariant: ForwardPlus opaque froxel fragment shader module create");
        let forward_plus_pipeline = ctx
            .create_graphics_pipeline_forward_plus(
                &GraphicsPipelineDesc {
                    vertex_module: &forward_plus_vs,
                    vertex_entry: c"main",
                    fragment_module: &forward_plus_fs,
                    fragment_entry: c"main",
                    color_formats: &[RASTER_COLOR_FORMAT],
                    depth_format: Some(Format::D32Sfloat),
                    topology: PrimitiveTopology::TriangleList,
                    vertex_layout: Some(VertexBufferLayout {
                        stride: MESH_VERTEX_STRIDE as u32,
                        attributes: &attributes,
                    }),
                    push_constant_bytes: GBUFFER_PUSH_BYTES as u32,
                    bind_group_layout: Some(&forward_layout0),
                    blend: None,
                    cull_mode: CullMode::None,
                    depth_bias: None,
                },
                forward_layout1.set_layout(),
            )
            .expect("invariant: ForwardPlus opaque froxel graphics pipeline create");
        // SAFETY: both modules were created on `device` and are consumed by the pipeline
        // create; each is destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, forward_plus_fs);
            RhiDevice::destroy_shader_module(device, forward_plus_vs);
        }

        // ── Multi-paradigm render-path plan, rung R-SDFFWD (+ the TAA-under-VB `VIEWT` rung):
        // the `sdf_forward_march` pass's Set-0 vocabulary bind-group LAYOUT (14 bindings,
        // matching the shader's own binding table doc — `shaders/sdf_forward_march.comp.hlsl`'s
        // header) + its FOUR `{HAS_MESH} x {VIEWT}` compute pipeline variants.
        // Built UNCONDITIONALLY at boot (the Forward v1 trio's own "cheap, no per-frame cost
        // either way" precedent — `ResolvedRenderPath` does not reach `boot()`'s call site); only
        // a Forward-family-resolved boot with the SDF leg present ever RECORDS the pass. ALL
        // pipeline variants are built against this ONE layout object (the code-review-fixed "one
        // layout per pipeline family" discipline `forward_layout0` already establishes) at Set 0,
        // + `forward_layout1` at Set 1 (the shadow set, REUSED VERBATIM — no separate layout):
        // @12 is HAS_MESH-referenced only and @13 VIEWT-referenced only, bound-but-unread by the
        // other variants (the R2 contract).
        let sdf_forward_march_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    // t0: edit-list header (READ-ONLY).
                    BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    // t1: LightBuf (Lighting L0 light table).
                    BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    // t2: Materials (PBR material table).
                    BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    // b3: Camera (80-byte extent/camera block).
                    BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                    // u4: gLit STORAGE image.
                    BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                    // t5: M1/M4 level-0 pointer grid.
                    BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    // t6/s6: M2/M4 level-0 trilinear atlas (combined).
                    BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
                    // t7: M4 level-1 pointer grid.
                    BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    // t8/s8: M4 level-1 trilinear atlas (combined).
                    BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
                    // t9: M4 level-2 pointer grid.
                    BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    // t10/s10: M4 level-2 trilinear atlas (combined).
                    BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
                    // b11: BrickLevels UBO (M4Level clip-map geometry, don't-care while brick_levels==0).
                    BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                    // t12: gForwardDepth SAMPLED (declared for ALL pipeline variants — the R2
                    // bound-but-unread contract; the mesh-less SPIR-V never statically references it).
                    BindGroupLayoutEntry { binding: 12, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
                    // u13: gViewT STORAGE (TAA-under-VB; VIEWT-variant-referenced only — the
                    // no-VIEWT SPIR-V never statically references it, the SAME R2 contract).
                    BindGroupLayoutEntry { binding: 13, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                ],
            },
        )
        .expect("invariant: sdf_forward_march Set-0 bind-group layout create");
        let sdf_forward_march_cs = RhiDevice::create_shader_module(device, sdf_forward_march_spirv())
            .expect("invariant: sdf_forward_march HAS_MESH compute shader module create");
        let sdf_forward_march_pipeline = ctx
            .create_compute_pipeline_forward(
                &ComputePipelineDesc {
                    module: &sdf_forward_march_cs,
                    entry: c"main",
                    push_constant_bytes: SDF_FORWARD_MARCH_PUSH_BYTES,
                    bind_group_layout: Some(&sdf_forward_march_layout),
                    spec_constants: &[],
                },
                forward_layout1.set_layout(),
            )
            .expect("invariant: sdf_forward_march HAS_MESH compute pipeline create");
        let sdf_forward_march_sdfonly_cs =
            RhiDevice::create_shader_module(device, sdf_forward_march_sdfonly_spirv())
                .expect("invariant: sdf_forward_march mesh-less compute shader module create");
        let sdf_forward_march_sdfonly_pipeline = ctx
            .create_compute_pipeline_forward(
                &ComputePipelineDesc {
                    module: &sdf_forward_march_sdfonly_cs,
                    entry: c"main",
                    push_constant_bytes: SDF_FORWARD_MARCH_PUSH_BYTES,
                    bind_group_layout: Some(&sdf_forward_march_layout),
                    spec_constants: &[],
                },
                forward_layout1.set_layout(),
            )
            .expect("invariant: sdf_forward_march mesh-less compute pipeline create");
        // TAA-under-VB: the two `VIEWT` gViewT-producing siblings (same layout, same push).
        let sdf_forward_march_viewt_cs =
            RhiDevice::create_shader_module(device, sdf_forward_march_viewt_spirv())
                .expect("invariant: sdf_forward_march HAS_MESH+VIEWT compute shader module create");
        let sdf_forward_march_viewt_pipeline = ctx
            .create_compute_pipeline_forward(
                &ComputePipelineDesc {
                    module: &sdf_forward_march_viewt_cs,
                    entry: c"main",
                    push_constant_bytes: SDF_FORWARD_MARCH_PUSH_BYTES,
                    bind_group_layout: Some(&sdf_forward_march_layout),
                    spec_constants: &[],
                },
                forward_layout1.set_layout(),
            )
            .expect("invariant: sdf_forward_march HAS_MESH+VIEWT compute pipeline create");
        let sdf_forward_march_sdfonly_viewt_cs =
            RhiDevice::create_shader_module(device, sdf_forward_march_sdfonly_viewt_spirv())
                .expect("invariant: sdf_forward_march mesh-less VIEWT compute shader module create");
        let sdf_forward_march_sdfonly_viewt_pipeline = ctx
            .create_compute_pipeline_forward(
                &ComputePipelineDesc {
                    module: &sdf_forward_march_sdfonly_viewt_cs,
                    entry: c"main",
                    push_constant_bytes: SDF_FORWARD_MARCH_PUSH_BYTES,
                    bind_group_layout: Some(&sdf_forward_march_layout),
                    spec_constants: &[],
                },
                forward_layout1.set_layout(),
            )
            .expect("invariant: sdf_forward_march mesh-less VIEWT compute pipeline create");
        // SAFETY: all four modules were created on `device` and are consumed by their respective
        // pipeline create calls above; each is destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, sdf_forward_march_sdfonly_viewt_cs);
            RhiDevice::destroy_shader_module(device, sdf_forward_march_viewt_cs);
            RhiDevice::destroy_shader_module(device, sdf_forward_march_sdfonly_cs);
            RhiDevice::destroy_shader_module(device, sdf_forward_march_cs);
        }

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

                // The MV pipeline: identical to `raster_pipeline` (same 64-byte vertex layout, D32
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
                            stride: MESH_VERTEX_STRIDE as u32,
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

                // F8-mv: the combined pipeline — identical to `mv_pipeline` (same 64-byte
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
                            stride: MESH_VERTEX_STRIDE as u32,
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

        // ── Multi-paradigm render-path plan, rung R8: the VisibilityBuffer v1 (fused
        // `vb_resolve`) Set-0 layout + `vb_raster`/`vb_sky` pipelines — built UNCONDITIONALLY at
        // boot (the Forward v1 trio's own "cheap, no per-frame cost either way" precedent).
        // `vb_resolve_pipeline` itself is built LAZILY (`build_vb_resolve_pipeline`, mirroring
        // `build_textured_resources`'s deferred-build shape) because its Set 2 needs the
        // Decision-0 geometry table's layout, which does not exist at this call site (the SAME
        // "does not exist yet" reason `tex` is lazy — see that field's doc).
        //
        // Binding numbers match `forward_layout0`'s own for the shared subset (instances @0,
        // instance_materials @1, Camera @2, LightBuf @3, Materials @4) so `vb_sky_pipeline` can
        // reuse `forward_sky.{vs,fs}.hlsl`'s compiled SPIR-V verbatim (its FS references ONLY
        // Camera @2 + LightBuf @3, a bound-but-unread subset). `gVbId` @5 (SAMPLED) + `gLit` @6
        // (STORAGE) are VB-only, appended after the shared subset.
        let vb_layout0 = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry {
                        binding: 0,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::VERTEX | ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 1,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 2,
                        count: 1,
                        kind: DescriptorKind::UniformBuffer,
                        stage: ShaderStage::FRAGMENT | ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 3,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::FRAGMENT | ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 4,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 5,
                        count: 1,
                        kind: DescriptorKind::SampledImage,
                        stage: ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 6,
                        count: 1,
                        kind: DescriptorKind::StorageImage,
                        stage: ShaderStage::COMPUTE,
                    },
                    // VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2a
                    // (dark infra): `gClassify` @7 (COMPUTE-only STORAGE_BUFFER, the packed
                    // classify buffer `shaders/vb_classify_common.hlsli` declares). Added to
                    // this ONE shared layout object BEFORE any VB pipeline is built below (R5 —
                    // a set built against a DIFFERENT, structurally-identical layout object is
                    // silently incompatible with a pipeline built against this one), so
                    // `vb_raster_pipeline`/`vb_sky_pipeline`/`vb_resolve_pipeline` all rebuild
                    // against the 8-binding layout; bound-but-unread by their frozen SPIR-V
                    // (none of the three declares a `binding(7,0)`).
                    BindGroupLayoutEntry {
                        binding: 7,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                    // VB-SV0 (docs/VB-SV0-SDF-SHADOW-PLAN.md §2), rung S2 ("dark infra"): the SDF
                    // edit-list SSBO at slot 10 — `Buf`, the analytic `field_distance` source the
                    // three VB lit-producer tails now declare. Added to this ONE shared layout
                    // object BEFORE any VB pipeline is built (the SAME R5 reason `gClassify` @7
                    // was: a set built against a structurally-identical-but-DISTINCT layout object
                    // is silently incompatible), so `vb_raster`/`vb_sky`/`vb_resolve`/`vb_shade*`
                    // all rebuild against the 9-binding layout and the ones whose SPIR-V never
                    // declares a `binding(10, 0)` simply carry it bound-but-unread.
                    //
                    // Slot 10 and not 8: 8/9 are `ClusterGrid`/`LightIndexList` in the WIDER
                    // `vb_layout0_froxel`, so 8 is free only in scenes that never arm the froxel
                    // cull — a silent, scene-config-dependent collision. 10 is free in both, and
                    // using the SAME slot in both layouts is what lets ONE `Buf` declaration in
                    // the shared tail source serve the froxel and non-froxel pipelines alike.
                    BindGroupLayoutEntry {
                        binding: 10,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                ],
            },
        )
        .expect("invariant: VB Set-0 bind-group layout create");

        let vb_raster_vs = RhiDevice::create_shader_module(device, vb_raster_vs_spirv())
            .expect("invariant: VB raster vertex shader module create");
        let vb_raster_fs = RhiDevice::create_shader_module(device, vb_raster_fs_spirv())
            .expect("invariant: VB raster fragment shader module create");
        // Rung-R8 GPU regression fix (code review): `RhiDevice::create_graphics_pipeline` (the
        // PLAIN builder) hardcodes `VK_COMPARE_OP_LESS` + depth-write ON — the standard forward-Z
        // contract. `vb_raster`'s depth image is cleared to `0.0` and needs `VK_COMPARE_OP_GREATER`
        // (Decision 4, hardware reverse-Z): under `LESS` against a `0.0` clear, a reverse-Z depth
        // value (always `> 0.0`) NEVER satisfies the test, so EVERY fragment failed depth and
        // `vb_id`/`vb_depth` never received a single write (confirmed by a GPU diagnostic: zero
        // non-sentinel pixels even after the `mvp`-matrix fix). `create_graphics_pipeline_vb_raster`
        // (the 1-set GREATER-compare, write-ON builder — mirrors `create_graphics_pipeline_forward`'s
        // own reverse-Z contract) is the correct one.
        let vb_raster_pipeline = ctx
            .create_graphics_pipeline_vb_raster(&GraphicsPipelineDesc {
                vertex_module: &vb_raster_vs,
                vertex_entry: c"main",
                fragment_module: &vb_raster_fs,
                fragment_entry: c"main",
                // The `vb_id` R32G32_UINT color attachment + VB's OWN reverse-Z depth
                // (`forward.depth`, REUSED — `VbTargets`'s doc).
                color_formats: &[Format::R32G32Uint],
                depth_format: Some(Format::D32Sfloat),
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: Some(VertexBufferLayout {
                    stride: MESH_VERTEX_STRIDE as u32,
                    attributes: &attributes,
                }),
                push_constant_bytes: GBUFFER_PUSH_BYTES as u32,
                bind_group_layout: Some(&vb_layout0),
                blend: None,
                cull_mode: CullMode::None,
                depth_bias: None,
            })
            .expect("invariant: VB raster graphics pipeline create");
        // SAFETY: both modules were created on `device` and are consumed by the pipeline
        // create; each is destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, vb_raster_fs);
            RhiDevice::destroy_shader_module(device, vb_raster_vs);
        }

        // The VB v1 sky pipeline — REUSES the EXISTING `forward_sky.{vs,fs}.hlsl` compiled
        // SPIR-V verbatim (byte-identical shader modules to `forward_sky_pipeline`'s own), a NEW
        // pipeline object built against `vb_layout0` (`GBufferScene::vb_sky_pipeline`'s doc).
        let vb_sky_vs = RhiDevice::create_shader_module(device, forward_sky_vs_spirv())
            .expect("invariant: VB sky vertex shader module create");
        let vb_sky_fs = RhiDevice::create_shader_module(device, forward_sky_fs_spirv())
            .expect("invariant: VB sky fragment shader module create");
        let vb_sky_pipeline = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &vb_sky_vs,
                vertex_entry: c"main",
                fragment_module: &vb_sky_fs,
                fragment_entry: c"main",
                color_formats: &[RASTER_COLOR_FORMAT],
                depth_format: None,
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: None,
                push_constant_bytes: 0,
                bind_group_layout: Some(&vb_layout0),
                blend: None,
                cull_mode: CullMode::None,
                depth_bias: None,
            },
        )
        .expect("invariant: VB sky graphics pipeline create");
        // SAFETY: both modules were created on `device` and are consumed by the pipeline
        // create; each is destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, vb_sky_fs);
            RhiDevice::destroy_shader_module(device, vb_sky_vs);
        }

        // The per-slot VB instance-model SSBO ring — Decision 0's 64-byte `VbInstanceRow` stride
        // (a DEDICATED ring, distinct from `instance_rings`' 48-byte `InstanceModelCol`), sized
        // to the SAME `INSTANCE_CAPACITY` (rung R8 v1 scope cut: no growth-past-cap support yet).
        let vb_instance_ring_bytes =
            (INSTANCE_CAPACITY * boyko_render::instance_model::VB_INSTANCE_ROW_BYTES) as u64;
        let vb_instance_rings: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            ctx.create_buffer(&BufferDesc {
                size: vb_instance_ring_bytes,
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::HostVisibleCoherent,
            })
            .expect("invariant: VB instance ring buffer create")
        });

        let dispatch_group_count_x = (cw * ch).div_ceil(LOCAL_SIZE_X);
        // SSAA (W1): the dispatch grid must cover every composite pixel — see the
        // enumeration comment at the top of this fn. A future edit that keyed this to
        // `native` instead of `composite` would silently UNDER-DISPATCH the marcher/resolve
        // at any SSAA scale (a correctness bug with no crash); this fires immediately in debug.
        debug_assert!(
            (dispatch_group_count_x as u64) * (LOCAL_SIZE_X as u64) >= (cw as u64) * (ch as u64),
            "invariant: dispatch grid covers composite pixel count at any SSAA scale"
        );

        // VB-P1d: the froxel cull/shade GPU-timestamp bench collector — armed ONLY when
        // `BOYKO_VB_BENCH` (presence-gate; any value) is set AND the device supports
        // timestamps. Unset (every golden/host/interactive boot, the DEFAULT) ⇒ `None` ⇒
        // `Self::scene` threads `vb_gpu_timing: None` ⇒ the `record_vb` command stream is
        // byte-identical to the pre-VB-P1d path. This is the SOLE env read this fn performs
        // (every other input is a plain fn parameter, `boot`'s own doc's discipline) — kept
        // isolated to this one bench-only capability so the rest of `boot` stays untouched.
        let vb_bench_requested = std::env::var("BOYKO_VB_BENCH").is_ok();
        let vb_bench_timestamps_usable = ctx.device_caps().timestamps_usable();
        // O2: a requested-but-declined bench is otherwise SILENT — `vb_bench` stays `None`,
        // `vb_bench_armed()` reads `false`, and the runner's whole bench loop never arms, never
        // prints, never returns (the windowed test just runs indefinitely). Diagnose it loudly
        // here, the ONE place that knows both halves of the gate.
        if vb_bench_requested && !vb_bench_timestamps_usable {
            eprintln!(
                "VB-P1d bench: BOYKO_VB_BENCH set but device timestamps are unusable — bench disabled."
            );
        }
        let vb_bench = (vb_bench_requested && vb_bench_timestamps_usable).then(|| {
            let pools: [VulkanQueryPool; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
                RhiDevice::create_query_pool(ctx, &QueryPoolDesc { count: 2 * VB_PASS_COUNT })
                    .expect("invariant: VB-P1d bench query-pool create")
            });
            VbTimestampCollector::new(pools)
        });

        // VB-SV0 rung S1.5: the Deferred marcher bench collector — the SAME presence-gate +
        // device-cap shape as `vb_bench` above, on its own env knob so the two benches can never
        // arm each other by accident. Unset (every golden/host/interactive boot, the DEFAULT) ⇒
        // `None` ⇒ `Self::scene` threads `sv0_gpu_timing: None` ⇒ byte-identical stream.
        let sv0_bench_requested = std::env::var("BOYKO_SV0_BENCH").is_ok();
        // Same O2 reasoning as the VB-P1d gate: a requested-but-declined bench would otherwise be
        // silent (the windowed test just runs forever with no print), so say so here — the one
        // place that knows both halves of the gate.
        if sv0_bench_requested && !vb_bench_timestamps_usable {
            eprintln!(
                "VB-SV0 S1.5 bench: BOYKO_SV0_BENCH set but device timestamps are unusable — bench disabled."
            );
        }
        let sv0_bench = (sv0_bench_requested && vb_bench_timestamps_usable).then(|| {
            let pools: [VulkanQueryPool; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
                RhiDevice::create_query_pool(ctx, &QueryPoolDesc { count: 2 * SV0_PASS_COUNT })
                    .expect("invariant: VB-SV0 S1.5 bench query-pool create")
            });
            Sv0TimestampCollector::new(pools)
        });

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
            #[cfg(feature = "hwrt")]
            tlas_accel_rebind_pending: [false; FRAMES_IN_FLIGHT],
            raster_pipeline_pm,
            pm_instance_material_layout,
            pm_instance_material_rings,
            pm_bind_groups,
            // Textured-PBR T6c: built LAZILY (see `Self::build_textured_resources`'s doc) —
            // `boot()` itself never constructs the TEXTURED pipeline (the bindless
            // texture-array table does not exist yet at this call site).
            tex: None,
            vertex_buffer,
            marcher,
            vocab_layout,
            edit_list,
            camera_ring,
            tiles_buffer,
            clipmap,
            brick_levels_ubo,
            resolve_pipeline,
            resolve_layout,
            resolve_pipeline_wrap,
            #[cfg(feature = "hwrt")]
            resolve_pipeline_hwrt,
            #[cfg(feature = "hwrt")]
            shadow_denoise_pipelines,
            #[cfg(feature = "hwrt")]
            shadow_atrous_pipeline,
            #[cfg(feature = "hwrt")]
            shadow_temporal_pipeline,
            #[cfg(feature = "hwrt")]
            vb_shadow_vis_pipeline,
            #[cfg(feature = "hwrt")]
            ray_shadow_ubo,
            light_table,
            light_staging,
            light_dir: DEFAULT_SUN_DIR,
            present_pipeline,
            present_layout,
            present_sampler,
            depth_sampler,
            fxaa_pipeline,
            fxaa_sampler,
            smaa_edge_pipeline,
            smaa_weight_pipeline,
            smaa_blend_pipeline,
            smaa_weight_layout,
            smaa_blend_layout,
            smaa_sampler,
            smaa_area_tex,
            smaa_search_tex,
            ssaa_pipeline,
            taa_resolve_pipeline,
            taa_resolve_layout,
            taa_linear_sampler,
            rcas_pipeline,
            rcas_layout,
            ssao_pipelines,
            ssao_vb_pipelines,
            vb_ssao_layout,
            vb_geo_aux_layout,
            vb_split_layout1,
            vb_geo_pipeline: None,
            vb_shade_split_pipeline: None,
            vb_shade_split_tex_pipeline: None,
            #[cfg(feature = "hwrt")]
            vb_geo_mv_pipeline: None,
            #[cfg(feature = "hwrt")]
            vb_shade_split_hwrt_pipeline: None,
            #[cfg(feature = "hwrt")]
            vb_shade_split_tex_hwrt_pipeline: None,
            ssao_layout,
            ssao_atrous_read8_pipeline,
            ssao_atrous_interior_pipeline,
            ssao_atrous_write8_pipeline,
            ssao_atrous_layout,
            viewt_from_depth_pipeline,
            viewt_from_depth_layout,
            viewt_from_vb_depth_pipeline,
            viewt_from_vb_depth_layout,
            csm,
            forward_pipeline,
            forward_sky_pipeline,
            forward_layout0,
            forward_layout1,
            forward_prepass_pipeline,
            forward_plus_pipeline,
            sdf_forward_march_pipeline,
            sdf_forward_march_sdfonly_pipeline,
            sdf_forward_march_viewt_pipeline,
            sdf_forward_march_sdfonly_viewt_pipeline,
            sdf_forward_march_layout,
            dispatch_group_count_x,
            vb_layout0,
            vb_raster_pipeline,
            vb_sky_pipeline,
            // Built LAZILY — see `Self::vb_resolve_pipeline`'s doc.
            vb_resolve_pipeline: None,
            // VB-P2 classification plan, rung P2a: built LAZILY by
            // `Self::build_vb_classify_pipelines` — see that fn's doc.
            vb_classify_count_pipeline: None,
            vb_classify_scan_pipeline: None,
            vb_classify_scatter_pipeline: None,
            vb_shade_pipeline: None,
            // Textured-PBR rung TV0: built LAZILY by `Self::build_vb_shade_textured_pipeline`
            // — see that fn's doc.
            vb_shade_tex_pipeline: None,
            vb_instance_rings,
            // VB-P1a/P1b: built LAZILY by `Self::build_froxel_light_cull`, gated on the arm bit
            // `ResolvedRenderPath::froxel_light_cull` (VB path AND `LightingConfig::
            // clusters_enabled`, default OFF — an owner opt-in) — see that fn's doc.
            cluster_cull_pipeline: None,
            cull_layout: None,
            cluster_grid: None,
            light_index: None,
            light_index_alloc: None,
            cluster_cull_push: ClusterCullPush::UNARMED,
            cluster_count: 0,
            cluster_cull_hier: None,
            cluster_boot_packed_dims: 0,
            vb_layout0_froxel: None,
            vb_resolve_froxel_pipeline: None,
            vb_shade_froxel_pipeline: None,
            vb_shade_tex_froxel_pipeline: None,
            vb_bench,
            sv0_bench,
        }
    }

    /// Textured-PBR T6c: builds the TEXTURED gbuffer producer pipeline + its dedicated
    /// per-instance-material SSBO ring + per-FIF bind groups, storing them in [`Self::tex`]
    /// for the remaining process lifetime. Called ONCE from `run_windowed`, AFTER the
    /// bindless texture-array table (`BindlessTextureTable`) exists — its creation is
    /// deferred past `boot()`/`finish()` (`runner.rs`'s boot-order comment: the fallible
    /// create needs `AssetRefcountPlugin` resources only guaranteed present after every
    /// plugin's `build()` has run), so the TEXTURED pipeline's set-1 layout
    /// (`bindless.set().set_layout()`) is unavailable at [`Self::boot`] time.
    ///
    /// Mirrors [`Self::boot`]'s PM pipeline construction (@862-947 of the pre-T6c source),
    /// widened for the 2-set layout ([`VulkanContext::create_graphics_pipeline_bindless`])
    /// and the 5-attribute textured vertex layout (position/normal/color/uv/tangent).
    ///
    /// # Panics
    /// Panics (`expect("invariant: ...")`) on any RHI create failure — mirrors
    /// [`Self::boot`]'s contract (a device OOM at scene-boot time is a setup failure).
    pub(crate) fn build_textured_resources(
        &mut self,
        ctx: &VulkanContext,
        bindless: &BindlessTextureTable,
    ) {
        let device = ctx;

        // ── The set-0 TEXTURED instance-material layout + the FIF-ringed SSBOs + bind
        // groups. A SEPARATE layout from `pm_instance_material_layout` (a wider element
        // stride — `PerInstanceMaterialTex`, 48 B, vs `PerInstanceMaterial`'s 32 B).
        let tex_instance_material_layout = RhiDevice::create_bind_group_layout(
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
        .expect("invariant: TEX instance-material bind-group layout create");

        // ── The 2-SET TEXTURED mesh-MRT G-buffer producer graphics pipeline (pass A).
        // Set 0 = `tex_instance_material_layout` (VERTEX); set 1 = the bindless
        // texture-array set's layout (FRAGMENT) — via `create_graphics_pipeline_bindless`
        // (T6c plan Decision D5), NOT the generic `RhiDevice::create_graphics_pipeline`.
        let tex_vs = RhiDevice::create_shader_module(device, gbuffer_mrt_tex_vs_spirv())
            .expect("invariant: TEX mesh-MRT vertex shader module create");
        let tex_fs = RhiDevice::create_shader_module(device, gbuffer_mrt_tex_fs_spirv())
            .expect("invariant: TEX mesh-MRT fragment shader module create");
        // The widened 5-attribute vertex layout (T6c plan Decision D6): position@0/
        // normal@12/color@24 (unchanged from the base pipeline) + uv@40/tangent@48 (new —
        // `boyko_render::mesh::Vertex`'s trailing fields), all against the SAME 64-byte
        // `MESH_VERTEX_STRIDE` (every mesh already carries this stride, T3).
        let tex_attributes = [
            VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
            VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
            VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
            VertexAttribute { location: 3, offset: 40, format: VertexFormat::Float32x2 },
            VertexAttribute { location: 4, offset: 48, format: VertexFormat::Float32x4 },
        ];
        let raster_pipeline_tex = ctx
            .create_graphics_pipeline_bindless(
                &GraphicsPipelineDesc {
                    vertex_module: &tex_vs,
                    vertex_entry: c"main",
                    fragment_module: &tex_fs,
                    fragment_entry: c"main",
                    // 3 base G-buffer attachments + the 4th `gPbr` (T6a) — always 4 color
                    // formats declared; the recorder's `color_attachment_count` matches the
                    // bound rendering scope (4 on every TEXTURED frame, W2-b).
                    color_formats: &[
                        RASTER_COLOR_FORMAT,
                        RASTER_COLOR_FORMAT,
                        RASTER_COLOR_FORMAT,
                        TEX_GPBR_COLOR_FORMAT,
                    ],
                    depth_format: Some(Format::D32Sfloat),
                    topology: PrimitiveTopology::TriangleList,
                    vertex_layout: Some(VertexBufferLayout {
                        stride: MESH_VERTEX_STRIDE as u32,
                        attributes: &tex_attributes,
                    }),
                    push_constant_bytes: GBUFFER_PUSH_BYTES as u32,
                    bind_group_layout: Some(&tex_instance_material_layout),
                    blend: None,
                    cull_mode: CullMode::None,
                    depth_bias: None,
                },
                bindless.set().set_layout(),
            )
            .expect("invariant: TEX mesh-MRT graphics pipeline create");
        // SAFETY: both modules were created on `device` and are consumed by the pipeline
        // create; each is destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, tex_fs);
            RhiDevice::destroy_shader_module(device, tex_vs);
        }

        let tex_material_ring_bytes = (TEX_INSTANCE_CAPACITY * PER_INSTANCE_MATERIAL_TEX_BYTES) as u64;
        let tex_instance_material_rings: [BoundBuffer; FRAMES_IN_FLIGHT] =
            core::array::from_fn(|_| {
                let b = RhiDevice::create_buffer(
                    device,
                    &BufferDesc {
                        size: tex_material_ring_bytes,
                        usage: BufferUsage::STORAGE,
                        location: MemoryLocation::HostVisibleCoherent,
                    },
                )
                .expect("invariant: TEX instance-material SSBO ring slot create");
                let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                    .expect("invariant: host-visible TEX instance-material SSBO is mapped");
                zero_fill(mapped, tex_material_ring_bytes as usize);
                b
            });
        let tex_bind_groups: [VulkanBindGroup; FRAMES_IN_FLIGHT] = core::array::from_fn(|i| {
            RhiDevice::create_bind_group(
                device,
                &BindGroupDesc {
                    layout: &tex_instance_material_layout,
                    entries: &[
                        BindGroupEntry::StorageBuffer { buffer: &self.instance_rings[i] },
                        BindGroupEntry::StorageBuffer { buffer: &tex_instance_material_rings[i] },
                    ],
                },
            )
            .expect("invariant: TEX instance-material bind group create")
        });

        self.tex = Some(TexturedResources {
            raster_pipeline_tex,
            tex_instance_material_layout,
            tex_instance_material_rings,
            tex_bind_groups,
            bindless_set: bindless.set().set(),
        });
    }

    /// Multi-paradigm render-path plan, rung R8: builds [`Self::vb_resolve_pipeline`] (the
    /// FUSED `vb_resolve.comp.hlsl` compute pipeline) — deferred past [`Self::boot`] for the
    /// SAME reason [`Self::build_textured_resources`] is (that fn's doc): `geometry_set`'s
    /// descriptor-set LAYOUT (Set 2, the Decision-0 bindless geometry table) does not exist at
    /// `boot()`'s call site — the live `MeshGeometryTable` is constructed by `boyko_app::runner`
    /// only on a `VisibilityBuffer`-resolved boot, AFTER `boot()` returns. Called ONCE, from
    /// `runner.rs`, immediately after a successful `MeshGeometryTable::new`.
    pub(crate) fn build_vb_resolve_pipeline(
        &mut self,
        ctx: &VulkanContext,
        geometry_set: &boyko_rhi_vulkan::geometry_bindless::VulkanGeometryBindlessSet,
    ) {
        let device = ctx;
        let vb_resolve_cs = RhiDevice::create_shader_module(device, vb_resolve_spirv())
            .expect("invariant: VB resolve compute shader module create");
        let vb_resolve_pipeline = ctx
            .create_compute_pipeline_vb(
                &ComputePipelineDesc {
                    module: &vb_resolve_cs,
                    entry: c"main",
                    // The 64-byte push constant (`vb_resolve.comp.hlsl`'s `PushConstants`: one
                    // `float4x4 view_proj`).
                    push_constant_bytes: 64,
                    bind_group_layout: Some(&self.vb_layout0),
                    spec_constants: &[],
                },
                self.forward_layout1.set_layout(),
                geometry_set.set_layout(),
            )
            .expect("invariant: VB resolve compute pipeline create");
        // SAFETY: the module was created on `device` and is consumed by the pipeline create;
        // destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, vb_resolve_cs);
        }
        self.vb_resolve_pipeline = Some(vb_resolve_pipeline);
    }

    /// VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2a (dark infra,
    /// unwired): builds the three 1-set classify pipelines (`vb_classify_count`/`_scan`/
    /// `_scatter`, each via the GENERIC `RhiDevice::create_compute_pipeline` against
    /// [`Self::vb_layout0`] — plan P2-1, no dedicated `_vb1` helper) + the 3-set `vb_shade`
    /// pipeline (via [`VulkanContext::create_compute_pipeline_vb`], the SAME 3-set builder
    /// [`Self::build_vb_resolve_pipeline`] uses — Set 0 = `vb_layout0`, Set 1 =
    /// `forward_layout1`, Set 2 = `geometry_set`). Deferred past [`Self::boot`] for the SAME
    /// reason [`Self::build_vb_resolve_pipeline`] is (that fn's doc): `vb_shade` needs the
    /// Decision-0 geometry table's Set-2 layout, which does not exist at `boot()`'s call site.
    /// Called ONCE, from `runner.rs`, immediately after [`Self::build_vb_resolve_pipeline`].
    ///
    /// DARK INFRA (rung P2a): nothing declares/records against these four pipelines yet —
    /// `record_vb`/`declare_vb_graph` are untouched, the fused `vb_resolve` still shades every
    /// VB frame. This fn only builds+stores them so a later rung (P2b/P2c) can wire them in
    /// without another plumbing pass.
    pub(crate) fn build_vb_classify_pipelines(
        &mut self,
        ctx: &VulkanContext,
        geometry_set: &boyko_rhi_vulkan::geometry_bindless::VulkanGeometryBindlessSet,
    ) {
        let device = ctx;

        let vb_classify_count_cs = RhiDevice::create_shader_module(device, vb_classify_count_spirv())
            .expect("invariant: VB classify count compute shader module create");
        let vb_classify_count_pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &vb_classify_count_cs,
                entry: c"main",
                push_constant_bytes: 4,
                bind_group_layout: Some(&self.vb_layout0),
                spec_constants: &[],
            },
        )
        .expect("invariant: VB classify count compute pipeline create");
        // SAFETY: the module was created on `device` and is consumed by the pipeline create;
        // destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, vb_classify_count_cs);
        }

        let vb_classify_scan_cs = RhiDevice::create_shader_module(device, vb_classify_scan_spirv())
            .expect("invariant: VB classify scan compute shader module create");
        let vb_classify_scan_pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &vb_classify_scan_cs,
                entry: c"main",
                // The 4-byte `PushConstants { uint material_count; }` (`vb_classify_scan.comp
                // .hlsl`'s own -- a LOOP BOUND only, see that file's + `vb_classify_common.hlsli`'s
                // doc).
                push_constant_bytes: 4,
                bind_group_layout: Some(&self.vb_layout0),
                spec_constants: &[],
            },
        )
        .expect("invariant: VB classify scan compute pipeline create");
        // SAFETY: the module was created on `device` and is consumed by the pipeline create;
        // destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, vb_classify_scan_cs);
        }

        let vb_classify_scatter_cs =
            RhiDevice::create_shader_module(device, vb_classify_scatter_spirv())
                .expect("invariant: VB classify scatter compute shader module create");
        let vb_classify_scatter_pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &vb_classify_scatter_cs,
                entry: c"main",
                push_constant_bytes: 4,
                bind_group_layout: Some(&self.vb_layout0),
                spec_constants: &[],
            },
        )
        .expect("invariant: VB classify scatter compute pipeline create");
        // SAFETY: the module was created on `device` and is consumed by the pipeline create;
        // destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, vb_classify_scatter_cs);
        }

        let vb_shade_cs = RhiDevice::create_shader_module(device, vb_shade_spirv())
            .expect("invariant: VB shade compute shader module create");
        let vb_shade_pipeline = ctx
            .create_compute_pipeline_vb(
                &ComputePipelineDesc {
                    module: &vb_shade_cs,
                    entry: c"main",
                    // The SAME 64-byte push constant `vb_resolve_pipeline` declares (view_proj)
                    // -- `vb_shade`'s shading tail is character-identical (plan D3), so its
                    // push-constant shape is too.
                    push_constant_bytes: 64,
                    bind_group_layout: Some(&self.vb_layout0),
                    spec_constants: &[],
                },
                self.forward_layout1.set_layout(),
                geometry_set.set_layout(),
            )
            .expect("invariant: VB shade compute pipeline create");
        // SAFETY: the module was created on `device` and is consumed by the pipeline create;
        // destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, vb_shade_cs);
        }

        self.vb_classify_count_pipeline = Some(vb_classify_count_pipeline);
        self.vb_classify_scan_pipeline = Some(vb_classify_scan_pipeline);
        self.vb_classify_scatter_pipeline = Some(vb_classify_scatter_pipeline);
        self.vb_shade_pipeline = Some(vb_shade_pipeline);
    }

    /// Textured-PBR rung TV0 (`RENDER-PARITY-PLAN.md` §2.3): builds
    /// [`Self::vb_shade_tex_pipeline`] (the `vb_shade.comp.hlsl` `-D TEXTURED=1` compute
    /// pipeline) — deferred past [`Self::boot`]/[`Self::build_vb_classify_pipelines`] for a
    /// widened version of the SAME reason those two are (their own docs): this pipeline needs
    /// BOTH the Decision-0 geometry table's Set-2 layout (`geometry_set`, constructed by
    /// `boyko_app::runner` only on a `VisibilityBuffer`-resolved boot) AND the shared bindless
    /// texture-array table's Set-3 layout (`bindless.set().set_layout()`, built even later —
    /// after `app.finish()` drains every plugin/startup system, `Self::build_textured_resources`'s
    /// own doc). Called ONCE, from `runner.rs`, immediately after
    /// [`Self::build_textured_resources`] — the LAST of the three dependencies to become
    /// available — gated on the geometry table existing (mirrors
    /// [`Self::build_vb_classify_pipelines`]'s own call-site gate).
    ///
    /// # Panics
    /// Panics (`expect("invariant: ...")`) on any RHI create failure — mirrors
    /// [`Self::boot`]'s contract (a device OOM at scene-boot time is a setup failure).
    pub(crate) fn build_vb_shade_textured_pipeline(
        &mut self,
        ctx: &VulkanContext,
        geometry_set: &boyko_rhi_vulkan::geometry_bindless::VulkanGeometryBindlessSet,
        bindless: &BindlessTextureTable,
    ) {
        let device = ctx;

        let vb_shade_tex_cs = RhiDevice::create_shader_module(device, vb_shade_tex_spirv())
            .expect("invariant: VB shade TEXTURED compute shader module create");
        let vb_shade_tex_pipeline = ctx
            .create_compute_pipeline_vb_textured(
                &ComputePipelineDesc {
                    module: &vb_shade_tex_cs,
                    entry: c"main",
                    // The SAME 64-byte push constant `vb_shade_pipeline`/`vb_resolve_pipeline`
                    // declare (view_proj) -- the TEXTURED shading tail reads the SAME geometry-
                    // fetch reprojection matrix, unchanged shape.
                    push_constant_bytes: 64,
                    bind_group_layout: Some(&self.vb_layout0),
                    spec_constants: &[],
                },
                self.forward_layout1.set_layout(),
                geometry_set.set_layout(),
                bindless.set().set_layout(),
            )
            .expect("invariant: VB shade TEXTURED compute pipeline create");
        // SAFETY: the module was created on `device` and is consumed by the pipeline create;
        // destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, vb_shade_tex_cs);
        }

        self.vb_shade_tex_pipeline = Some(vb_shade_tex_pipeline);
    }

    /// Rung R9b (docs/R9-VB-SPLIT-PLAN.md §6): builds the split pair — `vb_geo` (3-set: Set 0 =
    /// [`Self::vb_layout0`], Set 1 = [`Self::vb_geo_aux_layout`], Set 2 = the geometry table)
    /// and `vb_shade_split` (3-set: Set 1 = [`Self::vb_split_layout1`]) + the `-D TEXTURED=1`
    /// sibling (4-set, iff `bindless` exists — `build_vb_shade_textured_pipeline`'s own
    /// two-dependency reason). Deferred past [`Self::boot`] because the Decision-0 geometry
    /// table's Set-2 layout does not exist at `boot()`'s call site (the SAME reason as
    /// [`Self::build_vb_resolve_pipeline`]). Called ONCE from `runner.rs`, right after the
    /// other VB deferred builds.
    pub(crate) fn build_vb_split_pipelines(
        &mut self,
        ctx: &VulkanContext,
        geometry_set: &boyko_rhi_vulkan::geometry_bindless::VulkanGeometryBindlessSet,
        bindless: Option<&BindlessTextureTable>,
    ) {
        let device = ctx;

        let vb_geo_cs = RhiDevice::create_shader_module(device, vb_geo_spirv())
            .expect("invariant: R9b vb_geo compute shader module create");
        let vb_geo_pipeline = ctx
            .create_compute_pipeline_vb(
                &ComputePipelineDesc {
                    module: &vb_geo_cs,
                    entry: c"main",
                    // The 64-byte `view_proj` push (`vb_geo.comp.hlsl` — `vb_resolve`'s shape).
                    push_constant_bytes: 64,
                    bind_group_layout: Some(&self.vb_layout0),
                    spec_constants: &[],
                },
                self.vb_geo_aux_layout.set_layout(),
                geometry_set.set_layout(),
            )
            .expect("invariant: R9b vb_geo compute pipeline create");
        // SAFETY: the module was created on `device` and is consumed by the pipeline create;
        // destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, vb_geo_cs);
        }
        self.vb_geo_pipeline = Some(vb_geo_pipeline);

        let vb_shade_split_cs = RhiDevice::create_shader_module(device, vb_shade_split_spirv())
            .expect("invariant: R9b vb_shade_split compute shader module create");
        let vb_shade_split_pipeline = ctx
            .create_compute_pipeline_vb(
                &ComputePipelineDesc {
                    module: &vb_shade_split_cs,
                    entry: c"main",
                    push_constant_bytes: 64,
                    bind_group_layout: Some(&self.vb_layout0),
                    spec_constants: &[],
                },
                self.vb_split_layout1.set_layout(),
                geometry_set.set_layout(),
            )
            .expect("invariant: R9b vb_shade_split compute pipeline create");
        // SAFETY: as above.
        unsafe {
            RhiDevice::destroy_shader_module(device, vb_shade_split_cs);
        }
        self.vb_shade_split_pipeline = Some(vb_shade_split_pipeline);

        if let Some(bindless) = bindless {
            let cs = RhiDevice::create_shader_module(device, vb_shade_split_tex_spirv())
                .expect("invariant: R9b vb_shade_split TEXTURED compute shader module create");
            let p = ctx
                .create_compute_pipeline_vb_textured(
                    &ComputePipelineDesc {
                        module: &cs,
                        entry: c"main",
                        push_constant_bytes: 64,
                        bind_group_layout: Some(&self.vb_layout0),
                        spec_constants: &[],
                    },
                    self.vb_split_layout1.set_layout(),
                    geometry_set.set_layout(),
                    bindless.set().set_layout(),
                )
                .expect("invariant: R9b vb_shade_split TEXTURED compute pipeline create");
            // SAFETY: as above.
            unsafe {
                RhiDevice::destroy_shader_module(device, cs);
            }
            self.vb_shade_split_tex_pipeline = Some(p);
        }

        // Rung R9d: the hwrt shadow-chain siblings — same 3-set (4-set for the TEXTURED variant)
        // creates as their software siblings above, gated ADDITIONALLY on `ctx.ray_query_enabled()`
        // (an RT-only variant that reads the denoised/undenoised `gShadowVis`).
        #[cfg(feature = "hwrt")]
        if ctx.ray_query_enabled() {
            let vb_geo_mv_cs = RhiDevice::create_shader_module(device, vb_geo_mv_spirv())
                .expect("invariant: R9d vb_geo_mv compute shader module create");
            let vb_geo_mv_pipeline = ctx
                .create_compute_pipeline_vb(
                    &ComputePipelineDesc {
                        module: &vb_geo_mv_cs,
                        entry: c"main",
                        push_constant_bytes: 64,
                        bind_group_layout: Some(&self.vb_layout0),
                        spec_constants: &[],
                    },
                    self.vb_geo_aux_layout.set_layout(),
                    geometry_set.set_layout(),
                )
                .expect("invariant: R9d vb_geo_mv compute pipeline create");
            // SAFETY: as above.
            unsafe {
                RhiDevice::destroy_shader_module(device, vb_geo_mv_cs);
            }
            self.vb_geo_mv_pipeline = Some(vb_geo_mv_pipeline);

            let vb_shade_split_hwrt_cs =
                RhiDevice::create_shader_module(device, vb_shade_split_hwrt_spirv())
                    .expect("invariant: R9d vb_shade_split HWRT compute shader module create");
            let vb_shade_split_hwrt_pipeline = ctx
                .create_compute_pipeline_vb(
                    &ComputePipelineDesc {
                        module: &vb_shade_split_hwrt_cs,
                        entry: c"main",
                        push_constant_bytes: 64,
                        bind_group_layout: Some(&self.vb_layout0),
                        spec_constants: &[],
                    },
                    self.vb_split_layout1.set_layout(),
                    geometry_set.set_layout(),
                )
                .expect("invariant: R9d vb_shade_split HWRT compute pipeline create");
            // SAFETY: as above.
            unsafe {
                RhiDevice::destroy_shader_module(device, vb_shade_split_hwrt_cs);
            }
            self.vb_shade_split_hwrt_pipeline = Some(vb_shade_split_hwrt_pipeline);

            if let Some(bindless) = bindless {
                let cs = RhiDevice::create_shader_module(device, vb_shade_split_tex_hwrt_spirv())
                    .expect("invariant: R9d vb_shade_split TEXTURED HWRT compute shader module create");
                let p = ctx
                    .create_compute_pipeline_vb_textured(
                        &ComputePipelineDesc {
                            module: &cs,
                            entry: c"main",
                            push_constant_bytes: 64,
                            bind_group_layout: Some(&self.vb_layout0),
                            spec_constants: &[],
                        },
                        self.vb_split_layout1.set_layout(),
                        geometry_set.set_layout(),
                        bindless.set().set_layout(),
                    )
                    .expect("invariant: R9d vb_shade_split TEXTURED HWRT compute pipeline create");
                // SAFETY: as above.
                unsafe {
                    RhiDevice::destroy_shader_module(device, cs);
                }
                self.vb_shade_split_tex_hwrt_pipeline = Some(p);
            }
        }
    }

    /// VB-P1a/P1b: builds the ENTIRE froxel light-cull machinery — the L1
    /// `cluster_cull` compute pipeline + its OWN Set-0 layout, the `ClusterGrid`/
    /// `LightIndexList`/`LightIndexAlloc` device-local buffers (Principle 0 — VM-native
    /// [`BoundBuffer`]s, never a `std::Vec`/`HashMap` side store), the froxel-only
    /// `vb_layout0_froxel` Set-0 layout (`vb_layout0`'s own 0..7 PLUS `ClusterGrid` @8 +
    /// `LightIndexList` @9 — a DISTINCT layout OBJECT, `vb_layout0` itself stays UNCHANGED), and
    /// the three `_froxel` VB shading pipelines (`vb_resolve_froxel`/`vb_shade_froxel`/
    /// `vb_shade_tex_froxel`, mirroring [`Self::build_vb_resolve_pipeline`]/
    /// [`Self::build_vb_classify_pipelines`]/[`Self::build_vb_shade_textured_pipeline`]'s own
    /// pipeline shapes, built against THIS wider layout instead of `vb_layout0`).
    ///
    /// GATED entirely behind `ResolvedRenderPath::froxel_light_cull` at the `boyko_app::runner`
    /// call site — armed (VB-P1b) iff the booted scene's `LightingConfig::clusters_enabled` is
    /// `true` under `RenderPath::VisibilityBuffer`; every other boot (unarmed scenes, and every
    /// non-VB path) never calls this fn, so every field it would populate stays `None`/zeroed,
    /// [`Self::scene`] threads that through unchanged, and every existing (unarmed) golden stays
    /// byte-identical (the 0%-gate). Mirrors [`Self::build_vb_shade_textured_pipeline`]'s
    /// two-dependency deferred-build shape: called ONCE from `runner.rs`, after
    /// `MeshGeometryTable::new` AND the bindless texture table both exist, iff the arm bit is
    /// armed.
    ///
    /// `cluster_config` sizes the buffers/push (`ClusterConfig::default()` at every current call
    /// site — no owner-facing override is wired yet).
    ///
    /// VB-P1e D11/H4: `hier_cull` selects WHICH of the two cull arms is built. `true` — **the
    /// production default since the arm-default flip**, and what `boyko_app::runner` selects when
    /// `BOYKO_VB_HIER_CULL` is unset — builds the `-D HIER=1` 256-wide
    /// `cluster_cull_hier_spirv()` arm; `false` builds the base 64-wide `cluster_cull_spirv()`
    /// arm, kept selectable as the opt-out and as the equality oracle's permanent reference.
    /// H3 proved on hardware that the two arms emit the same per-froxel sets in the same order,
    /// and H5 proved the frame is byte-identical through the whole pipeline, so the flip is a
    /// pure performance change (22.5× on the cull at N=512, and 1.4× FASTER even at N=8 — there
    /// is no low-N penalty to trade against). Exactly ONE pipeline is ever built per boot —
    /// [`Self::cluster_cull_pipeline`] holds whichever arm was selected, and
    /// [`Self::cluster_cull_hier`] records WHICH one.
    ///
    /// # Panics
    /// Panics (`expect("invariant: ...")`) on any RHI create failure — mirrors every other VB
    /// pipeline builder's contract (a device OOM at scene-boot time is a setup failure). Also
    /// panics (a release `assert!`, D11) if any of `cluster_config`'s grid dims exceeds 8 bits —
    /// the header pack this fn feeds the HIER push is lossy past that contract.
    pub(crate) fn build_froxel_light_cull(
        &mut self,
        ctx: &VulkanContext,
        geometry_set: &boyko_rhi_vulkan::geometry_bindless::VulkanGeometryBindlessSet,
        bindless: &BindlessTextureTable,
        cluster_config: ClusterConfig,
        hier_cull: bool,
    ) {
        let device = ctx;

        // --- The L1 cluster-cull compute pipeline + its OWN Set-0 layout ({ camera UBO @0,
        // light table SSBO @1, ClusterGrid SSBO @2, LightIndexList SSBO @3, LightIndexAlloc SSBO
        // @4 } — matching `cluster_cull.hlsl`'s own binding table, the SAME shape the L1
        // host-oracle test harness (`sdf_gbuffer_hybrid.rs`) builds by hand). A DEDICATED
        // layout, unrelated to `vb_layout0`/`vb_layout0_froxel` — the cull pass is its OWN 1-set
        // pipeline. Shared by BOTH arms (VB-P1e D11): the `-D HIER=1` shader widens only the
        // PUSH range, not the descriptor bindings. ---
        let cull_layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry {
                        binding: 0,
                        count: 1,
                        kind: DescriptorKind::UniformBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 1,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 2,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 3,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 4,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                ],
            },
        )
        .expect("invariant: L1 cull Set-0 bind-group layout create");

        // VB-P1e H4: select the arm's own SPIR-V + push-constant range. `hier_cull` is a
        // boot-frozen choice — the SAME pipeline slot below holds whichever arm was selected,
        // never both at once.
        let (cull_module_words, cull_push_constant_bytes): (&'static [u32], u32) = if hier_cull {
            (cluster_cull_hier_spirv(), CLUSTER_CULL_HIER_PUSH_BYTES)
        } else {
            (cluster_cull_spirv(), CLUSTER_CULL_PUSH_BYTES)
        };
        let cull_cs = RhiDevice::create_shader_module(device, cull_module_words)
            .expect("invariant: L1 cluster-cull compute shader module create");
        let cluster_cull_pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &cull_cs,
                entry: c"main",
                push_constant_bytes: cull_push_constant_bytes,
                bind_group_layout: Some(&cull_layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: L1 cluster-cull compute pipeline create");
        // SAFETY: the module was created on `device` and is consumed by the pipeline create;
        // destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, cull_cs);
        }

        // --- The L1 cluster buffers (Principle 0: VM-native `BoundBuffer`s, DEVICE_LOCAL —
        // never a std::Vec/HashMap side store). Sized from `cluster_config`, mirroring
        // `sdf_gbuffer_hybrid.rs`'s own host-oracle buffer sizing. ---
        let cluster_count = cluster_config.cluster_count();
        let cluster_grid = ctx
            .create_buffer(&BufferDesc {
                size: (cluster_count as u64) * 8, // uint2 {offset, count} per froxel
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::DeviceLocal,
            })
            .expect("invariant: L1 ClusterGrid storage buffer create");
        let light_index = ctx
            .create_buffer(&BufferDesc {
                size: (cluster_config.index_list_cap as u64) * 4,
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::DeviceLocal,
            })
            .expect("invariant: L1 LightIndexList storage buffer create");
        let light_index_alloc = ctx
            .create_buffer(&BufferDesc {
                size: 4,
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::DeviceLocal,
            })
            .expect("invariant: L1 LightIndexAlloc storage buffer create");

        // VB-P1e D11: the `<= 255`-per-dim contract that keeps `ClusterConfig::packed_dims()`'s
        // mapping lossless is a `debug_assert!` only inside that method
        // (`crates/boyko_render/src/light.rs`) — the OR it guards has no masking, so an
        // out-of-contract dim would silently corrupt the packed word this fn feeds the HIER push
        // below. Promoted to a release `assert!` HERE: a boot-time, once-per-process check on a
        // setup path (Principle 1 is not engaged), the cheapest point that keeps the MAPPING
        // honest even in a release build where `packed_dims`'s own debug_assert compiles out.
        assert!(
            cluster_config.dim_x <= 0xFF && cluster_config.dim_y <= 0xFF && cluster_config.dim_z <= 0xFF,
            "invariant: cluster dims must each fit in 8 bits for the header pack (dim_x={}, dim_y={}, dim_z={})",
            cluster_config.dim_x,
            cluster_config.dim_y,
            cluster_config.dim_z,
        );

        self.cluster_cull_push = ClusterCullPush::new(
            cluster_config.z_near,
            cluster_config.z_far,
            cluster_config.max_lights_per_cluster,
            cluster_config.index_list_cap,
        );
        self.cluster_count = cluster_count;
        let boot_packed_dims = cluster_config.packed_dims();
        self.cluster_boot_packed_dims = boot_packed_dims;
        // VB-P1e D11/H4: `Some` iff the pipeline just built above is the HIER arm — the group
        // count + the 24-byte push bytes [`Self::scene`] threads into
        // `GBufferScene::cluster_cull_hier`, which the record site (`vb.rs`) dispatches INSTEAD
        // of `cluster_count`/`cluster_cull_push` when `Some`.
        self.cluster_cull_hier = hier_cull.then(|| {
            let push = ClusterCullHierPush::new(
                cluster_config.z_near,
                cluster_config.z_far,
                cluster_config.max_lights_per_cluster,
                cluster_config.index_list_cap,
                boot_packed_dims,
                cluster_count,
            );
            let mut push_bytes = [0u8; CLUSTER_CULL_HIER_PUSH_BYTES as usize];
            push_bytes.copy_from_slice(push.as_bytes());
            ClusterCullHierDispatch { groups: cluster_config.hier_group_count(), push: push_bytes }
        });
        self.cluster_cull_pipeline = Some(cluster_cull_pipeline);
        self.cull_layout = Some(cull_layout);
        self.cluster_grid = Some(cluster_grid);
        self.light_index = Some(light_index);
        self.light_index_alloc = Some(light_index_alloc);

        // --- `vb_layout0_froxel` — a NEW 10-binding Set-0 layout: `vb_layout0`'s own 0..7 PLUS
        // `ClusterGrid` @8 + `LightIndexList` @9. Do NOT touch `vb_layout0` itself (byte-identity
        // of the base 8-binding descriptor-set shape). ---
        let vb_layout0_froxel = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry {
                        binding: 0,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::VERTEX | ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 1,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 2,
                        count: 1,
                        kind: DescriptorKind::UniformBuffer,
                        stage: ShaderStage::FRAGMENT | ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 3,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::FRAGMENT | ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 4,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 5,
                        count: 1,
                        kind: DescriptorKind::SampledImage,
                        stage: ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 6,
                        count: 1,
                        kind: DescriptorKind::StorageImage,
                        stage: ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 7,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 8,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                    BindGroupLayoutEntry {
                        binding: 9,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                    // VB-SV0 (docs/VB-SV0-SDF-SHADOW-PLAN.md §2), rung S2 ("dark infra"): the SDF
                    // edit-list SSBO at slot 10 — the SAME entry `vb_layout0` gains, at the SAME
                    // slot, so ONE `Buf` declaration in the shared tail sources binds correctly
                    // against either layout. Slot 10 rather than 8 precisely because 8/9 are the
                    // froxel pair right above: reusing 8 would be a collision visible only in
                    // scenes that arm the cull, and no validation layer on this box reports it.
                    // Binding numbers need not be contiguous — only the ENTRY COUNT is capped
                    // (`MAX_BIND_GROUP_BINDINGS = 24`), so 10 entries -> 11 is well inside it.
                    BindGroupLayoutEntry {
                        binding: 10,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    },
                ],
            },
        )
        .expect("invariant: VB froxel Set-0 bind-group layout create");

        // `vb_resolve_froxel` — the SAME 3-set shape `build_vb_resolve_pipeline` builds, against
        // the wider `vb_layout0_froxel`.
        let vb_resolve_froxel_cs = RhiDevice::create_shader_module(device, vb_resolve_froxel_spirv())
            .expect("invariant: VB resolve FROXEL compute shader module create");
        let vb_resolve_froxel_pipeline = ctx
            .create_compute_pipeline_vb(
                &ComputePipelineDesc {
                    module: &vb_resolve_froxel_cs,
                    entry: c"main",
                    push_constant_bytes: 64,
                    bind_group_layout: Some(&vb_layout0_froxel),
                    spec_constants: &[],
                },
                self.forward_layout1.set_layout(),
                geometry_set.set_layout(),
            )
            .expect("invariant: VB resolve FROXEL compute pipeline create");
        // SAFETY: the module was created on `device` and is consumed by the pipeline create;
        // destroyed once; no GPU work is in flight yet.
        unsafe {
            RhiDevice::destroy_shader_module(device, vb_resolve_froxel_cs);
        }

        // `vb_shade_froxel` — the SAME 3-set shape `build_vb_classify_pipelines`'s own
        // `vb_shade` build uses, against the wider `vb_layout0_froxel`.
        let vb_shade_froxel_cs = RhiDevice::create_shader_module(device, vb_shade_froxel_spirv())
            .expect("invariant: VB shade FROXEL compute shader module create");
        let vb_shade_froxel_pipeline = ctx
            .create_compute_pipeline_vb(
                &ComputePipelineDesc {
                    module: &vb_shade_froxel_cs,
                    entry: c"main",
                    push_constant_bytes: 64,
                    bind_group_layout: Some(&vb_layout0_froxel),
                    spec_constants: &[],
                },
                self.forward_layout1.set_layout(),
                geometry_set.set_layout(),
            )
            .expect("invariant: VB shade FROXEL compute pipeline create");
        // SAFETY: as above.
        unsafe {
            RhiDevice::destroy_shader_module(device, vb_shade_froxel_cs);
        }

        // `vb_shade_tex_froxel` — the SAME 4-set shape `build_vb_shade_textured_pipeline` uses,
        // against the wider `vb_layout0_froxel`.
        let vb_shade_tex_froxel_cs = RhiDevice::create_shader_module(device, vb_shade_tex_froxel_spirv())
            .expect("invariant: VB shade TEXTURED+FROXEL compute shader module create");
        let vb_shade_tex_froxel_pipeline = ctx
            .create_compute_pipeline_vb_textured(
                &ComputePipelineDesc {
                    module: &vb_shade_tex_froxel_cs,
                    entry: c"main",
                    push_constant_bytes: 64,
                    bind_group_layout: Some(&vb_layout0_froxel),
                    spec_constants: &[],
                },
                self.forward_layout1.set_layout(),
                geometry_set.set_layout(),
                bindless.set().set_layout(),
            )
            .expect("invariant: VB shade TEXTURED+FROXEL compute pipeline create");
        // SAFETY: as above.
        unsafe {
            RhiDevice::destroy_shader_module(device, vb_shade_tex_froxel_cs);
        }

        self.vb_layout0_froxel = Some(vb_layout0_froxel);
        self.vb_resolve_froxel_pipeline = Some(vb_resolve_froxel_pipeline);
        self.vb_shade_froxel_pipeline = Some(vb_shade_froxel_pipeline);
        self.vb_shade_tex_froxel_pipeline = Some(vb_shade_tex_froxel_pipeline);
    }

    /// Cheap, allocation-free steady-state check (asset-streaming plan F7 review W1):
    /// `true` iff `needed` exceeds slot `slot`'s current instance-family capacity.
    /// Touches no World resource at all (`host.gpu` is a plain `WindowHost` field) —
    /// call this BEFORE paying for the (rare) [`Self::grow_instance_family_if_needed`]
    /// path's NonSend `RetiredGpuBuffers` take-out. Asset-streaming plan F7-hwrt
    /// (task#11): on an RT device this now ALSO triggers REAL growth (the former W3
    /// hard-cap early-return is removed) — [`Self::grow_instance_family_if_needed`]
    /// dispatches to [`Self::grow_instance_family_rt`], which grows the TLAS/mv sides in
    /// lockstep, past [`INSTANCE_CAPACITY`], up to the SAME [`MAX_INSTANCE_CAP`] ceiling
    /// the non-RT leg shares (no separate RT ceiling).
    #[inline]
    pub(crate) fn needs_instance_grow(&self, needed: u32, slot: usize) -> bool {
        needed > self.instance_capacity[slot]
    }

    /// Asset-streaming plan F7-hwrt (task#11): the LOCKSTEP instance-family-ring grow
    /// BOTH legs share — `instance_rings[s]` + `pm_instance_material_rings[s]` (defer
    /// old, no seed) + rebind `instance_bind_groups[s]`@0 / `pm_bind_groups[s]`@0/@1 /
    /// (textured-PBR T6c review W1) `tex_bind_groups[s]`@0 + the interp pair/out-slot
    /// co-grow ([`InterpGpuProd::grow_slot`], which itself repoints `interp_bg[s]`@0/@1/@2
    /// against the just-grown `instance_rings[s]`).
    /// Extracted from the pre-task#11 `grow_instance_family_if_needed`'s body — behavior
    /// is IDENTICAL to that body's non-RT portion (verified by
    /// [`Self::grow_instance_family_nonrt`], its sole caller before this split).
    /// [`Self::grow_instance_family_rt`] is the second (new) caller. Does NOT touch
    /// `self.instance_capacity[s]` — the caller sets it once its OWN leg's grow (mv/tlas
    /// on the RT leg) has also landed.
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
    /// The caller guarantees slot `s`'s in-flight fence was waited THIS frame — every
    /// descriptor set this fn repoints (`instance_bind_groups[s]`, `pm_bind_groups[s]`,
    /// `tex_bind_groups[s]` when `self.tex.is_some()`, `interp`'s `interp_bg[s]`) is
    /// therefore non-command-buffer-pending.
    unsafe fn grow_shared_instance_rings(
        &mut self,
        s: usize,
        new_cap: u32,
        ctx: &VulkanContext,
        retired: &mut RetiredGpuBuffers,
        epoch: u64,
    ) {
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

        // SAFETY: slot `s`'s fence was waited this frame (this fn's caller contract
        // above) — `instance_bind_groups[s]`'s set is non-pending, so rewriting its
        // binding in place is sound.
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
        // RT-LEG-GROWTH COUPLING (task#11): DONE — RT-leg instance-family growth is
        // implemented in [`Self::grow_instance_family_rt`] + `TlasResources::grow_slot` +
        // `MotionVecResources::grow_slot`. The full rebind matrix (every descriptor set
        // aliasing `instance_rings`/`prev_instance_rings`/`pm_instance_material_rings`/
        // the TLAS AS handle) is enforced by the `PACK_GROWN_BINDINGS`/`MV_GROWN_BINDINGS`
        // debug_asserts in those two `grow_slot`s plus
        // `GBufferTargets::tlas_accel_sets`'s `expected_tlas_accel_ring_count` guard —
        // NOT by hand-auditing this comment.
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
        // SAFETY: slot `s`'s fence was waited this frame (this fn's caller contract
        // above) — `pm_bind_groups[s]`'s set is non-pending, so rewriting BOTH its
        // bindings in place is sound. Binding 0 repoints to the SAME grown
        // `instance_rings[s]` rebound above; binding 1 to the just-grown material ring.
        unsafe {
            rebind_storage_buffer(ctx, &self.pm_bind_groups[s], 0, &self.instance_rings[s]);
            rebind_storage_buffer(ctx, &self.pm_bind_groups[s], 1, &self.pm_instance_material_rings[s]);
        }

        // Textured-PBR T6c (review W1 — latent silent device-UAF fix): `tex_bind_groups[s]`'s
        // binding 0 shares the SAME growable `instance_rings[s]` index space as
        // `pm_bind_groups[s]`'s binding 0 — a mechanical mirror of the rebind immediately
        // above, restricted to the ONE growable binding. Binding 1
        // (`tex_instance_material_rings[s]`) is FIXED at boot `INSTANCE_CAPACITY` (T6c's
        // disclosed non-participation in this grow — see `TexturedResources`'s doc) and is
        // therefore NOT rebound here (there is no grown buffer to point it at). WITHOUT this
        // rebind, growing `instance_rings[s]` would defer the OLD ring (freed
        // `RETIRE_DELAY` frames later, see `retired.push` above) while `tex_bind_groups[s]`
        // @0 kept pointing at it — a later TEXTURED frame would then bind a descriptor
        // referencing freed device memory (a silent device-UAF; no validation layer on this
        // box to catch it). `self.tex` may be `None` (the TEXTURED pipeline never got built
        // — e.g. the bindless table failed to create), in which case there is no
        // `tex_bind_groups[s]` to rebind.
        if let Some(tex) = self.tex.as_ref() {
            // SAFETY: slot `s`'s fence was waited this frame (this fn's caller contract
            // above) — `tex.tex_bind_groups[s]`'s set is non-pending, so rewriting its
            // binding 0 in place is sound. Repoints to the SAME just-grown
            // `instance_rings[s]` the `instance_bind_groups[s]`/`pm_bind_groups[s]`@0
            // rebinds above use.
            unsafe {
                rebind_storage_buffer(ctx, &tex.tex_bind_groups[s], 0, &self.instance_rings[s]);
            }
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
    }

    /// Asset-streaming plan F7 §7.3 (task#11: split out of the former
    /// `grow_instance_family_if_needed`, MOVED VERBATIM — behavior byte-identical): grows
    /// the FENCED slot's non-RT instance family via [`Self::grow_shared_instance_rings`]
    /// to `next_pow2(needed)` iff `needed` exceeds that slot's current capacity — called
    /// BEFORE `upload_instance_models` fills the (possibly grown) ring this frame.
    ///
    /// # Steady-state cost
    ///
    /// The caller is expected to have already consulted [`Self::needs_instance_grow`]
    /// before paying for the NonSend take-out this call requires; the internal `needed
    /// <= self.instance_capacity[s]` re-check below is a defensive belt-and-suspenders,
    /// not the steady-state gate anymore.
    ///
    /// # Safety
    ///
    /// The caller guarantees `token` proves THIS frame's fence wait for slot `token.slot()`
    /// — every descriptor set this fn repoints (`instance_bind_groups[s]`, `interp`'s
    /// `interp_bg[s]`) is therefore non-command-buffer-pending.
    unsafe fn grow_instance_family_nonrt(
        &mut self,
        needed: u32,
        ctx: &VulkanContext,
        token: &FrameWriteToken,
        retired: &mut RetiredGpuBuffers,
        epoch: u64,
    ) {
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

        // SAFETY: `token` proves slot `s`'s fence was waited THIS frame (this fn's
        // caller contract above) — every set `grow_shared_instance_rings` repoints is
        // non-pending.
        unsafe {
            self.grow_shared_instance_rings(s, new_cap, ctx, retired, epoch);
        }

        self.instance_capacity[s] = new_cap;
    }

    /// Asset-streaming plan F7-hwrt (task#11): grows the FENCED slot's RT instance family
    /// — the shared rings ([`Self::grow_shared_instance_rings`]) PLUS the RT-only mv/tlas
    /// sides — to `next_pow2(needed)` iff `needed` exceeds that slot's current capacity.
    /// Only reachable once `self.tlas.is_some()` (the caller's dispatch gate in
    /// [`Self::grow_instance_family_if_needed`]).
    ///
    /// # mv grow is CONDITIONAL
    ///
    /// An RT device without `shadow_denoise_storage_ok()` has `tlas.is_some()` but
    /// `mv.is_none()` (see [`MotionVecResources`]'s own boot gate,
    /// `ray_query_enabled() && shadow_denoise_storage_ok()`) — growing `mv`
    /// unconditionally would panic on that real, test-box-invisible configuration.
    ///
    /// # Safety
    ///
    /// The caller guarantees `token` proves slot `token.slot()`'s fence was waited THIS
    /// frame — every descriptor set this fn (transitively) repoints is non-pending.
    #[cfg(feature = "hwrt")]
    unsafe fn grow_instance_family_rt(
        &mut self,
        needed: u32,
        ctx: &VulkanContext,
        token: &FrameWriteToken,
        retired: &mut RetiredGpuBuffers,
        epoch: u64,
    ) {
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

        // SAFETY: `token` proves slot `s`'s fence was waited THIS frame — every set
        // `grow_shared_instance_rings` repoints is non-pending.
        unsafe {
            self.grow_shared_instance_rings(s, new_cap, ctx, retired, epoch);
        }

        if let Some(mv) = self.mv.as_mut() {
            // SAFETY: `token` proves slot `s`'s fence was waited THIS frame — neither
            // `bind_groups[s]` nor `mvpm_bind_groups[s]` is command-buffer-pending;
            // `instance_rings[s]`/`pm_instance_material_rings[s]` are the just-grown
            // buffers `grow_shared_instance_rings` produced above.
            unsafe {
                mv.grow_slot(
                    ctx,
                    s,
                    new_cap,
                    &self.instance_rings[s],
                    &self.pm_instance_material_rings[s],
                    retired,
                    epoch + RETIRE_DELAY,
                );
            }
        }

        {
            let tlas = self
                .tlas
                .as_mut()
                .expect("invariant: grow_instance_family_rt is only reached when tlas.is_some()");
            // SAFETY: `token` proves slot `s`'s fence was waited THIS frame —
            // `bind_groups[s]` is non-pending; `instance_rings[s]` is the just-grown
            // buffer above.
            unsafe {
                tlas.grow_slot(ctx, s, new_cap, &self.instance_rings[s], retired, epoch + RETIRE_DELAY);
            }
        }

        self.instance_capacity[s] = new_cap;
        self.tlas_accel_rebind_pending[s] = true;
    }

    /// Asset-streaming plan F7 §7.3, extended by F7-hwrt (task#11): grows the FENCED
    /// slot's instance family (shared by both legs) iff `needed` exceeds that slot's
    /// current capacity — called BEFORE `upload_instance_models` fills the (possibly
    /// grown) ring this frame. Dispatches to [`Self::grow_instance_family_rt`] on an RT
    /// device (`self.tlas.is_some()`), else [`Self::grow_instance_family_nonrt`].
    /// `mv.is_some() ⟹ tlas.is_some()` (`mv`'s own boot gate additionally requires
    /// `shadow_denoise_storage_ok()`, on top of the SAME `ray_query_enabled()` gate
    /// `tlas` boots under), so checking `tlas.is_some()` alone is exhaustive — the former
    /// `|| self.mv.is_some()` (the pre-task#11 W3 early-return) was redundant.
    ///
    /// # Safety
    ///
    /// The caller guarantees `token` proves THIS frame's fence wait for slot
    /// `token.slot()` — every descriptor set either dispatch target repoints is
    /// therefore non-command-buffer-pending.
    pub(crate) unsafe fn grow_instance_family_if_needed(
        &mut self,
        needed: u32,
        ctx: &VulkanContext,
        token: &FrameWriteToken,
        retired: &mut RetiredGpuBuffers,
        epoch: u64,
    ) {
        #[cfg(feature = "hwrt")]
        if self.tlas.is_some() {
            // SAFETY: `token` proves slot `token.slot()`'s fence was waited THIS frame —
            // every set `grow_instance_family_rt` (transitively) repoints is non-pending.
            unsafe {
                self.grow_instance_family_rt(needed, ctx, token, retired, epoch);
            }
            return;
        }
        // SAFETY: `token` proves slot `token.slot()`'s fence was waited THIS frame —
        // every set `grow_instance_family_nonrt` (transitively) repoints is non-pending.
        unsafe {
            self.grow_instance_family_nonrt(needed, ctx, token, retired, epoch);
        }
    }

    /// Asset-streaming plan F7-hwrt (task#11): slot `s`'s CURRENT persistent-TLAS
    /// acceleration structure — the runner's `repoint_tlas_accel` rebind target once
    /// [`Self::grow_instance_family_rt`] has flagged `tlas_accel_rebind_pending[s]`.
    #[cfg(feature = "hwrt")]
    pub(crate) fn current_tlas_accel(&self, s: usize) -> &BoundAccelStruct {
        self.tlas
            .as_ref()
            .expect("invariant: current_tlas_accel is only called on an RT device (tlas.is_some())")
            .resolve_accels()[s]
    }

    /// Assembles this frame's [`GBufferScene`] ON THE STACK (plan D7 — POD +
    /// refs, zero alloc): the static bundles + this frame's `mvp` push, the
    /// fenced slot's instance bind group, and the gathered draw batch list.
    ///
    /// R4 wiring: SDF empty, brick/coarse/atlas/interp OFF (their always-bound resources
    /// are valid placeholders); Render P7-Q2 SSAO is armed from `ssao_variant` (see its
    /// param doc) — OFF (`None`) unless the owner resolved a non-`Off` `SsaoQuality`;
    /// lighting is ECS-owned —
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
        // Asset-system rung A1: the GPU mirror of the World-owned `Assets<Material>`
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
        // Textured-PBR T6c: `true` iff THIS gather scattered at least one bound bindless
        // texture slot (`MeshRenderScratch::any_textured_material`, read by the runner
        // AFTER `gather_material_tex_into`). Gates `raster_pipeline_tex`/`tex_bind_group`/
        // `bindless_set` below — `false` on every non-textured scene, so the recorder binds
        // the FROZEN base/pm pipeline.
        any_textured_material: bool,
        // Render terminator-softening: `true` iff `LightingConfig::terminator_softening > 0`
        // (read by the runner from the `LightingConfig` resource, the SAME `world.try_resource`
        // pattern `ddgi_enabled` uses). Selects [`Self::resolve_pipeline_wrap`] in place of
        // [`Self::resolve_pipeline`] below — `false` (the default) binds the base pipeline, the
        // byte-identical 0%-gate (`deferred_pbr.hlsl`'s frozen-base discipline).
        terminator_wrap: bool,
        // Anti-aliasing Stage 1: the owner-resolved AA technique
        // ([`boyko_render::ResolvedAa::mode`]). `AaMode::Off` (the default) ⇒
        // `GBufferScene::aa == None` (the 0%-gate: no `aa_out`, no FXAA pass, present
        // samples `lit`). `AaMode::Fxaa` arms the FXAA activation below.
        aa_mode: AaMode,
        // Anti-aliasing Stage 4 (TAA W5): `boyko_render::taa_state::TaaState::advance`'s
        // consumed-this-frame reset flag (`true` on TAA's first armed frame or a resize) —
        // threaded from `boyko_app::runner` the SAME way `terminator_wrap` is, so the RHI layer
        // never reads `World` directly. Read ONLY when `aa_mode == AaMode::Taa` arms
        // `TaaActivation::reset` below; ignored (and harmless) otherwise.
        taa_reset: bool,
        // TAA rung T3: the owner-set post-resolve sharpen mode
        // (`boyko_render::taa_config::TaaConfig::sharpen`), threaded from `boyko_app::runner`
        // the SAME way `aa_mode`/`taa_reset` are. `SharpenMode::None` (the default) ⇒
        // `GBufferScene::rcas == None` (the 0%-gate: no `taa_resolved`, no `rcas_set`, the
        // resolve writes `aa_out` directly). `SharpenMode::Rcas` arms the RCAS activation below
        // ONLY when `aa_mode == AaMode::Taa` ALSO holds — RCAS is a pure post-process over the
        // resolve's OWN output, never standalone (see [`RcasActivation`]'s doc).
        sharpen: SharpenMode,
        // TAA rung T3: the owner-set [`SharpenMode::Rcas`] strength in `[0, 1]`
        // (`boyko_render::taa_config::TaaConfig::rcas_sharpness`), threaded the SAME way
        // `sharpen` is. Read ONLY when `sharpen == SharpenMode::Rcas` AND `aa_mode ==
        // AaMode::Taa` arm [`RcasActivation::sharpness`] below; ignored (and harmless)
        // otherwise.
        rcas_sharpness: f32,
        // Render P7-Q2: the owner-resolved SSAO quality's variant index
        // ([`boyko_render::ResolvedSsao::variant`]) — `Some(0/1/2)` for Low/Medium/High,
        // `None` for [`boyko_render::SsaoQuality::Off`] (the default). Threaded from
        // `boyko_app::runner` the SAME way `aa_mode` is (a per-frame `World` read via
        // `try_resource`), so the RHI layer never reads `World` directly. Selects
        // `Self::ssao_pipelines[v]` below; `None` ⇒ `GBufferScene::ssao == None` (the
        // 0%-gate). The resolve's `ssao_mode` header gate is armed SEPARATELY, in
        // lock-step, by `boyko_render::sync_ssao_light_gate` (through the
        // `collect_lights` → light-table upload pipeline, independent of this fn).
        ssao_variant: Option<usize>,
        // The SSAO edge-avoiding à-trous denoise chain: the owner-resolved, ALREADY-CLAMPED pass
        // count ([`boyko_render::ResolvedSsao::atrous_levels`] — `0` or
        // `2..=boyko_render::MAX_SSAO_ATROUS_LEVELS`). Threaded from `boyko_app::runner` the SAME
        // way `ssao_variant` is; `resolve_ssao` already forces this to `0` whenever
        // `ssao_variant.is_none()` (SSAO itself off), so the two can never disagree — no
        // additional `debug_assert` needed beyond the ceiling clamp below. Feeds
        // `SsaoActivation::atrous_levels`; `0` ⇒ the recorder dispatches NO à-trous pass (the
        // resolve reads the raw gather — the byte-identical pre-dispatch-wiring path).
        ssao_atrous_levels: u32,
        // Multi-paradigm render-path plan, rung R1: the boot-committed render-path selection
        // (`WindowHost::resolved_render_path` — Decision 1, resolved ONCE, never re-derived
        // per frame, unlike every other arming input above). Converted at THIS seam into the
        // plain-POD [`ResolvedRenderPathGpu`] (`boyko_render` → `boyko_rhi_vulkan` dependency
        // direction — this crate cannot see `boyko_rhi_vulkan` types the other way around, so
        // the conversion cannot live as a `From` impl in either crate; a free fn here is the
        // only orphan-rule-clean seam). DEAD-BUT-THREADED: nothing reads `GBufferScene::
        // resolved_render_path` yet (R2 wires the declarator dispatch).
        resolved_render_path: boyko_render::ResolvedRenderPath,
        // Multi-paradigm render-path plan, rung R-SDFFWD: the host-precomputed
        // `boyko_render::view::forward_view_z_coeffs(view.near, view.far)` reverse-Z decode pair
        // — `SdfForwardMarchPush::has_mesh`'s `view_z_a`/`view_z_b` arguments. `boyko_app::runner`
        // computes these at the SAME site it builds `mvp` via `forward_gbuffer_push_from_view`
        // (which needs the identical `view.near`/`view.far`), so `scene()` itself never touches
        // `ViewUniform`. Don't-care under every other leg/path (see
        // `GBufferScene::sdf_forward_view_z_a`'s doc).
        sdf_forward_view_z_a: f32,
        sdf_forward_view_z_b: f32,
        // TAA-under-VB: the `viewt_from_depth_rz` `gViewT`-producer push's reverse-Z decode
        // `A`/`B` (`boyko_render::view::forward_view_z_coeffs`) — the SAME formula
        // `sdf_forward_view_z_a`/`_b` above use, but gated on TAA-under-VB arming instead of the
        // SDF-forward-march arming (VB×Mesh never marches, so those two would stay `(0.0, 0.0)`
        // don't-care for the exact frames this pass needs real coefficients). `boyko_app::runner`
        // computes these at the SAME site it builds `mvp`/`sdf_forward_view_z_a` (the identical
        // `view.near`/`view.far` single-source discipline); `scene()` itself never touches
        // `ViewUniform`. Don't-care (`0.0, 0.0`) whenever [`GBufferScene::viewt_from_vb_depth`]
        // resolves to `None` below.
        vb_viewt_view_z_a: f32,
        vb_viewt_view_z_b: f32,
        // Multi-paradigm render-path plan, rung R8: the live Decision-0 geometry table's Set,
        // threaded from `boyko_app::runner`'s World read (`NonSendRes<MeshGeometryTableSlot>`) —
        // `scene()` itself never touches `World` (the SAME "host reads, threads the plain value"
        // discipline every other config knob above follows). `Some` only on a
        // `VisibilityBuffer`-resolved boot with the device-cap armed
        // (`resolved_render_path.vb_geometry_table`); `None` otherwise.
        vb_geometry_set: Option<&'a boyko_rhi_vulkan::geometry_bindless::VulkanGeometryBindlessSet>,
        // VB-SV0 rung S1.5: THIS frame's `FineMarcherPush::lighting_flags`, when the S1.5 bench
        // is driving an interleaved paired A/B over it. `None` (every non-bench frame — the
        // DEFAULT) keeps the shipped `LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO` literal, so the
        // pushed bytes are byte-identical to the pre-S1.5 path. `Some(f)` stamps `f` verbatim:
        // the bench alternates `SHADOWS|AO` (the ARMED phase) with `0` (the CLEARED phase), which
        // is exactly the `sdf_gbuffer_composite.hlsl:1865` / `:1805` gate around the shadow + AO
        // marches SV0 proposes to inline. A per-frame PUSH CONSTANT, not a descriptor and not a
        // pipeline key — flipping it needs no re-record and no pipeline rebuild, which is what
        // makes the two phases of a pair comparable (see `GBufferScene::lighting_flags`).
        sv0_bench_lighting_flags: Option<u32>,
        device: &VulkanContext,
    ) -> GBufferScene<'a> {
        debug_assert!(
            light_upload.unwrap_or(0) <= LIGHT_TABLE_CAPACITY,
            "invariant: the staged light table fits the device table capacity"
        );
        debug_assert!(
            ssao_variant.is_none_or(|v| v < SSAO_QUALITY_COUNT),
            "invariant: ssao_variant must index Self::ssao_pipelines (0..SSAO_QUALITY_COUNT)"
        );
        debug_assert!(
            ssao_variant.is_some() || ssao_atrous_levels == 0,
            "invariant: ssao_atrous_levels > 0 requires ssao_variant.is_some() (resolve_ssao forces this)"
        );

        // Multi-paradigm render-path plan, rung R3 (P1 fix — orchestrator architecture
        // decision): mesh-shadow producers (CSM cascade depth, the punctual spot/point atlas
        // depth, and under `hwrt` the per-frame TLAS pack/build + the shadow_vis/à-trous/
        // temporal denoise chain) are MESH-LEG-OWNED — they rasterize/trace MESH casters only.
        // The SDF leg gets its shadows from the marcher's baked soft march
        // (`ShadowSources::SDF_SOFT_MARCH`), never from these. Under `!mesh_leg` (`Deferred ×
        // Sdf`) they must be structurally ABSENT (capability = component presence, not a
        // runtime flag), suppressed HERE — the single scene-assembly seam — so every downstream
        // use in this fn (the `csm`/`atlas_punctual` `GBufferScene` fields below, and the
        // `hwrt` `tlas`/`shadow` activations, which key off `csm.is_some()`/`tlas_enabled`)
        // derives from the SAME gated locals and can never disagree. `Deferred × Both`/`Mesh`
        // keep `mesh_leg == true` ⇒ `.filter(|_| true)` is the identity ⇒ byte-identical.
        let mesh_leg = resolved_render_path.mesh_leg;
        let csm = csm.filter(|_| mesh_leg);
        let atlas = atlas.filter(|_| mesh_leg);
        #[cfg(feature = "hwrt")]
        let tlas_enabled = tlas_enabled && mesh_leg;

        // Multi-paradigm render-path plan, rung R3b — the SDF-owned-producer half of the R3
        // scene-assembly seam (the mirror-image gate to `mesh_leg` above): SDFDDGI's probe-update
        // pass injects indirect irradiance onto `is_sdf_lit` pixels ONLY (the SDF leg's own
        // geometry) — it is SDF-OWNED, so it must be structurally ABSENT under `!sdf_leg`
        // (`Deferred × Mesh`), suppressed HERE at the same single seam. `Deferred × Both`/`Sdf`
        // keep `sdf_leg == true` ⇒ `&& true` is the identity ⇒ byte-identical. (The P0 coarse
        // tile-cull is ALSO SDF-owned/marcher-serving, but `coarse: None` unconditionally below —
        // never wired by this production seam yet — so it needs no additional gate here; the
        // async `light_upload` is light-generic, not leg-owned, so it is untouched.)
        let sdf_leg = resolved_render_path.sdf_leg;

        // TAA supports two path families today: `Deferred` (any legs — the marcher/gbuffer
        // resolve own `gViewT`) and `VisibilityBuffer` (any legs — `viewt_from_depth_rz` on the
        // marcher-less Mesh config, the VIEWT-variant `sdf_forward_march` composite on the
        // SDF-carrying legs; see `viewt_from_vb_depth` above/below).
        // `ResolvedRenderPath::taa_supported()` is the SINGLE predicate every TAA gate reads —
        // this degrade, the resolver's own `cap_forward_v1_consumers` narrowing
        // (`RenderPathDegrade::ForwardTaaNotYetImplemented`), and `boyko_app::runner`'s
        // `taa_armed_now` arm-state all consume it, so the three can
        // never disagree (a split-brain half-armed state would mean jitter with no accumulator,
        // or an armed `aa_out` with no matching dispatch). But the AA ACTIVATION arming below
        // reads only `aa_mode` (from `AaConfig`), NOT the resolved path — so an `AaMode::Taa`
        // request on an unsupported combination (Forward/ForwardPlus) would arm
        // `scene.taa` (and thus `aa_out`) while the recorder runs NO temporal resolve, leaving
        // `aa_out` armed with no matching AA dispatch (the VB/forward AA blocks assert exactly
        // that — a debug panic, a never-written `aa_out` sampled in release). Degrade an
        // unsupported TAA request to `Off` HERE, at the single point where `aa_mode` feeds every
        // AA activation, so `aa_out` never arms without a pass. FXAA / SMAA / SSAA are NOT capped
        // and arm as usual; a `taa_supported()` TAA request (Deferred or VB, any legs) is
        // byte-UNCHANGED (the `taa_armed` / `taa_rcas` / `vb_taa` goldens hold; the SDF-carrying
        // VB legs gain TAA support at the VIEWT rung).
        let aa_mode = if aa_mode == AaMode::Taa && !resolved_render_path.taa_supported() {
            AaMode::Off
        } else {
            aa_mode
        };
        // ...and the SAME hole existed for the other three modes, which the comment above admitted
        // ("FXAA / SMAA / SSAA are NOT capped and arm as usual") without closing. `targets.rs`
        // arms `aa_out` on `scene.aa || scene.smaa || scene.ssaa || scene.taa` with NO path term,
        // and the present blit repoints every slot at `aa_out` whenever it is `Some` — so on
        // Forward/ForwardPlus, whose recorder holds no AA block at all (`passes/forward.rs` has
        // zero AA sites, and `declare_forward_graph` declares no AA pass), an FXAA/SMAA/SSAA
        // request presented a NEVER-WRITTEN image. Same defect, same single choke point, three
        // modes wider.
        //
        // Kept as a SECOND, wider degrade rather than folded into the one above, and the ordering
        // is deliberate: the two predicates select the same paths today but answer different
        // questions (see `post_process_aa_supported`'s doc). Collapsing them would mean a future
        // Forward AA seam — which flips `post_process_aa_supported` first and `taa_supported`
        // later — silently re-arms TAA on a path with no temporal machinery. Two narrow gates that
        // can diverge beat one wide gate that cannot.
        let aa_mode = if aa_mode != AaMode::Off && !resolved_render_path.post_process_aa_supported()
        {
            AaMode::Off
        } else {
            aa_mode
        };
        let ddgi_enabled = ddgi_enabled && sdf_leg;

        // VB-P2 classification plan, rung P2c (the P1-4 owner-decided selector,
        // `GBufferScene::vb_use_classified`'s own doc): `BOYKO_VB_FORCE_CLASSIFIED` is the
        // orchestrator's dev/golden channel to force the classified `vb_shade` path on real
        // hardware ahead of TV0 — mirrors `boyko_app::plugins`'s `BOYKO_AA`/`BOYKO_RENDER_PATH`
        // launch-env seam. Read once per frame (this fn's own per-frame assembly seam, not a
        // hot inner loop) rather than cached at boot, so a running process can be toggled by
        // re-launch without a rebuild.
        let vb_force_classified = std::env::var("BOYKO_VB_FORCE_CLASSIFIED").is_ok();
        // Textured-PBR rung TV0: the VB sibling of `any_textured_material`'s own
        // `raster_pipeline_tex`/`tex_bind_group`/`bindless_set` gating below — `true` iff THIS
        // frame's gather bound a non-zero material texture slot AND the TEXTURED `vb_shade`
        // pipeline + the TEXTURED resources both exist (mirrors `GBufferScene::vb_tex_active`'s
        // own condition, evaluated here pre-construction since `GBufferScene` does not exist
        // yet at this seam).
        let vb_tex_active_this_frame =
            any_textured_material && self.vb_shade_tex_pipeline.is_some() && self.tex.is_some();
        let vb_use_classified = vb_force_classified || vb_tex_active_this_frame;

        // Multi-paradigm render-path plan, rung R3b: the `viewt_from_depth` push's mesh-depth
        // ray-t normalizer needs THIS frame's camera mode — read from the SAME `cam_eye.w` lane
        // (`mvp` bytes @76..80, `GBUFFER_PUSH_BYTES`'s doc: 0.0 = ortho, 1.0 = perspective) the
        // raster VERTEX push already carries (there is only ONE camera per frame; no second
        // source to desync from). Don't-care under every OTHER leg (the pass is not recorded).
        let camera_mode_w = f32::from_le_bytes(
            mvp[76..80].try_into().expect("invariant: mvp is GBUFFER_PUSH_BYTES (>= 80) long"),
        );
        let camera_mode = if camera_mode_w != 0.0 { CAM_MODE_PERSPECTIVE } else { CAM_MODE_ORTHO };

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

        // VB-P1a/P1b: the L1 cluster-cull activation — `Some` only when
        // `Self::build_froxel_light_cull` ran (gated on `ResolvedRenderPath::froxel_light_cull`,
        // which resolves true on the VB path when the owner sets `LightingConfig::
        // clusters_enabled`; it DEFAULTS off, so this `.zip` chain is `None` for a scene that
        // never opts in — the 0%-gate — and `Some` for one that does).
        // Threading via the SAME `Option::zip` idiom `shadow` above uses keeps this
        // a single expression rather than five independent `.as_ref()` calls that could disagree.
        let cluster_cull_bits = self
            .cluster_cull_pipeline
            .as_ref()
            .zip(self.cull_layout.as_ref())
            .zip(self.cluster_grid.as_ref())
            .zip(self.light_index.as_ref())
            .zip(self.light_index_alloc.as_ref())
            .map(|((((pipeline, layout), grid), index), alloc)| (pipeline, layout, grid, index, alloc));
        let (cluster_cull, cull_layout, cluster_grid, light_index, light_index_alloc) =
            match cluster_cull_bits {
                Some((pipeline, layout, grid, index, alloc)) => {
                    (Some(pipeline), Some(layout), Some(grid), Some(index), Some(alloc))
                }
                None => (None, None, None, None, None),
            };
        // The 16-byte `ClusterCullPush` bytes — meaningless (never read by the recorder) while
        // `cluster_cull` is `None`, so a zeroed push is the honest value then.
        let mut cluster_cull_push = [0u8; CLUSTER_CULL_PUSH_BYTES as usize];
        if cluster_cull.is_some() {
            cluster_cull_push.copy_from_slice(self.cluster_cull_push.as_bytes());
        }
        let cluster_count = if cluster_cull.is_some() { self.cluster_count } else { 0 };

        GBufferScene {
            raster_pipeline: &self.raster_pipeline,
            vertex_buffer: &self.vertex_buffer,
            vertex_count: 6,
            mvp,
            instance_bind_group,
            marcher: &self.marcher,
            // Multi-paradigm render-path plan, rung R3b (`Deferred × Mesh` — the SDF leg fully
            // off): armed exactly when the resolved legs are `GeometryLegs::Mesh` (`mesh_leg &&
            // !sdf_leg`) — the marcher is not dispatched then, so this pass is the sole `gViewT`
            // producer. `Deferred × Both`/`Sdf` keep this `None` (the marcher itself writes
            // `gViewT`) — byte-identical to every pre-R3b frame.
            viewt_from_depth: (mesh_leg && !sdf_leg).then(|| ViewtFromDepthActivation {
                pipeline: &self.viewt_from_depth_pipeline,
                layout: &self.viewt_from_depth_layout,
                mesh_view_t_norm: mesh_view_t_norm(camera_mode),
            }),
            // TAA-under-VB + rung R9b: `vb_viewt` arms for (a) the marcher-less TAA config
            // (`mesh_leg && !sdf_leg && Taa` — the shipped Track-A arm; on an SDF-carrying TAA
            // leg the `VIEWT`-variant marcher is the sole producer) OR (b) the split's SSAO
            // (`mesh_geo_shade_split && ssao armed` — the PRE-TAIL slot: the gather needs the
            // gViewT lane TAA-independently; under `Both`+SSAO+TAA BOTH producers arm and the
            // marcher stays the LAST declared writer — `declare_vb_graph`'s revised asserts).
            // `ssao_armed_now` reads the SAME freeze-clamped `ResolvedSsao` the `scene.ssao`
            // activation below reads, so the two can never disagree.
            viewt_from_vb_depth: (matches!(resolved_render_path.path, boyko_render::RenderPath::VisibilityBuffer)
                && mesh_leg
                && ((!sdf_leg && aa_mode == AaMode::Taa)
                    || (resolved_render_path.mesh_geo_shade_split && ssao_variant.is_some())))
                .then_some(ViewtFromVbDepthActivation {
                    pipeline: &self.viewt_from_vb_depth_pipeline,
                    layout: &self.viewt_from_vb_depth_layout,
                    view_z_a: vb_viewt_view_z_a,
                    view_z_b: vb_viewt_view_z_b,
                }),
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
            cluster_cull,
            cull_layout,
            cluster_grid,
            light_index,
            light_index_alloc,
            cluster_cull_push,
            cluster_count,
            cluster_cull_hier: self.cluster_cull_hier,
            // Render terminator-softening: swap in the wrap-variant pipeline when armed (the
            // `terminator_wrap` param doc above); both pipelines share `resolve_layout` (the
            // variant adds no descriptor), so no other field here changes.
            resolve_pipeline: if terminator_wrap {
                &self.resolve_pipeline_wrap
            } else {
                &self.resolve_pipeline
            },
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
            // VB-SV0 rung S1.5: the bench's per-frame A/B value when it is driving, else the
            // shipped literal (the `None` default — byte-identical push bytes).
            lighting_flags: sv0_bench_lighting_flags
                .unwrap_or(LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO),
            light_dir: self.light_dir,
            // Render P7-Q2: `ssao_variant.is_some()` (the owner-resolved `SsaoQuality != Off`)
            // ⇒ `Some` — arms the SSAO compute activation against the selected pre-compiled
            // variant pipeline (`Self::ssao_pipelines[v]`) + the SHARED `ssao_layout`. `None`
            // (the default, `SsaoQuality::Off`) ⇒ the 0%-gate: no SSAO pass recorded,
            // `GBufferTargets` never builds `ssao_set`. The resolve's `ssao_mode` header gate
            // is armed in LOCK-STEP by the SEPARATE `boyko_render::sync_ssao_light_gate`
            // system (bridges `SsaoConfig` into `LightingConfig::ssao_mode`, word 11 of the
            // light header) — not by this fn (see `ssao_variant`'s param doc above).
            ssao: ssao_variant.map(|v| SsaoActivation {
                pipeline: &self.ssao_pipelines[v],
                layout: &self.ssao_layout,
                atrous_levels: ssao_atrous_levels,
            }),
            // The SSAO à-trous chain's STABLE boot pipelines/layout — ALWAYS `Some` (built
            // UNCONDITIONALLY above, no RT/device gate for the pipeline CREATE itself).
            // DECOUPLED from `ssao` above (which is `None` whenever SSAO itself is off): the
            // set-builder (`GBufferTargets::build_ssao_atrous_sets`) reads THESE fields directly,
            // so the role-keyed sets exist before a later frame arms `ssao.atrous_levels` — no
            // resize/rebuild needed (mirrors `resolve_layout_denoise_hwrt`'s decoupling doc).
            ssao_atrous_read8_pipeline: Some(&self.ssao_atrous_read8_pipeline),
            ssao_atrous_interior_pipeline: Some(&self.ssao_atrous_interior_pipeline),
            ssao_atrous_write8_pipeline: Some(&self.ssao_atrous_write8_pipeline),
            ssao_atrous_layout: Some(&self.ssao_atrous_layout),
            // Anti-aliasing Stage 1: `AaMode::Off` (the default) ⇒ `None` — the 0%-gate
            // (no `aa_out`, no FXAA pass, present samples `lit`). `AaMode::Fxaa` arms the
            // FXAA activation against the boot-built pipeline + dedicated LINEAR sampler.
            aa: matches!(aa_mode, AaMode::Fxaa)
                .then(|| AaActivation { pipeline: &self.fxaa_pipeline, sampler: &self.fxaa_sampler }),
            // Anti-aliasing Stage 2: `AaMode::Smaa` ⇒ `Some` — arms the 3-pass SMAA activation
            // against the boot-built pipelines/layouts/sampler/LUTs. Mutually exclusive with
            // `aa` above by construction (`matches!` on the SAME `aa_mode`, and `AaMode` is a
            // single enum — at most one arm matches).
            smaa: matches!(aa_mode, AaMode::Smaa).then(|| SmaaActivation {
                edge_pipeline: &self.smaa_edge_pipeline,
                weight_pipeline: &self.smaa_weight_pipeline,
                blend_pipeline: &self.smaa_blend_pipeline,
                weight_layout: &self.smaa_weight_layout,
                blend_layout: &self.smaa_blend_layout,
                sampler: &self.smaa_sampler,
                area_tex: &self.smaa_area_tex,
                search_tex: &self.smaa_search_tex,
            }),
            // Anti-aliasing Stage 3: `AaMode::Ssaa` ⇒ `Some` — arms the SSAA downsample
            // activation against the boot-built pipeline + the shared NEAREST
            // `present_sampler` (the shader's `.Load` ignores it). Mutually exclusive with
            // `aa`/`smaa` above by construction (same single-enum `matches!` discipline).
            // UNLIKE `aa`/`smaa`, `aa_mode == Ssaa` here is host-authoritative — it can only
            // occur when `boyko_app::runner`'s read-site lock forced it because the host
            // armed the 2× `composite_extent` at boot (`WindowHost::ssaa_armed`).
            ssaa: matches!(aa_mode, AaMode::Ssaa)
                .then(|| SsaaActivation { pipeline: &self.ssaa_pipeline, sampler: &self.present_sampler }),
            // Anti-aliasing Stage 4 (W5): `AaMode::Taa` ⇒ `Some` — arms the temporal-resolve
            // activation against the boot-built pipeline/layout/sampler. Mutually exclusive with
            // `aa`/`smaa`/`ssaa` above by construction (same single-enum `matches!` discipline).
            // `reset` is the runner's already-consumed `TaaState::advance()` result for THIS
            // frame (see this fn's `taa_reset` param doc).
            taa: matches!(aa_mode, AaMode::Taa).then(|| TaaActivation {
                resolve_pipeline: &self.taa_resolve_pipeline,
                resolve_layout: &self.taa_resolve_layout,
                color_formats: &[Format::R8G8B8A8Unorm],
                linear_sampler: &self.taa_linear_sampler,
                reset: taa_reset,
            }),
            // TAA rung T3: `sharpen == SharpenMode::Rcas` ⇒ `Some` — arms the RCAS activation
            // against the boot-built pipeline/layout. Guarded ALSO on `aa_mode == AaMode::Taa`
            // (RCAS is meaningless without the resolve it post-processes): a `SharpenMode::Rcas`
            // config on any other `aa_mode` degrades to `None` here rather than arming a
            // standalone pass (`GBufferTargets::create`'s `debug_assert!` would otherwise trip).
            rcas: (matches!(aa_mode, AaMode::Taa) && matches!(sharpen, SharpenMode::Rcas))
                .then_some(RcasActivation {
                    rcas_pipeline: &self.rcas_pipeline,
                    rcas_layout: &self.rcas_layout,
                    sharpness: rcas_sharpness,
                }),
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
            // VB-P1d: the froxel cull/shade bench collector, armed ONLY when `BOYKO_VB_BENCH`
            // was read at `Self::boot` time AND the device supports timestamps
            // (`Self::vb_bench_armed`'s own doc) — `None` on every other boot (every golden/
            // host/interactive run), so the recorded command stream stays byte-identical.
            vb_gpu_timing: self.vb_bench.as_ref(),
            // VB-SV0 rung S1.5: the Deferred marcher bench collector, armed ONLY when
            // `BOYKO_SV0_BENCH` was read at `Self::boot` time AND the device supports timestamps
            // (`Self::sv0_bench_armed`'s own doc) — `None` on every other boot, so the recorded
            // command stream stays byte-identical.
            sv0_gpu_timing: self.sv0_bench.as_ref(),
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
            // Textured-PBR T6c: the TEXTURED gate + refs. `tex_enabled` is the per-frame
            // `any_textured_material` read; the pipeline/bind-group/bindless-set refs are
            // `Some` iff BOTH `any_textured_material` AND `self.tex` exists (`self.tex` is
            // built LAZILY after boot — see `Self::build_textured_resources`; it may be
            // permanently `None` if that build never ran). `false`/`None` on every
            // non-textured scene ⇒ the recorder binds the FROZEN base/pm pipeline. This
            // frame's bind group is `tex.tex_bind_groups[slot]` (the FENCED slot).
            tex_enabled: any_textured_material,
            raster_pipeline_tex: any_textured_material
                .then(|| self.tex.as_ref().map(|t| &t.raster_pipeline_tex))
                .flatten(),
            tex_bind_group: any_textured_material
                .then(|| self.tex.as_ref().map(|t| &t.tex_bind_groups[slot]))
                .flatten(),
            bindless_set: any_textured_material
                .then(|| self.tex.as_ref().map(|t| t.bindless_set))
                .flatten(),
            // Multi-paradigm render-path plan, rung R4b-b: the Forward v1 mesh pipeline + its
            // descriptor-set layouts (built UNCONDITIONALLY at boot, `GpuSceneBundles::boot`'s
            // doc) + the raw instance-model/instance-material ring refs `ForwardTargets::build`
            // (`boyko_rhi_vulkan::present::targets`) folds into Forward's OWN Set-0 bind group.
            // Code-review fix: `Some(...)` ALWAYS — these resources genuinely exist at every
            // boot regardless of `resolved_render_path.path` (the `Option` on `GBufferScene`
            // exists so a NON-production test fixture can honestly say `None` instead of
            // threading a semantically-wrong placeholder; production never needs to).
            // Multi-paradigm render-path plan, rung R5 (ForwardPlus, code-review fix): only
            // `forward_pipeline` (the OPAQUE variant: `VK_COMPARE_OP_GREATER` base FS vs
            // `VK_COMPARE_OP_EQUAL` froxel FS) is path-conditional now — Forward/ForwardPlus are
            // boot-mutually-exclusive (Decision 1), so exactly one of `self.forward_pipeline`/
            // `self.forward_plus_pipeline` is ever threaded from this seam.
            // `forward_layout0`/`forward_layout1`/`forward_sky_pipeline`/the instance rings are
            // UNCONDITIONAL — `forward_layout0` is now the ONE unified 7-binding layout every
            // Forward-family pipeline (incl. `forward_pipeline` itself) is built against, fixing
            // the two-distinct-layout-handles bug a prior revision shipped (`boot`'s doc).
            forward_pipeline: Some(
                if resolved_render_path.path == boyko_render::RenderPath::ForwardPlus {
                    &self.forward_plus_pipeline
                } else {
                    &self.forward_pipeline
                },
            ),
            forward_sky_pipeline: Some(&self.forward_sky_pipeline),
            forward_layout0: Some(&self.forward_layout0),
            forward_layout1: Some(&self.forward_layout1),
            forward_instance_ring: Some(&self.instance_rings),
            forward_instance_material_ring: Some(&self.pm_instance_material_rings),
            // Multi-paradigm render-path plan, rung R5 (ForwardPlus): the depth PRE-PASS
            // pipeline — genuinely exists at every boot (built UNCONDITIONALLY, `boot`'s doc),
            // the SAME `Some(...)` ALWAYS discipline as the Forward v1 trio above; only ever
            // RECORDED when `GBufferScene::path_needs_depth_prepass` holds.
            forward_prepass_pipeline: Some(&self.forward_prepass_pipeline),
            // Multi-paradigm render-path plan, rung R-SDFFWD: the `sdf_forward_march` pipeline
            // pair + their shared Set-0 layout — genuinely exist at every boot (built
            // UNCONDITIONALLY, `boot`'s doc), the SAME `Some(...)` ALWAYS discipline as the
            // Forward v1 trio above; only ever RECORDED when `GBufferScene::path_has_sdf_forward`
            // holds.
            sdf_forward_march_pipeline: Some(&self.sdf_forward_march_pipeline),
            sdf_forward_march_sdfonly_pipeline: Some(&self.sdf_forward_march_sdfonly_pipeline),
            sdf_forward_march_viewt_pipeline: Some(&self.sdf_forward_march_viewt_pipeline),
            sdf_forward_march_sdfonly_viewt_pipeline: Some(&self.sdf_forward_march_sdfonly_viewt_pipeline),
            sdf_forward_march_layout: Some(&self.sdf_forward_march_layout),
            brick_levels_ubo: Some(&self.brick_levels_ubo),
            sdf_forward_view_z_a,
            sdf_forward_view_z_b,
            // Multi-paradigm render-path plan, rung R8: the VB v1 pipelines + layout + instance
            // ring — `vb_layout0`/`vb_raster_pipeline`/`vb_sky_pipeline`/`vb_instance_rings` exist
            // at EVERY boot (built UNCONDITIONALLY, `boot`'s doc, the SAME `Some(...)` ALWAYS
            // discipline as the Forward v1 trio above); `vb_resolve_pipeline` is `Some` only
            // AFTER `build_vb_resolve_pipeline` ran (a `VisibilityBuffer`-resolved boot with the
            // device-cap armed); `vb_geometry_set` is threaded straight from this fn's own param
            // (the live table itself lives in `World`, not on `Self`).
            vb_raster_pipeline: Some(&self.vb_raster_pipeline),
            vb_sky_pipeline: Some(&self.vb_sky_pipeline),
            vb_resolve_pipeline: self.vb_resolve_pipeline.as_ref(),
            vb_layout0: Some(&self.vb_layout0),
            vb_instance_ring: Some(&self.vb_instance_rings),
            vb_geometry_set,
            // VB-P2 classification plan, rung P2a (dark infra): `Some` only AFTER
            // `build_vb_classify_pipelines` ran (the SAME `vb_resolve_pipeline` `Option` shape
            // above). Nothing reads these fields yet — `record_vb`/`declare_vb_graph` are
            // untouched this rung; threaded here so a later rung (P2b/P2c) needs no further
            // plumbing.
            vb_classify_count_pipeline: self.vb_classify_count_pipeline.as_ref(),
            vb_classify_scan_pipeline: self.vb_classify_scan_pipeline.as_ref(),
            vb_classify_scatter_pipeline: self.vb_classify_scatter_pipeline.as_ref(),
            vb_shade_pipeline: self.vb_shade_pipeline.as_ref(),
            // VB-P2 classification plan, rung P2b: the classify `scan` pass's loop-bound push
            // constant (`GBufferScene::vb_classify_material_count`'s own doc — a valid upper
            // bound on any live `MaterialId` this frame could reference).
            vb_classify_material_count: material_table.capacity_rows(),
            // VB-P2 classification plan, rung P2c: the classified-vs-fused `lit`-producer
            // selector (`GBufferScene::vb_use_classified`'s own doc) — the per-frame local
            // computed above.
            vb_use_classified,
            // Textured-PBR rung TV0: the TEXTURED `vb_shade` pipeline + the raw TEXTURED
            // instance-material ring, threaded the SAME way `raster_pipeline_tex`/`bindless_set`
            // above are — `Some` iff `self.tex` exists (device-agnostic, unconditioned on
            // `any_textured_material`: `GBufferTargets` needs the ring reference to build
            // `vb_set0_tex` once per extent, independent of any SPECIFIC frame's texture usage).
            vb_shade_tex_pipeline: self.vb_shade_tex_pipeline.as_ref(),
            // VB-P1a/P1b: `Some` only after `Self::build_froxel_light_cull` ran (gated on the
            // owner-opt-in arm bit, default OFF) — see that fn's doc.
            vb_layout0_froxel: self.vb_layout0_froxel.as_ref(),
            vb_resolve_froxel_pipeline: self.vb_resolve_froxel_pipeline.as_ref(),
            vb_shade_froxel_pipeline: self.vb_shade_froxel_pipeline.as_ref(),
            vb_shade_tex_froxel_pipeline: self.vb_shade_tex_froxel_pipeline.as_ref(),
            vb_tex_instance_material_ring: self.tex.as_ref().map(|t| &t.tex_instance_material_rings),
            // Rung R9b: the split pair + VB SSAO gather. The deferred-built pipelines are
            // `Option`-threaded as-is (`Some` after `build_vb_split_pipelines` ran — the
            // `vb_resolve_pipeline` shape); the boot layouts/trio are `Some(...)` ALWAYS.
            vb_geo_pipeline: self.vb_geo_pipeline.as_ref(),
            vb_shade_split_pipeline: self.vb_shade_split_pipeline.as_ref(),
            vb_shade_split_tex_pipeline: self.vb_shade_split_tex_pipeline.as_ref(),
            vb_geo_aux_layout: Some(&self.vb_geo_aux_layout),
            vb_split_layout1: Some(&self.vb_split_layout1),
            ssao_vb_pipeline: ssao_variant.map(|v| &self.ssao_vb_pipelines[v]),
            vb_ssao_layout: Some(&self.vb_ssao_layout),
            // Rung R9d: the VB hardware shadow chain. The boot-built pipeline/layout are
            // `Option`-threaded as-is (`Some` only on an RT device); `vb_geo_mv_pipeline`/
            // `vb_shade_split_hwrt_pipeline`/`vb_shade_split_tex_hwrt_pipeline` are `Option`-
            // threaded like `vb_geo_pipeline`/`vb_shade_split_pipeline`/`vb_shade_split_tex_pipeline`
            // above (`Some` after `build_vb_split_pipelines` ran on an RT device).
            #[cfg(feature = "hwrt")]
            vb_shadow_vis_pipeline: self.vb_shadow_vis_pipeline.as_ref().map(|(p, _)| p),
            #[cfg(feature = "hwrt")]
            vb_shadow_vis_layout: self.vb_shadow_vis_pipeline.as_ref().map(|(_, l)| l),
            #[cfg(feature = "hwrt")]
            vb_geo_mv_pipeline: self.vb_geo_mv_pipeline.as_ref(),
            #[cfg(feature = "hwrt")]
            vb_shade_split_hwrt_pipeline: self.vb_shade_split_hwrt_pipeline.as_ref(),
            #[cfg(feature = "hwrt")]
            vb_shade_split_tex_hwrt_pipeline: self.vb_shade_split_tex_hwrt_pipeline.as_ref(),
            // Multi-paradigm render-path plan, rung R1: the plain-POD conversion (see this
            // fn's `resolved_render_path` param doc for why it cannot be a `From` impl).
            resolved_render_path: to_gpu_resolved_render_path(&resolved_render_path),
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

    /// Textured-PBR T6c: the FENCED slot's TEXTURED instance-material SSBO — the write
    /// target of the runner's per-frame
    /// [`upload_instance_materials_tex`](boyko_render::upload_instance_materials_tex).
    /// Returns `None` if [`Self::build_textured_resources`] never ran (`self.tex` absent —
    /// e.g. the bindless texture table failed to create). The sibling in-flight frame binds
    /// the OTHER slot — the same lock-free discipline as the base instance ring.
    #[inline]
    pub(crate) fn tex_instance_material_slot(&self, slot: usize) -> Option<&BoundBuffer> {
        self.tex.as_ref().map(|t| &t.tex_instance_material_rings[slot])
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

    /// VB-P1e D11: the BOOT-frozen `ClusterConfig::packed_dims()` snapshot
    /// [`Self::build_froxel_light_cull`] sized every L1 buffer from — `0` (meaningless) while
    /// [`Self::cluster_cull_pipeline`] is `None`. `boyko_app::runner`'s per-frame debug-only
    /// assert compares this against the LIVE `ClusterConfig` Resource to catch an owner system
    /// stomping the Resource after boot (a frame-level tripwire, not a fix — VB-P1k tracks the
    /// underlying `ClusterGrid`-reader skew this cannot close).
    #[inline]
    pub(crate) fn cluster_boot_packed_dims(&self) -> u32 {
        self.cluster_boot_packed_dims
    }

    /// `true` iff [`Self::build_froxel_light_cull`] actually ran and populated
    /// [`Self::cluster_boot_packed_dims`] — the SAME `#[inline] fn ... .is_some()` idiom
    /// [`Self::vb_bench_armed`] uses. `boyko_app::runner`'s per-frame boot/live dims
    /// `debug_assert_eq!` must gate on THIS, not on `ResolvedRenderPath::froxel_light_cull`
    /// alone (P1-2, adversarial review): `froxel_light_cull` is strictly WIDER than the
    /// condition that actually built the snapshot (`build_froxel_light_cull` additionally
    /// requires a live `MeshGeometryTableSlot`, which `froxel_light_cull` does not — see
    /// `render_path_config.rs`'s `vb_geometry_table`/`froxel_light_cull` fields), so a shipped
    /// `VisibilityBuffer` + `GeometryLegs::Sdf` boot (or any boot where
    /// `MeshGeometryTable::new` degrades to `None`) would otherwise compare a live non-zero
    /// `ClusterConfig` against a snapshot that was never written (`0`), panicking every frame
    /// in a debug build.
    #[inline]
    pub(crate) fn cluster_cull_armed(&self) -> bool {
        self.cluster_cull_pipeline.is_some()
    }

    /// VB-P1d: `true` iff the froxel cull/shade GPU-timestamp bench collector was armed at
    /// [`Self::boot`] (`BOYKO_VB_BENCH` set AND the device supports timestamps). The runner
    /// reads this ONCE, before the frame loop starts, to decide whether to drive the bench
    /// accumulation + summary print (see `boyko_app::runner`'s VB-P1d block) — `false` on
    /// every non-bench run, so that whole block is dead code there.
    #[inline]
    pub(crate) fn vb_bench_armed(&self) -> bool {
        self.vb_bench.is_some()
    }

    /// VB-P1e H0: reads back frame `fi`'s bench triple — `(cull_reset_ns, cull_dispatch_ns,
    /// shade_ns)`, masked + period-scaled by `RhiDevice::read_query_pool_ns` — from the armed
    /// bench collector. `None` iff [`Self::vb_bench_armed`] is `false` (the collector was never
    /// created; the runner never calls this then). `cull_reset_ns + cull_dispatch_ns` is
    /// REPORTED as `froxel_cull_ns` in place of VB-P1d's original single bracket (VB-P1e's H0
    /// split it into `VbTimedPass::CullReset`/`CullDispatch` so the fixed-cost hypothesis in
    /// `VB-P1E-HIERARCHICAL-CULL-PLAN.md` §1.2 could be attributed instead of assumed).
    ///
    /// The sum is NOT claimed to reproduce the pre-split bracket exactly: `CullDispatch`'s begin
    /// is a `TOP_OF_PIPE` write recorded after a `dstStage = COMPUTE` barrier, which does not
    /// order it, so the sum may double-count up to `cull_reset_ns`. That was measured and is
    /// SMALL — see [`crate::runner`]'s bench-print doc for the numbers and why it no longer
    /// gates anything.
    ///
    /// All three queries are unconditionally written every bench-armed frame — PROVIDED the
    /// caller upholds the bench's fused/classified-only precondition (`boyko_app::runner`'s
    /// VB-P1d block asserts it before ever calling this): `VbTimedPass::CullReset`/
    /// `CullDispatch` run even when the froxel arm itself is not boot-built (reporting
    /// near-zero ns); `VbTimedPass::VbShade` brackets whichever of `vb_shade`
    /// (classified)/`vb_resolve` (fused) this frame's `mesh_leg` + `vb_use_classified` select —
    /// mutually exclusive, always exactly one, ON A NON-SPLIT FRAME. The VB split lit-producer
    /// (`vb_shade_split`, armed by `resolved_render_path.mesh_geo_shade_split` — a pre-light
    /// consumer: SSAO/DDGI/SSR/shadow-denoise/Temporal under `VisibilityBuffer`) is UNBRACKETED
    /// and OUT OF SCOPE for this bench (`vb.rs`'s own VB-P1d doc on its three-way producer
    /// choice); a split frame would reset-but-never-write the VbShade pair, hanging the
    /// `VK_QUERY_RESULT_WAIT_BIT` readback below — this is why the caller MUST assert
    /// `!mesh_geo_shade_split` first.
    ///
    /// VB-SV0 rung S1.5: `true` iff the Deferred marcher bench collector was armed at
    /// [`Self::boot`] (`BOYKO_SV0_BENCH` set AND the device supports timestamps). The runner
    /// reads this ONCE, before the frame loop starts, to decide whether to drive the paired A/B
    /// + summary print — `false` on every non-bench run, so that whole block is dead code there.
    #[inline]
    pub(crate) fn sv0_bench_armed(&self) -> bool {
        self.sv0_bench.is_some()
    }

    /// VB-SV0 rung S1.5: reads back frame `fi`'s Deferred marcher dispatch cost in RAW TIMESTAMP
    /// TICKS from the armed bench collector. `None` iff [`Self::sv0_bench_armed`] is `false` (the
    /// collector was never created; the runner never calls this then).
    ///
    /// # Why ticks and not nanoseconds
    ///
    /// The GPU timestamp counter advances on a LATTICE — a hardware granularity of `G >= 1` tick
    /// that no Vulkan limit reports (`timestampPeriod` is the ns-per-tick SCALE, not the STEP).
    /// S1.5 must state its own resolution, so the runner recovers `G` empirically as the GCD of
    /// the raw per-frame tick counts. That is an INTEGER property: reading nanoseconds here and
    /// dividing the period back out would launder the measurement through the very scale factor
    /// under examination, and a float round-trip is not evidence of an integer lattice. The
    /// runner multiplies by `timestamp_period` itself, once, at the report boundary.
    ///
    /// The single [`Sv0TimedPass::Marcher`] pair is written on exactly the frames the recorder
    /// dispatches the marcher, so the caller MUST hold the marcher-carrying-path precondition
    /// (`boyko_app::runner`'s S1.5 block asserts `RenderPath::Deferred` + a marching leg before
    /// ever calling this) — otherwise the `VK_QUERY_RESULT_WAIT_BIT` readback below waits on a
    /// query this frame never wrote and hangs, the same hazard `read_vb_bench_ns` documents for
    /// the VB split producer.
    ///
    /// # Panics
    /// Panics (`expect("invariant: ...")`) if the readback fails — a bench-only diagnostic path
    /// (never reached on the shipped default), so a query-pool read failure here is a
    /// setup/driver bug, not a recoverable per-frame condition.
    pub(crate) fn read_sv0_marcher_ticks(&self, ctx: &VulkanContext, fi: usize) -> Option<u64> {
        let collector = self.sv0_bench.as_ref()?;
        let mut scratch = [0u64; (2 * SV0_PASS_COUNT) as usize];
        let mut out_ticks = [0u64; SV0_PASS_COUNT as usize];
        RhiDevice::read_query_pool_ticks(
            ctx,
            collector.pool(fi),
            SV0_PASS_COUNT,
            &mut scratch,
            &mut out_ticks,
        )
        .expect("invariant: VB-SV0 S1.5 bench query-pool readback");
        Some(out_ticks[Sv0TimedPass::Marcher.slot() as usize])
    }

    /// # Panics
    /// Panics (`expect("invariant: ...")`) if the readback fails — a bench-only diagnostic
    /// path (never reached on the shipped default), so a query-pool read failure here is a
    /// setup/driver bug, not a recoverable per-frame condition.
    pub(crate) fn read_vb_bench_ns(&self, ctx: &VulkanContext, fi: usize) -> Option<(f64, f64, f64)> {
        let collector = self.vb_bench.as_ref()?;
        let mut scratch = [0u64; (2 * VB_PASS_COUNT) as usize];
        let mut out_ns = [0.0f64; VB_PASS_COUNT as usize];
        RhiDevice::read_query_pool_ns(ctx, collector.pool(fi), VB_PASS_COUNT, &mut scratch, &mut out_ns)
            .expect("invariant: VB-P1d bench query-pool readback");
        Some((
            out_ns[VbTimedPass::CullReset.slot() as usize],
            out_ns[VbTimedPass::CullDispatch.slot() as usize],
            out_ns[VbTimedPass::VbShade.slot() as usize],
        ))
    }

    /// Tears every bundle down in reverse dependency order — the showcase
    /// teardown list, minus the resources R3 does not create (staging,
    /// per-mesh instanced buffers). Render P7-Q2's SSAO pipelines/layout ARE
    /// created here (see [`Self::ssao_pipelines`]) and are torn down below.
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
            // Multi-paradigm render-path plan, rung R4b-b: the Forward v1 mesh raster pipeline
            // + its descriptor-set layouts, created right after `csm` in `boot` — destroyed
            // here, right after it (reverse acquisition). The sky pipeline was created right
            // after `forward_pipeline`, so it is destroyed right after it here too.
            RhiDevice::destroy_graphics_pipeline(ctx, self.forward_pipeline);
            RhiDevice::destroy_graphics_pipeline(ctx, self.forward_sky_pipeline);
            RhiDevice::destroy_bind_group_layout(ctx, self.forward_layout0);
            RhiDevice::destroy_bind_group_layout(ctx, self.forward_layout1);
            // Multi-paradigm render-path plan, rung R5 (ForwardPlus): the depth-prepass
            // pipeline + the froxel opaque pipeline, created right after the Forward v1 trio
            // above — destroyed here, same reverse-acquisition order. Both are built against
            // `self.forward_layout0` (the unified layout, already destroyed above) — no separate
            // layout to tear down.
            RhiDevice::destroy_graphics_pipeline(ctx, self.forward_prepass_pipeline);
            RhiDevice::destroy_graphics_pipeline(ctx, self.forward_plus_pipeline);
            // Multi-paradigm render-path plan, rung R-SDFFWD: the `sdf_forward_march` pipeline
            // pair + their shared dedicated Set-0 layout, created right after the ForwardPlus
            // pair above — destroyed here, same reverse-acquisition order. Each pipeline OWNS
            // its own 2-set pipeline layout (`create_compute_pipeline_forward`'s doc), so
            // `destroy_compute_pipeline` tears that down too; `sdf_forward_march_layout` is the
            // SEPARATE Set-0 `VkDescriptorSetLayout` object both pipelines' layouts embed a copy
            // of at creation (Vulkan permits destroying a descriptor-set-layout handle once every
            // pipeline layout built against it exists — the SAME precedent `forward_layout1`
            // being destroyed before `forward_plus_pipeline`, above, already establishes).
            RhiDevice::destroy_compute_pipeline(ctx, self.sdf_forward_march_pipeline);
            RhiDevice::destroy_compute_pipeline(ctx, self.sdf_forward_march_sdfonly_pipeline);
            RhiDevice::destroy_compute_pipeline(ctx, self.sdf_forward_march_viewt_pipeline);
            RhiDevice::destroy_compute_pipeline(ctx, self.sdf_forward_march_sdfonly_viewt_pipeline);
            RhiDevice::destroy_bind_group_layout(ctx, self.sdf_forward_march_layout);
            RhiDevice::destroy_graphics_pipeline(ctx, self.present_pipeline);
            RhiDevice::destroy_graphics_pipeline(ctx, self.fxaa_pipeline);
            // Anti-aliasing Stage 2: the three SMAA pipelines — `smaa_edge_pipeline` shares
            // `present_layout` with `fxaa_pipeline` (both destroyed before it, below); the
            // weight/blend pipelines' own dedicated layouts are destroyed right after.
            RhiDevice::destroy_graphics_pipeline(ctx, self.smaa_edge_pipeline);
            RhiDevice::destroy_graphics_pipeline(ctx, self.smaa_weight_pipeline);
            RhiDevice::destroy_graphics_pipeline(ctx, self.smaa_blend_pipeline);
            RhiDevice::destroy_bind_group_layout(ctx, self.smaa_weight_layout);
            RhiDevice::destroy_bind_group_layout(ctx, self.smaa_blend_layout);
            // Anti-aliasing Stage 3: the SSAA downsample pipeline — shares `present_layout`
            // with `fxaa_pipeline`/`smaa_edge_pipeline` (destroyed before it, below). No
            // dedicated sampler to tear down (reuses `present_sampler`, destroyed later).
            RhiDevice::destroy_graphics_pipeline(ctx, self.ssaa_pipeline);
            RhiDevice::destroy_bind_group_layout(ctx, self.present_layout);
            RhiDevice::destroy_compute_pipeline(ctx, self.resolve_pipeline);
            // Render terminator-softening: the wrap-variant pipeline shares `resolve_layout`
            // with `resolve_pipeline` — both pipelines are destroyed before their shared layout.
            RhiDevice::destroy_compute_pipeline(ctx, self.resolve_pipeline_wrap);
            RhiDevice::destroy_bind_group_layout(ctx, self.resolve_layout);
            // TAA W5 (C3 fix): the taa_resolve compute pipeline + its 8-binding layout, both built
            // UNCONDITIONALLY at boot (every config incl. AaMode::Off), like fxaa/smaa/ssaa. Pipeline
            // before its layout — else a per-renderer boot leaks a pipeline + layout + sampler.
            RhiDevice::destroy_compute_pipeline(ctx, self.taa_resolve_pipeline);
            RhiDevice::destroy_bind_group_layout(ctx, self.taa_resolve_layout);
            // TAA rung T3: the RCAS compute pipeline + its 2-binding layout, built
            // UNCONDITIONALLY at boot (like `taa_resolve_pipeline` above). Pipeline before layout.
            RhiDevice::destroy_compute_pipeline(ctx, self.rcas_pipeline);
            RhiDevice::destroy_bind_group_layout(ctx, self.rcas_layout);
            // Render P7-Q2: the 3 SSAO pipelines share `ssao_layout` — pipelines before the
            // shared layout (reverse creation order), built UNCONDITIONALLY at boot (every
            // config incl. `SsaoQuality::Off`), like fxaa/smaa/ssaa/taa above.
            let [ssao_low, ssao_medium, ssao_high] = self.ssao_pipelines;
            RhiDevice::destroy_compute_pipeline(ctx, ssao_low);
            RhiDevice::destroy_compute_pipeline(ctx, ssao_medium);
            RhiDevice::destroy_compute_pipeline(ctx, ssao_high);
            RhiDevice::destroy_bind_group_layout(ctx, self.ssao_layout);
            // Rung R9b: the VB split objects — deferred-built pipelines (Option-guarded) first,
            // then the boot gather trio, then the three boot layouts (reverse creation order).
            if let Some(p) = self.vb_shade_split_tex_pipeline {
                RhiDevice::destroy_compute_pipeline(ctx, p);
            }
            if let Some(p) = self.vb_shade_split_pipeline {
                RhiDevice::destroy_compute_pipeline(ctx, p);
            }
            if let Some(p) = self.vb_geo_pipeline {
                RhiDevice::destroy_compute_pipeline(ctx, p);
            }
            // Rung R9d: the hwrt shadow-chain split siblings — `Option`-guarded (present only on
            // an RT device), destroyed in the SAME reverse-creation order as their software twins.
            #[cfg(feature = "hwrt")]
            if let Some(p) = self.vb_shade_split_tex_hwrt_pipeline {
                RhiDevice::destroy_compute_pipeline(ctx, p);
            }
            #[cfg(feature = "hwrt")]
            if let Some(p) = self.vb_shade_split_hwrt_pipeline {
                RhiDevice::destroy_compute_pipeline(ctx, p);
            }
            #[cfg(feature = "hwrt")]
            if let Some(p) = self.vb_geo_mv_pipeline {
                RhiDevice::destroy_compute_pipeline(ctx, p);
            }
            // Rung VB-P1b (W5): the froxel light-cull machinery `build_froxel_light_cull` built —
            // `Option`-guarded (allocated only when `ResolvedRenderPath::froxel_light_cull` armed
            // at boot; every field stays `None` on an unarmed boot, so this whole block is a
            // no-op there). Torn down in reverse acquisition order (that fn's own build order):
            // the three `vb_layout0_froxel`-built compute pipelines, then that shared layout, then
            // the cluster buffers (alloc counter, index list, grid), then the cull pipeline + its
            // own dedicated 1-set layout.
            if let Some(p) = self.vb_shade_tex_froxel_pipeline {
                RhiDevice::destroy_compute_pipeline(ctx, p);
            }
            if let Some(p) = self.vb_shade_froxel_pipeline {
                RhiDevice::destroy_compute_pipeline(ctx, p);
            }
            if let Some(p) = self.vb_resolve_froxel_pipeline {
                RhiDevice::destroy_compute_pipeline(ctx, p);
            }
            if let Some(layout) = self.vb_layout0_froxel {
                RhiDevice::destroy_bind_group_layout(ctx, layout);
            }
            if let Some(buf) = self.light_index_alloc {
                RhiDevice::destroy_buffer(ctx, buf);
            }
            if let Some(buf) = self.light_index {
                RhiDevice::destroy_buffer(ctx, buf);
            }
            if let Some(buf) = self.cluster_grid {
                RhiDevice::destroy_buffer(ctx, buf);
            }
            if let Some(p) = self.cluster_cull_pipeline {
                RhiDevice::destroy_compute_pipeline(ctx, p);
            }
            if let Some(layout) = self.cull_layout {
                RhiDevice::destroy_bind_group_layout(ctx, layout);
            }
            // VB-P1d: the froxel cull/shade bench query pools, `Option`-guarded (built only
            // under `BOYKO_VB_BENCH` + a timestamp-capable device — every other boot leaves
            // this `None`, a no-op here).
            if let Some(collector) = self.vb_bench {
                for pool in collector.into_pools() {
                    RhiDevice::destroy_query_pool(ctx, pool);
                }
            }
            // VB-SV0 rung S1.5: the Deferred marcher bench query pools, `Option`-guarded exactly
            // like `vb_bench` above (built only under `BOYKO_SV0_BENCH` + a timestamp-capable
            // device — every other boot leaves this `None`, a no-op here).
            if let Some(collector) = self.sv0_bench {
                for pool in collector.into_pools() {
                    RhiDevice::destroy_query_pool(ctx, pool);
                }
            }
            let [vb_ssao_low, vb_ssao_medium, vb_ssao_high] = self.ssao_vb_pipelines;
            RhiDevice::destroy_compute_pipeline(ctx, vb_ssao_low);
            RhiDevice::destroy_compute_pipeline(ctx, vb_ssao_medium);
            RhiDevice::destroy_compute_pipeline(ctx, vb_ssao_high);
            RhiDevice::destroy_bind_group_layout(ctx, self.vb_split_layout1);
            RhiDevice::destroy_bind_group_layout(ctx, self.vb_geo_aux_layout);
            RhiDevice::destroy_bind_group_layout(ctx, self.vb_ssao_layout);
            // The SSAO à-trous denoise chain: the 3 role-keyed pipelines share
            // `ssao_atrous_layout` — pipelines before the shared layout (reverse creation order),
            // built UNCONDITIONALLY at boot (every config, like the gather pipelines above).
            RhiDevice::destroy_compute_pipeline(ctx, self.ssao_atrous_read8_pipeline);
            RhiDevice::destroy_compute_pipeline(ctx, self.ssao_atrous_interior_pipeline);
            RhiDevice::destroy_compute_pipeline(ctx, self.ssao_atrous_write8_pipeline);
            RhiDevice::destroy_bind_group_layout(ctx, self.ssao_atrous_layout);
            // Multi-paradigm render-path plan, rung R3b: the `viewt_from_depth` pipeline + its
            // 2-binding layout, built UNCONDITIONALLY at boot (like the SSAO pipelines above).
            // Pipeline before its layout (reverse creation order).
            RhiDevice::destroy_compute_pipeline(ctx, self.viewt_from_depth_pipeline);
            RhiDevice::destroy_bind_group_layout(ctx, self.viewt_from_depth_layout);
            // TAA-under-VB: the `viewt_from_depth_rz` REVERSE-Z sibling + its 3-binding layout,
            // built UNCONDITIONALLY at boot (like `viewt_from_depth_pipeline` above). Pipeline
            // before its layout (reverse creation order).
            RhiDevice::destroy_compute_pipeline(ctx, self.viewt_from_vb_depth_pipeline);
            RhiDevice::destroy_bind_group_layout(ctx, self.viewt_from_vb_depth_layout);
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
            // Rung R9d: the VB split's dedicated shadow-vis gather pipeline + its 7-binding
            // layout, `Option`-guarded (present only on an RT device). Pipeline before layout.
            #[cfg(feature = "hwrt")]
            if let Some((pipeline, layout)) = self.vb_shadow_vis_pipeline {
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
            // Textured-PBR T6c: the TEXTURED resources bind the SHARED `instance_rings` (@0)
            // plus their own instance-material ring (@1); torn down BEFORE the shared instance
            // bind groups/rings below (bind groups first, then the pipeline/layout, then the
            // owned buffers) — mirrors the PM teardown ordering immediately above.
            // `Option`-guarded: `self.tex` is `None` if `build_textured_resources` never ran
            // (e.g. the bindless texture table failed to create). The bindless texture-array
            // descriptor SET/layout itself is owned and torn down separately by
            // `BindlessTextureTable::destroy` — not touched here.
            if let Some(tex) = self.tex {
                for bg in tex.tex_bind_groups {
                    RhiDevice::destroy_bind_group(ctx, bg);
                }
                RhiDevice::destroy_graphics_pipeline(ctx, tex.raster_pipeline_tex);
                RhiDevice::destroy_bind_group_layout(ctx, tex.tex_instance_material_layout);
                for b in tex.tex_instance_material_rings {
                    RhiDevice::destroy_buffer(ctx, b);
                }
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
            RhiDevice::destroy_sampler(ctx, self.fxaa_sampler);
            RhiDevice::destroy_sampler(ctx, self.smaa_sampler);
            RhiDevice::destroy_sampler(ctx, self.taa_linear_sampler);
            // Anti-aliasing Stage 2: the two boot-resident SMAA LUT textures (no dependents —
            // no set still references them once the SMAA sets above are torn down by the
            // `Renderer`/`GBufferTargets` teardown that runs before this fn).
            RhiDevice::destroy_texture(ctx, self.smaa_area_tex);
            RhiDevice::destroy_texture(ctx, self.smaa_search_tex);
            RhiDevice::destroy_sampler(ctx, self.depth_sampler);
            RhiDevice::destroy_buffer(ctx, self.vertex_buffer);
            RhiDevice::destroy_buffer(ctx, self.tiles_buffer);
            self.clipmap.destroy(ctx);
            RhiDevice::destroy_buffer(ctx, self.brick_levels_ubo);
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

#[cfg(test)]
mod tests {
    use boyko_render::{
        GeometryLegs, RenderPath, RenderPathConsumers, RenderPathDeviceCaps, ResolvedRenderPath,
        resolve_rules,
    };

    use super::*;

    /// P2-1(a): the 0%-gate carrier converts to the `ResolvedRenderPathGpu` 0%-gate default —
    /// a never-resolved world's `GBufferScene::resolved_render_path` matches what a booted world
    /// resolving the default `RenderPathConfig` would ALSO produce (byte-identity anchor).
    #[test]
    fn to_gpu_resolved_render_path_default_matches_gpu_default() {
        let default_resolved = ResolvedRenderPath::default();
        assert_eq!(to_gpu_resolved_render_path(&default_resolved), ResolvedRenderPathGpu::default());
    }

    /// P2-1(b): a rich, non-default carrier (Forward + SSAO + shadow-temporal — both pre-light
    /// consumers armed, so `needs_depth_prepass`/`prepass_writes_motion`/`thin_aux` are all
    /// non-trivially set) round-trips through [`to_gpu_resolved_render_path`] field-for-field.
    /// Built via `resolve_rules` (not a hand-written literal) so the derived fields are a
    /// REALISTIC, internally-consistent combination, not a possibly-inconsistent guess.
    #[test]
    fn to_gpu_resolved_render_path_round_trips_a_non_default_carrier() {
        let consumers =
            RenderPathConsumers { ssao_on: true, shadow_temporal_on: true, ..RenderPathConsumers::default() };
        let caps = RenderPathDeviceCaps::new(true);
        let resolved = resolve_rules(RenderPath::Forward, GeometryLegs::Mesh, consumers, caps);

        // Sanity: this carrier is genuinely non-default (both Decision-8 flags fired).
        assert!(resolved.needs_depth_prepass);
        assert!(resolved.prepass_writes_motion);

        let gpu = to_gpu_resolved_render_path(&resolved);
        assert_eq!(gpu.path, resolved.path as u32);
        assert_eq!(gpu.legs, resolved.legs as u32);
        assert_eq!(gpu.mesh_leg, resolved.mesh_leg);
        assert_eq!(gpu.sdf_leg, resolved.sdf_leg);
        assert_eq!(gpu.sdf_forward_marched, resolved.sdf_forward_marched);
        assert_eq!(gpu.needs_depth_prepass, resolved.needs_depth_prepass);
        assert_eq!(gpu.prepass_writes_motion, resolved.prepass_writes_motion);
        assert_eq!(gpu.mesh_geo_shade_split, resolved.mesh_geo_shade_split);
        assert_eq!(gpu.sdf_geo_shade_split, resolved.sdf_geo_shade_split);
        assert_eq!(gpu.sdf_surface_cache, resolved.sdf_surface_cache);
        assert_eq!(gpu.vb_geometry_table, resolved.vb_geometry_table);
        assert_eq!(gpu.depth_kind, resolved.depth_kind as u32);
        assert_eq!(gpu.thin_aux, resolved.thin_aux.bits());
        assert_eq!(gpu.shadow, resolved.shadow.bits());
        assert_eq!(gpu.froxel_light_cull, resolved.froxel_light_cull);
    }
}
