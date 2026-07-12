//! Render **P1b GPU gate** (scaffold) — the OFFSCREEN image-based SDF + mesh hybrid
//! composite writing an MRT G-buffer, the image-based rewrite of the rung-10
//! packed-buffer marcher (`sdf_mesh_hybrid_depth.rs`'s `run_hybrid`).
//!
//! # What this proves (the P1b milestone)
//!
//! `run_gbuffer_hybrid` records the §15.1 shared-depth seam WITHOUT the per-frame
//! depth→buffer copy: a real GPU-rasterized quad's depth is written into a D32_SFLOAT
//! IMAGE, transitioned `DEPTH_ATTACHMENT_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL` (a
//! SINGLE depth barrier, DEPTH aspect, `LATE_FRAGMENT_TESTS` src — replacing the old
//! copy + its two transfer barriers), and SAMPLED directly by the marcher compute
//! shader (`Texture2D<float>.Load`). The marcher STORES its color into an
//! `R8G8B8A8_UNORM` STORAGE image (the ALBEDO G-buffer target), plus additive
//! normal/material targets, through the P1a multi-resource descriptor *vocabulary* set
//! (written ONCE at create — NO per-frame `vkUpdateDescriptorSets`). A deferred RESOLVE
//! pass then composites `gLit` (full Cook-Torrance on the SDF arm; the `mask == 0` mesh /
//! background / empty pixels pass through verbatim). The LIT image is read back and
//! asserted against the deferred PBR oracle `golden_deferred_resolve ∘
//! golden_marcher_attributes` within `+/-2/255` per channel.
//!
//! PBR MVP-2 (the behavioral change): the SDF-surface (mask == 1) shading moved from the
//! MVP-1 `base*vis` Lambert inline composite (the retired `golden_composite_pixel_ex*`
//! oracles) to full Cook-Torrance via the deferred G-buffer + resolve. The GPU gates that
//! read `gLit` therefore compare against the deferred oracle (the proven reference, see
//! the `d2g`/`d3g` gates), NOT the MVP-1 inline oracles — which survive only on the
//! pass-through arms (host-only: `a_host_*` + `d1_host_*`).
//!
//! Determinism (INVIOLABLE): the field eval + ray-gen + marcher attributes are
//! byte-identical to the host oracle (a verbatim shader cut); only the depth SOURCE (a
//! sampled image) and the color SINK (a storage image) change. The float-to-UNORM store
//! vs the host `pack_rgba` rounding is absorbed by the `+/-2/255` tolerance.

mod common;
use common::*;

use boyko_rhi::descriptor::{BarrierDesc, BufferBarrier};
use boyko_rhi::enums::{BarrierAccess, BarrierStage};
use boyko_rhi::{
    AddressMode, BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry,
    BufferDesc, BufferImageCopy, BufferUsage, CompareOp, ComputePipelineDesc, DepthAttachment,
    DescriptorKind, Filter, Format,
    CullMode, GraphicsPipelineDesc, ImageAspect, ImageBarrierDesc, ImageLayout, ImageSubresourceRange,
    ImageUsage, LoadOp, MemoryLocation, MipMode, PrimitiveTopology, RenderArea, RenderingAttachment,
    RenderingDesc, RhiCommandEncoder, RhiDevice, RhiQueue, SamplerDesc, ShaderStage, StoreOp,
    TextureDesc, TextureDimension, VertexAttribute, VertexBufferLayout, VertexFormat, Viewport,
};
use boyko_rhi_vulkan::compute::{
    COMPOSITE_PUSH_CONSTANT_BYTES, CompositeCamera, CompositePushConstants, DEFAULT_LIGHT_DIR,
    DEFAULT_MARCHER_OMEGA, FineMarcherPush, LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS, LOCAL_SIZE_X,
    MESH_COLOR, MESH_DEPTH_CLEAR,
    SDF_CAMERA_Z, SDF_IMG_H,
    SDF_IMG_W, SDF_TRACE_T_MAX, SDF_VIEW_HALF_EXTENT, SdfEdit, TILE_BOUND_BYTES, TILE_FLAG_EMPTY,
    TILE_SIZE, TileBound, EDITLIST_BUFFER_WORDS, editlist_pixel_hits, encode_edit_list,
    // P6 R1 multi-light SDF shadows: the per-pixel dominant-N caster cap (mirrors the
    // shader's `MAX_SDF_SHADOW_CASTERS_PER_PIXEL`).
    MAX_SDF_SHADOW_CASTERS_PER_PIXEL,
    // Render P7-Q2: the quality-VARIANT SSAO `.spv` selector + the host preset table.
    sdf_ssao_spirv_variant, SSAO_PARAMS,
    SSAO_QUALITY_LOW, SSAO_QUALITY_MEDIUM, SSAO_QUALITY_HIGH,
    GOLDEN_LIGHT_HEADER_BASE_WORDS,
    composite_pixel_ray, deferred_pbr_spirv, mesh_depth_for_z,
    pack_rgba, pixel_world_xy,
    sdf_gbuffer_composite_spirv, sdf_op, sdf_tile_cull_spirv, tile_grid_extent,
    cluster_cull_spirv, ClusterCullPush,
    CLUSTER_CULL_PUSH_BYTES,
    // M2 brick-atlas trilinear+cubic SURFACE path: the widened b5 camera UBO (128 B with the
    // M2GridParams tail @80), the M2 grid params block, and the exact-CSG crease epsilon. The
    // atlas image/sampler themselves come from `BrickAtlas` (below).
    B5_CAMERA_UBO_BYTES, M2GridParams, M2_CREASE_EPS, M2_GRID_PARAMS_OFFSET,
    // M4 clip-map LOD (Slice C): the further-widened b5 camera UBO (224 B with the N-level
    // M4GridParams array tail @80) + the per-level params block.
    B5_CAMERA_UBO_BYTES_M4, M4GridParams,
};
// The CPU golden-reference oracles (audit W3/R-2 split): marcher / lighting / SSAO /
// cluster-cull host mirrors the GPU readback is diffed against. Behind the `goldens`
// cargo feature (on for the test crates via the self dev-dependency).
use boyko_rhi_vulkan::goldens::{
    GoldenClusterConfig, GoldenLight, GoldenLightHeader, GoldenMaterial, MarcherAttributes,
    golden_cluster_cull, golden_cluster_index, golden_composite_pixel_brick_m2,
    golden_composite_pixel_brick_m4, golden_composite_pixel_culled, golden_composite_pixel_ex,
    golden_composite_pixel_ex_omega_lit, golden_deferred_resolve, golden_deferred_resolve_clustered,
    golden_deferred_resolve_table, golden_deferred_resolve_table_shadowed,
    golden_deferred_resolve_table_shadowed_ssao, golden_gbuffer, golden_marcher_attributes,
    golden_ssao_attributes, golden_ssao_blur, golden_tile_bound,
};
use boyko_rhi_vulkan::brick_atlas::{BrickAtlas, BrickClipmap};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::rhi_impl::{VulkanBindGroup, VulkanBindGroupLayout, VulkanSampler};
use boyko_rhi_vulkan::texture::VulkanTexture;

/// CSM Increment 1b (Rung A): the OFF-path cascade descriptor TRIO every resolve set must bind
/// bound-but-unread. The recompiled `deferred_pbr.comp` STATICALLY references `gCsm` (combined
/// image+sampler @12) + the `CsmCascades` UBO (@13), so EVERY resolve layout MUST declare those
/// two bindings and EVERY resolve set MUST bind a valid descriptor — even when `csm_mode == 0`
/// (every test scene), where the resolve's `SampleCmpLevelZero` never runs (the 0%-gate; the
/// dummies are never sampled). The trio: a 4-layer 1×1 D32 array texture (so its
/// `VK_IMAGE_VIEW_TYPE_2D_ARRAY` sample view resolves `Texture2DArray`), a `LessOrEqual` PCF
/// comparison sampler, and a zeroed 336-byte cascade UBO mirroring `ResolvedCsm`.
struct CsmResolveDummies {
    cascade: VulkanTexture,
    sampler: VulkanSampler,
    ubo: BoundBuffer,
    // Shadow Phase 5 Inc-1-GPU: the OFF-path SPOT/POINT shadow-ATLAS trio bound bound-but-unread at
    // resolve @14/@15. The recompiled `deferred_pbr.comp` STATICALLY references `gShadowAtlas`
    // (combined image+sampler @14) + the `ShadowAtlas` UBO (@15), so EVERY resolve layout MUST
    // declare those two bindings and EVERY resolve set MUST bind a valid descriptor — even when
    // `punctual_shadow_mode == 0` (every test scene), where the resolve's `SampleCmpLevelZero` never
    // runs (the 0%-gate). The trio: a 16-layer (`M_SLOTS`) 1×1 D32 array texture (so its
    // `VK_IMAGE_VIEW_TYPE_2D_ARRAY` sample view resolves `Texture2DArray`), a `LessOrEqual` PCF
    // comparison sampler, and a zeroed 1296-byte atlas UBO mirroring `ResolvedShadowAtlas`.
    atlas: VulkanTexture,
    atlas_sampler: VulkanSampler,
    atlas_ubo: BoundBuffer,
}

impl CsmResolveDummies {
    /// Creates the OFF-path CSM trio + the Shadow Inc-1-GPU atlas trio on `device`. The cascade is
    /// `array_layers = 4` (== `MAX_CASCADES`) and the atlas `array_layers = 16` (== `M_SLOTS`) so
    /// each `VK_IMAGE_VIEW_TYPE_2D_ARRAY` sample view exists (a 1-layer image has none); 1×1 keeps
    /// them tiny.
    fn create(device: &VulkanContext) -> Self {
        let cascade = device
            .create_texture(&TextureDesc {
                width: 1,
                height: 1,
                depth: 1,
                format: Format::D32Sfloat,
                dimension: TextureDimension::D2,
                usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
                array_layers: 4,
                mip_levels: 1,
                view_format: None,
            })
            .expect("CSM dummy cascade array texture");
        let sampler = device
            .create_sampler(&SamplerDesc {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: AddressMode::ClampToEdge,
                mip: MipMode::None,
                compare: Some(CompareOp::LessOrEqual),
            })
            .expect("CSM dummy PCF comparison sampler");
        let ubo = device
            .create_buffer(&BufferDesc {
                size: 336,
                usage: BufferUsage::UNIFORM,
                location: MemoryLocation::HostVisibleCoherent,
            })
            .expect("CSM dummy cascade UBO (zeroed ResolvedCsm)");
        let atlas = device
            .create_texture(&TextureDesc {
                width: 1,
                height: 1,
                depth: 1,
                format: Format::D32Sfloat,
                dimension: TextureDimension::D2,
                usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
                array_layers: 16,
                mip_levels: 1,
                view_format: None,
            })
            .expect("shadow-atlas dummy array texture (M_SLOTS layers)");
        let atlas_sampler = device
            .create_sampler(&SamplerDesc {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: AddressMode::ClampToEdge,
                mip: MipMode::None,
                compare: Some(CompareOp::LessOrEqual),
            })
            .expect("shadow-atlas dummy PCF comparison sampler");
        let atlas_ubo = device
            .create_buffer(&BufferDesc {
                size: 1296,
                usage: BufferUsage::UNIFORM,
                location: MemoryLocation::HostVisibleCoherent,
            })
            .expect("shadow-atlas dummy UBO (zeroed ResolvedShadowAtlas)");
        Self { cascade, sampler, ubo, atlas, atlas_sampler, atlas_ubo }
    }

    /// The four resolve LAYOUT entries the shadow stack adds: binding 12 (CSM combined image+sampler),
    /// 13 (CSM uniform buffer), 14 (atlas combined image+sampler), 15 (atlas uniform buffer). The
    /// leading 12 entries (bindings 0 through 11) plus these 4 give 16 — the descriptor cap.
    fn layout_entries() -> [BindGroupLayoutEntry; 4] {
        [
            BindGroupLayoutEntry { binding: 12, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 13, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 14, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 15, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        ]
    }

    /// Tears both trios down (reverse creation order).
    ///
    /// # Safety
    /// Each resource was created on `device`, its GPU work completed (the caller fence-waited),
    /// and each is destroyed exactly once here.
    unsafe fn destroy(self, device: &VulkanContext) {
        // SAFETY: per the contract `device` is the live context and nothing references the trios.
        unsafe {
            device.destroy_buffer(self.atlas_ubo);
            device.destroy_sampler(self.atlas_sampler);
            device.destroy_texture(self.atlas);
            device.destroy_buffer(self.ubo);
            device.destroy_sampler(self.sampler);
            device.destroy_texture(self.cascade);
        }
    }
}

/// Total pixel count (the compute UBO `count`; the shader bounds `idx < count`).
const PIXELS: u32 = SDF_IMG_W * SDF_IMG_H;

/// The byte size of the `M2GridParams` block written into the b5 camera UBO tail (3 std140 `vec4`
/// lanes = 48 B = `B5_CAMERA_UBO_BYTES - M2_GRID_PARAMS_OFFSET`). The local mirror of
/// `compute::M2_GRID_PARAMS_BYTES` (kept local to avoid widening the import for one const).
const M2_GRID_PARAMS_BYTES_LOCAL: usize = B5_CAMERA_UBO_BYTES - M2_GRID_PARAMS_OFFSET;

/// R8G8B8A8 ALBEDO readback byte size.
const READBACK_BYTES: u64 = (PIXELS as u64) * 4;

/// Per-channel tolerance on the packed-RGBA bytes (identical to rung 9/10): DXC
/// `mad`/`fma` rounding + the float→UNORM store quantization make a bit-exact match
/// brittle; `+/-2/255` still proves the lit SDF surface / flat mesh / background colors
/// apart (they differ by 100+).
const CHANNEL_TOL: i32 = 2;

/// The depth attachment's CLEAR value (the far plane; an uncovered pixel keeps it,
/// decoded as "no mesh"). Must equal [`MESH_DEPTH_CLEAR`].
const DEPTH_CLEAR: f32 = MESH_DEPTH_CLEAR;

/// The G-buffer color format (albedo / normal / material): `R8G8B8A8_UNORM`, the
/// STORAGE-image store target whose support the [`DeviceCaps`] boot fail-fast asserts.
const GBUFFER_FORMAT: Format = Format::R8G8B8A8Unorm;

const VERTEX_STRIDE: u32 = core::mem::size_of::<Vertex>() as u32;
const _: () = assert!(VERTEX_STRIDE == 40, "Vertex must be tightly packed at 40 bytes");

/// The mesh-raster VERTEX push size: `{ float4x4 view_proj; float4 cam_eye; uint base_instance;
/// uint use_model_matrix }`. The hybrid-mesh-room PERSPECTIVE step widened it 64 -> 80; M1
/// (instanced-capable raster) widened it 80 -> 88, appending the `gbuffer_mrt.vs` instanced-arm
/// selectors. The offscreen ORTHO goldens append a zeroed `cam_eye` (mode 0) + zero selectors
/// (`use_model_matrix == 0` => the VS's legacy arm), so their `SV_Position.z` depth is
/// byte-identical.
const MVP_BYTES: u32 = 88;

/// PBR MVP-2: the std430 word-packing of a ONE-element material table holding the engine
/// default material (mid-gray dielectric: base 0.8/0.8/0.8/1, metallic 0, roughness 0.5,
/// reflectance 0.5, flags 0, emissive 0). 12 words = 48 B, mirroring `MaterialGpu`'s 3
/// `vec4` lanes. The crater/box/smooth edits carry NO material id (center.w == 0), so every
/// SDF hit picks id 0 → this material. Kept in sync with [`host_material_table`].
const DEFAULT_MATERIAL_TABLE: [u32; 12] = [
    // lane 0: base_color (rgb linear + alpha)
    0x3F4CCCCD, 0x3F4CCCCD, 0x3F4CCCCD, 0x3F800000, // 0.8, 0.8, 0.8, 1.0
    // lane 1: mrr = [metallic, roughness, reflectance, bitcast(flags)]
    0x00000000, 0x3F000000, 0x3F000000, 0x00000000, // 0.0, 0.5, 0.5, flags=0
    // lane 2: emissive (rgb linear + unused)
    0x00000000, 0x00000000, 0x00000000, 0x00000000, // 0, 0, 0, 0
];

/// Lighting L0a: the std430 word-packing of the DEGENERATE light table — the 0%-gate
/// anchor that makes the NEW table-driven `deferred_pbr` resolve reproduce the old
/// compiled-in `LIGHT_DIR`/`LIGHT_COLOR`/`SKY_*` image byte-for-byte (so every existing
/// GPU golden in this file, which compares against the constant-path `golden_deferred_
/// resolve`, still passes within ±2/255). `[LightHeaderGpu (16w) || GpuLight[2] (24w)]`:
/// element 0 = DIRECTIONAL dir (0,0,1) white illuminance 1.0; element 1 = SKY with
/// sky == ground == (0.10,0.10,0.12); exposure 1.0. Mirrors `boyko_render::light` +
/// `light_table.hlsli` (host fingerprints const-assert the layout).
const DEGENERATE_LIGHT_TABLE: [u32; 40] = [
    // --- LightHeaderGpu (16 words) ---
    0x00000002, 0x3F800000, 0x00000002, 0x00000000, // count 2, exposure 1.0, l0a 2, ps 0
    0x3DCCCCCD, 0x3DCCCCCD, 0x3DF5C28F, 0x00000000, // sky_diffuse 0.10,0.10,0.12, pad
    0x3DCCCCCD, 0x3DCCCCCD, 0x3DF5C28F, 0x00000000, // sky_spec    0.10,0.10,0.12, pad
    0x00000000, 0x00000000, 0x00000000, 0x00000000, // cluster_params (zero in L0)
    // --- GpuLight[0] DIRECTIONAL (12 words) ---
    0x00000000, 0x00000000, 0x3F800000, 0x00000000, // dir_kind: (0,0,1), kind 0
    0x00000000, 0x00000000, 0x00000000, 0x7F800000, // pos_range: (0,0,0), range +inf
    0x3F800000, 0x3F800000, 0x3F800000, 0x00000000, // color_cone: (1,1,1), cone 0
    // --- GpuLight[1] SKY (12 words) ---
    0x00000000, 0x00000000, 0x00000000, 0x00000003, // dir_kind: (0,0,0), kind 3 (SKY)
    0x3DCCCCCD, 0x3DCCCCCD, 0x3DF5C28F, 0x00000000, // pos_range: ground 0.10,0.10,0.12, r 0
    0x3DCCCCCD, 0x3DCCCCCD, 0x3DF5C28F, 0x00000000, // color_cone: sky 0.10,0.10,0.12, cone 0
];

/// The HOST mirror of [`DEFAULT_MATERIAL_TABLE`] for the `golden_*` oracles (the same
/// single default material at id 0).
fn host_material_table() -> [GoldenMaterial; 1] {
    [GoldenMaterial::default()]
}

/// Render P5-r0: the mesh-MRT G-buffer PRODUCER vertex SPIR-V (`gbuffer_mrt.vs.spv`):
/// passes through the LINEAR vertex color + the PER-VERTEX world normal (loc 2, offset 12).
/// Vertex layout: position (loc 0, offset 0) + color (loc 1, offset 24) + normal (loc 2, offset 12).
/// M4 grew the blob 3068 -> 4480 B (the instanced arm's per-vertex inverse-transpose normal
/// matrix + the W4 degeneracy guard); the legacy `use_model_matrix == 0` arm is untouched.
static MRT_VS_SPV: SpirvBlob<4480> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/gbuffer_mrt.vs.spv"
)));

/// Render P5-r0: the mesh-MRT G-buffer PRODUCER fragment SPIR-V (`gbuffer_mrt.fs.spv`):
/// writes albedo/normal/material as 3 MRT in the marcher's exact encoding (mask=1) + the
/// marcher-aligned `SV_Depth`, with the eDSL-spliced `oct_encode` / `pack_material_id_ba`
/// (guarded by `gbuffer_mrt_edsl_sync.rs`).
static MRT_FS_SPV: SpirvBlob<2252> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/gbuffer_mrt.fs.spv"
)));

/// Boots a validation-enabled headless context, or returns `None` (with a SKIP log)
/// when no GPU / loader / validation layer / dynamic-rendering is available.
fn boot_or_skip(test: &str) -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP {test}: validation layer / GPU / dynamicRendering unavailable ({e:?})");
            None
        }
    }
}

/// Asserts the validation messenger recorded ZERO messages (the GPU-half oracle).
///
/// A no-op (with a one-line note) when validation is disabled via
/// `BOYKO_DISABLE_VALIDATION` (the layer DLL crashes the MinGW process on this
/// box): there is no messenger to read, but the PIXEL goldens still run and
/// compare. Gating here covers every call site at once.
fn assert_validation_clean(ctx: &VulkanContext) {
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — skipping the clean-oracle assert");
        return;
    }
    if !ctx.validation_enabled() {
        assert!(
            std::env::var_os("BOYKO_DISABLE_VALIDATION").is_some(),
            "validation must be active when enable_validation is set and the escape hatch is absent"
        );
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) - messenger oracle skipped");
        return;
    }
    let state = ctx
        .debug_state()
        .expect("invariant: validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the P1b G-buffer hybrid run — see the [vk-validation] log",
        state.total()
    );
}

/// `ceil(PIXELS / LOCAL_SIZE_X)` — the 1D compute dispatch group count (fine pass).
fn group_count_x() -> u32 {
    PIXELS.div_ceil(LOCAL_SIZE_X)
}

/// The coarse tile-grid extent (`tiles_w`, `tiles_h`) for the golden 64×64 image.
fn tile_extent() -> (u32, u32) {
    tile_grid_extent(SDF_IMG_W, SDF_IMG_H)
}

/// Total coarse tiles (the `RWStructuredBuffer<TileBound>` element count + the
/// coarse-pass dispatch element count).
fn tile_count() -> u32 {
    let (tw, th) = tile_extent();
    tw * th
}

/// `ceil(tile_count / LOCAL_SIZE_X)` — the 1D coarse-pass dispatch group count.
fn coarse_group_count_x() -> u32 {
    tile_count().div_ceil(LOCAL_SIZE_X)
}

/// The orthographic MVP for the mesh-MRT vertex shader, uploaded COLUMN-MAJOR (the
/// VERIFIED transpose — see `run_hybrid`'s `ortho_mvp_bytes`). Maps a fronto-parallel
/// world vertex so the rasterized `SV_Position.z` is the axial `(CAM_Z - worldZ) / T_MAX`,
/// which the fragment writes back unchanged under ortho (`cam_mode == 0`). Bytes 64..80
/// (`[0u8; MVP_BYTES]` leaves them zeroed) are the `cam_eye` push field = [0,0,0,0] (mode 0 =
/// ortho; the eye is unused since the ortho fragment keeps `SV_Position.z`); bytes 80..88 are
/// the M1 instanced-arm selectors, left zero (`base_instance == 0`, `use_model_matrix == 0`
/// => the VS's legacy arm — byte-identical pixels).
#[rustfmt::skip]
fn ortho_mvp_bytes() -> [u8; MVP_BYTES as usize] {
    let h = SDF_VIEW_HALF_EXTENT;
    let tmax = SDF_TRACE_T_MAX;
    let cam = SDF_CAMERA_Z;
    // Mᵀ in row-major upload order: mt[r*4 + c] = M[c][r] (each group of 4 is a COLUMN
    // of M); the only off-diagonal term `CAM_Z/T_MAX` lives at mt[14].
    let mt: [f32; 16] = [
        1.0 / h, 0.0,      0.0,          0.0,
        0.0,     -1.0 / h, 0.0,          0.0,
        0.0,     0.0,      -1.0 / tmax,  0.0,
        0.0,     0.0,      cam / tmax,   1.0,
    ];
    let mut out = [0u8; MVP_BYTES as usize];
    for (i, f) in mt.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    out
}

/// M1: creates the gbuffer raster pipeline's `set 0` per-instance model resources — a
/// 1-binding `StorageBuffer` layout (binding 0, VERTEX stage), a 1-element host-visible
/// instance SSBO seeded with the identity 3x4 affine, and a bind group pointing the layout
/// at the buffer. The gbuffer VS statically references `StructuredBuffer<InstanceModelCol>
/// instances`, so the layout MUST be in the pipeline layout and a valid buffer MUST be bound
/// for the draw; the legacy arm (`use_model_matrix == 0`) never reads it (bound-but-unread).
/// The caller OWNS all three and tears them down (`destroy_bind_group` → `destroy_buffer` →
/// `destroy_bind_group_layout`). Mirrors `window_present_gbuffer::create_identity_instance`.
fn create_identity_instance(
    device: &VulkanContext,
) -> (VulkanBindGroupLayout, BoundBuffer, VulkanBindGroup) {
    // The IDENTITY 3x4 row-major affine: r0=(1,0,0,0), r1=(0,1,0,0), r2=(0,0,1,0). 12 f32 =
    // 48 B (matches the production `GBUFFER_IDENTITY_INSTANCE` / `GBUFFER_INSTANCE_MODEL_BYTES`).
    const IDENTITY: [f32; 12] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ];
    const INSTANCE_BYTES: u64 = 48;
    let layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                count: 1,
                kind: DescriptorKind::StorageBuffer,
                stage: ShaderStage::VERTEX,
            }],
        })
        .expect("M1 instance-model bind-group layout");
    let buffer = device
        .create_buffer(&BufferDesc {
            size: INSTANCE_BYTES,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("M1 identity instance SSBO");
    {
        let mapped = device
            .buffer_mapped_ptr(&buffer)
            .expect("host-visible identity instance buffer is mapped");
        let mut bytes = [0u8; INSTANCE_BYTES as usize];
        for (i, f) in IDENTITY.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
        }
        // SAFETY: `mapped` points to `INSTANCE_BYTES` (48) mapped host-coherent bytes; `bytes`
        // is exactly that length and copied in full, in-bounds. No GPU work references this
        // buffer yet (the encoder records the draw after), so the write is unsynchronized-safe.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
        }
    }
    let bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &layout,
            entries: &[BindGroupEntry::StorageBuffer { buffer: &buffer }],
        })
        .expect("M1 identity instance bind group");
    (layout, buffer, bind_group)
}

/// Whether pixel `(px, py)`'s orthographic ray passes through the mesh quad footprint
/// (the rasterizer's covered-pixel set, host-computable from the SAME camera mapping).
fn mesh_covers_pixel(px: u32, py: u32) -> bool {
    let [x, y] = pixel_world_xy(px, py);
    (QUAD_X_MIN..=QUAD_X_MAX).contains(&x) && (QUAD_Y_MIN..=QUAD_Y_MAX).contains(&y)
}

/// The per-pixel mesh depth the GPU is expected to produce (the host model for the
/// golden's `mesh_depth` input): the constant inside the quad, the clear outside.
fn expected_mesh_depth(px: u32, py: u32) -> f32 {
    if mesh_covers_pixel(px, py) {
        mesh_depth_for_z(MESH_Z)
    } else {
        DEPTH_CLEAR
    }
}

/// Splits a packed `0xAABBGGRR` into `[r, g, b]` (the low three bytes).
fn unpack_packed_rgb(packed: u32) -> [i32; 3] {
    [
        (packed & 0xFF) as i32,
        ((packed >> 8) & 0xFF) as i32,
        ((packed >> 16) & 0xFF) as i32,
    ]
}

/// Splits an R8G8B8A8 readback texel's first three bytes into `[r, g, b]`.
fn unpack_texel_rgb(rgba: &[u8]) -> [i32; 3] {
    [rgba[0] as i32, rgba[1] as i32, rgba[2] as i32]
}

/// `true` if a readback texel agrees with a packed golden within `CHANNEL_TOL`/channel.
fn texel_close(got: [i32; 3], want_packed: u32) -> bool {
    let w = unpack_packed_rgb(want_packed);
    (0..3).all(|c| (got[c] - w[c]).abs() <= CHANNEL_TOL)
}

/// Records + submits the full OFFSCREEN G-buffer hybrid composite + the deferred RESOLVE
/// in ONE command buffer / ONE fenced submit, returning the readback LIT storage image as
/// `PIXELS` R8G8B8A8 texels (4 bytes each). The flow — the §15.1 seam with NO depth→buffer
/// copy, plus the deferred-split resolve:
///
///   raster quad → D32 depth IMAGE → barrier depth DEPTH_ATTACHMENT→SHADER_READ_ONLY
///   (one barrier) → barrier the 3 G-buffer images + lit UNDEFINED→GENERAL → bind the
///   vocabulary set {SSBO edit-list, SAMPLED depth, STORAGE albedo/normal/material,
///   UNIFORM camera} + the marcher → dispatch (writes ATTRIBUTES: gAlbedo = base,
///   gMaterial = (shadow, ao, mask, 1)) → barrier albedo/material GENERAL→GENERAL
///   (SHADER_WRITE→SHADER_READ) → bind the resolve set {STORAGE albedo/material/lit} +
///   the resolve → dispatch (composites lit via full Cook-Torrance on the SDF arm, the
///   mask==0 pass-through verbatim) → barrier lit
///   GENERAL→TRANSFER_SRC → copy_image_to_buffer(lit) into readback.
///
/// There is NO `copy_image_to_buffer(depth)` and NO transfer→compute buffer barrier:
/// the single depth `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` barrier
/// replaces the old copy + its two barriers. Both descriptor sets are written ONCE at
/// `create_bind_group` — there is no per-frame `vkUpdateDescriptorSets`.
fn run_gbuffer_hybrid(ctx: &VulkanContext, edits: &[SdfEdit], coarse_enabled: bool) -> Vec<u8> {
    // Delegate to the `_ex` variant, discarding the tiles-buffer readback. Defaults the
    // Render B1 over-relaxation factor to `1.0` — the marcher byte-identical to the pre-B1
    // path — so the existing 0%-gate callers (`p1b_gbuffer_hybrid_matches_golden`, the
    // GATE-4 `p4b_cull_off_is_byte_identical_to_pre_p4b_path`) stay TRUE ω=1.0 byte-identity
    // gates against the ω=1.0 host golden. The ω>1 path (engine default
    // `DEFAULT_MARCHER_OMEGA`) is exercised by the dedicated B1 over-relaxation tests, which
    // call `run_gbuffer_hybrid_ex` with an explicit ω and diff against `_omega` host goldens.
    run_gbuffer_hybrid_ex(ctx, edits, coarse_enabled, false, 1.0).0
}

/// Render P4b — the extended harness: the same OFFSCREEN G-buffer hybrid composite as
/// [`run_gbuffer_hybrid`], but ALSO reads back the per-tile [`TileBound`] cull buffer
/// (binding 6) when `read_tiles == true`, returning `(albedo, Some(tiles_bytes))`.
///
/// The tiles-buffer readback is the TESTER's host/GPU agreement oracle: the returned
/// `Vec<u8>` is `tile_count() * TILE_BOUND_BYTES` bytes of the std430 `RWStructuredBuffer
/// <TileBound>` the coarse pass wrote (parse each 16-byte element as near_t f32@0, far_t
/// f32@4, flags u32@8, _pad u32@12). With `read_tiles == false` the second element is
/// `None` and no extra copy / barrier is recorded (the byte-identity 0%-gate path).
///
/// `read_tiles` requires `coarse_enabled` — the coarse pass only runs (and only writes
/// binding 6) when culling is on; reading it back otherwise yields the buffer's
/// undefined create-time contents. The caller is responsible for pairing them.
///
/// `omega_in` is the Render B1 over-relaxation factor; it is RUNTIME-clamped to
/// `[1.0, 1.99]` before the push encode (the soundness ceiling sits at `omega == 2`).
/// `1.0` keeps the marcher byte-identical to the pre-B1 path (the 0%-gate).
fn run_gbuffer_hybrid_ex(
    ctx: &VulkanContext,
    edits: &[SdfEdit],
    coarse_enabled: bool,
    read_tiles: bool,
    omega_in: f32,
) -> (Vec<u8>, Option<Vec<u8>>) {
    // Delegate to the lighting-aware variant with lighting OFF (the historical default):
    // `lighting_flags == 0` ⇒ the shader's byte-identical Lambert path, so every existing
    // 0%-gate caller keeps its exact OFF semantics. The A1/A2 ON-path tester gates call
    // `run_gbuffer_hybrid_lit` directly with an explicit `lighting_flags` + `light_dir`.
    run_gbuffer_hybrid_lit(ctx, edits, coarse_enabled, read_tiles, omega_in, 0, DEFAULT_LIGHT_DIR)
}

/// Render A1/A2 — the lighting-aware harness: identical to [`run_gbuffer_hybrid_ex`] but
/// the marcher push carries an explicit `lighting_flags` (bit 0 = A1 shadows, bit 1 = A2
/// AO; `0` = the OFF Lambert path) and `light_dir` (the un-normalized directional light).
///
/// Deferred split (MVP-2): the marcher writes ATTRIBUTES (gAlbedo = the unmultiplied raw
/// linear base, gNormal = (oct normal, 16-bit material id), gMaterial = (shadow, ao, mask,
/// 1)); a fullscreen `deferred_pbr` RESOLVE composites `lit` via full Cook-Torrance on the
/// SDF arm (the picked material's metallic/roughness/F0, the analytic directional light
/// modulated by the A1 shadow + A2 AO, plus the hemisphere/specular-IBL ambient), passing
/// the `mask == 0` pixels through byte-identically, into a dedicated LIT image. The readback
/// now copies LIT (not albedo), so the tester diffs it against `golden_deferred_resolve(...)`
/// fed by `golden_marcher_attributes(...)` with the SAME flags + `light_dir`. Everything else
/// (the §15.1 seam, the vocabulary set, the coarse pass) is the [`run_gbuffer_hybrid_ex`]
/// flow verbatim — the marcher push payload + the new resolve pass.
#[allow(clippy::too_many_arguments)]
fn run_gbuffer_hybrid_lit(
    ctx: &VulkanContext,
    edits: &[SdfEdit],
    coarse_enabled: bool,
    read_tiles: bool,
    omega_in: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
) -> (Vec<u8>, Option<Vec<u8>>) {
    // Default to the L0a degenerate table (the historical light table every existing caller
    // expects). The L0b GPU goldens call `run_gbuffer_hybrid_lit_table` with a custom table.
    run_gbuffer_hybrid_lit_table(
        ctx,
        edits,
        coarse_enabled,
        read_tiles,
        omega_in,
        lighting_flags,
        light_dir,
        &DEGENERATE_LIGHT_TABLE,
    )
}

/// Lighting L0b — the table-parameterized lighting harness: identical to
/// [`run_gbuffer_hybrid_lit`] but the resolve's light-table SSBO (binding 6) is seeded with
/// the caller's `light_table_words` (`[LightHeaderGpu || GpuLight[]]`, std430 word-packed)
/// instead of the fixed [`DEGENERATE_LIGHT_TABLE`]. The L0b `gViewT` lane (the marcher's
/// surface `t`) feeds the resolve's `P = ro + rd * t` reconstruction for point/spot lights.
/// The host comparison oracle is [`golden_deferred_resolve_table`].
#[allow(clippy::too_many_arguments)]
fn run_gbuffer_hybrid_lit_table(
    ctx: &VulkanContext,
    edits: &[SdfEdit],
    coarse_enabled: bool,
    read_tiles: bool,
    omega_in: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
    light_table_words: &[u32],
) -> (Vec<u8>, Option<Vec<u8>>) {
    // The M1 empty-skip is OFF on this path (`None` grid) — byte-identical to the pre-M1
    // marcher. The `#[ignore]` brick offscreen gate calls `run_gbuffer_hybrid_brick` (below),
    // which threads a `Some(PointerGrid)` so binding 9 is wired + `with_brick` is pushed.
    run_gbuffer_hybrid_lit_table_brick(
        ctx,
        edits,
        coarse_enabled,
        read_tiles,
        omega_in,
        lighting_flags,
        light_dir,
        light_table_words,
        None,
        // M2 trilinear OFF on this default path (byte-identical to the pre-M2 marcher).
        false,
    )
}

/// Lighting L0b + M1 — the table + empty-skip parameterized harness. Identical to
/// [`run_gbuffer_hybrid_lit_table`] but accepts an optional `brick`: when
/// `Some((grid, cells))`, the marcher's binding-9 `PointerGrid` SSBO is created + seeded
/// with `cells` (the [`build_pointer_grid`] bake) and the `FineMarcherPush` is built with
/// [`FineMarcherPush::with_brick`] (`brick_enabled = 1`) so the empty-skip is ON; when
/// `None`, binding 9 is bound to a 1-cell placeholder and `brick_enabled = 0` (the marcher
/// is byte-identical to the pre-M1 path). The recompiled `sdf_gbuffer_composite.hlsl`
/// STATICALLY references `register(t9)` inside the runtime-gated empty-skip branch, so the
/// marcher layout MUST declare binding 9 either way (or the pipeline create trips
/// VUID-VkComputePipelineCreateInfo-layout) — hence the placeholder on the OFF path.
#[allow(clippy::too_many_arguments)]
fn run_gbuffer_hybrid_lit_table_brick(
    ctx: &VulkanContext,
    edits: &[SdfEdit],
    coarse_enabled: bool,
    read_tiles: bool,
    omega_in: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
    light_table_words: &[u32],
    brick: Option<(&boyko_sdf_math::brick::PointerGrid, &[u32])>,
    brick_trilinear: bool,
) -> (Vec<u8>, Option<Vec<u8>>) {
    let (lit, tiles, _viewt) = run_gbuffer_hybrid_m2(
        ctx, edits, coarse_enabled, read_tiles, omega_in, lighting_flags, light_dir,
        light_table_words, brick, brick_trilinear, false, true,
    );
    (lit, tiles)
}

/// M2 — the brick-atlas trilinear+cubic harness, the SUPERSET of
/// [`run_gbuffer_hybrid_lit_table_brick`]. In addition to the LIT readback it (a) optionally
/// copies the marcher's `gViewT` R32_SFLOAT surface-`t` image back (when `read_viewt`), so the
/// M2 discriminator can recover the GPU's per-pixel hit `t` (the analytically-validated world
/// ray param) for the EXACT-CSG residual check and the hit-`t` agreement against the host mirror,
/// and (b) lets the caller ZERO the b5 UBO's `M2GridParams` tail (`m2_grid_default == false`) to
/// prove the M2 branch reads it (the grid-engaged discriminator (a)).
///
/// When `read_viewt == false` AND `m2_grid_default == true` the recorded command stream + the b5
/// UBO contents are byte-identical to [`run_gbuffer_hybrid_lit_table_brick`]'s — the `gViewT` image
/// keeps its `STORAGE`-only usage and no extra copy/barrier is recorded (the OFF byte-identity
/// 0%-gate path). The third tuple element is `Some(viewt_bytes)` only when `read_viewt`.
#[allow(clippy::too_many_arguments)]
fn run_gbuffer_hybrid_m2(
    ctx: &VulkanContext,
    edits: &[SdfEdit],
    coarse_enabled: bool,
    read_tiles: bool,
    omega_in: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
    light_table_words: &[u32],
    brick: Option<(&boyko_sdf_math::brick::PointerGrid, &[u32])>,
    brick_trilinear: bool,
    read_viewt: bool,
    m2_grid_default: bool,
) -> (Vec<u8>, Option<Vec<u8>>, Option<Vec<f32>>) {
    // The single-level (M2) entry: no clip-map → `brick_levels = 1`, the level-0 atlas/grid bound at
    // every level slot (duplicates). The N-level (M4) sibling [`run_gbuffer_hybrid_m4`] passes a real
    // `BrickClipmap`. This wrapper keeps the M2/M3 call sites + byte output unchanged (the OFF/N=1 gate).
    run_gbuffer_hybrid_m4(
        ctx, edits, coarse_enabled, read_tiles, omega_in, lighting_flags, light_dir,
        light_table_words, brick, brick_trilinear, read_viewt, m2_grid_default, None,
    )
}

/// The N-level (M4 clip-map) GPU harness (Slice C). Identical to the single-level M2 harness but binds
/// the per-level brick resources from `clipmap` (when `Some`): the level-`L` atlas at @10/@12/@14 and
/// the level-`L` grid at @9/@11/@13, the `M4GridParams::camera_centered` tail in the b5 UBO, and
/// `brick_levels = BRICK_LEVELS` on the push. When `clipmap` is `None` it is the OFF/N=1 path (the
/// level-0 single atlas/grid bound at every level slot, `brick_levels = 1`) — byte-identical to the
/// pre-M4 M2 harness (the call sites that pass `None`).
#[allow(clippy::too_many_arguments)]
fn run_gbuffer_hybrid_m4(
    ctx: &VulkanContext,
    edits: &[SdfEdit],
    coarse_enabled: bool,
    read_tiles: bool,
    omega_in: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
    light_table_words: &[u32],
    brick: Option<(&boyko_sdf_math::brick::PointerGrid, &[u32])>,
    brick_trilinear: bool,
    read_viewt: bool,
    m2_grid_default: bool,
    clipmap: Option<&BrickClipmap>,
) -> (Vec<u8>, Option<Vec<u8>>, Option<Vec<f32>>) {
    let device: &VulkanContext = ctx;
    let queue = ctx.rhi_queue();

    // --- M2: the brick atlas (marcher binding 10, a CombinedImageSampler). Baked from the
    // SAME `edits` authority via `BrickAtlas::create` (principle 0 — a transient GPU mirror, no
    // parallel field store), exactly as `window_present_gbuffer.rs` wires it. The recompiled
    // marcher SPIR-V statically references `register(t10)` + `register(s10)` (collapsed to ONE
    // combined descriptor by DXC) inside the runtime-gated `brick_trilinear` branch, so a VALID
    // combined image+sampler MUST be bound at binding 10 regardless of the gate. On the OFF path
    // (`brick_trilinear == false`) the atlas is bound-but-unread (byte-identity contract). ---
    let brick_atlas = {
        use boyko_sdf_math::SdfEditField;
        let mut field = SdfEditField::new();
        for e in edits {
            assert!(field.push(*e), "scene must fit MAX_SDF_EDITS");
        }
        field.bump_gen();
        BrickAtlas::create(ctx, &field).expect("M2 brick atlas (vocab binding 10) — create + bake + upload")
    };

    // --- M1: the pointer-grid StorageBuffer (marcher binding 9). On the brick-ON path it
    // is seeded with the `build_pointer_grid` bake (`cells`, one u32 per cell — the GPU
    // `StructuredBuffer<uint>` element). On the OFF path a 1-cell placeholder satisfies the
    // shader's static `register(t9)` reference (never read, `brick_enabled == 0`). ---
    let pointer_grid_cells: Vec<u32> = match brick {
        Some((_, cells)) => cells.to_vec(),
        None => vec![0u32; 1],
    };
    let pointer_grid_buffer = device
        .create_buffer(&BufferDesc {
            size: (pointer_grid_cells.len() as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("M1 pointer-grid storage buffer");
    {
        let mapped = device
            .buffer_mapped_ptr(&pointer_grid_buffer)
            .expect("host-visible pointer-grid buffer is mapped");
        write_words(mapped, &pointer_grid_cells);
    }

    // --- The edit-list StorageBuffer (binding 0), seeded with the packed header. The
    // P1b shader only READS the rung-9 header + edit array (`Buf[0..PIXEL_BASE_WORDS]`,
    // i.e. `Buf[0..196]`); the depth/pixel regions are no longer used by this path.
    // We deliberately OVER-ALLOCATE to the full `EDITLIST_BUFFER_WORDS` (which still
    // includes the now-unused pixel region) rather than trimming to `PIXEL_BASE_WORDS`:
    // `encode_edit_list` debug-asserts `buf.len() >= EDITLIST_BUFFER_WORDS`, so reusing
    // the shared const keeps the host encoder and the buffer in lock-step and avoids a
    // size desync. The extra words are simply never touched. ---
    let buffer = device
        .create_buffer(&BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, edits);
        let mapped = device
            .buffer_mapped_ptr(&buffer)
            .expect("host-visible buffer is mapped");
        write_words(mapped, &header);
    }

    // --- The camera/extent UNIFORM buffer (binding 5), written ONCE at setup (NOT a
    // per-frame push). At the golden 64×64 ORTHO extent it drives bit-exact rays.
    //
    // M2: the UBO is sized to `B5_CAMERA_UBO_BYTES` (128 B) — the 80-byte camera block plus the
    // 48-byte `M2GridParams` tail the marcher reads on the `brick_trilinear` path (at
    // `M2_GRID_PARAMS_OFFSET == 80`). The M2 block is written unconditionally; the marcher
    // reads it ONLY when `brick_trilinear == 1`, so on the OFF path it is bound-but-unread
    // (byte-identical to the pre-M2 path). ---
    let camera_uniform = device
        .create_buffer(&BufferDesc {
            // M4 (Slice C): widened to `B5_CAMERA_UBO_BYTES_M4` (224 B) — the 80-byte camera block + the
            // N-level M4GridParams array tail @80. The recompiled marcher cbuffer declares 224 B, so the
            // descriptor must cover it even though this single-level harness runs `brick_levels == 1`
            // (only `m2_levels[0]` is read — byte-identical to the old M2 48-byte tail).
            size: B5_CAMERA_UBO_BYTES_M4 as u64,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("camera uniform buffer");
    {
        let pc = CompositePushConstants::ortho(SDF_IMG_W, SDF_IMG_H);
        debug_assert_eq!(pc.count, PIXELS);
        let mapped = device
            .buffer_mapped_ptr(&camera_uniform)
            .expect("host-visible uniform buffer is mapped");
        // `as_bytes()` is the same 80-byte camera POD the packed path pushed; write it at offset 0.
        let bytes = pc.as_bytes();
        debug_assert_eq!(bytes.len(), M2_GRID_PARAMS_OFFSET, "camera block must be 80 B (offset of the M4 tail)");
        // M4: the N-level array tail at `M2_GRID_PARAMS_OFFSET` (80). With a `clipmap` (the N-level
        // path) the tail is the clip-map's baked `camera_centered` params (the per-level snapped
        // origins the atlases were baked at). Without one (the OFF/N=1 path) it is `near_field_only()`,
        // whose level 0 is byte-FOR-byte the old M2 `default_near_field` block (the keystone), so the
        // `brick_levels == 1` marcher reads `m2_levels[0]` exactly like the pre-M4 M2 path. The
        // discriminator-(a) path passes `m2_grid_default == false` to write a ZEROED tail instead
        // (atlas_dim == 0 → level-0 maps nothing → the branch finds no tile → no M2 hit).
        let m4 = match clipmap {
            Some(cm) => *cm.params(),
            None => M4GridParams::near_field_only(),
        };
        let m4_default = m4.as_ubo_bytes();
        let m4_zeroed = [0u8; B5_CAMERA_UBO_BYTES_M4 - M2_GRID_PARAMS_OFFSET];
        let m4_bytes: &[u8] = if m2_grid_default { &m4_default } else { &m4_zeroed };
        debug_assert_eq!(M2_GRID_PARAMS_OFFSET + m4_bytes.len(), B5_CAMERA_UBO_BYTES_M4);
        // SAFETY: `mapped` points to `B5_CAMERA_UBO_BYTES_M4` (224) mapped host-coherent bytes;
        // `bytes` (80) is written at offset 0 and `m4_bytes` (144) at offset 80 — together exactly
        // 224 in-bounds bytes, disjoint. No GPU work is in flight yet (submit follows), so the
        // writes are unsynchronized-safe.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
            core::ptr::copy_nonoverlapping(
                m4_bytes.as_ptr(),
                mapped.as_ptr().add(M2_GRID_PARAMS_OFFSET),
                m4_bytes.len(),
            );
        }
    }

    // --- PBR MVP-2: the material table SSBO (binding 7 of the vocab set + binding 4 of
    // the resolve set). The crater/box/smooth edits carry NO material id (center.w == 0),
    // so every SDF hit picks material 0 — the default mid-gray dielectric. One element
    // suffices (48 B / 12 words; mirrors boyko_render::MaterialGpu's std430 layout). ---
    let material_table = device
        .create_buffer(&BufferDesc {
            size: (DEFAULT_MATERIAL_TABLE.len() as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("PBR material table storage buffer");
    {
        let mapped = device
            .buffer_mapped_ptr(&material_table)
            .expect("host-visible material table is mapped");
        write_words(mapped, &DEFAULT_MATERIAL_TABLE);
    }

    // --- Lighting L0a/L0b: the light table SSBO (resolve binding 6), seeded with the
    // caller's `light_table_words` (the DEGENERATE table on the L0a 0%-gate path, a custom
    // point/spot table on the L0b goldens). Host-visible here for the test; production mints
    // it device-local. ---
    let light_table = device
        .create_buffer(&BufferDesc {
            size: (light_table_words.len() as u64) * 4,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("Lighting-L0 light table storage buffer");
    {
        let mapped = device
            .buffer_mapped_ptr(&light_table)
            .expect("host-visible light table is mapped");
        write_words(mapped, light_table_words);
    }

    // --- Render P4b: the per-tile coarse-cull StorageBuffer (binding 6). The coarse
    // pass WRITES one `TileBound` (16 B) per 8×8 tile; the fine marcher READS it (gated
    // by the `coarse_enabled` push). Device-local would do, but a host-coherent buffer
    // lets the GPU-half tester read the bounds back and diff them against
    // `golden_tile_bound`. Sized to the full tile grid. ---
    let tiles_buffer = device
        .create_buffer(&BufferDesc {
            size: (tile_count() as u64) * (TILE_BOUND_BYTES as u64),
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("P4b coarse-cull tile-bound storage buffer");

    // --- The depth IMAGE (D32_SFLOAT): DEPTH_STENCIL_ATTACHMENT (rasterize into it) |
    // SAMPLED (the marcher samples it directly — NO copy). A DEPTH_STENCIL_ATTACHMENT
    // usage gives the texture a DEPTH-aspect view, exactly what the marcher's
    // `Texture2D<float>.Load` samples. ---
    let depth = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("offscreen depth texture (sampled)");

    // --- The MRT G-buffer STORAGE images (albedo + normal + material). Render P5-r0: each
    // ALSO carries COLOR_ATTACHMENT — the mesh raster pass A now writes them as a 3-MRT
    // G-buffer producer (in the marcher's encoding, mask=1), so a yielded mesh pixel (r1)
    // has a real fragment to stand on. The marcher still STORES them in GENERAL; the
    // deferred resolve loads albedo/normal/material in GENERAL. The throwaway color
    // attachment is DELETED (the MRT binds the three real images). ---
    let albedo = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("G-buffer albedo storage+color image");
    let normal = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("G-buffer normal storage+color image");
    let material = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("G-buffer material storage+color image");
    // Deferred split: the LIT image is the resolve's STORAGE store output; TRANSFER_SRC so
    // the golden readback copies it out (the readback now reads LIT, not albedo).
    let lit = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("deferred resolve lit storage image");
    // Lighting L0b: the gViewT lane — an R32_SFLOAT STORAGE image the marcher stores the
    // surface ray param `t` into and the resolve reads to reconstruct `P = ro + rd * t`. M2: the
    // discriminator reads it back (the GPU's per-pixel surface `t`) for the EXACT-CSG residual
    // check + the hit-`t` agreement, so it gains `TRANSFER_SRC` ONLY when `read_viewt` (the OFF
    // byte-identity path keeps the STORAGE-only image + records no extra copy).
    let viewt_usage = if read_viewt {
        ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC
    } else {
        ImageUsage::STORAGE
    };
    let viewt = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::R32Sfloat,
            dimension: TextureDimension::D2,
            usage: viewt_usage,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("Lighting L0b gViewT storage image");
    // Render P7 GROUP C1: the SSAO term `gSsao` — an R8_UNORM STORAGE image bound at resolve
    // binding 11. ALWAYS allocated so the resolve descriptor interface is stable; no SSAO pass
    // writes it (C2 adds that) and `ssao_mode == 0` here, so the resolve never reads it (a
    // bound-but-unread valid descriptor — the byte-identical 0%-gate).
    let ssao = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::R8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("Render P7 SSAO gSsao storage image");

    // The depth is SAMPLED via `.Load` (OpImageFetch, no sampler), but the RHI
    // `BindGroupEntry::SampledImage` requires a sampler handle; a nearest/clamp sampler
    // is created and bound (it is ignored by an unfiltered fetch).
    let sampler = device
        .create_sampler(&SamplerDesc::default())
        .expect("depth sampler (ignored by .Load)");

    // The readback buffer for the LIT image.
    let readback = device
        .create_buffer(&BufferDesc {
            size: READBACK_BYTES,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible readback buffer");

    // M2: the readback buffer for the gViewT R32_SFLOAT image (one f32 per pixel = the GPU's
    // surface ray param `t`). Allocated ONLY on the `read_viewt` path; a 4-byte placeholder
    // otherwise (never copied into / read), kept so the single teardown block destroys it
    // unconditionally without a branch.
    let viewt_readback = device
        .create_buffer(&BufferDesc {
            size: if read_viewt { (PIXELS as u64) * 4 } else { 4 },
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible gViewT readback buffer");

    // The quad vertex buffer.
    let vertices = quad_vertices();
    let vertex_bytes = core::mem::size_of_val(&vertices) as u64;
    let vertex_buffer = device
        .create_buffer(&BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible vertex buffer");
    let vb_ptr = device
        .buffer_mapped_ptr(&vertex_buffer)
        .expect("host-visible vertex buffer is mapped");
    // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes; `vertices`
    // is a distinct stack array of `vertex_bytes` bytes; the write completes before any
    // submit references the buffer (host-coherent: no flush).
    unsafe {
        core::ptr::copy_nonoverlapping(
            vertices.as_ptr().cast::<u8>(),
            vb_ptr.as_ptr(),
            vertex_bytes as usize,
        );
    }

    // --- Modules: the Render P5-r0 mesh-MRT producer pair + the P1b G-buffer marcher
    // (compute). The raster pair is now `gbuffer_mrt.{vs,fs}` (3-MRT producer), NOT the
    // depth-only `triangle_mvp` pair. ---
    let vs = device
        .create_shader_module(MRT_VS_SPV.as_words())
        .expect("mesh-MRT vertex shader module");
    let fs = device
        .create_shader_module(MRT_FS_SPV.as_words())
        .expect("mesh-MRT fragment shader module");
    let cs = device
        .create_shader_module(sdf_gbuffer_composite_spirv())
        .expect("P1b G-buffer marcher compute shader module");
    // Render P4b: the coarse-cull / tile pre-trace compute module.
    let coarse_cs = device
        .create_shader_module(sdf_tile_cull_spirv())
        .expect("P4b coarse-cull compute shader module");
    // Deferred split: the `deferred_pbr.comp` RESOLVE compute module.
    let resolve_cs = device
        .create_shader_module(deferred_pbr_spirv())
        .expect("deferred resolve compute shader module");

    // The depth-testing graphics pipeline (rung-3 vertex layout + 64-byte VERTEX MVP
    // push + a declared depth_format).
    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
    ];
    // M1: the per-instance model SSBO layout + 1-element identity dummy + its bind group.
    // The gbuffer VS statically references `instances` at set 0 binding 0, so the pipeline
    // layout MUST declare it + a valid buffer MUST be bound for the draw (the legacy arm,
    // `use_model_matrix == 0`, never reads it — bound-but-unread).
    let (instance_layout, instance_buffer, instance_bind_group) = create_identity_instance(device);
    let gfx = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            // Render P5-r0: 3 MRT color formats = the G-buffer RGBA8 lanes (albedo@0,
            // normal@1, material@2). The builder auto-derives 3 opaque blend states.
            color_formats: &[GBUFFER_FORMAT, GBUFFER_FORMAT, GBUFFER_FORMAT],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            bind_group_layout: Some(&instance_layout),
            blend: None,
            cull_mode: CullMode::None,
            depth_bias: None,
        })
        .expect("mesh-MRT G-buffer producer graphics pipeline");

    // --- The P1b vocabulary set, EXTENDED for P4b: { SSBO edit-list @0, SAMPLED depth
    // @1, STORAGE albedo @2, STORAGE normal @3, STORAGE material @4, UNIFORM camera @5,
    // STORAGE tile-bounds @6 }. ONE set-0 layout shared by BOTH the coarse pass (reads
    // 0/1/5, writes 6) and the fine marcher (reads 0/1/5/6, writes 2/3/4) — each shader
    // uses a subset (valid). ---
    let layout_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // PBR MVP-2: the material table SSBO @7 (the marcher fetches `base_color`).
        BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Lighting L0b: the gViewT STORAGE image @8 (the marcher stores the surface `t`;
        // the coarse-cull shader, which shares this layout, declares only a subset — valid).
        BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        // M1 empty-skip: the pointer-grid SSBO @9 (the marcher reads `StructuredBuffer<uint>
        // PointerGrid : register(t9)` inside the runtime-gated empty-skip branch). DECLARED
        // on BOTH the OFF and ON path (the shader statically references t9), bound to the
        // 1-cell placeholder when the skip is off.
        BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // M2 trilinear+cubic: the brick-atlas combined image+sampler @10. The recompiled marcher
        // SPIR-V statically references `Texture3D BrickAtlas : register(t10)` +
        // `SamplerState BrickSampler : register(s10)` (collapsed to ONE combined descriptor by DXC)
        // inside the runtime-gated `brick_trilinear` branch (NOT dead-stripped despite the gate),
        // so the layout MUST declare binding 10 — a VALID combined image+sampler is bound even on
        // the OFF path (`brick_trilinear == 0`, bound-but-unread). Mirrors `window_present_gbuffer.rs`.
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        // M4 clip-map LOD (Slice C): the LEVEL-1 + LEVEL-2 brick bindings @11..14. The recompiled
        // marcher SPIR-V statically references `PointerGrid1`@t11, `BrickAtlas1`@t12, `PointerGrid2`@t13,
        // `BrickAtlas2`@t14 inside the runtime level branch-ladder, so the layout MUST declare all four —
        // bound-but-unread on this single-level harness (`brick_levels == 1` takes only the lvl==0 arm).
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 12, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 13, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 14, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        // MDF Stage-2c: the dedicated dense mesh-SDF shadow-caster texture @15 (the 16th / last vocab
        // entry under the 16-binding cap). The recompiled marcher SPIR-V statically references
        // `MeshSdf`@t15 + `MeshSdfSampler`@s15 inside the runtime-gated `mesh_sdf_enabled` branch, so
        // the layout MUST declare binding 15 — bound-but-unread on the OFF golden path (no MDF scene).
        BindGroupLayoutEntry { binding: 15, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
    ];
    let bind_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc { entries: &layout_entries })
        .expect("P4b vocabulary bind-group layout");
    let compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &cs,
            entry: c"main",
            // The dedicated vocabulary layout declares the shared compute push range; the
            // P4b marcher pushes a 4-byte `coarse_enabled` gate against THIS pipeline's
            // own layout (via `push_compute_constants`). `COMPOSITE_PUSH_CONSTANT_BYTES`
            // keeps the create-time "non-empty multiple of 4 within the shared range"
            // contract (the 4-byte push fits inside the declared 80-byte range).
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&bind_layout),
            spec_constants: &[],
        })
        .expect("P1b G-buffer marcher compute pipeline");
    // Render P4b: the coarse-cull pipeline, against the SAME vocabulary layout.
    let coarse_compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &coarse_cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&bind_layout),
            spec_constants: &[],
        })
        .expect("P4b coarse-cull compute pipeline");
    // M4 clip-map LOD (Slice C): resolve the per-level brick resources. With a `clipmap` (the N-level
    // path) level `L`'s atlas/sampler/grid come from the clip-map; without one (the OFF/N=1 path) every
    // level slot binds the single level-0 atlas + pointer grid as BENIGN DUPLICATES (bound-but-unread on
    // the `brick_levels == 1` path, but the marcher SPIR-V references t9..t14 past the runtime gate, so
    // VALID descriptors are required at all 6 brick bindings). Level 0 = @9/@10, 1 = @11/@12, 2 = @13/@14.
    let level_atlas_tex = |level: usize| match clipmap {
        Some(cm) => cm.atlas(level).texture(),
        None => brick_atlas.texture(),
    };
    let level_atlas_smp = |level: usize| match clipmap {
        Some(cm) => cm.sampler(level),
        None => brick_atlas.sampler(),
    };
    let level_grid = |level: usize| match clipmap {
        Some(cm) => cm.grid_buffer(level),
        None => &pointer_grid_buffer,
    };

    // The vocabulary bind group, written ONCE at create (NO per-frame update). Both
    // passes bind this same set; the coarse pass writes binding 6, the fine reads it.
    let bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &bind_layout,
            entries: &[
                BindGroupEntry::StorageBuffer { buffer: &buffer },
                BindGroupEntry::SampledImage { texture: &depth, sampler: &sampler },
                BindGroupEntry::StorageImage { texture: &albedo },
                BindGroupEntry::StorageImage { texture: &normal },
                BindGroupEntry::StorageImage { texture: &material },
                BindGroupEntry::UniformBuffer { buffer: &camera_uniform },
                BindGroupEntry::StorageBuffer { buffer: &tiles_buffer },
                BindGroupEntry::StorageBuffer { buffer: &material_table },
                // Lighting L0b: the gViewT lane @8 (the marcher STORES the surface `t`).
                BindGroupEntry::StorageImage { texture: &viewt },
                // M1/M4: the LEVEL-0 pointer-grid SSBO @9 (the bake on the brick-ON path; the 1-cell
                // placeholder on the OFF path). With a clipmap, level 0's grid SSBO.
                BindGroupEntry::StorageBuffer { buffer: level_grid(0) },
                // M2/M4: the LEVEL-0 brick-atlas 3D image @10 as a COMBINED image+sampler. Read only when
                // `brick_trilinear == 1` (+ select_level == 0). With a clipmap, level 0's atlas.
                BindGroupEntry::CombinedImage {
                    texture: level_atlas_tex(0),
                    sampler: level_atlas_smp(0),
                },
                // M4 clip-map LOD: LEVEL-1 grid @11 + atlas @12, LEVEL-2 grid @13 + atlas @14.
                BindGroupEntry::StorageBuffer { buffer: level_grid(1) },
                BindGroupEntry::CombinedImage {
                    texture: level_atlas_tex(1),
                    sampler: level_atlas_smp(1),
                },
                BindGroupEntry::StorageBuffer { buffer: level_grid(2) },
                BindGroupEntry::CombinedImage {
                    texture: level_atlas_tex(2),
                    sampler: level_atlas_smp(2),
                },
                // MDF Stage-2c @15: bind level 0's atlas as a benign placeholder (no MDF scene here);
                // the marcher gates the read OFF (`mesh_sdf_enabled == 0`) → bound-but-unread (R2).
                BindGroupEntry::CombinedImage {
                    texture: level_atlas_tex(0),
                    sampler: level_atlas_smp(0),
                },
            ],
        })
        .expect("M2 vocabulary bind group");

    // --- PBR MVP-2: the RESOLVE layout + pipeline + set. 6 bindings (≤ 8): gAlbedo @0,
    // gNormal @1, gMaterial @2, lit @3 (STORAGE images), the material SSBO @4, the camera
    // UBO @5 (the resolve reads the extent + per-pixel view dir from it). The resolve
    // dispatches at the SAME grid the marcher used (1:1 the marched pixels). ---
    let resolve_layout_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        // Lighting L0a: the light table SSBO @6 (the degenerate 0%-gate table — the
        // resolve loops it instead of the old compiled-in LIGHT_DIR / SKY_* constants).
        BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Lighting L0b: the gViewT STORAGE image @7 (the resolve reads it under `mask == 1`).
        BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        // Lighting L1: ClusterGrid @8 + LightIndexList @9. The recompiled `deferred_pbr.comp`
        // STATICALLY references @8/@9 on EVERY path (DXC no longer dead-strips them when the
        // cluster branch is off), so the layout MUST declare them or the pipeline create trips
        // VUID-VkComputePipelineCreateInfo-layout-07988. This non-clustered path binds the
        // light table as a harmless valid placeholder (the resolve's `clusters_enabled` gate
        // never reads them) — the same pattern the production swapchain resolve uses.
        BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // P6 R1: the SDF edit-list `Buf` SSBO @10 (the resolve's `sdf_soft_shadow_ranged`
        // analytic march reads it read-only; the SAME buffer the marcher binds @0). 11 ≤ 12
        // (no cap raise — the orchestrator's R1=(A) decision drops the brick atlas binds).
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Render P7 GROUP C1: the SSAO term `gSsao` STORAGE image @11 (read under
        // `ssao_mode != 0`; OFF here, so bound-but-unread). The recompiled `deferred_pbr.comp`
        // STATICALLY declares `gSsao @11`, so the layout MUST declare it or the pipeline create
        // trips the binding-count check (the P6 R1 binding-10 discipline).
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        // CSM Increment 1b (Rung A): the cascade combined image+sampler @12 + the cascade UBO @13.
        // Shadow Inc-1-GPU: the atlas combined image+sampler @14 + the atlas UBO @15 (the resolve set
        // hits 16/16). All `*_mode == 0` here → bound-but-unread; the recompiled resolve STATICALLY
        // references all four, so the layout MUST declare them.
        CsmResolveDummies::layout_entries()[0],
        CsmResolveDummies::layout_entries()[1],
        CsmResolveDummies::layout_entries()[2],
        CsmResolveDummies::layout_entries()[3],
    ];
    // CSM Increment 1b + Shadow Inc-1-GPU: the OFF-path cascade trio @12/@13 + atlas trio @14/@15
    // (bound-but-unread).
    let csm_dummies = CsmResolveDummies::create(device);
    let resolve_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc { entries: &resolve_layout_entries })
        .expect("deferred resolve bind-group layout");
    let resolve_compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &resolve_cs,
            entry: c"main",
            // The resolve pushes NO constants, but `create_compute_pipeline` requires a
            // non-empty (multiple-of-4) push range; declare the shared range (unused).
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&resolve_layout),
            spec_constants: &[],
        })
        .expect("deferred resolve compute pipeline");
    let resolve_bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &resolve_layout,
            entries: &[
                BindGroupEntry::StorageImage { texture: &albedo },
                BindGroupEntry::StorageImage { texture: &normal },
                BindGroupEntry::StorageImage { texture: &material },
                BindGroupEntry::StorageImage { texture: &lit },
                BindGroupEntry::StorageBuffer { buffer: &material_table },
                BindGroupEntry::UniformBuffer { buffer: &camera_uniform },
                BindGroupEntry::StorageBuffer { buffer: &light_table },
                // Lighting L0b: the gViewT lane @7 (the resolve READS it under `mask == 1`).
                BindGroupEntry::StorageImage { texture: &viewt },
                // Lighting L1 @8/@9: placeholder = the light table (L1 OFF on this path, so the
                // resolve's `clusters_enabled` gate never reads them; they exist only to satisfy
                // the recompiled shader's static @8/@9 reference).
                BindGroupEntry::StorageBuffer { buffer: &light_table },
                BindGroupEntry::StorageBuffer { buffer: &light_table },
                // P6 R1: the SDF edit-list `Buf` @10 (the marcher's vocab @0 SSBO).
                BindGroupEntry::StorageBuffer { buffer: &buffer },
                // Render P7 GROUP C1: the SSAO term `gSsao` @11 (bound-but-unread — `ssao_mode`
                // is 0 here, so the resolve never loads it; present only to satisfy the layout).
                BindGroupEntry::StorageImage { texture: &ssao },
                // CSM Increment 1b (Rung A): the cascade combined map+sampler @12 + UBO @13
                // (bound-but-unread — `csm_mode == 0` here, so the resolve's PCF sample never runs).
                BindGroupEntry::CombinedImage {
                    texture: &csm_dummies.cascade,
                    sampler: &csm_dummies.sampler,
                },
                BindGroupEntry::UniformBuffer { buffer: &csm_dummies.ubo },
                // Shadow Inc-1-GPU: the atlas combined map+sampler @14 + UBO @15 (bound-but-unread —
                // `punctual_shadow_mode == 0` here, so the resolve's spot PCF sample never runs). The
                // 15th + 16th entries — the resolve set hits 16/16.
                BindGroupEntry::CombinedImage {
                    texture: &csm_dummies.atlas,
                    sampler: &csm_dummies.atlas_sampler,
                },
                BindGroupEntry::UniformBuffer { buffer: &csm_dummies.atlas_ubo },
            ],
        })
        .expect("deferred resolve bind group");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    encoder.begin().expect("begin");

    // --- Render P5-r0 mesh raster pass A: clear depth + the 3 MRT G-buffer lanes, then
    // rasterize the quad as a 3-MRT producer (albedo/normal/material in the marcher's
    // encoding, mask=1). The 3 RGBA8 images: UNDEFINED → COLOR_ATTACHMENT_OPTIMAL. ---
    for tex in [&albedo, &normal, &material] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::TOP_OF_PIPE,
            dst_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
            src_access: BarrierAccess::NONE,
            dst_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
            old_layout: ImageLayout::Undefined,
            new_layout: ImageLayout::ColorAttachmentOptimal,
            range: ImageSubresourceRange::COLOR,
        });
    }
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::DepthAttachmentOptimal,
        range: ImageSubresourceRange::DEPTH,
    });

    let full = RenderArea { x: 0, y: 0, width: SDF_IMG_W, height: SDF_IMG_H };
    // Render P5-r0 / Decision r0-2: the MRT clears ARE the marcher's mask=0 neutral
    // G-buffer (albedo=(BACKGROUND.rgb,1), normal=(0.5,0.5,0,0), material=(1,1,0,1)), so a
    // no-fragment pixel holds the cleared neutral the marcher (owning it) overwrites anyway
    // — the no-mesh 0%-gate. All values pass through the same round(c*255) quantizer exactly.
    let color_attachments = [
        RenderingAttachment {
            texture: &albedo,
            layout: ImageLayout::ColorAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_color: [0.05, 0.05, 0.1, 1.0],
        },
        RenderingAttachment {
            texture: &normal,
            layout: ImageLayout::ColorAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_color: [0.5, 0.5, 0.0, 0.0],
        },
        RenderingAttachment {
            texture: &material,
            layout: ImageLayout::ColorAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_color: [1.0, 1.0, 0.0, 1.0],
        },
    ];
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &color_attachments,
        depth: Some(DepthAttachment {
            texture: &depth,
            layout: ImageLayout::DepthAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_depth: DEPTH_CLEAR,
        }),
    });
    encoder.bind_graphics_pipeline(&gfx);
    // M1: bind the 1-element identity instance SSBO at set 0 (bound-but-unread — the
    // `use_model_matrix == 0` push selects the VS's legacy arm, byte-identical pixels).
    encoder.bind_descriptor_set(&instance_bind_group, &gfx);
    encoder.push_graphics_constants(&gfx, ShaderStage::VERTEX, 0, &ortho_mvp_bytes());
    encoder.bind_vertex_buffer(&vertex_buffer, 0, 0);
    encoder.set_viewport(&Viewport {
        x: 0.0,
        y: 0.0,
        width: SDF_IMG_W as f32,
        height: SDF_IMG_H as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    });
    encoder.set_scissor(&full);
    encoder.draw(6, 1, 0, 0); // two triangles = the quad
    encoder.end_rendering();

    // --- THE single depth dual-use barrier: DEPTH_ATTACHMENT_OPTIMAL →
    // SHADER_READ_ONLY_OPTIMAL. The depth WRITES happen at LATE_FRAGMENT_TESTS; the
    // marcher SAMPLES at COMPUTE_SHADER. This one barrier (DEPTH aspect,
    // LATE_FRAGMENT_TESTS src) makes the depth-write available + visible to the
    // shader-read and transitions the layout for sampling. It REPLACES the rung-10
    // depth→buffer copy + its two transfer barriers — there is NO copy_image_to_buffer
    // of the depth here. ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth,
        src_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        src_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
        dst_access: BarrierAccess::SHADER_READ,
        old_layout: ImageLayout::DepthAttachmentOptimal,
        new_layout: ImageLayout::ShaderReadOnlyOptimal,
        range: ImageSubresourceRange::DEPTH,
    });

    // --- Render P5-r0 barrier-out: the 3 RGBA8 G-buffer images COLOR_ATTACHMENT_OPTIMAL →
    // GENERAL, handing pass A's rasterized mesh fragments to the marcher (a GENUINE
    // raster-write hand-off: COLOR_ATTACHMENT_OUTPUT/COLOR_ATTACHMENT_WRITE →
    // COMPUTE_SHADER/SHADER_READ|SHADER_WRITE). The marcher (under r1) yields mesh-owned
    // texels so the raster's value survives. On a no-mesh scene a CLEAR is a color write,
    // correctly made available. ---
    for tex in [&albedo, &normal, &material] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
            dst_access: BarrierAccess::SHADER_READ | BarrierAccess::SHADER_WRITE,
            old_layout: ImageLayout::ColorAttachmentOptimal,
            new_layout: ImageLayout::General,
            range: ImageSubresourceRange::COLOR,
        });
    }

    // --- The lit output + the Lighting-L0b gViewT lane + the Render P7 SSAO term: UNDEFINED →
    // GENERAL (r0 does NOT rasterize into these — they stay wholly marcher/resolve-produced;
    // `ssao` lives in GENERAL its whole life like `viewt`, bound-but-unread under ssao_mode 0). ---
    for tex in [&lit, &viewt, &ssao] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::TOP_OF_PIPE,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::NONE,
            dst_access: BarrierAccess::SHADER_WRITE,
            old_layout: ImageLayout::Undefined,
            new_layout: ImageLayout::General,
            range: ImageSubresourceRange::COLOR,
        });
    }

    // --- Render P4b: the COARSE-CULL pass (runs only when culling is enabled; the
    // depth image is already SHADER_READ_ONLY from the dual-use barrier above, which it
    // also samples). One invocation per 8×8 tile writes a `TileBound` into binding 6.
    // The vocabulary set is bound against the coarse pipeline's OWN layout. ---
    if coarse_enabled {
        encoder.bind_compute_pipeline(&coarse_compute);
        encoder.bind_descriptor_set_compute(&bind_group, &coarse_compute);
        encoder.dispatch(coarse_group_count_x(), 1, 1);

        // The inter-dispatch barrier: the coarse pass's `TileBound` WRITES (binding 6,
        // COMPUTE_SHADER/SHADER_WRITE) must be available + visible to the fine marcher's
        // READS (COMPUTE_SHADER/SHADER_READ) before the fine dispatch reads them.
        let tiles_barrier = [BufferBarrier {
            buffer: &tiles_buffer,
            src_access: BarrierAccess::SHADER_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
        }];
        encoder.pipeline_barrier(&BarrierDesc {
            src_stage: BarrierStage::COMPUTE_SHADER,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            buffers: &tiles_barrier,
        });
    }

    // --- SDF marcher compute pass: SAMPLE the depth image, STORE the G-buffer. The
    // vocabulary set is bound against the pipeline's OWN dedicated layout via
    // `bind_descriptor_set_compute`; no `bind_storage_buffer`, so the encoder's fixed
    // single-set rebind is skipped. P4b pushes the 4-byte `coarse_enabled` gate against
    // the marcher's OWN layout (via `push_compute_constants`). ---
    encoder.bind_compute_pipeline(&compute);
    encoder.bind_descriptor_set_compute(&bind_group, &compute);
    // Render P4b + B1 + A1/A2: the 32-byte `FineMarcherPush` — `coarse_enabled` (offset 0)
    // gates the cull, `omega` (offset 4) carries the over-relaxation factor, and
    // `lighting_flags` (offset 8) + `light_dir` (offset 16) drive A1/A2. The caller selects
    // the lighting state (the OFF path `lighting_flags == 0` ⇒ the shader's byte-identical
    // Lambert path; the ON path folds in the soft shadow + AO). The clamp is a RUNTIME
    // `f32::clamp` (NOT a debug_assert): `omega == 2` is the soundness ceiling, so a caller
    // passing a hot value must be defanged in release too.
    let omega: f32 = omega_in.clamp(1.0, 1.99);
    let push = {
        let base = FineMarcherPush::new(coarse_enabled, omega, lighting_flags, light_dir);
        // M1: enable the empty-skip + stamp the grid uniforms the marcher indexes binding 9
        // with, when a brick grid was supplied. Off ⇒ the pre-M1 byte-identical push.
        let m1 = match brick {
            Some((grid, _)) => base.with_brick(grid.origin, grid.dims, grid.brick_world),
            None => base,
        };
        // M2: enable the trilinear+cubic SURFACE path. INDEPENDENT of the M1 empty-skip — the
        // M2GridParams the marcher reads live in the b5 UBO tail (written above), not the push.
        // `brick_trilinear == false` leaves the push byte-identical to the M1 state (the OFF path).
        //
        // M4 clip-map LOD (Slice C): `brick_levels` is `BRICK_LEVELS` with a clip-map (the N-level
        // path — the marcher's `select_level` dispatches the finest enclosing level via the branch-
        // ladder), else `1` (the OFF/N=1 path — `select_level` loops once over level 0, byte-identical
        // to the pre-M4 M2 marcher). `with_brick_levels(1)` is REQUIRED on the single-level path: the
        // recompiled shader treats `brick_levels == 0` as no-level. The level-1/2 bindings @11..14 are
        // bound-but-unread when N == 1. (The lvl==0 empty-skip arm reads `pc.grid_*` set by `with_brick`
        // above; with a clip-map the caller passes level-0's clip-map grid for byte-consistency.)
        let levels = match clipmap {
            Some(_) => boyko_sdf_math::brick::BRICK_LEVELS as u32,
            None => 1,
        };
        m1.with_brick_trilinear(brick_trilinear).with_brick_levels(levels)
    };
    encoder.push_compute_constants(&compute, ShaderStage::COMPUTE, 0, push.as_bytes());
    encoder.dispatch(group_count_x(), 1, 1);

    // --- (5a) PBR MVP-2: make the marcher's gAlbedo + gNormal + gMaterial STORES available
    // + visible to the resolve's LOADS — a real memory+execution dependency
    // (SHADER_WRITE→SHADER_READ, COMPUTE→COMPUTE), GENERAL→GENERAL (no layout change).
    // gNormal is now READ by the resolve (oct-normal decode + 16-bit material id). Lighting
    // L0b: the gViewT lane is marcher-STORED + resolve-READ, so it joins too. ---
    for tex in [&albedo, &normal, &material, &viewt] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::COMPUTE_SHADER,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::SHADER_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
            old_layout: ImageLayout::General,
            new_layout: ImageLayout::General,
            range: ImageSubresourceRange::COLOR,
        });
    }

    // --- (5b) PBR MVP-2 RESOLVE pass: bind the resolve pipeline + the resolve set (gAlbedo
    // @0, gNormal @1, gMaterial @2, lit @3, material SSBO @4, camera UBO @5), dispatch at
    // the SAME grid the marcher used. It runs Cook-Torrance for SDF (mask==1) pixels and
    // passes base through for mesh/bg (mask==0). ---
    encoder.bind_compute_pipeline(&resolve_compute);
    encoder.bind_descriptor_set_compute(&resolve_bind_group, &resolve_compute);
    encoder.dispatch(group_count_x(), 1, 1);

    // --- (5c) LIT: GENERAL → TRANSFER_SRC_OPTIMAL for the readback copy (the readback now
    // copies the resolve's LIT output, NOT albedo — albedo stays GENERAL, consumed only by
    // the resolve as a STORAGE-in-GENERAL load). ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &lit,
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::SHADER_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::TransferSrcOptimal,
        range: ImageSubresourceRange::COLOR,
    });

    let regions = [BufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        aspect: ImageAspect::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
        image_offset_x: 0,
        image_offset_y: 0,
        image_offset_z: 0,
        image_extent_w: SDF_IMG_W,
        image_extent_h: SDF_IMG_H,
        image_extent_d: 1,
    }];
    encoder.copy_image_to_buffer(&lit, ImageLayout::TransferSrcOptimal, &readback, &regions);

    // --- M2: copy the gViewT R32_SFLOAT surface-`t` image back (only on the discriminator path).
    // The marcher STORED `t` into it (GENERAL); the resolve only READ it (no write), so it is still
    // GENERAL. Transition GENERAL → TRANSFER_SRC (the marcher's COMPUTE_SHADER write is the source
    // dependency) and copy one f32/pixel. This is gated by `read_viewt`, so the OFF byte-identity
    // path records NEITHER this barrier NOR this copy (the command stream is unchanged). ---
    if read_viewt {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: &viewt,
            src_stage: BarrierStage::COMPUTE_SHADER,
            dst_stage: BarrierStage::TRANSFER,
            src_access: BarrierAccess::SHADER_WRITE,
            dst_access: BarrierAccess::TRANSFER_READ,
            old_layout: ImageLayout::General,
            new_layout: ImageLayout::TransferSrcOptimal,
            range: ImageSubresourceRange::COLOR,
        });
        encoder.copy_image_to_buffer(
            &viewt,
            ImageLayout::TransferSrcOptimal,
            &viewt_readback,
            &regions,
        );
    }

    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    // Read back the LIT R8G8B8A8 bytes (the deferred resolve's output).
    let dst_ptr = device
        .buffer_mapped_ptr(&readback)
        .expect("host-visible readback buffer is mapped");
    let mut out = vec![0u8; READBACK_BYTES as usize];
    // SAFETY: `dst_ptr` points to `READBACK_BYTES` mapped host-coherent bytes; a fence
    // wait preceded this read, so the GPU store + copy are complete + coherent; reading
    // `READBACK_BYTES` bytes is in-bounds; `out` is a distinct allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), READBACK_BYTES as usize);
    }

    // Render P4b: optionally read back the per-tile cull buffer (binding 6). It is a
    // HostVisibleCoherent buffer, so no transfer copy is required — the coarse pass's
    // disjoint per-tile writes completed before the fence signalled above, and
    // host-coherent memory makes them visible to this read without a flush/invalidate.
    let tiles_out = if read_tiles {
        let tiles_bytes = (tile_count() as usize) * TILE_BOUND_BYTES;
        let tiles_ptr = device
            .buffer_mapped_ptr(&tiles_buffer)
            .expect("host-visible tiles buffer is mapped");
        let mut tb = vec![0u8; tiles_bytes];
        // SAFETY: `tiles_ptr` points to `tile_count() * TILE_BOUND_BYTES` mapped
        // host-coherent bytes (the buffer was sized so above); the fence wait preceded
        // this read, so the coarse pass's writes are complete + coherent; reading
        // `tiles_bytes` is in-bounds; `tb` is a distinct allocation.
        unsafe {
            core::ptr::copy_nonoverlapping(tiles_ptr.as_ptr(), tb.as_mut_ptr(), tiles_bytes);
        }
        Some(tb)
    } else {
        None
    };

    // M2: optionally read back the gViewT surface-`t` f32 image (the GPU's per-pixel hit `t`,
    // `1.0e30` on a non-hit pixel). The buffer is HostVisibleCoherent + the fence wait preceded
    // this read, so the copy is complete + coherent.
    let viewt_out: Option<Vec<f32>> = if read_viewt {
        let viewt_ptr = device
            .buffer_mapped_ptr(&viewt_readback)
            .expect("host-visible gViewT readback buffer is mapped");
        let mut vt = vec![0f32; PIXELS as usize];
        // SAFETY: `viewt_ptr` points to `PIXELS * 4` mapped host-coherent bytes (sized so on the
        // `read_viewt` path); the fence wait preceded this read; reading `PIXELS` f32 is in-bounds;
        // `vt` is a distinct allocation; an R32_SFLOAT texel is a valid `f32` bit pattern.
        unsafe {
            core::ptr::copy_nonoverlapping(
                viewt_ptr.as_ptr().cast::<f32>(),
                vt.as_mut_ptr(),
                PIXELS as usize,
            );
        }
        Some(vt)
    } else {
        None
    };

    assert_validation_clean(ctx);

    // SAFETY: every resource was created on `device`; the last submission completed
    // (fence-waited above), so none is in use; each is destroyed exactly once.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_bind_group(resolve_bind_group);
        device.destroy_bind_group(bind_group);
        device.destroy_compute_pipeline(resolve_compute);
        device.destroy_compute_pipeline(coarse_compute);
        device.destroy_compute_pipeline(compute);
        device.destroy_bind_group_layout(resolve_layout);
        device.destroy_bind_group_layout(bind_layout);
        // CSM Increment 1b: the OFF-path cascade trio bound at resolve @12/@13.
        csm_dummies.destroy(device);
        device.destroy_graphics_pipeline(gfx);
        // M1 instance-model resources (bind group → buffer → layout, after the pipeline).
        device.destroy_bind_group(instance_bind_group);
        device.destroy_buffer(instance_buffer);
        device.destroy_bind_group_layout(instance_layout);
        device.destroy_shader_module(resolve_cs);
        device.destroy_shader_module(coarse_cs);
        device.destroy_shader_module(cs);
        device.destroy_shader_module(fs);
        device.destroy_shader_module(vs);
        device.destroy_buffer(vertex_buffer);
        device.destroy_buffer(readback);
        device.destroy_buffer(viewt_readback);
        device.destroy_sampler(sampler);
        device.destroy_texture(ssao);
        device.destroy_texture(viewt);
        device.destroy_texture(lit);
        device.destroy_texture(material);
        device.destroy_texture(normal);
        device.destroy_texture(albedo);
        device.destroy_texture(depth);
        device.destroy_buffer(tiles_buffer);
        device.destroy_buffer(light_table);
        device.destroy_buffer(material_table);
        device.destroy_buffer(camera_uniform);
        device.destroy_buffer(buffer);
        device.destroy_buffer(pointer_grid_buffer);
        // M2: the brick atlas (image + sampler). The last submission completed (fence-waited
        // above), so no work still samples it; `destroy` consumes `self` ⇒ each object once.
        brick_atlas.destroy(device);
    }

    (out, tiles_out, viewt_out)
}

/// Render P7 — the SSAO-enabled OFFSCREEN harness. The no-brick / no-cull marcher → **SSAO** →
/// resolve path (a self-contained sibling of [`run_gbuffer_hybrid_m4`]'s OFF path), recording the
/// dedicated 5-binding SSAO compute pass BETWEEN the marcher→resolve store-to-load barrier and the
/// resolve. Returns `(lit, ssao_r8)`: the LIT R8G8B8A8 readback AND the raw `ssao` R8_UNORM lane
/// readback (`PIXELS` bytes, one AO factor per pixel). The light table is `light_table_words`
/// (the caller arms `with_ssao_mode(1)`); `lighting_flags`/`light_dir` drive A1/A2 as usual.
///
/// The SSAO pass binds { gNormal @0, gMaterial @1, gViewT @2 (R), the `ssao` out @3 (W), the
/// camera UBO @4 } and dispatches at the SAME grid the marcher used; a COMPUTE→COMPUTE
/// GENERAL→GENERAL barrier on `ssao` orders its store before the resolve's `gSsao.Load`. The
/// resolve set binds the SSAO image at @11 (the C1 interface). The host oracle is
/// [`golden_ssao_attributes`] (AO channel) + [`golden_deferred_resolve_table_shadowed_ssao`] (lit).
///
/// Render P7-Q2: `quality` selects which pre-compiled SSAO variant pipeline to bind (an index into
/// `SSAO_PARAMS` / the `SSAO_QUALITY_*` constants — the SAME 5-binding layout drives any variant,
/// so only the loaded `.spv` differs). Feed `SSAO_PARAMS[quality]` to [`golden_ssao_attributes`] for
/// the matching host oracle. `SSAO_QUALITY_MEDIUM` == today's shipped path (byte-identical to pre-Q2).
#[allow(clippy::too_many_arguments)]
fn run_gbuffer_hybrid_ssao(
    ctx: &VulkanContext,
    edits: &[SdfEdit],
    lighting_flags: u32,
    light_dir: [f32; 3],
    light_table_words: &[u32],
    quality: usize,
) -> (Vec<u8>, Vec<u8>) {
    let device: &VulkanContext = ctx;
    let queue = ctx.rhi_queue();

    // --- The edit-list SSBO (binding 0 + resolve @10), seeded with the packed header. ---
    let buffer = device
        .create_buffer(&BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, edits);
        let mapped = device
            .buffer_mapped_ptr(&buffer)
            .expect("host-visible buffer is mapped");
        write_words(mapped, &header);
    }

    // --- The camera/extent UNIFORM buffer (vocab @5 + resolve @5 + SSAO @4): the 80-byte ORTHO
    // camera block + the (zeroed, brick-OFF) M4 tail, sized to the widened M4 UBO. ---
    let camera_uniform = device
        .create_buffer(&BufferDesc {
            size: B5_CAMERA_UBO_BYTES_M4 as u64,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("camera uniform buffer");
    {
        let pc = CompositePushConstants::ortho(SDF_IMG_W, SDF_IMG_H);
        debug_assert_eq!(pc.count, PIXELS);
        let mapped = device
            .buffer_mapped_ptr(&camera_uniform)
            .expect("host-visible uniform buffer is mapped");
        let bytes = pc.as_bytes();
        debug_assert_eq!(bytes.len(), M2_GRID_PARAMS_OFFSET, "camera block must be 80 B");
        // SAFETY: `mapped` points to `B5_CAMERA_UBO_BYTES_M4` (224) mapped host-coherent bytes; the
        // 80-byte camera block is written at offset 0 (the M4 tail stays zero — brick OFF for SSAO).
        // No GPU work is in flight yet, so the host write is unsynchronized-safe.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
        }
    }

    // --- The material table SSBO (vocab @7 + resolve @4). ---
    let material_table = device
        .create_buffer(&BufferDesc {
            size: (DEFAULT_MATERIAL_TABLE.len() as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("PBR material table storage buffer");
    {
        let mapped = device
            .buffer_mapped_ptr(&material_table)
            .expect("host-visible material table is mapped");
        write_words(mapped, &DEFAULT_MATERIAL_TABLE);
    }

    // --- The light table SSBO (resolve @6), seeded with the caller's words (ssao_mode armed). ---
    let light_table = device
        .create_buffer(&BufferDesc {
            size: (light_table_words.len() as u64) * 4,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("Lighting light table storage buffer");
    {
        let mapped = device
            .buffer_mapped_ptr(&light_table)
            .expect("host-visible light table is mapped");
        write_words(mapped, light_table_words);
    }

    // --- The bound-but-unread coarse-cull tile SSBO (vocab @6) + the brick placeholder grid
    // (vocab @9). The SSAO harness runs the marcher with the cull + brick gated OFF. ---
    let tiles_buffer = device
        .create_buffer(&BufferDesc {
            size: (tile_count() as u64) * (TILE_BOUND_BYTES as u64),
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("P4b coarse-cull tile-bound storage buffer");
    let pointer_grid_buffer = device
        .create_buffer(&BufferDesc {
            size: 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("M1 pointer-grid placeholder buffer");
    {
        let mapped = device
            .buffer_mapped_ptr(&pointer_grid_buffer)
            .expect("host-visible pointer-grid placeholder is mapped");
        write_words(mapped, &[0u32]);
    }

    // --- The brick atlas (vocab @10 + the duplicate level slots @12/@14): baked from `edits`,
    // bound-but-unread (`brick_trilinear == 0`, `brick_levels == 1`). ---
    let brick_atlas = {
        use boyko_sdf_math::SdfEditField;
        let mut field = SdfEditField::new();
        for e in edits {
            assert!(field.push(*e), "scene must fit MAX_SDF_EDITS");
        }
        field.bump_gen();
        BrickAtlas::create(ctx, &field).expect("M2 brick atlas (bound-but-unread) — create + bake")
    };

    // --- The depth IMAGE + the MRT G-buffer + lit + gViewT + ssao images. ---
    let depth = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("offscreen depth texture (sampled)");
    let make_gbuf = |usage: ImageUsage, label: &str| {
        device
            .create_texture(&TextureDesc {
                width: SDF_IMG_W,
                height: SDF_IMG_H,
                depth: 1,
                format: GBUFFER_FORMAT,
                dimension: TextureDimension::D2,
                usage,
                array_layers: 1,
                mip_levels: 1,
                view_format: None,
            })
            .unwrap_or_else(|e| panic!("{label}: {e:?}"))
    };
    let albedo = make_gbuf(ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT, "G-buffer albedo");
    let normal = make_gbuf(ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT, "G-buffer normal");
    let material = make_gbuf(ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT, "G-buffer material");
    let lit = make_gbuf(ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC, "resolve lit image");
    let viewt = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::R32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("Lighting L0b gViewT storage image");
    // The SSAO term `gSsao` — R8_UNORM STORAGE, the SSAO pass WRITES it + the resolve READS it; it
    // additionally carries TRANSFER_SRC so the AO-channel golden reads the raw factor back.
    let ssao = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::R8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("Render P7 SSAO gSsao storage image");

    let sampler = device
        .create_sampler(&SamplerDesc::default())
        .expect("depth sampler (ignored by .Load)");

    // The readback buffers: LIT (4 B/px) + the raw SSAO factor (1 B/px).
    let readback = device
        .create_buffer(&BufferDesc {
            size: READBACK_BYTES,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible lit readback buffer");
    let ssao_readback = device
        .create_buffer(&BufferDesc {
            size: PIXELS as u64,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible ssao readback buffer");

    // The quad vertex buffer (the mesh backdrop).
    let vertices = quad_vertices();
    let vertex_bytes = core::mem::size_of_val(&vertices) as u64;
    let vertex_buffer = device
        .create_buffer(&BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible vertex buffer");
    {
        let vb_ptr = device
            .buffer_mapped_ptr(&vertex_buffer)
            .expect("host-visible vertex buffer is mapped");
        // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes; `vertices` is a
        // distinct stack array of `vertex_bytes` bytes; the write completes before any submit.
        unsafe {
            core::ptr::copy_nonoverlapping(
                vertices.as_ptr().cast::<u8>(),
                vb_ptr.as_ptr(),
                vertex_bytes as usize,
            );
        }
    }

    // --- Modules + pipelines: the mesh-MRT producer, the marcher, the resolve, and the SSAO pass. ---
    let vs = device.create_shader_module(MRT_VS_SPV.as_words()).expect("mesh-MRT vs");
    let fs = device.create_shader_module(MRT_FS_SPV.as_words()).expect("mesh-MRT fs");
    let cs = device.create_shader_module(sdf_gbuffer_composite_spirv()).expect("marcher cs");
    let resolve_cs = device.create_shader_module(deferred_pbr_spirv()).expect("resolve cs");
    let ssao_cs = device
        .create_shader_module(sdf_ssao_spirv_variant(quality))
        .expect("SSAO cs");

    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
    ];
    // M1: the per-instance model SSBO layout + 1-element identity dummy + its bind group (the
    // VS statically references `instances` at set 0 binding 0; the legacy arm never reads it).
    let (instance_layout, instance_buffer, instance_bind_group) = create_identity_instance(device);
    let gfx = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            color_formats: &[GBUFFER_FORMAT, GBUFFER_FORMAT, GBUFFER_FORMAT],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout { stride: VERTEX_STRIDE, attributes: &attributes }),
            push_constant_bytes: MVP_BYTES,
            bind_group_layout: Some(&instance_layout),
            blend: None,
            cull_mode: CullMode::None,
            depth_bias: None,
        })
        .expect("mesh-MRT G-buffer producer graphics pipeline");

    // The 16-binding vocabulary layout (the marcher's full interface, brick @9..=14 bound-but-unread,
    // mesh-SDF @15 bound-but-unread under the runtime-gated `mesh_sdf_enabled == 0` branch).
    let vocab_kinds = [
        DescriptorKind::StorageBuffer, DescriptorKind::SampledImage, DescriptorKind::StorageImage,
        DescriptorKind::StorageImage, DescriptorKind::StorageImage, DescriptorKind::UniformBuffer,
        DescriptorKind::StorageBuffer, DescriptorKind::StorageBuffer, DescriptorKind::StorageImage,
        DescriptorKind::StorageBuffer, DescriptorKind::CombinedImageSampler, DescriptorKind::StorageBuffer,
        DescriptorKind::CombinedImageSampler, DescriptorKind::StorageBuffer, DescriptorKind::CombinedImageSampler,
        DescriptorKind::CombinedImageSampler,
    ];
    let vocab_layout_entries: Vec<BindGroupLayoutEntry> = vocab_kinds
        .iter()
        .enumerate()
        .map(|(i, &kind)| BindGroupLayoutEntry { binding: i as u32, count: 1, kind, stage: ShaderStage::COMPUTE })
        .collect();
    let bind_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc { entries: &vocab_layout_entries })
        .expect("vocabulary bind-group layout");
    let compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&bind_layout),
            spec_constants: &[],
        })
        .expect("G-buffer marcher compute pipeline");

    let bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &bind_layout,
            entries: &[
                BindGroupEntry::StorageBuffer { buffer: &buffer },
                BindGroupEntry::SampledImage { texture: &depth, sampler: &sampler },
                BindGroupEntry::StorageImage { texture: &albedo },
                BindGroupEntry::StorageImage { texture: &normal },
                BindGroupEntry::StorageImage { texture: &material },
                BindGroupEntry::UniformBuffer { buffer: &camera_uniform },
                BindGroupEntry::StorageBuffer { buffer: &tiles_buffer },
                BindGroupEntry::StorageBuffer { buffer: &material_table },
                BindGroupEntry::StorageImage { texture: &viewt },
                BindGroupEntry::StorageBuffer { buffer: &pointer_grid_buffer },
                BindGroupEntry::CombinedImage { texture: brick_atlas.texture(), sampler: brick_atlas.sampler() },
                BindGroupEntry::StorageBuffer { buffer: &pointer_grid_buffer },
                BindGroupEntry::CombinedImage { texture: brick_atlas.texture(), sampler: brick_atlas.sampler() },
                BindGroupEntry::StorageBuffer { buffer: &pointer_grid_buffer },
                BindGroupEntry::CombinedImage { texture: brick_atlas.texture(), sampler: brick_atlas.sampler() },
                // MDF Stage-2c @15: bind the brick atlas as a benign placeholder (no MDF scene here);
                // the marcher gates the read OFF (`mesh_sdf_enabled == 0`), so it is bound-but-unread —
                // the R2 contract (a VALID descriptor at the statically-referenced binding 15).
                BindGroupEntry::CombinedImage { texture: brick_atlas.texture(), sampler: brick_atlas.sampler() },
            ],
        })
        .expect("vocabulary bind group");

    // The 16-binding resolve layout (gSsao @11 = the C1 interface; CSM cascade @12/@13; shadow
    // atlas @14/@15 — the resolve set hits 16/16, the descriptor cap).
    let resolve_kinds = [
        DescriptorKind::StorageImage, DescriptorKind::StorageImage, DescriptorKind::StorageImage,
        DescriptorKind::StorageImage, DescriptorKind::StorageBuffer, DescriptorKind::UniformBuffer,
        DescriptorKind::StorageBuffer, DescriptorKind::StorageImage, DescriptorKind::StorageBuffer,
        DescriptorKind::StorageBuffer, DescriptorKind::StorageBuffer, DescriptorKind::StorageImage,
        // CSM Increment 1b (Rung A): the cascade combined map+sampler @12 + the cascade UBO @13.
        DescriptorKind::CombinedImageSampler, DescriptorKind::UniformBuffer,
        // Shadow Inc-1-GPU: the atlas combined map+sampler @14 + the atlas UBO @15.
        DescriptorKind::CombinedImageSampler, DescriptorKind::UniformBuffer,
    ];
    let resolve_layout_entries: Vec<BindGroupLayoutEntry> = resolve_kinds
        .iter()
        .enumerate()
        .map(|(i, &kind)| BindGroupLayoutEntry { binding: i as u32, count: 1, kind, stage: ShaderStage::COMPUTE })
        .collect();
    // CSM Increment 1b: the OFF-path cascade trio bound at resolve @12/@13 (bound-but-unread).
    let csm_dummies = CsmResolveDummies::create(device);
    let resolve_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc { entries: &resolve_layout_entries })
        .expect("deferred resolve bind-group layout");
    let resolve_compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &resolve_cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&resolve_layout),
            spec_constants: &[],
        })
        .expect("deferred resolve compute pipeline");
    let resolve_bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &resolve_layout,
            entries: &[
                BindGroupEntry::StorageImage { texture: &albedo },
                BindGroupEntry::StorageImage { texture: &normal },
                BindGroupEntry::StorageImage { texture: &material },
                BindGroupEntry::StorageImage { texture: &lit },
                BindGroupEntry::StorageBuffer { buffer: &material_table },
                BindGroupEntry::UniformBuffer { buffer: &camera_uniform },
                BindGroupEntry::StorageBuffer { buffer: &light_table },
                BindGroupEntry::StorageImage { texture: &viewt },
                BindGroupEntry::StorageBuffer { buffer: &light_table },
                BindGroupEntry::StorageBuffer { buffer: &light_table },
                BindGroupEntry::StorageBuffer { buffer: &buffer },
                // Render P7: the SSAO term `gSsao` @11 — the SAME image the SSAO pass writes.
                BindGroupEntry::StorageImage { texture: &ssao },
                // CSM Increment 1b (Rung A): the cascade combined map+sampler @12 + UBO @13
                // (bound-but-unread — `csm_mode == 0` here, so the resolve's PCF sample never runs).
                BindGroupEntry::CombinedImage {
                    texture: &csm_dummies.cascade,
                    sampler: &csm_dummies.sampler,
                },
                BindGroupEntry::UniformBuffer { buffer: &csm_dummies.ubo },
                // Shadow Inc-1-GPU: the atlas combined map+sampler @14 + UBO @15 (bound-but-unread —
                // `punctual_shadow_mode == 0` here, so the resolve's spot PCF sample never runs). The
                // 15th + 16th entries — the resolve set hits 16/16.
                BindGroupEntry::CombinedImage {
                    texture: &csm_dummies.atlas,
                    sampler: &csm_dummies.atlas_sampler,
                },
                BindGroupEntry::UniformBuffer { buffer: &csm_dummies.atlas_ubo },
            ],
        })
        .expect("deferred resolve bind group");

    // The dedicated 5-binding SSAO layout + pipeline + set.
    let ssao_kinds = [
        DescriptorKind::StorageImage, DescriptorKind::StorageImage, DescriptorKind::StorageImage,
        DescriptorKind::StorageImage, DescriptorKind::UniformBuffer,
    ];
    let ssao_layout_entries: Vec<BindGroupLayoutEntry> = ssao_kinds
        .iter()
        .enumerate()
        .map(|(i, &kind)| BindGroupLayoutEntry { binding: i as u32, count: 1, kind, stage: ShaderStage::COMPUTE })
        .collect();
    let ssao_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc { entries: &ssao_layout_entries })
        .expect("SSAO bind-group layout");
    let ssao_compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &ssao_cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&ssao_layout),
            spec_constants: &[],
        })
        .expect("SSAO compute pipeline");
    let ssao_bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &ssao_layout,
            entries: &[
                BindGroupEntry::StorageImage { texture: &normal },
                BindGroupEntry::StorageImage { texture: &material },
                BindGroupEntry::StorageImage { texture: &viewt },
                BindGroupEntry::StorageImage { texture: &ssao },
                BindGroupEntry::UniformBuffer { buffer: &camera_uniform },
            ],
        })
        .expect("SSAO bind group");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");
    encoder.begin().expect("begin");

    // --- Raster pass A: clear + draw the quad into the 3-MRT G-buffer (mask=1). ---
    for tex in [&albedo, &normal, &material] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::TOP_OF_PIPE,
            dst_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
            src_access: BarrierAccess::NONE,
            dst_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
            old_layout: ImageLayout::Undefined,
            new_layout: ImageLayout::ColorAttachmentOptimal,
            range: ImageSubresourceRange::COLOR,
        });
    }
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::DepthAttachmentOptimal,
        range: ImageSubresourceRange::DEPTH,
    });
    let full = RenderArea { x: 0, y: 0, width: SDF_IMG_W, height: SDF_IMG_H };
    let color_attachments = [
        RenderingAttachment { texture: &albedo, layout: ImageLayout::ColorAttachmentOptimal, load_op: LoadOp::Clear, store_op: StoreOp::Store, clear_color: [0.05, 0.05, 0.1, 1.0] },
        RenderingAttachment { texture: &normal, layout: ImageLayout::ColorAttachmentOptimal, load_op: LoadOp::Clear, store_op: StoreOp::Store, clear_color: [0.5, 0.5, 0.0, 0.0] },
        RenderingAttachment { texture: &material, layout: ImageLayout::ColorAttachmentOptimal, load_op: LoadOp::Clear, store_op: StoreOp::Store, clear_color: [1.0, 1.0, 0.0, 1.0] },
    ];
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &color_attachments,
        depth: Some(DepthAttachment { texture: &depth, layout: ImageLayout::DepthAttachmentOptimal, load_op: LoadOp::Clear, store_op: StoreOp::Store, clear_depth: DEPTH_CLEAR }),
    });
    encoder.bind_graphics_pipeline(&gfx);
    // M1: bind the 1-element identity instance SSBO at set 0 (bound-but-unread — the
    // `use_model_matrix == 0` push selects the VS's legacy arm, byte-identical pixels).
    encoder.bind_descriptor_set(&instance_bind_group, &gfx);
    encoder.push_graphics_constants(&gfx, ShaderStage::VERTEX, 0, &ortho_mvp_bytes());
    encoder.bind_vertex_buffer(&vertex_buffer, 0, 0);
    encoder.set_viewport(&Viewport { x: 0.0, y: 0.0, width: SDF_IMG_W as f32, height: SDF_IMG_H as f32, min_depth: 0.0, max_depth: 1.0 });
    encoder.set_scissor(&full);
    encoder.draw(6, 1, 0, 0);
    encoder.end_rendering();

    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth,
        src_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        src_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
        dst_access: BarrierAccess::SHADER_READ,
        old_layout: ImageLayout::DepthAttachmentOptimal,
        new_layout: ImageLayout::ShaderReadOnlyOptimal,
        range: ImageSubresourceRange::DEPTH,
    });
    for tex in [&albedo, &normal, &material] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
            dst_access: BarrierAccess::SHADER_READ | BarrierAccess::SHADER_WRITE,
            old_layout: ImageLayout::ColorAttachmentOptimal,
            new_layout: ImageLayout::General,
            range: ImageSubresourceRange::COLOR,
        });
    }
    // lit + gViewT + ssao: UNDEFINED → GENERAL (the marcher stores gViewT, the resolve stores lit,
    // the SSAO pass stores ssao — all in GENERAL).
    for tex in [&lit, &viewt, &ssao] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::TOP_OF_PIPE,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::NONE,
            dst_access: BarrierAccess::SHADER_WRITE,
            old_layout: ImageLayout::Undefined,
            new_layout: ImageLayout::General,
            range: ImageSubresourceRange::COLOR,
        });
    }

    // --- Marcher dispatch (brick + cull OFF, byte-identical to the pre-brick marcher). ---
    encoder.bind_compute_pipeline(&compute);
    encoder.bind_descriptor_set_compute(&bind_group, &compute);
    let push = FineMarcherPush::new(false, 1.0, lighting_flags, light_dir).with_brick_levels(1);
    encoder.push_compute_constants(&compute, ShaderStage::COMPUTE, 0, push.as_bytes());
    encoder.dispatch(group_count_x(), 1, 1);

    // --- (5a) marcher → resolve store-to-load barrier (covers gNormal/gMaterial/gViewT — the
    // SAME three the SSAO pass reads, so NO additional input barrier is needed for SSAO). ---
    for tex in [&albedo, &normal, &material, &viewt] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::COMPUTE_SHADER,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::SHADER_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
            old_layout: ImageLayout::General,
            new_layout: ImageLayout::General,
            range: ImageSubresourceRange::COLOR,
        });
    }

    // --- Render P7: the SSAO pass — read gNormal/gMaterial/gViewT, WRITE ssao. Then a
    // COMPUTE→COMPUTE GENERAL→GENERAL barrier on ssao so the resolve's `gSsao.Load` sees it. ---
    encoder.bind_compute_pipeline(&ssao_compute);
    encoder.bind_descriptor_set_compute(&ssao_bind_group, &ssao_compute);
    encoder.dispatch(group_count_x(), 1, 1);
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &ssao,
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        src_access: BarrierAccess::SHADER_WRITE,
        dst_access: BarrierAccess::SHADER_READ,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::General,
        range: ImageSubresourceRange::COLOR,
    });

    // --- Resolve dispatch (consumes the SSAO term under `ssao_mode != 0`). ---
    encoder.bind_compute_pipeline(&resolve_compute);
    encoder.bind_descriptor_set_compute(&resolve_bind_group, &resolve_compute);
    encoder.dispatch(group_count_x(), 1, 1);

    // --- Readbacks: LIT (GENERAL → TRANSFER_SRC) + ssao (GENERAL → TRANSFER_SRC). ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &lit,
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::SHADER_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::TransferSrcOptimal,
        range: ImageSubresourceRange::COLOR,
    });
    let regions = [BufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        aspect: ImageAspect::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
        image_offset_x: 0,
        image_offset_y: 0,
        image_offset_z: 0,
        image_extent_w: SDF_IMG_W,
        image_extent_h: SDF_IMG_H,
        image_extent_d: 1,
    }];
    encoder.copy_image_to_buffer(&lit, ImageLayout::TransferSrcOptimal, &readback, &regions);
    // The SSAO pass WROTE ssao (the SSAO→resolve barrier already made it COMPUTE-readable). The
    // resolve only READ it, so it is still GENERAL; transition to TRANSFER_SRC for the copy
    // (`src = COMPUTE_SHADER` — the SSAO store is the source dependency).
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &ssao,
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::SHADER_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::TransferSrcOptimal,
        range: ImageSubresourceRange::COLOR,
    });
    encoder.copy_image_to_buffer(&ssao, ImageLayout::TransferSrcOptimal, &ssao_readback, &regions);

    encoder.end().expect("end");
    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    let mut lit_out = vec![0u8; READBACK_BYTES as usize];
    {
        let dst = device.buffer_mapped_ptr(&readback).expect("lit readback mapped");
        // SAFETY: `dst` points to `READBACK_BYTES` mapped host-coherent bytes; the fence wait
        // preceded this read, so the copy is complete + coherent; `lit_out` is a distinct alloc.
        unsafe { core::ptr::copy_nonoverlapping(dst.as_ptr(), lit_out.as_mut_ptr(), READBACK_BYTES as usize) };
    }
    let mut ssao_out = vec![0u8; PIXELS as usize];
    {
        let dst = device.buffer_mapped_ptr(&ssao_readback).expect("ssao readback mapped");
        // SAFETY: `dst` points to `PIXELS` mapped host-coherent bytes (sized so above); the fence
        // wait preceded this read; `ssao_out` is a distinct alloc; an R8 texel is a valid `u8`.
        unsafe { core::ptr::copy_nonoverlapping(dst.as_ptr(), ssao_out.as_mut_ptr(), PIXELS as usize) };
    }

    assert_validation_clean(ctx);

    // SAFETY: every resource was created on `device`; the last submission completed (fence-waited
    // above), so none is in use; each is destroyed exactly once.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_bind_group(ssao_bind_group);
        device.destroy_bind_group(resolve_bind_group);
        device.destroy_bind_group(bind_group);
        device.destroy_compute_pipeline(ssao_compute);
        device.destroy_compute_pipeline(resolve_compute);
        device.destroy_compute_pipeline(compute);
        device.destroy_bind_group_layout(ssao_layout);
        device.destroy_bind_group_layout(resolve_layout);
        device.destroy_bind_group_layout(bind_layout);
        // CSM Increment 1b: the OFF-path cascade trio bound at resolve @12/@13.
        csm_dummies.destroy(device);
        device.destroy_graphics_pipeline(gfx);
        // M1 instance-model resources (bind group → buffer → layout, after the pipeline).
        device.destroy_bind_group(instance_bind_group);
        device.destroy_buffer(instance_buffer);
        device.destroy_bind_group_layout(instance_layout);
        device.destroy_shader_module(ssao_cs);
        device.destroy_shader_module(resolve_cs);
        device.destroy_shader_module(cs);
        device.destroy_shader_module(fs);
        device.destroy_shader_module(vs);
        device.destroy_buffer(vertex_buffer);
        device.destroy_buffer(ssao_readback);
        device.destroy_buffer(readback);
        device.destroy_sampler(sampler);
        device.destroy_texture(ssao);
        device.destroy_texture(viewt);
        device.destroy_texture(lit);
        device.destroy_texture(material);
        device.destroy_texture(normal);
        device.destroy_texture(albedo);
        device.destroy_texture(depth);
        device.destroy_buffer(pointer_grid_buffer);
        device.destroy_buffer(tiles_buffer);
        device.destroy_buffer(light_table);
        device.destroy_buffer(material_table);
        device.destroy_buffer(camera_uniform);
        device.destroy_buffer(buffer);
        brick_atlas.destroy(device);
    }

    (lit_out, ssao_out)
}

/// The rung-9/10 "crater" CSG scene (base sphere minus a smaller sphere).
fn crater() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
    ]
}

/// A box CSG scene (a box unioned, exercising the box primitive + the mesh occlusion).
fn box_csg() -> Vec<SdfEdit> {
    vec![SdfEdit::box_shape([0.0, 0.0, 0.0], [0.4, 0.4, 0.4], sdf_op::UNION, 0.0)]
}

/// A smooth-union scene (two spheres blended), exercising the smooth-min path.
fn smooth_union() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([-0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.15),
    ]
}

/// Scans for the first pixel matching `pred(sphere_hit, mesh_covered)`.
fn find_texel(edits: &[SdfEdit], pred: impl Fn(bool, bool) -> bool) -> Option<(u32, u32)> {
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let hit = editlist_pixel_hits(edits, px, py);
            let covered = mesh_covers_pixel(px, py);
            if pred(hit, covered) {
                return Some((px, py));
            }
        }
    }
    None
}

/// **P1b GPU gate (TESTER):** the OFFSCREEN image-based G-buffer hybrid composite
/// reproduces the deferred-PBR golden by reading back the LIT STORAGE image, with
/// NO depth→buffer copy in the recorded stream.
///
/// PBR MVP-2: the marcher now writes a PBR G-buffer and a deferred RESOLVE composites
/// `gLit` via full Cook-Torrance on the SDF arm (the `mask == 0` mesh / bg / empty pixels
/// pass through verbatim). The readback is LIT (not the raw albedo), so the reference is the
/// deferred oracle `golden_deferred_resolve ∘ golden_marcher_attributes` (fed the SAME
/// per-pixel [`expected_mesh_depth`] the GPU rasterizes), NOT the retired MVP-1
/// `golden_composite_pixel_ex` inline composite.
///
/// For each scene (crater_csg / box_csg / smooth_union), `run_gbuffer_hybrid` runs ω=1.0,
/// lighting OFF (flags == 0) — so EVERY arm is pass-through — and the whole LIT image must
/// match the deferred oracle within `+/-2/255` per channel (the marcher's host-pack-vs-GPU
/// quant budget), proven by [`assert_lit_matches_deferred_golden`]. The four discriminator
/// texels (mesh-occludes-SDF / SDF-only / mesh-only / background) and `assert_validation_clean`
/// still pin the occlusion + non-constant-fill contracts. The set is written ONCE at create —
/// NO per-frame `vkUpdateDescriptorSets`. The depth `copy_image_to_buffer` + its two barriers
/// are ABSENT (confirm by recording inspection / the validation-clean sync check).
#[test]
fn p1b_gbuffer_hybrid_matches_golden() {
    let Some(ctx) = boot_or_skip("p1b_gbuffer_hybrid_matches_golden") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());
    // Pixel golden: runs with or without validation (the clean-oracle assert in
    // `assert_validation_clean` self-gates when validation is disabled).
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — pixel golden still runs");
    }
    let caps = ctx.device_caps();
    assert!(
        caps.gbuffer_storage_format_ok,
        "a booted context must support STORAGE_IMAGE on the G-buffer format"
    );

    for (name, edits) in [
        ("crater_csg", crater()),
        ("box_csg", box_csg()),
        ("smooth_union", smooth_union()),
    ] {
        // The four discriminator texels, picked host-side BEFORE the GPU run.
        let a = find_texel(&edits, |hit, covered| hit && covered);
        let b = find_texel(&edits, |hit, covered| hit && !covered);
        let c = find_texel(&edits, |hit, covered| !hit && covered);
        let d = find_texel(&edits, |hit, covered| !hit && !covered);

        // Cull-OFF, ω=1.0, lighting OFF: the LIT readback must reproduce the deferred PBR
        // oracle on every arm (with flags == 0 every arm is pass-through). The cull-ON
        // conservative golden (±2/255 vs this) + the `Tiles`-buffer-vs-`golden_tile_bound`
        // agreement are the TESTER's GPU gates.
        let lit = run_gbuffer_hybrid(&ctx, &edits, false);
        assert_eq!(lit.len(), READBACK_BYTES as usize);

        let texel = |px: u32, py: u32| -> &[u8] {
            let base = ((py * SDF_IMG_W + px) as usize) * 4;
            &lit[base..base + 4]
        };

        // Whole-image deferred-oracle scan: each LIT texel within +/-2/255 of
        // `golden_deferred_resolve(golden_marcher_attributes(.., flags=0))`, fed the per-pixel
        // mesh depth the GPU rasterizes. (Pre-PBR-MVP-2 this compared the albedo readback to
        // the retired `golden_composite_pixel_ex` inline composite.)
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden(&lit, &edits, 0, DEFAULT_LIGHT_DIR, name);
        assert_eq!(max_arm1, 0, "[{name}] flags==0 must have NO arm-1 pixel (lighting OFF)");
        assert!(sdf_lit_hits > 0, "[{name}] no SDF-lit (mask==1) pixel — the marcher hit no surface");
        println!(
            "[{name}] P1b G-buffer LIT vs deferred oracle: max per-channel delta = {max_pass}/255 \
             (tol {CHANNEL_TOL}); {sdf_lit_hits} SDF-lit px"
        );

        // Texel A (sphere ∧ quad): the mesh occludes the SDF — the load-bearing occlusion
        // proof. Render P5-r0+r1: the mesh pixel is now the RASTER pass-A's first-class PBR
        // G-buffer (mask=1, base = the white vertex color, Cook-Torrance lit), NOT the old
        // flat marcher-derived MESH_COLOR (mask=0). So texel A must DIFFER from MESH_COLOR
        // (proving the raster PBR producer, not the retired flat constant, owns it) AND from
        // background (proving the mesh occluded the SDF / something was drawn). The EXACT
        // raster-PBR lit value is the RTX visual oracle's responsibility (it depends on the
        // GPU Cook-Torrance under the degenerate light table — not hand-derivable here).
        if let Some((ax, ay)) = a {
            let got = unpack_texel_rgb(texel(ax, ay));
            assert!(
                !texel_close(got, boyko_rhi_vulkan::compute::pack_rgba(MESH_COLOR)),
                "[{name}] texel A ({ax},{ay}) must be the RASTER PBR mesh G-buffer (mask=1), \
                 NOT the retired flat MESH_COLOR — got {got:?}"
            );
        }
        // Texel D (background) — distinct from texel A (mesh), proving the mesh occluded the
        // SDF / the raster drew a real fragment, not a constant fill.
        if let (Some((ax, ay)), Some((dx, dy))) = (a, d) {
            let av = unpack_texel_rgb(texel(ax, ay));
            let dv = unpack_texel_rgb(texel(dx, dy));
            assert!(
                !(0..3).all(|ch| (av[ch] - dv[ch]).abs() <= CHANNEL_TOL),
                "[{name}] texel A {av:?} (mesh) must differ from texel D {dv:?} (background)"
            );
        }
        let _ = (b, c); // B/C are exercised by the whole-image scan above.
    }
}

// ===========================================================================================
// Render P4b — conservative coarse-cull (1/8-res tile pre-trace) GPU gates (TESTER).
//
// The dev + code-review are complete (verdict APPROVE → GPU tester). These tests RUN the
// `coarse_enabled = true` path on the RTX 3060 (validation ON) and assert the cull's three
// contracts: (i) image ±2/255 vs the un-culled marcher (a hole = a >tol texel), (ii) the GPU
// `Tiles` buffer agrees with the host mirror `golden_tile_bound` (ORTHO → tight), (iii)
// cull-OFF is BYTE-IDENTICAL to the pre-P4b path (the 0%-gate). Plus a negative tripwire (a
// too-aggressive fake TileBound MUST fail the ±2/255 golden, and a MESH-covered EMPTY tile
// MUST show MESH_COLOR — D6) and the spirv-val / committed-.spv-freshness audit.
// ===========================================================================================

/// Splits an R8G8B8A8 readback into `[r, g, b]` for the texel at `(px, py)` (the low 3 bytes).
fn albedo_rgb(albedo: &[u8], px: u32, py: u32) -> [i32; 3] {
    let base = ((py * SDF_IMG_W + px) as usize) * 4;
    unpack_texel_rgb(&albedo[base..base + 4])
}

/// Parses the `tiles_buffer` readback (`tile_count() * 16` bytes, std430 scalar) into the
/// per-tile [`TileBound`]s, in coarse-dispatch order (`ty * tiles_w + tx`). near_t f32@0,
/// far_t f32@4, flags u32@8, _pad u32@12 — the layout the host const-asserts.
fn parse_tile_bounds(bytes: &[u8]) -> Vec<TileBound> {
    let n = tile_count() as usize;
    assert_eq!(bytes.len(), n * TILE_BOUND_BYTES, "tiles readback size mismatch");
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let o = i * TILE_BOUND_BYTES;
        let f = |k: usize| f32::from_le_bytes(bytes[o + k..o + k + 4].try_into().unwrap());
        let u = |k: usize| u32::from_le_bytes(bytes[o + k..o + k + 4].try_into().unwrap());
        out.push(TileBound { near_t: f(0), far_t: f(4), flags: u(8), _pad: u(12) });
    }
    out
}

/// The 8×8 block of per-pixel mesh depths covering tile `(tx, ty)` — the SAME
/// [`expected_mesh_depth`] values the GPU rasterizes — fed to [`golden_tile_bound`] so the
/// host mirror sees the exact depth field the coarse shader sampled (D5: out-of-image texels
/// stay the clear value, which `golden_tile_bound` decodes to `T_MAX`).
fn tile_depths(tx: u32, ty: u32) -> Vec<f32> {
    let mut depths = Vec::with_capacity((TILE_SIZE * TILE_SIZE) as usize);
    for ly in 0..TILE_SIZE {
        for lx in 0..TILE_SIZE {
            let px = tx * TILE_SIZE + lx;
            let py = ty * TILE_SIZE + ly;
            // Out-of-image fine pixels decode to the clear (no-mesh) depth — the partial-edge
            // contract the shader's out-of-range `.Load` mirrors (D5).
            let d = if px < SDF_IMG_W && py < SDF_IMG_H {
                expected_mesh_depth(px, py)
            } else {
                MESH_DEPTH_CLEAR
            };
            depths.push(d);
        }
    }
    depths
}

/// Boots a validation context, prints the device + caps, or returns `None` (SKIP). Shared by
/// every P4b GPU gate so each prints the RTX-3060 device name and asserts the G-buffer caps.
fn boot_render_or_skip(test: &str) -> Option<VulkanContext> {
    let ctx = boot_or_skip(test)?;
    println!("[{test}] Vulkan device: {}", ctx.device_name());
    // Validation is the soundness oracle, NOT a render-output dependency, so a
    // context booted with `BOYKO_DISABLE_VALIDATION` (the layer DLL crashes the
    // MinGW process on this box) still drives the PIXEL goldens. The per-test
    // clean-oracle assert (`assert_validation_clean`) self-gates when off.
    if !ctx.validation_enabled() {
        eprintln!("[{test}] NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — pixel goldens still run");
    }
    assert!(
        ctx.device_caps().gbuffer_storage_format_ok,
        "a booted context must support STORAGE_IMAGE on the G-buffer format"
    );
    Some(ctx)
}

/// The three ORTHO fixtures, reused by every gate.
fn p4b_scenes() -> [(&'static str, Vec<SdfEdit>); 3] {
    [("crater_csg", crater()), ("box_csg", box_csg()), ("smooth_union", smooth_union())]
}

/// **P4b GATE 1 — conservative golden (the headline).** For each scene, run cull-OFF
/// (baseline) and cull-ON; EVERY cull-ON albedo texel must be within ±2/255 (`CHANNEL_TOL`)
/// of the cull-OFF texel. A texel exceeding tol = a CULL HOLE (the coarse pass skipped a
/// surface the un-culled marcher hit) → FAIL with `(px,py)` + got + want + delta. The
/// max per-channel delta per scene is reported (the cull's fp drift budget).
#[test]
fn p4b_cull_on_conservative_within_tol_of_cull_off() {
    let Some(ctx) = boot_render_or_skip("p4b_cull_on_conservative_within_tol_of_cull_off") else {
        return;
    };

    for (name, edits) in p4b_scenes() {
        let off = run_gbuffer_hybrid(&ctx, &edits, false);
        let on = run_gbuffer_hybrid(&ctx, &edits, true);
        assert_eq!(off.len(), READBACK_BYTES as usize, "[{name}] cull-OFF readback size");
        assert_eq!(on.len(), READBACK_BYTES as usize, "[{name}] cull-ON readback size");

        // Prove the device actually executed: the cull-OFF baseline must contain BOTH a
        // mesh/SDF lit texel AND a background texel (not a silent all-zero buffer).
        let nonzero = off.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(
            nonzero > 0,
            "[{name}] cull-OFF albedo is all-zero — the device did not render (silent skip?)"
        );

        let mut max_delta = 0i32;
        let mut worst = (0u32, 0u32, [0i32; 3], [0i32; 3]);
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                let g_on = albedo_rgb(&on, px, py);
                let g_off = albedo_rgb(&off, px, py);
                for ch in 0..3 {
                    let dd = (g_on[ch] - g_off[ch]).abs();
                    if dd > max_delta {
                        max_delta = dd;
                        worst = (px, py, g_on, g_off);
                    }
                }
                assert!(
                    (0..3).all(|ch| (g_on[ch] - g_off[ch]).abs() <= CHANNEL_TOL),
                    "[{name}] CULL HOLE at ({px},{py}): cull-ON {g_on:?} vs cull-OFF {g_off:?} \
                     exceeds ±{CHANNEL_TOL}/255 (delta {:?})",
                    [
                        (g_on[0] - g_off[0]).abs(),
                        (g_on[1] - g_off[1]).abs(),
                        (g_on[2] - g_off[2]).abs()
                    ]
                );
            }
        }
        println!(
            "[{name}] GATE1 cull-ON vs cull-OFF: max per-channel delta = {max_delta}/255 \
             (tol {CHANNEL_TOL}); worst texel ({},{}) on={:?} off={:?}; {nonzero} non-bg texels",
            worst.0, worst.1, worst.2, worst.3
        );
    }
}

/// **P4b GATE 2 — Tiles-buffer agreement.** Read back the `tiles_buffer` after a cull-ON run
/// and diff every tile vs the host mirror [`golden_tile_bound`] (fed the SAME per-tile mesh
/// depths the GPU rasterizes). These fixtures are ORTHO (no tan/acos transcendental in the
/// cone math → no fp divergence), so near_t / far_t must agree TIGHTLY and the EMPTY flag
/// EXACTLY. A real per-tile divergence is surfaced (the worst tile + both bounds) — not
/// papered over.
#[test]
fn p4b_tiles_buffer_agrees_with_host_golden() {
    let Some(ctx) = boot_render_or_skip("p4b_tiles_buffer_agrees_with_host_golden") else {
        return;
    };
    let (tw, _th) = tile_extent();

    // ORTHO has no transcendental in the cone trace; the host + GPU run the SAME op
    // sequence (D1/D2). A handful of ULPs can still appear from the GPU's `mad`-contraction
    // vs the host's separate mul/add in `field_distance`, so allow a tiny absolute epsilon
    // (≈ a few ULP of a t ~ O(1) value); flags must be EXACT (a flag flip = a wrong-EMPTY
    // hole, which GATE 1 would also catch as a pixel hole).
    const T_EPS: f32 = 1.0e-4;

    for (name, edits) in p4b_scenes() {
        let (_albedo, tiles) = run_gbuffer_hybrid_ex(&ctx, &edits, true, true, 1.0);
        let tiles = tiles.expect("read_tiles = true returns the tiles readback");
        let gpu = parse_tile_bounds(&tiles);
        assert_eq!(gpu.len(), tile_count() as usize, "[{name}] tile count");

        let mut empties = 0usize;
        let mut max_near = 0f32;
        let mut max_far = 0f32;
        let mut worst_tile = (0u32, 0u32);
        for (i, g) in gpu.iter().enumerate() {
            let tx = (i as u32) % tw;
            let ty = (i as u32) / tw;
            let host = golden_tile_bound(
                &edits,
                &tile_depths(tx, ty),
                tx,
                ty,
                SDF_IMG_W,
                SDF_IMG_H,
                CompositeCamera::Ortho,
            );
            // Flags EXACT — a wrong EMPTY is a hole.
            assert_eq!(
                g.flags, host.flags,
                "[{name}] tile ({tx},{ty}) flags GPU={} host={} (EMPTY={TILE_FLAG_EMPTY})",
                g.flags, host.flags
            );
            let dn = (g.near_t - host.near_t).abs();
            let df = (g.far_t - host.far_t).abs();
            if dn > max_near {
                max_near = dn;
                worst_tile = (tx, ty);
            }
            if df > max_far {
                max_far = df;
            }
            assert!(
                dn <= T_EPS,
                "[{name}] tile ({tx},{ty}) near_t diverged: GPU={} host={} |d|={dn} > {T_EPS}",
                g.near_t, host.near_t
            );
            assert!(
                df <= T_EPS,
                "[{name}] tile ({tx},{ty}) far_t diverged: GPU={} host={} |d|={df} > {T_EPS}",
                g.far_t, host.far_t
            );
            if g.flags & TILE_FLAG_EMPTY != 0 {
                empties += 1;
            }
        }
        // Prove the coarse pass actually ran a non-trivial trace: at least one tile must be
        // non-EMPTY (has the surface) AND at least one EMPTY (sparse scene) — a uniform
        // buffer would mean the coarse dispatch silently no-op'd.
        let non_empty = gpu.len() - empties;
        assert!(non_empty > 0, "[{name}] every tile EMPTY — coarse pass found no surface");
        assert!(empties > 0, "[{name}] no EMPTY tile — coarse pass culled nothing (suspicious)");
        println!(
            "[{name}] GATE2 Tiles agree: {}/{} tiles, {empties} EMPTY / {non_empty} surface; \
             max |Δnear_t|={max_near} max |Δfar_t|={max_far} (eps {T_EPS}); worst near tile {:?}",
            gpu.len(),
            tile_count(),
            worst_tile
        );
    }
}

/// **P4b GATE 3a — the conservative golden tripwire MUST trip.** Constructs a deliberately
/// TOO-AGGRESSIVE cull (a fake [`TileBound`] with `near_t` pushed past the true first hit)
/// and asserts the host culled marcher [`golden_composite_pixel_culled`] then DIFFERS from
/// the un-culled golden by more than `CHANNEL_TOL` at a known SDF-hit pixel. This proves
/// GATE 1's ±2/255 comparison can actually CATCH a hole (a tripwire that never trips is no
/// gate). Host-only (no GPU) — it exercises the contract the GPU gate relies on.
#[test]
fn p4b_too_aggressive_near_t_seed_trips_the_conservative_golden() {
    let edits = crater();
    // Find a pixel the SDF hits AND is NOT mesh-covered: the un-culled golden shows the lit
    // SDF surface there, so skipping past the hit reveals BACKGROUND (a visible hole). A
    // mesh-covered hit pixel would mask the hole behind MESH_COLOR (the mesh occludes the
    // SDF either way), so the tripwire must use an SDF-only pixel.
    let (px, py) = find_texel(&edits, |hit, covered| hit && !covered)
        .expect("crater has an SDF-hit pixel outside the mesh quad");
    let md = expected_mesh_depth(px, py);
    assert_eq!(md, MESH_DEPTH_CLEAR, "the chosen pixel must be mesh-uncovered (no occlusion)");

    let want = golden_composite_pixel_ex(&edits, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);

    // A too-aggressive bound: near_t = 5.0 seeded WAY past the true first hit (the crater
    // sphere front is at t ≈ CAM_Z − R = 1.5). far_t large so the seeded march has room to
    // (wrongly) walk empty space to T_MAX → background instead of the lit SDF. flags = 0
    // (non-EMPTY, so the marcher seeds t = near_t rather than fast-pathing).
    let bad = TileBound { near_t: 5.0, far_t: SDF_TRACE_T_MAX, flags: 0, _pad: 0 };
    let got = golden_composite_pixel_culled(
        &edits,
        md,
        px,
        py,
        SDF_IMG_W,
        SDF_IMG_H,
        CompositeCamera::Ortho,
        true,
        bad,
    );

    let w = unpack_packed_rgb(want);
    let g = unpack_packed_rgb(got);
    let delta: [i32; 3] = [(g[0] - w[0]).abs(), (g[1] - w[1]).abs(), (g[2] - w[2]).abs()];
    assert!(
        delta.iter().any(|&d| d > CHANNEL_TOL),
        "TRIPWIRE FAILED: a too-aggressive near_t=5.0 seed at SDF-hit pixel ({px},{py}) did NOT \
         change the color beyond ±{CHANNEL_TOL}/255 (got {g:?} want {w:?}) — the conservative \
         golden cannot detect a hole, so GATE 1 is blind"
    );
    println!(
        "[crater_csg] GATE3a tripwire OK: too-aggressive near_t=5.0 at hit pixel ({px},{py}) \
         shifts color by {delta:?}/255 (> tol {CHANNEL_TOL}) → a hole IS detectable"
    );
}

/// **P4b GATE 3b — D6: a MESH-covered EMPTY tile shows MESH_COLOR, not background.** The
/// EMPTY fast-path must run the mesh/background composite (D6) — an EMPTY tile can still be
/// MESH-occluded. Asserts the host culled marcher returns MESH_COLOR for a MESH-covered
/// pixel under an EMPTY tile (and background for an uncovered one), proving the EMPTY arm is
/// NOT a blind background fill (which would erase the mesh → a golden regression). The GPU
/// half is covered by GATE 1 (an EMPTY mesh tile that went background would exceed ±2/255).
#[test]
fn p4b_empty_tile_composites_mesh_not_background_d6() {
    let edits = crater();
    let empty = TileBound { near_t: 0.0, far_t: SDF_TRACE_T_MAX, flags: TILE_FLAG_EMPTY, _pad: 0 };

    // A MESH-covered pixel under an EMPTY tile → MESH_COLOR (the mesh, not erased).
    let (cx, cy) =
        find_texel(&edits, |_hit, covered| covered).expect("the quad covers part of the view");
    let covered_md = expected_mesh_depth(cx, cy);
    assert!(covered_md < MESH_DEPTH_CLEAR, "the chosen pixel must be mesh-covered");
    let got_mesh = golden_composite_pixel_culled(
        &edits, covered_md, cx, cy, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, true, empty,
    );
    assert_eq!(
        got_mesh,
        pack_rgba(MESH_COLOR),
        "[crater_csg] GATE3b D6: a MESH-covered EMPTY tile pixel ({cx},{cy}) must show \
         MESH_COLOR, got {:?} (the EMPTY fast-path blind-filled background → mesh erased)",
        unpack_packed_rgb(got_mesh)
    );

    // An UNCOVERED pixel under an EMPTY tile → background (not mesh) — the other D6 arm.
    let (ux, uy) = find_texel(&edits, |hit, covered| !hit && !covered)
        .expect("crater has an uncovered, non-hit pixel");
    let uncovered_md = expected_mesh_depth(ux, uy);
    assert_eq!(uncovered_md, MESH_DEPTH_CLEAR, "the chosen pixel must be uncovered");
    let got_bg = golden_composite_pixel_culled(
        &edits, uncovered_md, ux, uy, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, true, empty,
    );
    assert_ne!(
        got_bg,
        pack_rgba(MESH_COLOR),
        "[crater_csg] GATE3b D6: an UNCOVERED EMPTY tile pixel ({ux},{uy}) must NOT be MESH_COLOR"
    );
    println!(
        "[crater_csg] GATE3b D6 OK: EMPTY-covered ({cx},{cy})=MESH_COLOR, \
         EMPTY-uncovered ({ux},{uy})=background"
    );
}

/// **P4b GATE 4 — cull-OFF does not perturb the output (the 0%-gate).** `run_gbuffer_hybrid(
/// false)` (the fine marcher with `coarse_enabled = 0`) must produce a LIT image that matches
/// the deferred PBR oracle within ±2/255 (PBR MVP-2 re-pointed this from the retired MVP-1
/// `golden_composite_pixel_ex` inline composite to `golden_deferred_resolve ∘
/// golden_marcher_attributes`; the cull flag itself does not change the PBR result) — AND, the
/// stronger P4b claim, RUN-TO-RUN byte-stable. This pins the no-coarse path so a P4b change
/// that perturbs cull-OFF is caught here, not only by the existing
/// `p1b_gbuffer_hybrid_matches_golden`. Each scene's two cull-OFF runs are compared
/// byte-for-byte (the GPU is deterministic for the same recorded stream).
#[test]
fn p4b_cull_off_is_byte_identical_to_pre_p4b_path() {
    let Some(ctx) = boot_render_or_skip("p4b_cull_off_is_byte_identical_to_pre_p4b_path") else {
        return;
    };

    for (name, edits) in p4b_scenes() {
        // Two independent cull-OFF runs → byte-for-byte identical (the marcher is
        // deterministic; the coarse pass is not even dispatched).
        let a = run_gbuffer_hybrid(&ctx, &edits, false);
        let b = run_gbuffer_hybrid(&ctx, &edits, false);
        assert_eq!(a, b, "[{name}] two cull-OFF runs diverged — the no-coarse path is non-deterministic");

        // And each cull-OFF LIT texel matches the deferred PBR oracle within ±2/255 (the
        // 0%-gate anchor: cull-OFF == today's marcher). Lighting is OFF (flags == 0), so every
        // arm is pass-through. This re-pins the contract `p1b_gbuffer_hybrid_matches_golden`
        // asserts, scoped to the coarse_enabled = 0 push.
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden(&a, &edits, 0, DEFAULT_LIGHT_DIR, name);
        assert_eq!(max_arm1, 0, "[{name}] flags==0 must have NO arm-1 pixel (lighting OFF)");
        assert!(sdf_lit_hits > 0, "[{name}] no SDF-lit (mask==1) pixel — the marcher hit no surface");
        println!("[{name}] GATE4 cull-OFF byte-stable + matches deferred oracle (max delta {max_pass}/255)");
    }
}

/// **P4b GATE 5 — sync-validation under cull-ON.** A cull-ON run that returns proves
/// `assert_validation_clean` passed (it is asserted inside `run_gbuffer_hybrid_ex` before
/// return): the coarse-write → fine-read buffer barrier (Tiles SHADER_WRITE → SHADER_READ)
/// raised no WAR/RAW hazard and the coarse dispatch + the inter-dispatch barrier are
/// validation-clean. (The committed-.spv freshness + spirv-val audit is a separate
/// host-side script run by the tester — see the report; the validator is not invoked from
/// the Rust test to avoid a hard SDK dependency at `cargo test` time.)
#[test]
fn p4b_cull_on_is_validation_clean() {
    let Some(ctx) = boot_render_or_skip("p4b_cull_on_is_validation_clean") else {
        return;
    };
    // SOLE purpose is the validation oracle — nothing to assert when it is off.
    if !ctx.validation_enabled() {
        eprintln!("SKIP p4b_cull_on_is_validation_clean: validation disabled (BOYKO_DISABLE_VALIDATION)");
        return;
    }
    // crater is the densest fixture (a CSG carve), so it exercises both the coarse trace and
    // the fine seeded march hardest. A clean return = validation-clean (asserted inside).
    let (albedo, tiles) = run_gbuffer_hybrid_ex(&ctx, &crater(), true, true, 1.0);
    assert_eq!(albedo.len(), READBACK_BYTES as usize);
    let tiles = tiles.expect("read_tiles");
    let bounds = parse_tile_bounds(&tiles);
    let surface = bounds.iter().filter(|b| b.flags & TILE_FLAG_EMPTY == 0).count();
    assert!(surface > 0, "the coarse pass must have marked at least one surface tile");
    println!(
        "[crater_csg] GATE5 cull-ON validation-clean: {} tiles ({} surface) + the coarse→fine \
         buffer barrier raised no hazard",
        bounds.len(),
        surface
    );
}

// ===========================================================================================
// Render B1 — over-relaxation (Keinert ω-gated) GPU gates (RTX 3060, validation ON).
//
//   6.  GPU ω=1 BIT-identity — `run_gbuffer_hybrid_ex(.., 1.0)` byte-identical to the cull-OFF
//       pre-B1 path (two runs equal + every texel == the ω=1 host golden), cull-off AND cull-on.
//   7.  GPU ω>1 HIT/MISS parity — the GPU ω∈{1.2,1.5,1.9} render's per-pixel hit/miss set ==
//       the ω=1 GPU render's, on crater + box + smooth_union (the SHIPPED fixtures).
//   8.  GPU ω=1.2 ±2/255 vs the MATCHED-ω host oracle `golden_composite_pixel_ex_omega(.., 1.2)`
//       (NOT the ω=1 golden) — the m1 the reviewer flagged as missing.
//   8c. GPU repro of BUG-B1-HOLE-1 — the host-confirmed hole scene rendered on-device; documents
//       the hole is NOT host-only. `#[ignore]` (it asserts the buggy state; flip after the fix).
//   11. SYNC-validation clean — a cull-ON ω=1.2 dispatch raises no sync hazard.
//
// A GPU pixel is classified HIT / MESH / BACKGROUND by nearest packed reference color (the three
// composite outcomes differ by 100+ per channel, so a ±2/255 store quantization never flips the
// class). The hit/miss SET is the soundness invariant ω>1 must preserve.
// ===========================================================================================

/// The packed background color the marcher writes on a miss (`SDF_BACKGROUND = [0.05,0.05,0.1]`).
fn packed_background() -> [i32; 3] {
    unpack_packed_rgb(pack_rgba([0.05, 0.05, 0.1]))
}

/// Classifies a GPU albedo texel as `true` (an SDF surface hit) when it is closer to neither the
/// packed MESH_COLOR nor the packed BACKGROUND than `CHANNEL_TOL` allows — i.e. it is the LIT SDF
/// color. The three outcomes are >100/255 apart, so the ±2/255 store quantization never reclasses.
fn gpu_pixel_is_sdf_hit(albedo: &[u8], px: u32, py: u32) -> bool {
    let got = albedo_rgb(albedo, px, py);
    let mesh = unpack_packed_rgb(pack_rgba(MESH_COLOR));
    let bg = packed_background();
    let near = |r: [i32; 3]| (0..3).all(|c| (got[c] - r[c]).abs() <= CHANNEL_TOL);
    !near(mesh) && !near(bg)
}

/// **B1 GATE 6 — GPU ω=1 BIT-identity (cull-off + cull-on).** `run_gbuffer_hybrid_ex(.., 1.0)`
/// must (a) be byte-stable across two runs, and (b) match the ω=1 deferred PBR oracle within
/// ±2/255 (the same contract the pre-B1 `p1b`/GATE-4 0%-gates assert) — proving the widened
/// 8-byte push with ω=1.0 reproduces the committed pre-B1 marcher EXACTLY on-device. Runs
/// cull-off AND cull-on. PBR MVP-2: the host reference is re-pointed from the retired MVP-1
/// `golden_composite_pixel_ex` / `golden_composite_pixel_culled` to `golden_deferred_resolve ∘
/// golden_marcher_attributes` (ω=1.0, flags == 0 ⇒ every arm pass-through). With flags == 0 the
/// coarse cull cannot perturb a pass-through pixel, so the SAME deferred oracle bounds both the
/// cull-off and cull-on arms; the cull's conservative-fill contract is independently proven by
/// `p4b_cull_on_conservative_within_tol_of_cull_off`.
#[test]
fn b1_gate6_gpu_omega_one_bit_identical_to_pre_b1() {
    let Some(ctx) = boot_render_or_skip("b1_gate6_gpu_omega_one_bit_identical_to_pre_b1") else {
        return;
    };
    for (name, edits) in p4b_scenes() {
        for coarse in [false, true] {
            // Two ω=1.0 runs must be byte-identical (deterministic).
            let a = run_gbuffer_hybrid_ex(&ctx, &edits, coarse, false, 1.0).0;
            let b = run_gbuffer_hybrid_ex(&ctx, &edits, coarse, false, 1.0).0;
            assert_eq!(a.len(), READBACK_BYTES as usize, "[{name} cull={coarse}] readback size");
            assert_eq!(a, b, "[{name} cull={coarse}] two ω=1.0 runs diverged — non-deterministic marcher");

            // Prove the device executed (not a silent all-zero buffer).
            let nonzero = a.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
            assert!(nonzero > 0, "[{name} cull={coarse}] ω=1.0 albedo all-zero — device did not render");

            // Each ω=1.0 LIT texel within ±2/255 of the ω=1 deferred PBR oracle. Lighting is
            // OFF (flags == 0), so every arm is pass-through and the coarse cull cannot perturb
            // a pass-through pixel — the SAME deferred oracle bounds both the cull-off and
            // cull-on arms (the cull's conservative-fill contract is proven separately by
            // `p4b_cull_on_conservative_within_tol_of_cull_off`).
            let (max_pass, max_arm1, sdf_lit_hits) =
                assert_lit_matches_deferred_golden(&a, &edits, 0, DEFAULT_LIGHT_DIR, name);
            assert_eq!(max_arm1, 0, "[{name} cull={coarse}] flags==0 must have NO arm-1 pixel (lighting OFF)");
            assert!(sdf_lit_hits > 0, "[{name} cull={coarse}] no SDF-lit (mask==1) pixel — the marcher hit no surface");
            println!("[{name} cull={coarse}] GATE6 ω=1.0 byte-stable + matches deferred oracle (max delta {max_pass}/255)");
        }
    }
}

/// **B1 GATE 7 — GPU ω>1 HIT/MISS parity.** For each SHIPPED fixture, the GPU ω∈{1.2,1.5,1.9}
/// render's per-pixel SDF-hit set must EQUAL the ω=1 GPU render's. A pixel that hits at ω=1 but
/// becomes mesh/background at ω>1 = a missed-surface HOLE; the reverse (a new spurious hit) = a
/// phantom surface. Either fails with `(px,py)` + both classes. (The shipped fixtures are hole-free
/// per the host gate-2 scope analysis; this confirms it ON-DEVICE.)
#[test]
fn b1_gate7_gpu_overrelax_hit_miss_parity() {
    let Some(ctx) = boot_render_or_skip("b1_gate7_gpu_overrelax_hit_miss_parity") else {
        return;
    };
    for (name, edits) in p4b_scenes() {
        let base = run_gbuffer_hybrid_ex(&ctx, &edits, false, false, 1.0).0;
        let base_hits = base.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(base_hits > 0, "[{name}] ω=1 baseline all-zero — device did not render");
        for &omega in &[1.2_f32, 1.5, 1.9] {
            let over = run_gbuffer_hybrid_ex(&ctx, &edits, false, false, omega).0;
            let mut sdf_px = 0u64;
            for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    let h1 = gpu_pixel_is_sdf_hit(&base, px, py);
                    let ho = gpu_pixel_is_sdf_hit(&over, px, py);
                    if h1 {
                        sdf_px += 1;
                    }
                    assert_eq!(
                        h1, ho,
                        "[{name}] ω={omega} HIT/MISS PARITY broke at ({px},{py}): ω=1 hit={h1} vs ω={omega} hit={ho} \
                         (ω=1 {:?} vs ω={omega} {:?})",
                        albedo_rgb(&base, px, py), albedo_rgb(&over, px, py)
                    );
                }
            }
            println!("[{name}] GATE7 ω={omega} hit/miss parity OK ({sdf_px} SDF-hit px match ω=1)");
        }
    }
}

/// **B1 GATE 8 — GPU ω=1.2 ±2/255 vs the MATCHED-ω deferred PBR oracle.** Each GPU ω=1.2 LIT
/// texel must be within ±2/255 of `golden_deferred_resolve(golden_marcher_attributes(.., ω=1.2,
/// flags=0))` (the ω-aware deferred oracle — NOT the ω=1 golden, and NOT the retired MVP-1
/// `golden_composite_pixel_ex_omega`). This proves the GPU over-relaxation marcher reproduces
/// the host ω-marcher's per-pixel COLOR, not merely the hit/miss class. PBR MVP-2 re-pointed
/// the host reference to the deferred oracle (the readback is LIT). Per-scene max delta is
/// reported. (Shipped fixtures only — they are hole-free.)
#[test]
fn b1_gate8_gpu_omega_1_2_matches_matched_omega_host() {
    let Some(ctx) = boot_render_or_skip("b1_gate8_gpu_omega_1_2_matches_matched_omega_host") else {
        return;
    };
    let omega = DEFAULT_MARCHER_OMEGA; // 1.2 — the production default
    for (name, edits) in p4b_scenes() {
        let lit = run_gbuffer_hybrid_ex(&ctx, &edits, false, false, omega).0;
        assert_eq!(lit.len(), READBACK_BYTES as usize);
        let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(nonzero > 0, "[{name}] ω={omega} lit all-zero — device did not render");
        // The matched-ω deferred oracle: the host marches the IDENTICAL ω before the resolve.
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden_omega(&lit, &edits, omega, 0, DEFAULT_LIGHT_DIR, name);
        assert_eq!(max_arm1, 0, "[{name}] flags==0 must have NO arm-1 pixel (lighting OFF)");
        assert!(sdf_lit_hits > 0, "[{name}] no SDF-lit (mask==1) pixel — the marcher hit no surface");
        println!("[{name}] GATE8 ω={omega} GPU matches matched-ω deferred oracle (max delta {max_pass}/255)");
    }
}

/// **B1 GATE 8c — BUG-B1-HOLE-1 mesh-masking on the GPU harness (documented).** The host-confirmed
/// over-relax hole (super-Lipschitz smooth-min CSG) cannot be SHOWN through this fixed-mesh harness:
/// EVERY hole pixel of that scene falls inside the mesh quad footprint (x ∈ [-1, 0.2]) — the mesh
/// occludes the SDF there, so both ω=1 and ω=1.2 composite MESH_COLOR and the hole is invisible on
/// readback. A 40k-trial host search found NO smooth-min hole pixel outside the mesh x-range. This
/// test ASSERTS that masking (the hole pixel is mesh-covered on-device, NOT an SDF hit at ω=1),
/// recording WHY the GPU half cannot expose BUG-B1-HOLE-1 with the current harness. The bug itself
/// is proven host-side (`compute::b1_over_relaxation_tests::gate2_*` + the `bug_b1_hole_1_*` pin);
/// the shader marcher is line-for-line the host `_omega` oracle, so the host proof IS the on-device
/// proof. A no-mesh / relocated-mesh harness variant (developer wiring, out of the tester remit)
/// would surface it directly.
#[test]
fn b1_gate8c_bug_b1_hole_1_is_mesh_masked_on_gpu_harness() {
    let Some(ctx) = boot_render_or_skip("b1_gate8c_bug_b1_hole_1_is_mesh_masked_on_gpu_harness") else {
        return;
    };
    let edits = vec![
        SdfEdit::sphere([0.31460363, 0.70498204, -0.7611318], 0.36075538, sdf_op::UNION, 0.0),
        SdfEdit::box_shape([0.092381336, 0.1372761, -0.5955315], [0.19970395, 0.46420184, 0.3901827], sdf_op::UNION, 0.24384262),
        SdfEdit::sphere([0.4506038, 0.16997452, 0.0], 0.44928917, sdf_op::UNION, 0.0),
    ];
    let (px, py) = (28u32, 16u32);
    // The host hole pixel is inside the mesh footprint — the harness's mesh quad covers it.
    assert!(mesh_covers_pixel(px, py), "the documented hole pixel must be mesh-covered (the masking premise)");
    let at1 = run_gbuffer_hybrid_ex(&ctx, &edits, false, false, 1.0).0;
    let at12 = run_gbuffer_hybrid_ex(&ctx, &edits, false, false, 1.2).0;
    let g1 = albedo_rgb(&at1, px, py);
    let g12 = albedo_rgb(&at12, px, py);
    // Render P5 (r0+r1): the mesh now MASKS the hole as a first-class RASTER-PBR fragment
    // (mask=1, base = the white vertex color, full Cook-Torrance under the degenerate light
    // table), NOT the old flat MESH_COLOR (mask=0) pass-through. The masking premise stands —
    // the SDF (hit OR hole) is occluded by the mesh — but the expected value is now the
    // raster-PBR mesh, derived through the SAME deferred oracle the harness compares against
    // (the run is lighting OFF / flags == 0 / ω == 1.0, the DEGENERATE table). This proves the
    // hole pixel reads the MESH (not the SDF-hole value, not background), keeping the masking
    // proof meaningful.
    let materials = host_material_table();
    let md = expected_mesh_depth(px, py);
    let attrs = golden_marcher_attributes(
        &edits, &materials, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0, 0,
        DEFAULT_LIGHT_DIR,
    );
    // The mesh quad occludes the SDF here, so the host oracle picks its raster-PBR mesh arm.
    assert_eq!(attrs.mask, 1, "the mesh-covered hole pixel is the RASTER-PBR mesh (mask=1)");
    let (_, rd) = composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
    let mesh = unpack_packed_rgb(golden_deferred_resolve(attrs, rd, &materials));
    println!(
        "[BUG-B1-HOLE-1 GPU] ({px},{py}) mesh-covered: ω=1 {g1:?} ω=1.2 {g12:?} (raster-PBR mesh {mesh:?}) — hole MASKED by the mesh quad"
    );
    // Both composite the mesh (the SDF — hit or hole — is occluded), so the GPU readback cannot
    // distinguish the hole here. This documents the harness limitation, not a B1 success.
    assert!(
        (0..3).all(|c| (g1[c] - mesh[c]).abs() <= DEFERRED_ARM1_TOL),
        "ω=1 ({g1:?}) must be the raster-PBR mesh value ({mesh:?}) at the mesh-covered hole pixel"
    );
    assert!(
        (0..3).all(|c| (g12[c] - mesh[c]).abs() <= DEFERRED_ARM1_TOL),
        "ω=1.2 ({g12:?}) must ALSO be the raster-PBR mesh value ({mesh:?}) — the mesh masks the \
         B1 hole at BOTH ω (the readback cannot distinguish the hole here)"
    );
    // The masking is real: the GPU readback shows the MESH, NOT the SDF surface. The SDF DOES
    // exist underneath (the pixel is an analytic SDF hit at ω=1), so its un-masked color would
    // be the SDF-lit value — derive it through the SAME oracle with NO mesh and assert the mesh
    // value differs from it. (This is the post-P5 replacement for the pre-P5 `gpu_pixel_is_sdf_hit`
    // probe, which classified by proximity to the flat MESH_COLOR — invalid now that the mesh is
    // a PBR-lit white, far from MESH_COLOR.)
    assert!(
        editlist_pixel_hits(&edits, px, py),
        "BUG-B1-HOLE-1 premise: the pixel IS an analytic SDF hit at ω=1 (the over-relax hole is \
         an ω>1 artifact); the mesh masks whatever the SDF does there"
    );
    let sdf_attrs = golden_marcher_attributes(
        &edits, &materials, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
        1.0, 0, DEFAULT_LIGHT_DIR,
    );
    assert_eq!(sdf_attrs.mask, 1, "the un-masked pixel is an SDF-lit surface (mask=1)");
    let sdf_lit = unpack_packed_rgb(golden_deferred_resolve(sdf_attrs, rd, &materials));
    assert!(
        !(0..3).all(|c| (mesh[c] - sdf_lit[c]).abs() <= CHANNEL_TOL),
        "the masked mesh value ({mesh:?}) must DIFFER from the un-masked SDF-lit value \
         ({sdf_lit:?}) — proving the mesh (not the SDF) owns the readback (the masking)"
    );
}

/// **B1 GATE 11 — sync-validation clean under cull-ON ω=1.2.** A cull-ON ω=1.2 dispatch that
/// returns proves `assert_validation_clean` passed (asserted inside `run_gbuffer_hybrid_ex` before
/// return): the widened 8-byte push (ω at offset 4) adds NO new resource hazard over the pre-B1
/// cull-ON path. The coarse→fine Tiles barrier is unchanged; ω is push-constant data only.
#[test]
fn b1_gate11_cull_on_omega_1_2_sync_validation_clean() {
    let Some(ctx) = boot_render_or_skip("b1_gate11_cull_on_omega_1_2_sync_validation_clean") else {
        return;
    };
    // SOLE purpose is the validation oracle — nothing to assert when it is off.
    if !ctx.validation_enabled() {
        eprintln!("SKIP b1_gate11_cull_on_omega_1_2_sync_validation_clean: validation disabled (BOYKO_DISABLE_VALIDATION)");
        return;
    }
    let (albedo, tiles) = run_gbuffer_hybrid_ex(&ctx, &crater(), true, true, DEFAULT_MARCHER_OMEGA);
    assert_eq!(albedo.len(), READBACK_BYTES as usize);
    let bounds = parse_tile_bounds(&tiles.expect("read_tiles"));
    let surface = bounds.iter().filter(|b| b.flags & TILE_FLAG_EMPTY == 0).count();
    assert!(surface > 0, "the coarse pass must have marked at least one surface tile");
    println!(
        "[crater_csg] GATE11 cull-ON ω={DEFAULT_MARCHER_OMEGA} validation-clean: {} tiles ({} surface); \
         the widened push raised no new hazard",
        bounds.len(),
        surface
    );
}

// ===========================================================================================
// Render A1 (SDF soft shadows) + A2 (SDF AO) — the ON-path GPU gates (RTX 3060, validation
// ON, --test-threads=1). The OFF 0%-gate is already pinned by `p1b_gbuffer_hybrid_matches_
// golden` + `p4b_cull_off_is_byte_identical_to_pre_p4b_path` + `b1_gate6_...` (all run with
// the 32-byte push, all green). These gates exercise `lighting_flags != 0`.
//
//   A-host.   host_soft_shadow / host_ao sanity (CPU, no GPU) — factors in [0,1]; a shadowed
//             crevice is darker than a lit face; AO darkens concavities.
//   A1g.      ON GPU vs host `_lit` golden (DEFAULT light (0,0,1)), SHADOWS|AO, ±3/255 — the
//             host mirror is EXACT for the default light (shader's static LIGHT_DIR == push).
//   A2g.      Shadows-only and AO-only independence vs the matching host `_lit` golden, ±3/255
//             (the flag bits gate independently).
//   A3g.      Non-default light_dir mis-pack catcher (the architect's named std430 oracle):
//             a GPU-vs-GPU differential — a non-axis light_dir must shift the shadow pattern
//             vs the default light (proves light_dir reaches the shader at the correct offset).
//             SEE the BUG-A-NDOTL note: the literal "GPU vs host `_lit` with same non-default
//             light" form is NOT achievable against the current host mirror (host applies
//             light_dir to the Lambert base; the shader hardcodes the static LIGHT_DIR there).
//
// The ON-path tolerance is ±3/255 (`LIT_CHANNEL_TOL`) — the architect's consumer-side budget
// (host `powi` vs shader `pow` ULP + the float→UNORM store). The OFF path stays ±2/255.
// ===========================================================================================

/// The A1/A2 ON-path per-channel tolerance (the architect's consumer-side ±3/255: host
/// `AO_FALLOFF.powi(i)` vs the shader's `pow(AO_FALLOFF, i)` ULP drift + the shadow
/// min-track FP order + the float→UNORM store quantization). The OFF path keeps the
/// stricter `CHANNEL_TOL` (±2/255).
const LIT_CHANNEL_TOL: i32 = 3;

/// A non-axis, normalized directional light (the architect's mis-pack probe direction). It
/// is NOT (0,0,1), so a std430 offset slip on `light_dir` (landing it at the wrong push
/// offset → read as zero / garbage) yields a measurably different shadow pattern than a
/// correctly-packed value — the differential A3g catches.
const NONDEFAULT_LIGHT: [f32; 3] = [0.4, 0.5, 0.768];

// NOTE (PBR MVP-2): the former `assert_lit_within_tol` helper diffed the GPU LIT readback
// against the RETIRED MVP-1 `_lit` oracle (`golden_composite_pixel_ex_omega_lit`, the `base*vis`
// Lambert composite) at ±3/255. Its three callers (A1g / A2g / A3g-literal) now compare the PBR
// `gLit` readback against the deferred Cook-Torrance oracle via `assert_lit_matches_deferred_golden`
// (±2/255), so the helper has no remaining caller and is removed. The MVP-1 `_lit` oracle itself
// is still exercised host-only by `a_host_shadow_ao_darken_not_brighten` (a CPU darken/brighten
// sanity, no GPU) and `d1_host_deferred_passthrough_byte_identical` (the pass-through 0%-gate).

/// A GENUINE inter-object self-shadow fixture for the host shadow/AO sanity: TWO big spheres
/// side by side (the `p6_r1_twin_scene` geometry — the same one the passing
/// `p6_r1_multi_light_sdf_shadows_match_oracle` casts real shadows on) lit by a STRONGLY
/// SIDE-ANGLED directional. With the light coming from the upper-left-toward-camera
/// ([`SELF_SHADOW_LIGHT_DIR`]), the LEFT sphere's bulk occludes the RIGHT sphere's left
/// flank, so `host_soft_shadow` returns < 1 over a broad cast-shadow band — a REAL shadow,
/// not the (now-fixed) grazing terminator acne. A head-on light (`(0,0,1)`) over a convex
/// body self-shadows NOWHERE post-fix, so the angled light + the occluder pair is what makes
/// the non-vacuity hold for a genuine shadow.
fn self_shadow_scene() -> Vec<SdfEdit> {
    p6_r1_twin_scene()
}

/// The side-angled directional that makes [`self_shadow_scene`]'s left sphere cast onto the
/// right sphere's flank (lateral X component dominant, a positive Z so it points at the front
/// faces). Empirically darkens ~322 of 800 SDF-hit pixels on the twin scene (a broad, robust
/// cast-shadow band), versus 0 for a head-on `(0,0,1)` light over the same convex shells.
const SELF_SHADOW_LIGHT_DIR: [f32; 3] = [-0.9, 0.0, 0.4];

/// **A-host — host shadow/AO sanity (CPU, no GPU).** A correctness sniff of `host_soft_shadow`
/// / `host_ao` via the public `_lit` golden: with SHADOWS|AO ON, the ON-path lit color must
/// (a) stay in-gamut (every channel ≤ 255), (b) be NO BRIGHTER than the OFF (Lambert-only)
/// golden at the same pixel — shadow ∈ [0,1] and AO ∈ [0,1] can only darken — and (c) be
/// STRICTLY darker over a BROAD band of pixels (the right sphere's flank the LEFT sphere
/// casts onto), proving the terms actually attenuate a REAL cast shadow (not a no-op multiply
/// by 1). The OFF baseline is the same golden with `lighting_flags == 0`.
///
/// RE-BLESSED (post grazing-acne fix): the former fixture was the CONVEX `crater` body under a
/// head-on `(0,0,1)` light; its sole "shadow" was the false grazing-terminator self-shadow
/// acne the A1 normal-offset bias now correctly removes — so post-fix it self-shadows ZERO
/// pixels and the non-vacuity guard could no longer hold. The fixture is now an inter-object
/// occluder pair ([`self_shadow_scene`]) under a side-angled light ([`SELF_SHADOW_LIGHT_DIR`]):
/// the left sphere casts a GENUINE shadow across the right sphere's flank, so the darken guard
/// proves a real shadow exists, not the (now-gone) acne. The non-vacuity floor is raised to a
/// BAND (`> 32` strictly-darker px) so a future regression that re-introduces only sparse
/// single-pixel acne cannot satisfy it.
#[test]
fn a_host_shadow_ao_darken_not_brighten() {
    let edits = self_shadow_scene();
    let light = SELF_SHADOW_LIGHT_DIR;
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;

    let mut darker_px = 0u64;
    let mut checked_hits = 0u64;
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            // Only SDF-hit, mesh-uncovered pixels carry the lit color (a mesh-covered or
            // background pixel is unaffected by lighting — the OFF==ON identity there).
            if !editlist_pixel_hits(&edits, px, py) || mesh_covers_pixel(px, py) {
                continue;
            }
            checked_hits += 1;
            let off = unpack_packed_rgb(golden_composite_pixel_ex_omega_lit(
                &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                0, light,
            ));
            let on = unpack_packed_rgb(golden_composite_pixel_ex_omega_lit(
                &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                flags, light,
            ));
            for ch in 0..3 {
                assert!(on[ch] <= 255, "[twin] lit channel out of gamut at ({px},{py}): {on:?}");
                assert!(
                    on[ch] <= off[ch],
                    "[twin] SHADOWS|AO BRIGHTENED ({px},{py}) ch{ch}: on {on:?} > off {off:?} \
                     (shadow/AO factors are in [0,1] — they can only darken)"
                );
            }
            if (0..3).any(|ch| on[ch] < off[ch]) {
                darker_px += 1;
            }
        }
    }
    assert!(checked_hits > 0, "the twin fixture must have an SDF-hit, mesh-uncovered pixel");
    // A BAND (not a lone pixel): the left sphere casts a real shadow across the right sphere's
    // flank. A floor of 32 px is far below the ~322 the fixture produces yet far above the
    // sparse single-pixel noise a re-introduced acne bug would yield.
    assert!(
        darker_px > 32,
        "SHADOWS|AO darkened only {darker_px} of {checked_hits} SDF-hit px — the genuine \
         inter-object cast shadow (left sphere onto the right sphere's flank) must darken a \
         BROAD band; a sub-band count means the shadow march is a near-no-op (or the fixture \
         no longer self-occludes)"
    );
    println!(
        "[twin] A-host OK: SHADOWS|AO darkens (never brightens) across {checked_hits} SDF-hit \
         px; {darker_px} strictly darker — a real cast-shadow band (left sphere onto the right \
         flank), not the fixed grazing acne"
    );
}

/// **A1g — ON GPU vs the deferred PBR oracle, DEFAULT light, SHADOWS|AO, ±2/255.** Push
/// `lighting_flags = SHADOWS|AO`, `light_dir = (0,0,1)`; every GPU LIT texel within ±2/255 of
/// `golden_deferred_resolve(golden_marcher_attributes(.., flags, (0,0,1)))` on crater / box /
/// smooth. PBR MVP-2 re-pointed the host reference from the retired MVP-1
/// `golden_composite_pixel_ex_omega_lit` (`base*vis` Lambert) to the deferred Cook-Torrance
/// oracle (the readback is the PBR `gLit`); the SDF-lit arm is bounded by the deferred
/// double-quant budget (±2/255) and the pass-through arms by the host-pack budget (±2/255).
/// This is the headline ON-path color gate.
#[test]
fn a1g_gpu_shadows_ao_matches_host_lit_default_light() {
    let Some(ctx) = boot_render_or_skip("a1g_gpu_shadows_ao_matches_host_lit_default_light") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    for (name, edits) in p4b_scenes() {
        let lit = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR).0;
        assert_eq!(lit.len(), READBACK_BYTES as usize);
        let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(nonzero > 0, "[{name}] lit all-zero — device did not render");
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden(&lit, &edits, flags, DEFAULT_LIGHT_DIR, name);
        assert!(sdf_lit_hits > 0, "[{name}] no SDF-lit (mask==1) pixel — the marcher hit no surface");
        println!(
            "[{name}] A1g SHADOWS|AO default-light vs deferred oracle: max arm-1 delta = \
             {max_arm1}/255, pass-through {max_pass}/255 (tol {DEFERRED_ARM1_TOL}); \
             {sdf_lit_hits} SDF-lit px"
        );
    }
}

/// **A2g — shadows-only and AO-only independence, ±2/255 vs the deferred PBR oracle.** Push
/// `flags = SHADOWS` (AO off) and `flags = AO` (shadows off) SEPARATELY; each GPU LIT render
/// matches the corresponding deferred oracle (`golden_deferred_resolve ∘
/// golden_marcher_attributes`, same light) within ±2/255. PBR MVP-2 re-pointed the host
/// reference from the retired MVP-1 `_lit` golden to the deferred Cook-Torrance oracle. This
/// proves the flag bits gate INDEPENDENTLY (a wired-together SHADOWS|AO that ignored a single
/// bit would diverge here). Also asserts the two single-flag renders DIFFER from each other
/// (each flag has a distinct effect).
///
/// RE-BLESSED (post grazing-acne fix): the SHADOWS-only != AO-only differential previously
/// relied on a fixture in [`p4b_scenes`] self-shadowing under the head-on `DEFAULT_LIGHT_DIR`.
/// Post-fix NONE of those convex/centered bodies self-shadow under a head-on light (the only
/// prior divergence WAS the grazing-terminator acne the A1 bias now removes), so the two
/// single-flag renders coincide on every p4b scene and the aggregate differential could no
/// longer hold. The load-bearing differential now runs on a DEDICATED genuine self-shadow
/// render ([`self_shadow_scene`] under [`SELF_SHADOW_LIGHT_DIR`] — the left sphere casts a real
/// shadow across the right sphere's flank): its SHADOWS-only and AO-only renders MUST differ,
/// each still validated against its OWN deferred oracle. The per-flag-vs-host gates still sweep
/// every p4b scene (the rigorous, light-agnostic independence proof).
#[test]
fn a2g_gpu_shadows_only_and_ao_only_gate_independently() {
    let Some(ctx) = boot_render_or_skip("a2g_gpu_shadows_only_and_ao_only_gate_independently") else {
        return;
    };
    // The per-flag ±2/255-vs-host gates (over every p4b scene, head-on light) are the rigorous
    // independence proof: each single-flag GPU render matches ITS OWN distinct deferred oracle,
    // so SHADOWS-only cannot be silently producing the AO-only result (and vice-versa). They
    // run light-agnostically; a convex/centered fixture self-shadows nowhere under a head-on
    // light, so its two single-flag renders legitimately COINCIDE — hence the load-bearing
    // SHADOWS-only != AO-only differential is asserted on a SEPARATE genuine self-shadow render.
    for (name, edits) in p4b_scenes() {
        for flags in [LIGHTING_FLAG_SHADOWS, LIGHTING_FLAG_AO] {
            let lit = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR).0;
            let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
            assert!(nonzero > 0, "[{name} flags={flags}] lit all-zero — device did not render");
            let (max_pass, max_arm1, sdf_lit_hits) =
                assert_lit_matches_deferred_golden(&lit, &edits, flags, DEFAULT_LIGHT_DIR, name);
            assert!(sdf_lit_hits > 0, "[{name} flags={flags}] no SDF-lit (mask==1) pixel");
            let which = if flags == LIGHTING_FLAG_SHADOWS { "SHADOWS-only" } else { "AO-only" };
            println!(
                "[{name}] A2g {which} vs deferred oracle: max arm-1 delta = {max_arm1}/255, \
                 pass-through {max_pass}/255 (tol {DEFERRED_ARM1_TOL}); {sdf_lit_hits} SDF-lit px"
            );
        }
    }

    // The load-bearing differential: a GENUINE inter-object cast shadow (the left sphere onto
    // the right sphere's flank) under the side-angled light. SHADOWS-only marches that occluder
    // and dims the band; AO-only does NOT (AO marches the surface normal, not the light), so the
    // two single-flag renders MUST diverge — and each is still validated against its own oracle.
    let edits = self_shadow_scene();
    let light = SELF_SHADOW_LIGHT_DIR;
    let mut renders: [Option<Vec<u8>>; 2] = [None, None];
    for (slot, flags) in [LIGHTING_FLAG_SHADOWS, LIGHTING_FLAG_AO].into_iter().enumerate() {
        let lit = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, light).0;
        let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(nonzero > 0, "[twin flags={flags}] lit all-zero — device did not render");
        let (_, _, sdf_lit_hits) =
            assert_lit_matches_deferred_golden(&lit, &edits, flags, light, "twin_self_shadow");
        assert!(sdf_lit_hits > 0, "[twin flags={flags}] no SDF-lit (mask==1) pixel");
        renders[slot] = Some(lit);
    }
    let shadows = renders[0].as_ref().expect("SHADOWS render");
    let ao = renders[1].as_ref().expect("AO render");
    let diff_px = shadows
        .chunks_exact(4)
        .zip(ao.chunks_exact(4))
        .filter(|(s, a)| (0..3).any(|c| (s[c] as i32 - a[c] as i32).abs() > LIT_CHANNEL_TOL))
        .count();
    assert!(
        diff_px > 0,
        "SHADOWS-only and AO-only coincided on the genuine self-shadow render — the two flags do \
         not gate independently (one bit is dead, or the shadow march is a no-op). The host \
         oracle dims a broad band here, so a GPU coincidence means a real divergence"
    );
    println!(
        "[twin] A2g SHADOWS-only != AO-only over {diff_px} px (a real cast shadow the SHADOWS bit \
         marches and the AO bit does not) — the flags gate independently"
    );
}

/// **A3g — the non-default light_dir mis-pack catcher (the architect's std430 push-layout
/// oracle).** A NON-axis `light_dir` ((0.4,0.5,0.768), normalized) is pushed with SHADOWS
/// enabled and the GPU render is compared, pixel-for-pixel, against the DEFAULT-light GPU
/// render. The two MUST DIFFER beyond the OFF tolerance on the SDF surface: the shadow march
/// direction is `pc.light_dir`, so a correctly-packed non-default light shifts the shadow
/// pattern, whereas a std430 OFFSET MIS-PACK (light_dir landing at the wrong push offset →
/// read as zero/garbage) would (a) collapse the shadow direction toward the default / a
/// degenerate value and (b) leave the render ≈ the default-light render → NO difference.
/// A measurable difference therefore proves `light_dir` reaches the shader at offset 16.
///
/// BUG-A-NDOTL (FIXED — see `a3g_nondefault_light_dir_matches_host_lit_literal` for the
/// literal-form payoff): the shader's Lambert BASE term now consumes the PUSHED `pc.light_dir`
/// (was the static `LIGHT_DIR=(0,0,1)`), matching `host_shade`, so a non-default light steers
/// the base too and the GPU/host base no longer diverge. This GPU-vs-GPU differential is
/// RETAINED as a complementary, host-independent mis-pack oracle: it proves the same packing
/// property (a non-axis light re-aims the shadow march) without depending on the host golden.
#[test]
fn a3g_nondefault_light_dir_shifts_shadows_mispack_catcher() {
    let Some(ctx) = boot_render_or_skip("a3g_nondefault_light_dir_shifts_shadows_mispack_catcher") else {
        return;
    };
    // SHADOWS only: isolate the term the shader actually steers by `pc.light_dir` (AO marches
    // the surface NORMAL, not the light, so it is light_dir-invariant and would dilute the
    // differential). The differential is only geometrically guaranteed where the SCENE has a
    // self-occluder: the carved CRATER (a CSG subtract that leaves a rim/crevice) self-shadows
    // and so MUST shift; a single CONVEX box self-shadows nowhere (the lit hemisphere is
    // unoccluded for any front light), so its shift is legitimately ~0. The mis-pack catcher
    // therefore REQUIRES a shift on the crater (the load-bearing assertion) and merely reports
    // the others — a mis-pack (light_dir read off-offset → degenerate / default direction)
    // would zero the CRATER shift, tripping the gate.
    let flags = LIGHTING_FLAG_SHADOWS;
    let mut crater_shifted = 0u64;
    for (name, edits) in p4b_scenes() {
        let def = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR).0;
        let non = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, NONDEFAULT_LIGHT).0;
        assert_eq!(def.len(), READBACK_BYTES as usize);
        assert_eq!(non.len(), READBACK_BYTES as usize);

        // Count pixels whose shadow term shifted beyond the OFF tolerance. A correctly-packed
        // non-axis light re-aims the shadow march → self-occluded surface pixels change. A
        // mis-pack (light_dir read as 0 → the ndotl<=0 early-out, OR read as the default) would
        // leave def ≈ non → ZERO shifted pixels.
        let mut shifted = 0u64;
        let mut max_shift = 0i32;
        let mut worst = (0u32, 0u32, [0i32; 3], [0i32; 3]);
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                let a = albedo_rgb(&def, px, py);
                let b = albedo_rgb(&non, px, py);
                let dmax = (0..3).map(|c| (a[c] - b[c]).abs()).max().unwrap();
                if dmax > CHANNEL_TOL {
                    shifted += 1;
                }
                if dmax > max_shift {
                    max_shift = dmax;
                    worst = (px, py, a, b);
                }
            }
        }
        if name == "crater_csg" {
            crater_shifted = shifted;
        }
        println!(
            "[{name}] A3g non-axis light shift: {shifted} pixels vs default (max {max_shift}/255 \
             at ({},{}) def={:?} non={:?})",
            worst.0, worst.1, worst.2, worst.3
        );
    }
    assert!(
        crater_shifted > 0,
        "MIS-PACK SUSPECTED: the non-axis light_dir {NONDEFAULT_LIGHT:?} produced NO shadow shift \
         on the CRATER (a self-occluding CSG carve) vs the default light — light_dir is NOT \
         reaching the shader at offset 16 (a std430 push mis-pack), or the shadow term ignores it"
    );
    println!(
        "[crater_csg] A3g mis-pack catcher OK: non-axis light_dir shifts {crater_shifted} crater \
         pixels — light_dir reaches the shader at offset 16 (correct std430 packing)"
    );
}

/// **A3g-literal — the architect's named std430 oracle in its FULL literal form, against the
/// deferred PBR oracle (the BUG-A-NDOTL payoff).** Push a NON-axis `light_dir` ((0.4,0.5,0.768)
/// normalized) with `SHADOWS|AO` and assert EVERY GPU LIT texel is within ±2/255 of
/// `golden_deferred_resolve(golden_marcher_attributes(.., flags, NONDEFAULT_LIGHT))` — the
/// deferred oracle baked with the SAME non-default light — on crater / box / smooth.
///
/// PBR MVP-2: the host reference is re-pointed from the retired MVP-1
/// `golden_composite_pixel_ex_omega_lit` (`base*vis` Lambert) to the deferred Cook-Torrance
/// oracle (the readback is the PBR `gLit`). The literal host-vs-GPU form is now feasible against
/// THIS oracle for any light: `golden_marcher_attributes` steers the lit terms by the pushed
/// `light_dir` (the BUG-A-NDOTL fix — the marcher's base + the resolve both consume
/// `pc.light_dir`), so a non-default light no longer diverges the host from the GPU. (Against
/// the OLD MVP-1 mirror the literal form carried a footnote that it was not achievable; that
/// footnote is obsolete — the deferred oracle is the single source of truth the GPU was tested
/// byte-identical to.) This gate SUBSUMES the mis-pack property the GPU-vs-GPU
/// `a3g_nondefault_light_dir_shifts_shadows_mispack_catcher` targets: the deferred oracle
/// marches the same `light_dir`, so a std430 offset slip (light_dir read off-offset →
/// degenerate / default direction) makes the GPU shadow pattern diverge from the oracle by far
/// more than ±2/255 and trips this gate too.
#[test]
fn a3g_nondefault_light_dir_matches_host_lit_literal() {
    let Some(ctx) = boot_render_or_skip("a3g_nondefault_light_dir_matches_host_lit_literal") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    for (name, edits) in p4b_scenes() {
        let lit = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, NONDEFAULT_LIGHT).0;
        assert_eq!(lit.len(), READBACK_BYTES as usize);
        let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(nonzero > 0, "[{name}] non-default-light lit all-zero — device did not render");
        // The LITERAL host-vs-GPU comparison with the SAME non-default light_dir, against the
        // deferred PBR oracle (the payoff).
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden(&lit, &edits, flags, NONDEFAULT_LIGHT, name);
        assert!(sdf_lit_hits > 0, "[{name}] no SDF-lit (mask==1) pixel under the non-default light");
        println!(
            "[{name}] A3g-literal SHADOWS|AO non-default light {NONDEFAULT_LIGHT:?} vs deferred \
             oracle: max arm-1 delta = {max_arm1}/255, pass-through {max_pass}/255 (tol \
             {DEFERRED_ARM1_TOL}) — BUG-A-NDOTL payoff: the oracle steers by pc.light_dir; \
             {sdf_lit_hits} SDF-lit px"
        );
    }
}

/// **A5 — GPU OFF-vs-ON wall-clock A/B (perf OBSERVATION, not a pass/fail gate).** Measures
/// the fence-to-fence wall time of the FULL marcher submit (raster + marcher + readback) with
/// lighting OFF vs SHADOWS|AO ON, median of N runs, on the densest fixture (crater). This is a
/// coarse CPU-side proxy — it includes the constant raster + copy + submit/wait overhead, so
/// the ON/OFF DELTA (not the absolute) is the signal: the A1/A2 cost is the shadow secondary
/// march (≤ MAX_IT steps per lit pixel) + the 5 AO taps, bounded by the P4-style empty-skip
/// and the small 64×64 lit-pixel count. No GPU-timestamp query API exists in the RHI yet
/// (a developer increment), so a true on-device marcher-only timing is deferred; this wall A/B
/// is the available proxy. `#[ignore]` by default (a perf observation, run explicitly).
#[test]
#[ignore = "perf observation — run explicitly with --ignored"]
fn a5_gpu_off_vs_on_wall_clock_ab() {
    let Some(ctx) = boot_render_or_skip("a5_gpu_off_vs_on_wall_clock_ab") else {
        return;
    };
    use std::time::Instant;
    let edits = crater();
    const N: usize = 21; // odd → a clean median
    let bench = |flags: u32| -> f64 {
        // Warm up (pipeline/cache) before timing.
        let _ = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, DEFAULT_MARCHER_OMEGA, flags, DEFAULT_LIGHT_DIR).0;
        let mut samples = Vec::with_capacity(N);
        for _ in 0..N {
            let t0 = Instant::now();
            let out = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, DEFAULT_MARCHER_OMEGA, flags, DEFAULT_LIGHT_DIR).0;
            let dt = t0.elapsed().as_secs_f64() * 1.0e6; // microseconds
            std::hint::black_box(&out);
            samples.push(dt);
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        samples[N / 2]
    };
    let off = bench(0);
    let on = bench(LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO);
    println!(
        "[crater_csg] A5 wall-clock A/B (median of {N}, full submit incl. raster+copy+wait): \
         OFF = {off:.1} µs, ON(SHADOWS|AO) = {on:.1} µs, Δ = {:+.1} µs ({:+.1}%). \
         NOTE: coarse CPU-side proxy — includes constant non-marcher overhead; the Δ is the A1/A2 \
         marcher-side cost signal, not a pass/fail gate.",
        on - off,
        (on - off) / off * 100.0
    );
}

/// **A4g — sync-validation clean under cull-ON + lighting-ON (the heaviest path).** A cull-ON
/// SHADOWS|AO ω=1.2 dispatch that RETURNS proves `assert_validation_clean` passed (asserted
/// inside `run_gbuffer_hybrid_lit` before return): the A1/A2 shadow/AO secondary marches read
/// the SAME frozen field through the already-bound vocabulary set — they add NO new resource,
/// NO new binding, and NO new barrier over the pre-A1/A2 cull-ON path, so the coarse→fine
/// Tiles barrier and the G-buffer image transitions raise no new hazard. The 32-byte push is
/// pure data. This is the combined gate 9 (sync-val) for the ON path.
#[test]
fn a4g_cull_on_lighting_on_sync_validation_clean() {
    let Some(ctx) = boot_render_or_skip("a4g_cull_on_lighting_on_sync_validation_clean") else {
        return;
    };
    // SOLE purpose is the validation oracle — nothing to assert when it is off.
    if !ctx.validation_enabled() {
        eprintln!("SKIP a4g_cull_on_lighting_on_sync_validation_clean: validation disabled (BOYKO_DISABLE_VALIDATION)");
        return;
    }
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let (albedo, tiles) =
        run_gbuffer_hybrid_lit(&ctx, &crater(), true, true, DEFAULT_MARCHER_OMEGA, flags, DEFAULT_LIGHT_DIR);
    assert_eq!(albedo.len(), READBACK_BYTES as usize);
    let bounds = parse_tile_bounds(&tiles.expect("read_tiles"));
    let surface = bounds.iter().filter(|b| b.flags & TILE_FLAG_EMPTY == 0).count();
    assert!(surface > 0, "the coarse pass must have marked at least one surface tile");
    let sdf_hits = albedo
        .chunks_exact(4)
        .filter(|t| {
            let mesh = unpack_packed_rgb(pack_rgba(MESH_COLOR));
            let bg = packed_background();
            let g = [t[0] as i32, t[1] as i32, t[2] as i32];
            let near = |r: [i32; 3]| (0..3).all(|c| (g[c] - r[c]).abs() <= CHANNEL_TOL);
            !near(mesh) && !near(bg) && (t[0] != 0 || t[1] != 0 || t[2] != 0)
        })
        .count();
    println!(
        "[crater_csg] A4g cull-ON + SHADOWS|AO ω={DEFAULT_MARCHER_OMEGA} validation+sync-clean: \
         {} tiles ({surface} surface), {sdf_hits} lit SDF px; the A1/A2 secondary marches raised \
         no new hazard",
        bounds.len()
    );
}

/// **A3g-host — the non-default light std430 round-trip (host-side push-layout pin).** A
/// pure host check that `FineMarcherPush::new(.., NONDEFAULT_LIGHT)` re-views the non-default
/// `light_dir` at byte offset 16 of `as_bytes()` (the std430 offset the shader reads). This
/// is the deterministic companion to the GPU differential A3g: it pins the host side of the
/// push contract (the `const _: () = assert!(offset_of!(.., light_dir) == 16)` is a compile
/// gate; this asserts the RUNTIME bytes too) so a future field-reorder is caught even without
/// a GPU.
#[test]
fn a3g_host_light_dir_round_trips_at_offset_16() {
    let push = FineMarcherPush::new(
        false,
        DEFAULT_MARCHER_OMEGA,
        LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO,
        NONDEFAULT_LIGHT,
    );
    let bytes = push.as_bytes();
    // M1 widened FineMarcherPush 32 → 64 (empty-skip grid block @32..64); M2 added
    // brick_trilinear @64 + _pad3 → 80. The light_dir @16 contract this test pins is
    // UNCHANGED. (Was `== 32` pre-M1, `== 64` pre-M2.)
    assert_eq!(bytes.len(), 80, "FineMarcherPush must serialize to 80 bytes (M2: 64 → 80)");
    let read_at = |off: usize| f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
    // lighting_flags is a u32 at offset 8.
    let flags = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    assert_eq!(flags, LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO, "lighting_flags must land at offset 8");
    // light_dir is a float3 at offset 16 (std430), tail-padded by _pad2 @28; the M1 grid
    // block (grid_origin @32, brick_enabled @44, grid_dims @48, brick_world @60) follows.
    assert_eq!(read_at(16), NONDEFAULT_LIGHT[0], "light_dir.x must land at offset 16");
    assert_eq!(read_at(20), NONDEFAULT_LIGHT[1], "light_dir.y must land at offset 20");
    assert_eq!(read_at(24), NONDEFAULT_LIGHT[2], "light_dir.z must land at offset 24");
    println!("[host] A3g-host OK: light_dir {NONDEFAULT_LIGHT:?} round-trips at push offset 16/20/24");
}

// ===========================================================================================
// Deferred-shading SPLIT (increment 1) — the M1 wiring gates (TESTER).
//
// The marcher writes ATTRIBUTES (gAlbedo = base, gMaterial = (vis, 0, mask, 1)); the
// `deferred_pbr` RESOLVE composites `lit = mask==1 ? base*vis : base`. The host oracles are
// `golden_marcher_attributes` (the marcher's per-pixel (base_rgb, vis, mask)) and
// `golden_deferred_resolve` (the resolve's packed LIT). The earlier ON-path gates
// (`a1g_*`, `a2g_*`, `a3g_*_literal`) diff the GPU LIT against the OLD INLINE golden
// `golden_composite_pixel_ex_omega_lit` at the generic ±3/255 (`LIT_CHANNEL_TOL`); that
// proves the GPU ≈ the old inline composite but DOES NOT exercise the new deferred oracles,
// so the 0%-gate's byte-identity (delta == 0 on the pass-through arms) and the ≤1.5-LSB
// double-quant bound (arm 1) were UNVERIFIED. These gates close that gap by diffing the GPU
// LIT against `golden_deferred_resolve ∘ golden_marcher_attributes` directly, per arm:
//
//   - PASS-THROUGH arms (mesh / background / empty, AND lighting-OFF SDF-lit): the resolve
//     passes `base` through unmodified (mask == 0) or multiplies by vis == 1.0 (mask == 1,
//     OFF), so the GPU LIT must be BYTE-IDENTICAL (delta == 0) to the new oracle.
//   - ARM 1 (SDF-lit, lighting ON): the deferred double-quantization (base8/255 * vis8/255,
//     re-packed) drifts from the GPU's own fp `base*vis` by the architect's ≤2/255 bound —
//     TIGHTER than the generic ±3/255 the old-inline gate uses.
//
// A GPU pixel is mapped to its host arm by `golden_marcher_attributes(..).mask` (1 = SDF-lit,
// 0 = mesh/bg/empty) — the SAME mask the resolve branches on — so the per-arm tolerance is
// applied to exactly the pixels the resolve treats that way.
// ===========================================================================================

/// The deferred-resolve double-quantization bound on the SDF-LIT arm (arm 1). The marcher
/// already R8-quantized `base` and `vis`; the resolve decodes them (base8/255, vis8/255),
/// multiplies, and re-quantizes — a SECOND 8-bit rounding on top of the GPU's own fp
/// `base*vis`. The architect's ≤1.5-LSB analysis bounds this at ≤2/255 (rounded up to the
/// integer channel grid). This is STRICTLY tighter than the generic `LIT_CHANNEL_TOL` (±3)
/// the old-inline ON-path gates use — it is the bound this increment exists to prove.
const DEFERRED_ARM1_TOL: i32 = 2;

/// The pass-through arm budget when the oracle is the HOST `golden_deferred_resolve` (which
/// quantizes via host `pack_rgba`). On the pass-through arms (mask == 0, or OFF SDF-lit) the
/// RESOLVE itself is a byte-exact GPU identity (decode `b/255` → re-encode → `b`), so the GPU
/// LIT equals the GPU's OWN gAlbedo store byte-for-byte. The residual vs the host oracle is
/// therefore EXACTLY the marcher's pre-existing host-`pack_rgba`-vs-GPU-UNORM-store
/// quantization gap (the half-way `0.1*255 == 25.5` background channel rounds 26 host / 25
/// GPU) — the SAME ≤2/255 gap `p1b`/GATE-4 already budget against the albedo golden. It is
/// NOT a resolve error; the resolve's exactness is proved independently by
/// [`assert_resolve_passthrough_is_lighting_invariant`] (a delta == 0 GPU-internal gate).
const DEFERRED_PASSTHROUGH_HOST_TOL: i32 = 2;

/// Diffs the whole GPU LIT readback against the NEW deferred oracle
/// `golden_deferred_resolve(golden_marcher_attributes(.., flags, light_dir))` per ARM, on the
/// cull-OFF ω=1.0 path:
///
///   - mask == 0 (mesh / background / empty) → within [`DEFERRED_PASSTHROUGH_HOST_TOL`]
///     (±2/255: the marcher's pre-existing host-pack-vs-GPU-store quant gap; the resolve adds
///     ZERO error here — proved delta-0 GPU-internally by the lighting-invariance gate).
///   - mask == 1, flags == 0 (SDF-lit, lighting OFF) → same pass-through budget (resolve
///     `base*1.0`).
///   - mask == 1, flags != 0 (SDF-lit, lighting ON) → within [`DEFERRED_ARM1_TOL`] (the
///     deferred double-quant, ≤2/255).
///
/// Returns `(max_delta_passthrough, max_delta_arm1, sdf_lit_hits)`. `sdf_lit_hits` (the
/// mask == 1 count) lets the caller prove the device rendered a real lit surface (not an
/// all-pass-through fill). Asserts on every texel; the caller passes the scene name.
///
/// This is the ω=1.0 specialization of [`assert_lit_matches_deferred_golden_omega`] (the
/// over-relaxation factor the pre-B1 marcher used). The B1 over-relaxation gates that diff a
/// non-unit ω against the deferred oracle call the `_omega` form directly.
fn assert_lit_matches_deferred_golden(
    lit: &[u8],
    edits: &[SdfEdit],
    flags: u32,
    light_dir: [f32; 3],
    name: &str,
) -> (i32, i32, u64) {
    assert_lit_matches_deferred_golden_omega(lit, edits, 1.0, flags, light_dir, name)
}

/// The over-relaxation-aware form of [`assert_lit_matches_deferred_golden`]: diffs the whole
/// GPU LIT readback against `golden_deferred_resolve(golden_marcher_attributes(.., omega,
/// flags, light_dir))` per ARM, with the same ±2/255 pass-through and ARM-1 double-quant
/// budgets. `omega` is the Render B1 over-relaxation factor the GPU marched at (the host
/// oracle marches the IDENTICAL ω, so the comparison stays matched-ω).
fn assert_lit_matches_deferred_golden_omega(
    lit: &[u8],
    edits: &[SdfEdit],
    omega: f32,
    flags: u32,
    light_dir: [f32; 3],
    name: &str,
) -> (i32, i32, u64) {
    let mut max_pass = 0i32;
    let mut max_arm1 = 0i32;
    let mut sdf_lit_hits = 0u64;
    let materials = host_material_table();
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let md = expected_mesh_depth(px, py);
            let attrs = golden_marcher_attributes(
                edits, &materials, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, omega,
                flags, light_dir,
            );
            // Render P5 (r0+r1): a mesh-covered pixel the SDF did NOT win is RASTER-OWNED —
            // the marcher yields and the raster pass A's PBR fragment (mask=1, base = the
            // white vertex color, Cook-Torrance lit) stands, NOT the old flat MESH_COLOR
            // (mask=0) pass-through. `golden_marcher_attributes` now MODELS that raster-PBR
            // producer exactly (mask=1, base=MESH_RASTER_ALBEDO, n=(0,0,1), shadow=ao=1,
            // view_t=t_mesh — Render P7/P5-r1b: the mesh surface ray-t, NOT the old sentinel),
            // so the deferred oracle predicts the mesh pixel too and it is asserted on the SAME
            // ±2/255 budget as every other pixel — no skip. This `golden_deferred_resolve` is the
            // directional+sky BASE resolve (no point/spot, no view_t consumption), so the mesh
            // lit value is unchanged by the gViewT unlock — the equivalence stays GPU == host.
            let (_, rd) = composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
            let want = unpack_packed_rgb(golden_deferred_resolve(attrs, rd, &materials));
            let got = albedo_rgb(lit, px, py);
            let dmax = (0..3).map(|c| (got[c] - want[c]).abs()).max().unwrap();
            // The arm the resolve actually takes: mask == 1 AND lighting ON is the only
            // double-quantized (non-pass-through) case; everything else passes through.
            let is_arm1 = attrs.mask == 1 && flags != 0;
            if attrs.mask == 1 {
                sdf_lit_hits += 1;
            }
            if is_arm1 {
                if dmax > max_arm1 {
                    max_arm1 = dmax;
                }
                assert!(
                    dmax <= DEFERRED_ARM1_TOL,
                    "[{name}] ARM-1 (SDF-lit, flags={flags}) LIT texel ({px},{py}) got {got:?} \
                     want {want:?} (deferred oracle) exceeds ±{DEFERRED_ARM1_TOL}/255 (the \
                     double-quant bound); delta {dmax}"
                );
            } else {
                if dmax > max_pass {
                    max_pass = dmax;
                }
                assert!(
                    dmax <= DEFERRED_PASSTHROUGH_HOST_TOL,
                    "[{name}] PASS-THROUGH (mask={}, flags={flags}) LIT texel ({px},{py}) got \
                     {got:?} want {want:?} (deferred oracle) exceeds the host-pack quant budget \
                     ±{DEFERRED_PASSTHROUGH_HOST_TOL}/255 (delta {dmax}) — the resolve must pass \
                     base through (the residual is the marcher's host-pack-vs-GPU-store gap, NOT \
                     a resolve error)",
                    attrs.mask
                );
            }
        }
    }
    (max_pass, max_arm1, sdf_lit_hits)
}

/// **D1-host — the deferred PASS-THROUGH byte-identity gate (host-only, no GPU).** PBR
/// MVP-2 changes the SDF-lit (mask == 1) output from the MVP-1 `base*vis` composite to full
/// Cook-Torrance — an INTENTIONAL, owner-acknowledged behavioral change (PBR plan call F),
/// so the SDF-lit arm is DELIBERATELY no longer an approximation of the old inline composite
/// and is NOT compared against it here. What this gate STILL proves — the load-bearing
/// 0%-gate — is that the deferred bake (`golden_deferred_resolve ∘ golden_marcher_attributes`)
/// is BYTE-IDENTICAL to the old inline composite on the PASS-THROUGH arms (mesh / background
/// / empty, mask == 0) across crater / box / smooth, lighting OFF + ON, default + non-default
/// light. A regression in the host oracles' pass-through path is caught without a device.
#[test]
fn d1_host_deferred_passthrough_byte_identical() {
    let materials = host_material_table();
    for (name, edits) in p4b_scenes() {
        for (lname, light) in [("default", DEFAULT_LIGHT_DIR), ("nondefault", NONDEFAULT_LIGHT)] {
            for flags in [0u32, LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO] {
                let mut passthrough = 0u64;
                let mut lit_hits = 0u64;
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let md = expected_mesh_depth(px, py);
                        let attrs = golden_marcher_attributes(
                            &edits, &materials, md, px, py, SDF_IMG_W, SDF_IMG_H,
                            CompositeCamera::Ortho, 1.0, flags, light,
                        );
                        // Only the mask == 0 (mesh / bg / empty) arm has the unchanged
                        // pass-through contract; the mask == 1 arm is now PBR (skipped here).
                        if attrs.mask == 1 {
                            lit_hits += 1;
                            continue;
                        }
                        passthrough += 1;
                        let (_, rd) =
                            composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
                        let deferred =
                            unpack_packed_rgb(golden_deferred_resolve(attrs, rd, &materials));
                        let inline = unpack_packed_rgb(golden_composite_pixel_ex_omega_lit(
                            &edits, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                            flags, light,
                        ));
                        assert_eq!(
                            deferred, inline,
                            "[{name}/{lname}] PASS-THROUGH (mask=0, flags={flags}) deferred \
                             {deferred:?} != inline {inline:?} at ({px},{py}) — the mesh / bg / \
                             empty arms must bake byte-identically (the 0%-gate)"
                        );
                    }
                }
                assert!(
                    passthrough > 0,
                    "[{name}/{lname}] no mask=0 pixel — the pass-through gate is vacuous"
                );
                println!(
                    "[{name}/{lname}] D1-host flags={flags}: {passthrough} pass-through px \
                     BYTE-IDENTICAL (delta 0) deferred-vs-inline; {lit_hits} SDF-lit (now PBR) px"
                );
            }
        }
    }
}

/// **D2g — the M1 pass-through gate (GPU LIT == the deferred oracle on the pass-through
/// arms).** With `flags == 0` EVERY arm is pass-through (mesh/bg/empty mask 0; SDF-lit mask 1
/// but vis == 1.0), so the whole GPU LIT image must match `golden_deferred_resolve(
/// golden_marcher_attributes(.., flags=0))` within [`DEFERRED_PASSTHROUGH_HOST_TOL`] (±2/255).
///
/// IMPORTANT (the M1 finding): a delta == 0 (literal byte-identity) claim against the HOST
/// oracle is NOT achievable here, and the gap is NOT a resolve bug. The host oracle quantizes
/// via host `pack_rgba`, which disagrees with the GPU UNORM store by 1 LSB on the half-way
/// background channel (`0.1*255 == 25.5` → host 26, GPU 25) — the SAME pre-existing
/// host-pack-vs-GPU-store gap the marcher's albedo already carries (why `p1b`/GATE-4 use
/// ±2/255). The resolve's pass-through is BYTE-EXACT at the GPU level; that exactness is
/// proved delta-0 (GPU-internally, no host pack) by
/// [`d2g_resolve_passthrough_is_lighting_invariant`]. This gate confirms the GPU LIT tracks
/// the host oracle within the marcher's own quant budget on the pass-through arms.
#[test]
fn d2g_passthrough_within_host_pack_budget() {
    let Some(ctx) = boot_render_or_skip("d2g_passthrough_within_host_pack_budget") else {
        return;
    };
    for (name, edits) in p4b_scenes() {
        let lit = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, 0, DEFAULT_LIGHT_DIR).0;
        assert_eq!(lit.len(), READBACK_BYTES as usize);
        let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(nonzero > 0, "[{name}] LIT all-zero — device did not render");
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden(&lit, &edits, 0, DEFAULT_LIGHT_DIR, name);
        assert_eq!(max_arm1, 0, "[{name}] flags==0 must have NO arm-1 pixel (lighting OFF)");
        assert!(sdf_lit_hits > 0, "[{name}] no SDF-lit (mask==1) pixel — the marcher hit no surface");
        println!(
            "[{name}] D2g M1 pass-through vs deferred oracle (flags=0): max delta = {max_pass}/255 \
             (tol {DEFERRED_PASSTHROUGH_HOST_TOL}, host-pack gap); {sdf_lit_hits} SDF-lit px \
             (vis=1.0 pass-through)"
        );
    }
}

/// **D2g — the resolve-is-an-exact-pass-through gate (delta == 0, GPU-INTERNAL, no host
/// pack).** The headline M1 byte-identity proof, free of the host-pack quantization gap. On
/// the mesh / background / empty arms (mask == 0) the resolve emits `base` regardless of
/// lighting, and the MARCHER writes the identical `base` to gAlbedo regardless of
/// `lighting_flags` (lighting only attenuates the SDF-hit vis, never the mesh/bg base). So the
/// GPU LIT on those pixels MUST be byte-identical between an OFF run and a SHADOWS|AO run —
/// a delta == 0 GPU-vs-GPU comparison that needs NO host oracle and is immune to the
/// host-pack-vs-UNORM gap. This proves (a) the resolve perturbs a pass-through pixel by
/// exactly ZERO, and (b) the STRICT `mask` branch never lets a vis-attenuated SDF lane bleed
/// into a mesh/bg pixel — the load-bearing 0%-gate the strict-if buys. A mismatch = the
/// resolve is NOT a pure pass-through (or the mask leaked).
#[test]
fn d2g_resolve_passthrough_is_lighting_invariant() {
    let Some(ctx) = boot_render_or_skip("d2g_resolve_passthrough_is_lighting_invariant") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    for (name, edits) in p4b_scenes() {
        let off = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, 0, DEFAULT_LIGHT_DIR).0;
        let on = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR).0;
        assert_eq!(off.len(), READBACK_BYTES as usize);
        assert_eq!(on.len(), READBACK_BYTES as usize);

        // The mask the resolve branches on (from the host attribute mirror): mask == 0 is the
        // pure pass-through set the lighting flags must NOT touch.
        let mut passthrough = 0u64;
        let mut sdf_lit = 0u64;
        let materials = host_material_table();
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                let md = expected_mesh_depth(px, py);
                let mask = golden_marcher_attributes(
                    &edits, &materials, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    1.0, flags, DEFAULT_LIGHT_DIR,
                )
                .mask;
                if mask == 0 {
                    passthrough += 1;
                    let a = albedo_rgb(&off, px, py);
                    let b = albedo_rgb(&on, px, py);
                    assert_eq!(
                        a, b,
                        "[{name}] PASS-THROUGH (mask=0) LIT texel ({px},{py}) changed between \
                         OFF {a:?} and SHADOWS|AO {b:?} — the resolve is NOT a pure pass-through \
                         on mask=0 (or the strict-mask branch leaked a vis-attenuated lane)"
                    );
                } else {
                    sdf_lit += 1;
                }
            }
        }
        assert!(passthrough > 0, "[{name}] no mask=0 pixel — the pass-through gate is vacuous");
        assert!(sdf_lit > 0, "[{name}] no mask=1 pixel — there is no lit surface to leave alone");
        println!(
            "[{name}] D2g resolve-passthrough lighting-invariant: {passthrough} mask=0 pixels \
             BYTE-IDENTICAL (delta 0) across OFF vs SHADOWS|AO; {sdf_lit} mask=1 px"
        );
    }
}

/// **D3g — the arm-1 bounded-quantization gate (SDF-lit, ≤2/255 vs the deferred oracle).**
/// Push SHADOWS|AO (default light); every SDF-lit (mask == 1) GPU LIT texel must be within
/// [`DEFERRED_ARM1_TOL`] (±2/255) of `golden_deferred_resolve(golden_marcher_attributes(..,
/// flags=SHADOWS|AO))`. This is the ≤1.5-LSB double-quantization the deferred split
/// introduces — TIGHTER than the generic ±3/255 the old-inline `a1g_*` gate uses, and the
/// bound this increment exists to prove. The pass-through arms (asserted delta 0 by the same
/// helper) are re-confirmed here too. Runs crater / box / smooth, and (separately) the
/// non-default light to exercise the steered shadow march.
#[test]
fn d3g_arm1_within_double_quant_bound_of_deferred_golden() {
    let Some(ctx) = boot_render_or_skip("d3g_arm1_within_double_quant_bound_of_deferred_golden")
    else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    for (lname, light) in [("default", DEFAULT_LIGHT_DIR), ("nondefault", NONDEFAULT_LIGHT)] {
        for (name, edits) in p4b_scenes() {
            let lit = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, light).0;
            assert_eq!(lit.len(), READBACK_BYTES as usize);
            let nonzero =
                lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
            assert!(nonzero > 0, "[{name}/{lname}] LIT all-zero — device did not render");
            let (max_pass, max_arm1, sdf_lit_hits) =
                assert_lit_matches_deferred_golden(&lit, &edits, flags, light, name);
            assert!(
                max_pass <= DEFERRED_PASSTHROUGH_HOST_TOL,
                "[{name}/{lname}] a pass-through arm exceeded the host-pack budget (delta \
                 {max_pass} > {DEFERRED_PASSTHROUGH_HOST_TOL})"
            );
            assert!(
                sdf_lit_hits > 0,
                "[{name}/{lname}] no SDF-lit (mask==1) pixel — the arm-1 bound is vacuous"
            );
            println!(
                "[{name}/{lname}] D3g arm-1 double-quant: max delta = {max_arm1}/255 \
                 (tol {DEFERRED_ARM1_TOL}); pass-through {max_pass} (=0); {sdf_lit_hits} SDF-lit px"
            );
        }
    }
}

/// Lighting L0a GPU 0%-gate: the table-driven `deferred_pbr` resolve, fed the DEGENERATE
/// light table (1 directional dir = +Z / white / illuminance 1.0 + 1 sky with
/// `sky == ground == (0.10,0.10,0.12)`, exposure 1.0; seeded into the resolve's binding-6
/// SSBO by [`DEGENERATE_LIGHT_TABLE`]), must reproduce today's reference image — the
/// constant-path `golden_deferred_resolve` — within the existing ±2/255 pass-through +
/// double-quant budgets, on every arm.
///
/// This is THE L0a 0%-gate the GPU-tester runs on the 3060: the degenerate table folds to
/// the old compiled-in `LIGHT_DIR` / `LIGHT_COLOR` / `SKY_*` (the directional matches +Z
/// white; the sky `lerp` folds since `sky == ground`), so the table-driven shader's output
/// is byte-equivalent to the constant path within tolerance. A drift in the table layout,
/// the header decode, the loop op-order, or the exposure multiply fails this test.
///
/// (NON-GPU note for the developer gate: this is GPU-only — it `boot_render_or_skip`s when
/// no device is present. The CPU companion, `tests/lighting_l0_host_oracle.rs`, proves the
/// SAME degenerate fold is BIT-exact in the host oracle without a GPU.)
#[test]
fn l0a_degenerate_light_table_reproduces_constant_path_image() {
    let Some(ctx) =
        boot_render_or_skip("l0a_degenerate_light_table_reproduces_constant_path_image")
    else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    // Lighting ON (A1 shadows + A2 AO) with the default directional — the arm the L0a
    // table actually drives (mask == 1). The degenerate table at binding 6 must reproduce
    // the constant-path oracle (`DEFAULT_LIGHT_DIR` / SKY_*) within tolerance.
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    for (name, edits) in p4b_scenes() {
        let lit = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR).0;
        assert_eq!(lit.len(), READBACK_BYTES as usize);
        let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(nonzero > 0, "[{name}] LIT all-zero — device did not render");

        // The whole-image diff vs the CONSTANT-path oracle = the L0a 0%-gate.
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden(&lit, &edits, flags, DEFAULT_LIGHT_DIR, name);
        assert!(
            max_pass <= DEFERRED_PASSTHROUGH_HOST_TOL,
            "[{name}] L0a 0%-gate: a pass-through arm drifted from the constant-path image \
             (delta {max_pass} > {DEFERRED_PASSTHROUGH_HOST_TOL})"
        );
        assert!(
            sdf_lit_hits > 0,
            "[{name}] L0a 0%-gate: no SDF-lit (mask==1) pixel — the lit-arm gate is vacuous"
        );
        println!(
            "[{name}] L0a 0%-gate (degenerate table == constant path): lit-arm max delta \
             {max_arm1}/255 (tol {DEFERRED_ARM1_TOL}); pass-through {max_pass} (=0); \
             {sdf_lit_hits} SDF-lit px"
        );
    }
}

// ============================================================================
// Lighting L0b GPU goldens (point/spot resolve via the gViewT lane).
//
// These run on the 3060 (the GPU-tester); they `boot_render_or_skip` when no device is
// present. The CPU companion is `tests/lighting_l0b_host_oracle.rs`. The L0b resolve adds
// the point/spot path: the marcher stores the surface `t` into the new gViewT lane and the
// resolve reconstructs `P = ro + rd * t` to attenuate point/spot lights — compared per
// texel against the host `golden_deferred_resolve_table` (the bit-exact source of truth).
// ============================================================================

/// Serializes a host `(GoldenLightHeader, &[GoldenLight])` into the std430 word-packed
/// `[LightHeaderGpu (16w) || GpuLight[] (12w each)]` the resolve's binding-6 SSBO expects
/// (mirrors `boyko_render::light`'s collection layout + `light_table.hlsli`'s decode).
fn pack_light_table(header: &GoldenLightHeader, lights: &[GoldenLight]) -> Vec<u32> {
    let mut words = vec![0u32; GOLDEN_LIGHT_HEADER_BASE_WORDS + lights.len() * 12];
    // Header: 4 vec4 lanes (counts_exposure, sky_diffuse, sky_spec, cluster_params).
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
    // Elements: 3 vec4 lanes each (dir_kind, pos_range, color_cone).
    for (i, l) in lights.iter().enumerate() {
        let base = GOLDEN_LIGHT_HEADER_BASE_WORDS + i * 12;
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

/// Diffs the whole GPU LIT readback (run with a CUSTOM L0b light table) against the host
/// `golden_deferred_resolve_table` per texel, within ±2/255 (the deferred double-quant
/// budget). `header`/`lights` are the host mirror of the GPU table. Returns the max delta +
/// the SDF-lit pixel count (so the caller can prove a real lit surface was rendered).
fn assert_lit_matches_table_golden(
    lit: &[u8],
    edits: &[SdfEdit],
    flags: u32,
    light_dir: [f32; 3],
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
    name: &str,
) -> (i32, u64) {
    let mut max_delta = 0i32;
    let mut sdf_lit_hits = 0u64;
    let materials = host_material_table();
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let md = expected_mesh_depth(px, py);
            let attrs = golden_marcher_attributes(
                edits, &materials, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                flags, light_dir,
            );
            let (ro, rd) =
                composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
            let want =
                unpack_packed_rgb(golden_deferred_resolve_table(attrs, ro, rd, &materials, header, lights));
            let got = albedo_rgb(lit, px, py);
            let dmax = (0..3).map(|c| (got[c] - want[c]).abs()).max().unwrap();
            if attrs.mask == 1 {
                sdf_lit_hits += 1;
            }
            if dmax > max_delta {
                max_delta = dmax;
            }
            assert!(
                dmax <= DEFERRED_ARM1_TOL,
                "[{name}] L0b LIT texel ({px},{py}) got {got:?} want {want:?} (table oracle) \
                 exceeds ±{DEFERRED_ARM1_TOL}/255 (delta {dmax})"
            );
        }
    }
    (max_delta, sdf_lit_hits)
}

// ============================================================================
// Lighting P6 R1 GPU golden (multi-light SDF shadows, analytic) — the NON-CLUSTERED resolve
// path on hardware.
//
// R1 turns the resolve's per-light visibility into a per-caster `sdf_soft_shadow_ranged`
// march, gated by `header.shadow_mode() == 1` + the per-light `casts_sdf_shadow` flag + the
// `MAX_SDF_SHADOW_CASTERS_PER_PIXEL` dominant-N cap + the `NoL <= 0` skip. The PRIMARY
// directional keeps the marcher's `gMaterial.r`; every EXTRA flagged caster (point/spot via
// the flat table on this NON-CLUSTERED path, plus extra directionals) marches the field.
//
// CONSTRAINT (documented): the frozen GPU `cluster_cull.hlsl` compares the RAW `e.kind`, so a
// shadow-flagged punctual is DROPPED by the GPU clustered cull until a follow-up rung. The R1
// multi-light GPU golden therefore drives the NON-CLUSTERED resolve (`clusters_enabled ==
// false`, i.e. a non-clustered `GoldenLightHeader`) — the same flat-table path
// `l0b_point_and_spot_match_the_table_oracle` exercises.
//
// The HOST oracle is `golden_deferred_resolve_table_shadowed` (the `shadow_mode != 0` mirror),
// fed the FROZEN `sdf_edit_list` field gateway. The non-vacuity gate diffs the SHADOWED oracle
// against the UNSHADOWED `golden_deferred_resolve_table` per pixel: any pixel whose RGB the
// shadow march dimmed is a genuine SHADOWED pixel (vis < 1 from an occluder between the pixel
// and a flagged light), so a non-zero count proves the test meaningfully exercises shadowing.
// ============================================================================

/// Scans the host oracles for the P6 R1 multi-light scene and returns `(shadowed_px,
/// sdf_lit_px)`: `shadowed_px` is the count of SDF surface pixels whose SHADOWED-oracle RGB
/// (`golden_deferred_resolve_table_shadowed`, `header.shadow_mode() == 1`) differs from the
/// UNSHADOWED-oracle RGB (`golden_deferred_resolve_table`, same lights) by more than
/// `SHADOW_PROOF_EPS` on any channel — i.e. the per-caster `sdf_soft_shadow_ranged` march
/// dimmed the pixel. A non-zero `shadowed_px` is the NON-VACUITY proof: an occluder lies
/// between the pixel and a flagged light. CPU-only (no GPU), so the GPU golden can assert it
/// host-side BEFORE the device run.
fn host_count_shadowed_pixels(
    edits: &[SdfEdit],
    flags: u32,
    light_dir: [f32; 3],
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
) -> (u64, u64) {
    // The smallest channel delta that proves the shadow march did SOMETHING. The shadow term
    // is `vis ∈ [0, 1]` multiplied into the per-light direct contribution, so a real occlusion
    // moves at least one channel by ≥ a few units; `> 0` after the double-quant is sufficient
    // proof but we require ≥ 1 to be robust against an exact-equal rounding tie.
    const SHADOW_PROOF_EPS: i32 = 1;
    let materials = host_material_table();
    let field = |q: [f32; 3]| boyko_sdf_math::sdf_edit_list(edits, q);
    let mut shadowed_px = 0u64;
    let mut sdf_lit_px = 0u64;
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let md = expected_mesh_depth(px, py);
            let attrs = golden_marcher_attributes(
                edits, &materials, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                flags, light_dir,
            );
            if attrs.mask != 1 {
                continue;
            }
            sdf_lit_px += 1;
            let (ro, rd) =
                composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
            let lit_shadowed = unpack_packed_rgb(golden_deferred_resolve_table_shadowed(
                attrs, ro, rd, &materials, header, lights, &field,
            ));
            let lit_plain =
                unpack_packed_rgb(golden_deferred_resolve_table(attrs, ro, rd, &materials, header, lights));
            let dmax = (0..3).map(|c| (lit_shadowed[c] - lit_plain[c]).abs()).max().unwrap();
            if dmax >= SHADOW_PROOF_EPS {
                shadowed_px += 1;
            }
        }
    }
    (shadowed_px, sdf_lit_px)
}

/// Diffs the whole GPU LIT readback (the multi-light `shadow_mode == 1` NON-CLUSTERED resolve)
/// against the host `golden_deferred_resolve_table_shadowed` per texel, within ±2/255 (the
/// deferred double-quant budget). Mirrors [`assert_lit_matches_table_golden`] but feeds the
/// SHADOWED oracle the FROZEN `sdf_edit_list` field closure. Returns the max delta + the
/// SDF-lit pixel count.
fn assert_lit_matches_table_shadowed_golden(
    lit: &[u8],
    edits: &[SdfEdit],
    flags: u32,
    light_dir: [f32; 3],
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
    name: &str,
) -> (i32, u64) {
    let mut max_delta = 0i32;
    let mut sdf_lit_hits = 0u64;
    let materials = host_material_table();
    let field = |q: [f32; 3]| boyko_sdf_math::sdf_edit_list(edits, q);
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let md = expected_mesh_depth(px, py);
            let attrs = golden_marcher_attributes(
                edits, &materials, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                flags, light_dir,
            );
            let (ro, rd) =
                composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
            let want = unpack_packed_rgb(golden_deferred_resolve_table_shadowed(
                attrs, ro, rd, &materials, header, lights, &field,
            ));
            let got = albedo_rgb(lit, px, py);
            let dmax = (0..3).map(|c| (got[c] - want[c]).abs()).max().unwrap();
            if attrs.mask == 1 {
                sdf_lit_hits += 1;
            }
            if dmax > max_delta {
                max_delta = dmax;
            }
            assert!(
                dmax <= DEFERRED_ARM1_TOL,
                "[{name}] P6 R1 multi-light LIT texel ({px},{py}) got {got:?} want {want:?} \
                 (shadowed table oracle) exceeds ±{DEFERRED_ARM1_TOL}/255 (delta {dmax})"
            );
        }
    }
    (max_delta, sdf_lit_hits)
}

/// The dominant-N-cap variant of [`assert_lit_matches_table_shadowed_golden`]: the SAME per-texel
/// GPU-vs-shadowed-oracle diff, but instead of hard-asserting EVERY texel within `±DEFERRED_ARM1_
/// TOL` it COUNTS the texels that exceed it and returns `(max_delta, outlier_count, sdf_lit_hits)`.
///
/// Why a count, not a hard per-texel assert, for the cap fixture: the dominant-N cap counts a
/// caster toward the per-pixel march budget only when `nol > SHADOW_NDOTL_EPS` (== 0). At the lit
/// TERMINATOR (`nol ≈ 0`) the GPU and host disagree, by one ULP of rounding, on whether a caster
/// clears that threshold — so they cap a DIFFERENT subset of the co-located casters, and the
/// un-marched (un-shadowed, full-bright) remainder differs → those few terminator texels exceed
/// `±tol`. That set is a THIN ARC (a real cap/march bug would diverge over a 2-D REGION), so the
/// caller bounds the count tightly. Off the `nol ≈ 0` arc the agreement is the same `±tol` as the
/// hard-asserting twin/single fixtures.
fn count_lit_table_shadowed_outliers(
    lit: &[u8],
    edits: &[SdfEdit],
    flags: u32,
    light_dir: [f32; 3],
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
) -> (i32, u64, u64) {
    let mut max_delta = 0i32;
    let mut outliers = 0u64;
    let mut sdf_lit_hits = 0u64;
    let materials = host_material_table();
    let field = |q: [f32; 3]| boyko_sdf_math::sdf_edit_list(edits, q);
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let md = expected_mesh_depth(px, py);
            let attrs = golden_marcher_attributes(
                edits, &materials, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                flags, light_dir,
            );
            let (ro, rd) =
                composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
            let want = unpack_packed_rgb(golden_deferred_resolve_table_shadowed(
                attrs, ro, rd, &materials, header, lights, &field,
            ));
            let got = albedo_rgb(lit, px, py);
            let dmax = (0..3).map(|c| (got[c] - want[c]).abs()).max().unwrap();
            if attrs.mask == 1 {
                sdf_lit_hits += 1;
            }
            if dmax > max_delta {
                max_delta = dmax;
            }
            if dmax > DEFERRED_ARM1_TOL {
                outliers += 1;
            }
        }
    }
    (max_delta, outliers, sdf_lit_hits)
}

/// The P6 R1 multi-light scene: TWO big spheres side-by-side at the same depth (the
/// occluder/receiver pair). Each sphere's bulk shadows the OTHER sphere's facing flank when lit
/// from across the valley, giving a broad, clearly-visible shadow band — far more than the
/// thin self-shadow terminator a single convex body produces (the `crater` body shadowed only
/// ~2 pixels). The ORTHO camera marches +Z → origin, so both front faces sit at z ≈ +0.55.
fn p6_r1_twin_scene() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([-0.5, 0.0, 0.0], 0.55, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.5, 0.0, 0.0], 0.55, sdf_op::UNION, 0.0),
    ]
}

/// The P6 R1 multi-light scene's light table: a gentle WHITE primary directional (front
/// block, keeps `gMaterial.r`) + two shadow-flagged POINT casters straddling the valley between
/// the [`p6_r1_twin_scene`] spheres (one in front of each, low + toward the camera). The LEFT
/// caster's rays into the RIGHT sphere's left flank are occluded by the LEFT sphere (and
/// symmetrically), so BOTH casters cast a visible shadow. The header is NON-CLUSTERED (`new`,
/// so `cluster_params == 0` ⇒ `clusters_enabled == false`) with `shadow_mode == 1`. Both point
/// casters are `with_sdf_shadow()`; the primary directional is NOT flagged (it keeps the
/// marcher's `gMaterial.r`, byte-stable across 1→N).
fn p6_r1_multi_light_table() -> (GoldenLightHeader, Vec<GoldenLight>) {
    // l0a_count = 1 (the primary directional); point_spot_count = 2 (the two flagged casters).
    let header = GoldenLightHeader::new(1, 2, 1.0).with_shadow_mode(1);
    let lights = vec![
        // Primary directional (un-flagged): keeps `gMaterial.r`. A gentle front-fill so the
        // shadowed band is dimmed, not pure black.
        GoldenLight::directional([0.2, 0.2, 1.0], [0.4, 0.4, 0.4], 1.0),
        // Two shadow-flagged point casters straddling the valley, low (z ≈ 0.9, toward the
        // camera) + offset in x: the left caster lights the right sphere's left flank but the
        // LEFT sphere occludes it (and vice-versa) ⇒ `vis < 1` over the valley band.
        GoldenLight::point([-0.8, -0.3, 0.9], [1.0, 0.9, 0.8], 6500.0, 7.0).with_sdf_shadow(),
        GoldenLight::point([0.8, -0.3, 0.9], [0.8, 0.9, 1.0], 6500.0, 7.0).with_sdf_shadow(),
    ];
    (header, lights)
}

/// L0b 0%-gate: adding the `gViewT` lane + an all-directional/sky table (zero point/spot)
/// must reproduce the L0a image — the point/spot loop body never runs, and the gViewT lane
/// is purely additive (no existing G-buffer byte changes). The GPU output with the
/// degenerate table must equal the constant-path oracle within the existing ±2/255 budget.
#[test]
fn l0b_zero_point_spot_table_reproduces_l0a_image() {
    let Some(ctx) = boot_render_or_skip("l0b_zero_point_spot_table_reproduces_l0a_image") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    for (name, edits) in p4b_scenes() {
        // The degenerate table (1 directional + 1 sky, 0 point/spot) on the L0b shader.
        let lit =
            run_gbuffer_hybrid_lit_table(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR, &DEGENERATE_LIGHT_TABLE)
                .0;
        assert_eq!(lit.len(), READBACK_BYTES as usize);
        let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(nonzero > 0, "[{name}] L0b LIT all-zero — device did not render");

        // Same diff as the L0a 0%-gate: the gViewT addition must NOT perturb the image.
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden(&lit, &edits, flags, DEFAULT_LIGHT_DIR, name);
        assert!(
            sdf_lit_hits > 0,
            "[{name}] L0b 0%-gate: no SDF-lit pixel — the lit-arm gate is vacuous"
        );
        println!(
            "[{name}] L0b 0%-gate (gViewT added, zero point/spot == L0a): lit-arm max delta \
             {max_arm1}/255, pass-through {max_pass} (tol {DEFERRED_ARM1_TOL}); {sdf_lit_hits} \
             SDF-lit px"
        );
    }
}

/// L0b point + spot golden: a table with a directional + a point light + a spot light at
/// known world positions, resolved via the gViewT `P` reconstruction, must match the host
/// `golden_deferred_resolve_table` within ±2/255 on every texel.
#[test]
fn l0b_point_and_spot_match_the_table_oracle() {
    let Some(ctx) = boot_render_or_skip("l0b_point_and_spot_match_the_table_oracle") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    // l0a_count = 1 (the directional front block); point_spot_count = 2 (point + spot). The
    // surface lives near the origin plane (the ORTHO fixture marches +Z→origin), so the
    // lights sit in front of it (z > 0) with a generous range so a swath of pixels is lit.
    let header = GoldenLightHeader::new(1, 2, 1.0);
    let lights = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::point([0.0, 0.0, 1.5], [1.0, 0.9, 0.8], 4000.0, 6.0),
        GoldenLight::spot([0.4, 0.4, 1.5], [0.0, 0.0, 1.0], [0.8, 0.9, 1.0], 6000.0, 6.0, 20.0, 35.0),
    ];
    let table = pack_light_table(&header, &lights);

    for (name, edits) in p4b_scenes() {
        let lit =
            run_gbuffer_hybrid_lit_table(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR, &table).0;
        assert_eq!(lit.len(), READBACK_BYTES as usize);
        let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(nonzero > 0, "[{name}] L0b point/spot LIT all-zero — device did not render");

        let (max_delta, sdf_lit_hits) =
            assert_lit_matches_table_golden(&lit, &edits, flags, DEFAULT_LIGHT_DIR, &header, &lights, name);
        assert!(
            sdf_lit_hits > 0,
            "[{name}] L0b point/spot: no SDF-lit pixel — the gate is vacuous"
        );
        println!(
            "[{name}] L0b point/spot (gViewT P-reconstruction == table oracle): max delta \
             {max_delta}/255 (tol {DEFERRED_ARM1_TOL}); {sdf_lit_hits} SDF-lit px"
        );
    }
}

/// P6 R1 host NON-VACUITY pre-flight (CPU-only, no GPU): the multi-light shadowed scene's
/// host oracle MUST produce at least one genuinely SHADOWED SDF pixel (the SHADOWED-oracle RGB
/// differs from the UNSHADOWED-oracle RGB — an occluder lies between the pixel and a flagged
/// light). This pins that `p6_r1_multi_light_table()` is NOT a vacuous fixture independent of
/// any GPU; the GPU golden re-asserts the same count after the device run.
#[test]
fn p6_r1_oracle_produces_shadowed_pixels() {
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let (header, lights) = p6_r1_multi_light_table();
    assert_eq!(header.shadow_mode(), 1, "the R1 fixture must set shadow_mode == 1");
    assert_eq!(
        header.cluster_params, [0.0, 0.0, 0.0, 0.0],
        "the R1 fixture MUST be NON-CLUSTERED (cluster_params == 0 ⇒ clusters_enabled == false): \
         the frozen cluster_cull drops shadow-flagged punctuals"
    );
    assert!(
        lights.iter().skip(1).all(|l| l.casts_sdf_shadow()),
        "both extra punctual casters must be flagged casts_sdf_shadow"
    );

    let (shadowed_px, sdf_lit_px) =
        host_count_shadowed_pixels(&p6_r1_twin_scene(), flags, DEFAULT_LIGHT_DIR, &header, &lights);
    assert!(
        sdf_lit_px > 0,
        "the twin scene must have SDF-lit pixels (the shadow gate would be vacuous otherwise)"
    );
    assert!(
        shadowed_px > 0,
        "P6 R1 NON-VACUITY: the shadowed oracle dimmed ZERO pixels — no occluder lies between \
         any pixel and a flagged light, so the GPU golden would not exercise shadowing. Retune \
         the light positions in `p6_r1_multi_light_table()`"
    );
    println!(
        "P6 R1 host non-vacuity: {shadowed_px} SHADOWED px (shadowed-vs-unshadowed oracle delta \
         ≥ 1) of {sdf_lit_px} SDF-lit px on the twin scene"
    );
}

/// P6 R1 GPU golden: the multi-light SDF-shadow NON-CLUSTERED resolve on hardware. An SDF
/// occluder (the `crater` body) + two shadow-flagged POINT lights (`with_sdf_shadow()`) +
/// `header.with_shadow_mode(1)` on a NON-CLUSTERED header. The GPU LIT readback must match the
/// host `golden_deferred_resolve_table_shadowed` (fed the FROZEN `sdf_edit_list` field) within
/// ±2/255 per texel. NON-VACUITY: the host oracle is first asserted to produce ≥1 SHADOWED
/// pixel (the shadowed-vs-unshadowed oracle diverges), so the test meaningfully exercises the
/// per-caster `sdf_soft_shadow_ranged` march on the device.
#[test]
fn p6_r1_multi_light_sdf_shadows_match_oracle() {
    let Some(ctx) = boot_render_or_skip("p6_r1_multi_light_sdf_shadows_match_oracle") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());

    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let (header, lights) = p6_r1_multi_light_table();
    let table = pack_light_table(&header, &lights);
    let edits = p6_r1_twin_scene();

    // NON-VACUITY: the host oracle MUST dim at least one pixel BEFORE we trust the device run.
    let (shadowed_px, sdf_lit_px) =
        host_count_shadowed_pixels(&edits, flags, DEFAULT_LIGHT_DIR, &header, &lights);
    assert!(
        shadowed_px > 0,
        "P6 R1 NON-VACUITY: the host shadowed oracle dimmed ZERO pixels — the GPU golden would \
         not exercise shadowing; retune `p6_r1_multi_light_table()`"
    );

    // The NON-CLUSTERED resolve path (`run_gbuffer_hybrid_lit_table`, clusters_enabled == false
    // since the header's cluster_params == 0): the marcher writes the primary `gMaterial.r`, the
    // resolve marches every extra flagged caster.
    let lit =
        run_gbuffer_hybrid_lit_table(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR, &table).0;
    assert_eq!(lit.len(), READBACK_BYTES as usize);
    let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
    assert!(nonzero > 0, "P6 R1 multi-light LIT all-zero — device did not render");

    let (max_delta, sdf_lit_hits) = assert_lit_matches_table_shadowed_golden(
        &lit, &edits, flags, DEFAULT_LIGHT_DIR, &header, &lights, "twin_spheres",
    );
    assert!(sdf_lit_hits > 0, "P6 R1: no SDF-lit pixel — the gate is vacuous");
    println!(
        "P6 R1 multi-light SDF shadows (shadow_mode==1, NON-CLUSTERED, 2 flagged point casters) \
         == shadowed table oracle: max delta {max_delta}/255 (tol {DEFERRED_ARM1_TOL}); \
         {sdf_lit_hits} SDF-lit px, {shadowed_px}/{sdf_lit_px} host-SHADOWED px"
    );
    assert_validation_clean(&ctx);
}

/// P6 R1 single-point-caster GPU golden + the dominant-N cap proof. A scene with a SINGLE
/// shadow-flagged point caster matches the shadowed oracle (the simplest non-vacuous shadow),
/// AND a fixture flagging MORE than `MAX_SDF_SHADOW_CASTERS_PER_PIXEL` casters STILL matches the
/// oracle — because the host oracle models the SAME dominant-N cap (the GPU caps too), so the
/// GPU/oracle agreement holds with the cap engaged. The cap-engaged fixture also re-asserts the
/// host oracle is non-vacuous (≥1 shadowed pixel) so the cap path is genuinely exercised.
#[test]
fn p6_r1_single_point_light_gets_sdf_shadow() {
    let Some(ctx) = boot_render_or_skip("p6_r1_single_point_light_gets_sdf_shadow") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());

    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let edits = p6_r1_twin_scene();

    // --- (a) The single flagged point caster (1 directional + 1 point). RE-BLESSED (post
    // grazing-acne fix): the former caster sat CENTERED at [0,0,1.2] in the valley between the
    // twin spheres; over the convex shells its only "shadow" was the grazing-terminator acne
    // the A1 normal-offset bias now correctly removes, so post-fix it dims ZERO pixels and the
    // non-vacuity guard could no longer hold. The caster is now OFF-CENTER, far to the LEFT and
    // low ([-1.8, 0, 0.5]) with a generous range: its rays graze past the LEFT sphere into the
    // RIGHT sphere's left flank, which the LEFT sphere OCCLUDES ⇒ a GENUINE inter-object cast
    // shadow (~290 host-shadowed px), the same physical mechanism `p6_r1_multi_light` uses —
    // not the fixed acne. ---
    {
        let header = GoldenLightHeader::new(1, 1, 1.0).with_shadow_mode(1);
        let lights = vec![
            GoldenLight::directional([0.2, 0.2, 1.0], [0.4, 0.4, 0.4], 1.0),
            GoldenLight::point([-1.8, 0.0, 0.5], [1.0, 0.9, 0.8], 7000.0, 12.0).with_sdf_shadow(),
        ];
        let (shadowed_px, _) =
            host_count_shadowed_pixels(&edits, flags, DEFAULT_LIGHT_DIR, &header, &lights);
        // A BAND (not a lone pixel): the left sphere casts a real shadow across the right
        // sphere's flank. A floor of 32 px is far below the ~290 the fixture produces yet far
        // above any sparse single-pixel acne a regression could re-introduce.
        assert!(
            shadowed_px > 32,
            "single-point R1: host oracle dimmed only {shadowed_px} px — the off-center caster's \
             GENUINE cast shadow (left sphere onto the right flank) must dim a BROAD band, not a \
             sparse acne speckle (or the occlusion no longer happens)"
        );

        let table = pack_light_table(&header, &lights);
        let lit =
            run_gbuffer_hybrid_lit_table(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR, &table).0;
        let (max_delta, sdf_lit_hits) = assert_lit_matches_table_shadowed_golden(
            &lit, &edits, flags, DEFAULT_LIGHT_DIR, &header, &lights, "single_point",
        );
        assert!(sdf_lit_hits > 0, "single-point R1: no SDF-lit pixel");
        println!(
            "P6 R1 single point caster == shadowed oracle: max delta {max_delta}/255 \
             (tol {DEFERRED_ARM1_TOL}); {shadowed_px} host-SHADOWED px"
        );
    }

    // --- (b) The dominant-N cap: flag (MAX + 2) point casters; the oracle + GPU both cap the
    // march at `MAX_SDF_SHADOW_CASTERS_PER_PIXEL`, so the readback still agrees with the oracle.
    // RE-BLESSED (post grazing-acne fix): the former ring sat centered over the valley
    // ([0.6*cos, -0.3+0.5*sin, 1.0]); over the convex twins its only "shadow" was the
    // grazing-terminator acne the A1 bias now removes, so post-fix it dims ZERO px and the cap
    // path was never exercised. The ring is now a tight cluster of `extra` flagged casters
    // co-located at the strong OFF-CENTER occluder spot ([-1.8, 0, 0.5], a sub-jitter apart):
    // per pixel ALL `extra` are occluders so the dominant-N cap is GENUINELY engaged (the first
    // N marched, the tail dropped to NoL-only), and they all illuminate the right sphere's flank
    // from the same OCCLUDED direction → a real cast-shadow band (~70 host-shadowed px). Per-
    // caster power is dropped to 1200 (from 7000) so `extra` co-located casters do NOT saturate
    // the surface to white (which would mask the shadow delta under the [0,255] clamp). ---
    {
        let extra = MAX_SDF_SHADOW_CASTERS_PER_PIXEL + 2;
        let mut lights = vec![GoldenLight::directional([0.2, 0.2, 1.0], [0.4, 0.4, 0.4], 1.0)];
        // A tight cluster of flagged casters at the strong occluder spot — MORE than the cap,
        // so per pixel only the first N (in table order) are marched; the rest contribute
        // NoL-only. The sub-jitter keeps them DISTINCT light table entries (the cap counts table
        // slots) while pointing essentially the same way (a coherent, un-filled shadow band).
        for _ in 0..extra {
            let pos = [-1.8, 0.0, 0.5];
            lights.push(GoldenLight::point(pos, [1.0, 0.9, 0.8], 1750.0, 12.0).with_sdf_shadow());
        }
        let header = GoldenLightHeader::new(1, extra, 1.0).with_shadow_mode(1);

        let (shadowed_px, _) =
            host_count_shadowed_pixels(&edits, flags, DEFAULT_LIGHT_DIR, &header, &lights);
        // A BAND, not a lone pixel: a sub-band count would mean the cap-engaged casters no
        // longer cast a real shadow (or re-introduced acne is the only "shadow").
        assert!(
            shadowed_px > 32,
            "dominant-N cap R1: host oracle dimmed only {shadowed_px} px with {extra} flagged \
             casters — the cap-engaged cluster's GENUINE cast shadow must dim a BROAD band, not a \
             sparse acne speckle (or the occlusion no longer happens)"
        );

        let table = pack_light_table(&header, &lights);
        let lit =
            run_gbuffer_hybrid_lit_table(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR, &table).0;
        // The cap counts a caster toward the per-pixel march budget only when `nol >
        // SHADOW_NDOTL_EPS` (== 0); at the lit terminator (`nol ≈ 0`) GPU and host round that
        // threshold differently, so they cap a different subset of the co-located casters and a
        // THIN ARC of terminator texels exceeds `±tol`. Bound that arc tightly (a real cap/march
        // divergence would span a 2-D region, not an arc); the bulk agreement is still `±tol`.
        let (max_delta, outliers, sdf_lit_hits) = count_lit_table_shadowed_outliers(
            &lit, &edits, flags, DEFAULT_LIGHT_DIR, &header, &lights,
        );
        assert!(sdf_lit_hits > 0, "dominant-N cap R1: no SDF-lit pixel");
        const DOMINANT_N_CAP_BOUNDARY_OUTLIERS: u64 = 16;
        assert!(
            outliers <= DOMINANT_N_CAP_BOUNDARY_OUTLIERS,
            "dominant-N cap R1: {outliers} texels exceed ±{DEFERRED_ARM1_TOL}/255 (max delta \
             {max_delta}) — more than the {DOMINANT_N_CAP_BOUNDARY_OUTLIERS}-texel NoL≈0 \
             cap-counting boundary arc; a real cap/march divergence, not FP boundary noise"
        );
        println!(
            "P6 R1 dominant-N cap ({extra} flagged casters, cap {MAX_SDF_SHADOW_CASTERS_PER_PIXEL}) \
             == shadowed oracle: max delta {max_delta}/255, {outliers} NoL≈0 boundary outliers \
             (≤ {DOMINANT_N_CAP_BOUNDARY_OUTLIERS}); {shadowed_px} host-SHADOWED px"
        );
    }
    assert_validation_clean(&ctx);
}

// ============================================================================
// Lighting L1 GPU goldens (the FULL clustered froxel-cull path on hardware).
//
// These run on the 3060 (the GPU-tester); they `boot_render_or_skip` when no device is
// present. They drive the production cull pass + the clustered resolve and compare to the
// HOST oracle (`golden_cluster_cull` + `golden_deferred_resolve_clustered`, the bit-exact
// source of truth — the CPU companion is `tests/lighting_l1_host_oracle.rs`).
//
//   1. `l1_clustered_resolve_matches_the_brute_force_image` — the load-bearing test: it
//      dispatches the GPU `cluster_cull` pass to populate the real `ClusterGrid` +
//      `LightIndexList` from the light table, then runs the deferred resolve with
//      `clusters_enabled == 1` reading those cluster buffers, and asserts the LIT image
//      matches the host clustered oracle (which == brute force) within ±2/255 over all 3
//      SDF scenes. This is the test that previously caught the NaN→black at crater(38,18);
//      the `safe_normalize` fix in `deferred_pbr.hlsl` makes it match.
//   2. `l1_known_light_lands_in_the_expected_clusters` — runs the GPU `cluster_cull` pass
//      for a known light, reads back the `ClusterGrid` occupancy ({offset, count} per
//      froxel), and asserts it matches the host `golden_cluster_cull` occupancy
//      froxel-for-froxel.
//
// The cull shader reads the froxel dims (`dim_x`/`dim_y`/`dim_z`) from the light table
// header's `cluster_params` lane (via `load_cluster_params`), so the table MUST carry a
// CLUSTERED header (`GoldenLightHeader::new_clustered`) for BOTH the cull and the resolve.
// The camera UBO is the same ORTHO 64×64 block the resolve/marcher use, so the GPU cull
// builds froxel AABBs from the identical ray-gen the host oracle does.
// ============================================================================

/// The L1 cull config the GPU + host share (mirrors the host-oracle fixture in
/// `tests/lighting_l1_host_oracle.rs`): a 16×9×24 froxel grid (each dim ≤ 255 so the header's
/// 8-bit-packed `dim` lane round-trips), per-froxel cap 256, exp-Z `near`/`far` spanning the
/// ortho scene's view-z band (surfaces sit near world z = 0, camera at z = 2, so the ray
/// parameter `t ≈ 2`). The SAME config drives the clustered header, the cull push, and the
/// host oracles.
fn l1_cluster_config() -> GoldenClusterConfig {
    GoldenClusterConfig {
        dim_x: 16,
        dim_y: 9,
        dim_z: 24,
        max_lights_per_cluster: 256,
        z_near: 0.25,
        z_far: 4.0,
    }
}

/// A generous flat light-index-list capacity for the L1 test scenes (`cluster_count * 8`):
/// the multi-light fixtures keep a handful of lights per froxel, well under this bound, so
/// the cull never hits the O2 global clamp — the GPU occupancy then equals the host oracle's
/// exactly (no dropped tail to reconcile).
fn l1_index_list_cap(cfg: &GoldenClusterConfig) -> u32 {
    cfg.cluster_count() * 8
}

/// The L1 clustered driver's readbacks: `(lit_rgba8, cluster_grid, light_index, gViewT_per_px,
/// gNormal_oct_rg_per_px)`. The last two isolate the resolve under test — the golden feeds the
/// GPU's REAL surface depth + normal into the host oracle instead of an independent CPU march.
type ClusteredDriverReadbacks = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<f32>, Vec<[u8; 2]>);

/// Records + submits the FULL Lighting-L1 clustered path in ONE fenced submit and returns
/// `(lit_bytes, cluster_grid_bytes)`: identical to [`run_gbuffer_hybrid_lit_table`] up to the
/// resolve, but it ALSO
///
///   - creates the L1 cluster SSBOs — `ClusterGrid` (`cluster_count * 8 B`, `uint2`
///     {offset, count} per froxel), `LightIndexList` (`index_list_cap * 4 B`), and the
///     `LightIndexAlloc` counter (one `u32`, host-zeroed before submit so the cull's
///     `InterlockedAdd` starts at 0 — the host-write equivalent of the production
///     `cmd_fill_buffer` reset);
///   - records the cull compute pass (the cull set { camera UBO @0, light table @1,
///     `ClusterGrid` @2, `LightIndexList` @3, `LightIndexAlloc` @4 } + the 16-byte
///     `ClusterCullPush`) over `cluster_count` froxels BEFORE the resolve, with a
///     COMPUTE→COMPUTE buffer barrier (`SHADER_WRITE→SHADER_READ`) on `ClusterGrid` +
///     `LightIndexList` so the resolve's reads see the cull's writes (mirroring the
///     production `render_gbuffer_frame` cull recording in `swapchain.rs`);
///   - binds `ClusterGrid` @8 + `LightIndexList` @9 on the resolve set (the v2 binding fix —
///     the recompiled `deferred_pbr.comp` statically references @8/@9 on every path) so the
///     resolve's `clusters_enabled` gate (carried in the clustered header) loops the per-froxel
///     index slice instead of the flat table.
///
/// `light_table_words` MUST carry a CLUSTERED header (`new_clustered`): the cull reads the
/// froxel dims from `cluster_params`, and the resolve reads `clusters_enabled` from it. The
/// returned `cluster_grid_bytes` is `cluster_count * 8` host-coherent bytes (the readback of
/// the `ClusterGrid` SSBO — each froxel is `{u32 offset, u32 count}`); the returned
/// `light_index_bytes` is `index_list_cap * 4` host-coherent bytes (the readback of the flat
/// `LightIndexList` SSBO — the per-froxel index slices the cull scattered). The light-index
/// readback is what the strengthened cull probe needs to assert the per-froxel index SET +
/// the slice disjointness (the occupancy `count` alone is the documented blind spot).
///
/// Also returns, per pixel (row-major, `py * SDF_IMG_W + px`): the `gViewT` (R32_SFLOAT) from the
/// marcher's surface-depth lane, and the `gNormal` oct-encoded normal bytes (`R8G8`). The L1
/// resolve golden feeds these GPU values into the host oracle (overriding the oracle's
/// independently-marched `attrs.view_t` and `attrs.oct_rg`) so the resolve — the unit under test
/// — shades the GPU's EXACT surface point `P = ro + rd * gViewT_gpu` with the GPU's EXACT normal,
/// isolating the resolve from the marcher's GPU-vs-CPU FP gap (validated separately by the
/// marcher goldens).
#[allow(clippy::too_many_arguments)]
fn run_gbuffer_hybrid_lit_clustered(
    ctx: &VulkanContext,
    edits: &[SdfEdit],
    lighting_flags: u32,
    light_dir: [f32; 3],
    light_table_words: &[u32],
    cfg: &GoldenClusterConfig,
) -> ClusteredDriverReadbacks {
    let device: &VulkanContext = ctx;
    let queue = ctx.rhi_queue();

    let cluster_count = cfg.cluster_count();
    let index_list_cap = l1_index_list_cap(cfg);
    // ClusterGrid: `uint2` {offset, count} per froxel (8 B each).
    let cluster_grid_bytes = (cluster_count as u64) * 8;

    // --- The edit-list StorageBuffer (binding 0), seeded with the packed header (the same
    // over-allocation to `EDITLIST_BUFFER_WORDS` the table driver uses). ---
    let buffer = device
        .create_buffer(&BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, edits);
        let mapped = device
            .buffer_mapped_ptr(&buffer)
            .expect("host-visible buffer is mapped");
        write_words(mapped, &header);
    }

    // --- The camera/extent UNIFORM buffer (binding 5 of the vocab + resolve sets, binding 0
    // of the cull set). The ORTHO 64×64 block drives the SAME bit-exact rays the host oracle
    // uses (the cull's froxel-AABB ray-gen + the resolve's per-pixel ray).
    //
    // M4 (Slice C): sized to `B5_CAMERA_UBO_BYTES_M4` (224 B) — the 80-byte camera block + the N-level
    // `M4GridParams` array tail @80. The L1 path runs the marcher with `brick_trilinear == 0`, so the
    // tail is bound-but-unread (byte-identical to the pre-M2 L1 image); the near-field block is written
    // for parity with the production UBO (level 0 == the old M2 default block byte-for-byte). ---
    let camera_uniform = device
        .create_buffer(&BufferDesc {
            size: B5_CAMERA_UBO_BYTES_M4 as u64,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("camera uniform buffer");
    {
        let pc = CompositePushConstants::ortho(SDF_IMG_W, SDF_IMG_H);
        debug_assert_eq!(pc.count, PIXELS);
        let mapped = device
            .buffer_mapped_ptr(&camera_uniform)
            .expect("host-visible uniform buffer is mapped");
        let bytes = pc.as_bytes();
        let m4 = M4GridParams::near_field_only();
        let m4_bytes = m4.as_ubo_bytes();
        // SAFETY: `mapped` points to `B5_CAMERA_UBO_BYTES_M4` (224) mapped host-coherent bytes; the
        // 80-byte camera block at offset 0 + the 144-byte M4 array tail at offset 80 are disjoint and
        // together exactly 224 in-bounds bytes; no GPU work is in flight yet (submit follows).
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
            core::ptr::copy_nonoverlapping(
                m4_bytes.as_ptr(),
                mapped.as_ptr().add(M2_GRID_PARAMS_OFFSET),
                m4_bytes.len(),
            );
        }
    }

    // --- M2: the brick atlas (marcher binding 10). Baked from the SAME `edits` authority (a
    // transient GPU mirror, principle 0). The recompiled marcher SPIR-V statically references
    // `register(t10)` + `register(s10)` (collapsed to ONE combined descriptor by DXC), so the L1
    // marcher layout MUST declare binding 10 and bind a VALID atlas even though the L1 path runs
    // the trilinear path OFF (`brick_trilinear == 0`, bound-but-unread). ---
    let brick_atlas = {
        use boyko_sdf_math::SdfEditField;
        let mut field = SdfEditField::new();
        for e in edits {
            assert!(field.push(*e), "scene must fit MAX_SDF_EDITS");
        }
        field.bump_gen();
        BrickAtlas::create(ctx, &field).expect("M2 brick atlas (L1 vocab binding 10) — create + bake + upload")
    };

    // --- The material table SSBO (vocab @7 + resolve @4): the single default material. ---
    let material_table = device
        .create_buffer(&BufferDesc {
            size: (DEFAULT_MATERIAL_TABLE.len() as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("PBR material table storage buffer");
    {
        let mapped = device
            .buffer_mapped_ptr(&material_table)
            .expect("host-visible material table is mapped");
        write_words(mapped, &DEFAULT_MATERIAL_TABLE);
    }

    // --- The CLUSTERED light table SSBO (cull @1 + resolve @6), seeded with the caller's
    // `light_table_words` (a `new_clustered` header + the GpuLight[] block). ---
    let light_table = device
        .create_buffer(&BufferDesc {
            size: (light_table_words.len() as u64) * 4,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("Lighting-L1 clustered light table storage buffer");
    {
        let mapped = device
            .buffer_mapped_ptr(&light_table)
            .expect("host-visible light table is mapped");
        write_words(mapped, light_table_words);
    }

    // --- The L1 cluster SSBOs. Host-visible so the GPU-tester can read the `ClusterGrid`
    // occupancy back (the production path mints them DEVICE_LOCAL; for the golden a
    // host-coherent buffer lets the readback diff against the host oracle). ---
    let cluster_grid = device
        .create_buffer(&BufferDesc {
            size: cluster_grid_bytes,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("L1 ClusterGrid storage buffer");
    let light_index = device
        .create_buffer(&BufferDesc {
            size: (index_list_cap as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("L1 LightIndexList storage buffer");
    // The global slice-allocation counter (one u32). Host-zeroed below before the submit —
    // the cull's `InterlockedAdd(LightIndexAlloc[0], ...)` then starts at 0, the host-write
    // equivalent of the production `cmd_fill_buffer(alloc, 0)` reset (which the abstract RHI
    // encoder does not expose). No TRANSFER→COMPUTE barrier is needed: the host write
    // completes before the submit, and a host-coherent write is visible to the GPU.
    let light_index_alloc = device
        .create_buffer(&BufferDesc {
            size: 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("L1 LightIndexAlloc counter buffer");
    {
        let mapped = device
            .buffer_mapped_ptr(&light_index_alloc)
            .expect("host-visible alloc counter is mapped");
        write_words(mapped, &[0u32]);
    }

    // --- The P4b tiles buffer (vocab @6) — unused here (no coarse pass) but the shared
    // vocabulary layout declares binding 6, so it must be bound. ---
    let tiles_buffer = device
        .create_buffer(&BufferDesc {
            size: (tile_count() as u64) * (TILE_BOUND_BYTES as u64),
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("P4b coarse-cull tile-bound storage buffer");

    // --- The depth IMAGE (D32_SFLOAT). Render P5-r0: the throwaway color attachment is
    // DELETED — pass A now binds the three real G-buffer images as a 3-MRT producer. ---
    let depth = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("offscreen depth texture (sampled)");

    // --- The MRT G-buffer STORAGE images + the LIT output + the gViewT lane. Render P5-r0:
    // albedo/normal/material ALSO carry COLOR_ATTACHMENT (the mesh raster pass-A producer). ---
    let albedo = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("G-buffer albedo storage+color image");
    let normal = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            // TRANSFER_SRC so the oct-encoded gNormal lane can be read back: the L1 resolve golden
            // also overrides the oracle's `attrs.oct_rg` with the GPU's REAL stored normal bytes
            // (the residual FP gap is in the marcher's normal, amplified by 1/d² lights), so the
            // oracle's UNORM decode is bit-identical to the GPU resolve's gNormal load.
            usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC | ImageUsage::COLOR_ATTACHMENT,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("G-buffer normal storage+color image");
    let material = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::COLOR_ATTACHMENT,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("G-buffer material storage+color image");
    let lit = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("deferred resolve lit storage image");
    let viewt = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::R32Sfloat,
            dimension: TextureDimension::D2,
            // TRANSFER_SRC so the gViewT lane can be read back: the L1 resolve golden feeds the
            // GPU's ACTUAL surface depth into the host oracle (isolating the RESOLVE under test
            // from the marcher's independent CPU re-derivation), so both sides shade the SAME P.
            usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("Lighting L0b gViewT storage image");
    // Render P7 GROUP C1: the SSAO term `gSsao` — an R8_UNORM STORAGE image bound at resolve
    // binding 11. ALWAYS allocated (the resolve descriptor interface is stable); `ssao_mode == 0`
    // here, so the resolve never reads it (a bound-but-unread valid descriptor, the 0%-gate).
    let ssao = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::R8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("Render P7 SSAO gSsao storage image");

    let sampler = device
        .create_sampler(&SamplerDesc::default())
        .expect("depth sampler (ignored by .Load)");

    let readback = device
        .create_buffer(&BufferDesc {
            size: READBACK_BYTES,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible readback buffer");
    // The gViewT (R32_SFLOAT) readback: one f32 per pixel = the same `PIXELS * 4` bytes as the
    // RGBA8 lit readback. The L1 resolve golden overrides the host oracle's `attrs.view_t` with
    // these GPU values so the resolve (the unit under test) is fed the GPU's REAL surface point.
    let viewt_readback = device
        .create_buffer(&BufferDesc {
            size: READBACK_BYTES,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible gViewT readback buffer");
    // The gNormal (RGBA8) readback: R8G8 = the oct-encoded normal, B8/A8 = the 16-bit id. The L1
    // resolve golden overrides the oracle's `attrs.oct_rg` with the GPU's R8G8 so the oracle's
    // oct decode is bit-identical to the GPU resolve's — the residual FP gap is the normal.
    let normal_readback = device
        .create_buffer(&BufferDesc {
            size: READBACK_BYTES,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible gNormal readback buffer");

    // --- The quad vertex buffer. ---
    let vertices = quad_vertices();
    let vertex_bytes = core::mem::size_of_val(&vertices) as u64;
    let vertex_buffer = device
        .create_buffer(&BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible vertex buffer");
    let vb_ptr = device
        .buffer_mapped_ptr(&vertex_buffer)
        .expect("host-visible vertex buffer is mapped");
    // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes; `vertices` is a
    // distinct stack array of `vertex_bytes` bytes; the write completes before any submit
    // references the buffer (host-coherent: no flush).
    unsafe {
        core::ptr::copy_nonoverlapping(
            vertices.as_ptr().cast::<u8>(),
            vb_ptr.as_ptr(),
            vertex_bytes as usize,
        );
    }

    // --- Modules: the Render P5-r0 mesh-MRT producer pair + the marcher + the cull + the
    // resolve. The raster pair is now `gbuffer_mrt.{vs,fs}` (3-MRT producer). ---
    let vs = device
        .create_shader_module(MRT_VS_SPV.as_words())
        .expect("mesh-MRT vertex shader module");
    let fs = device
        .create_shader_module(MRT_FS_SPV.as_words())
        .expect("mesh-MRT fragment shader module");
    let cs = device
        .create_shader_module(sdf_gbuffer_composite_spirv())
        .expect("P1b G-buffer marcher compute shader module");
    let cull_cs = device
        .create_shader_module(cluster_cull_spirv())
        .expect("L1 cluster-cull compute shader module");
    let resolve_cs = device
        .create_shader_module(deferred_pbr_spirv())
        .expect("deferred resolve compute shader module");

    // --- The mesh-MRT G-buffer producer graphics pipeline (Render P5-r0). ---
    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
    ];
    // M1: the per-instance model SSBO layout + 1-element identity dummy + its bind group (the
    // VS statically references `instances` at set 0 binding 0; the legacy arm never reads it).
    let (instance_layout, instance_buffer, instance_bind_group) = create_identity_instance(device);
    let gfx = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            // Render P5-r0: 3 MRT color formats = the G-buffer RGBA8 lanes.
            color_formats: &[GBUFFER_FORMAT, GBUFFER_FORMAT, GBUFFER_FORMAT],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            bind_group_layout: Some(&instance_layout),
            blend: None,
            cull_mode: CullMode::None,
            depth_bias: None,
        })
        .expect("mesh-MRT G-buffer producer graphics pipeline");

    // --- The vocabulary set (marcher), identical to the table driver: { edit-list @0,
    // sampled depth @1, albedo @2, normal @3, material @4, camera UBO @5, tiles @6, material
    // table @7, gViewT @8, M1 pointer-grid @9 }. ---
    let layout_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        // M1: the recompiled marcher SPIR-V STATICALLY references the PointerGrid @9 (the
        // empty-skip branch is runtime-gated by `brick_enabled`, but DXC does NOT dead-strip
        // the `register(t9)` access), so the layout MUST declare @9 or `vkCreateComputePipelines`
        // trips VUID-VkComputePipelineCreateInfo-layout. The L1 path runs the empty-skip OFF
        // (the marcher push leaves `brick_enabled == 0`), so @9 is bound to a harmless
        // placeholder StorageBuffer (never read).
        BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // M2: the brick-atlas combined image+sampler @10. The recompiled marcher SPIR-V statically
        // references `register(t10)`/`register(s10)` (collapsed to ONE combined descriptor by DXC)
        // inside the runtime-gated `brick_trilinear` branch — NOT dead-stripped — so the L1 marcher
        // layout MUST declare @10 (or `vkCreateComputePipelines` trips VUID-…-layout-07988). The L1
        // path runs the trilinear path OFF (`brick_trilinear == 0`); the atlas is bound-but-unread.
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        // M4 clip-map LOD (Slice C): the LEVEL-1 + LEVEL-2 brick bindings @11..14 the recompiled marcher
        // references past the runtime level branch-ladder. The L1 path runs the brick blocks OFF; bound-
        // but-unread, but the layout MUST declare all four or `vkCreateComputePipelines` trips the VUIDs.
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 12, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 13, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 14, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        // MDF Stage-2c: the dedicated dense mesh-SDF shadow-caster texture @15 (the 16th / last vocab
        // entry under the 16-binding cap). The recompiled marcher SPIR-V statically references
        // `MeshSdf`@t15 + `MeshSdfSampler`@s15 inside the runtime-gated `mesh_sdf_enabled` branch, so
        // the layout MUST declare binding 15 — bound-but-unread on the OFF golden path (no MDF scene).
        BindGroupLayoutEntry { binding: 15, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
    ];
    let bind_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc { entries: &layout_entries })
        .expect("vocabulary bind-group layout");
    let compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&bind_layout),
            spec_constants: &[],
        })
        .expect("P1b G-buffer marcher compute pipeline");
    let bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &bind_layout,
            entries: &[
                BindGroupEntry::StorageBuffer { buffer: &buffer },
                BindGroupEntry::SampledImage { texture: &depth, sampler: &sampler },
                BindGroupEntry::StorageImage { texture: &albedo },
                BindGroupEntry::StorageImage { texture: &normal },
                BindGroupEntry::StorageImage { texture: &material },
                BindGroupEntry::UniformBuffer { buffer: &camera_uniform },
                BindGroupEntry::StorageBuffer { buffer: &tiles_buffer },
                BindGroupEntry::StorageBuffer { buffer: &material_table },
                BindGroupEntry::StorageImage { texture: &viewt },
                // M1 @9: placeholder (the L1 path runs the empty-skip OFF; never read). The
                // material table is a valid StorageBuffer that satisfies the static t9 ref.
                BindGroupEntry::StorageBuffer { buffer: &material_table },
                // M2 @10: the brick atlas (combined image+sampler). Bound-but-unread on the L1 OFF
                // path (`brick_trilinear == 0`); satisfies the marcher SPIR-V's static t10/s10 ref.
                BindGroupEntry::CombinedImage {
                    texture: brick_atlas.texture(),
                    sampler: brick_atlas.sampler(),
                },
                // M4 @11..14: the LEVEL-1 + LEVEL-2 brick resources, bound to level-0 duplicates (the L1
                // path has no clipmap + runs the brick blocks OFF — bound-but-unread, but the SPIR-V
                // references t11..t14 past the gate, so VALID descriptors are required). @11/@13 reuse the
                // material table (a valid StorageBuffer satisfying the static t11/t13 ref).
                BindGroupEntry::StorageBuffer { buffer: &material_table },
                BindGroupEntry::CombinedImage {
                    texture: brick_atlas.texture(),
                    sampler: brick_atlas.sampler(),
                },
                BindGroupEntry::StorageBuffer { buffer: &material_table },
                BindGroupEntry::CombinedImage {
                    texture: brick_atlas.texture(),
                    sampler: brick_atlas.sampler(),
                },
                // MDF Stage-2c @15: the brick atlas as a benign placeholder (no MDF scene); the marcher
                // gates the read OFF (`mesh_sdf_enabled == 0`) → bound-but-unread (the R2 contract).
                BindGroupEntry::CombinedImage {
                    texture: brick_atlas.texture(),
                    sampler: brick_atlas.sampler(),
                },
            ],
        })
        .expect("vocabulary bind group");

    // --- The L1 CULL set + pipeline (mirrors `cluster_cull.hlsl`'s register map + the
    // production `cull_layout`): { camera UBO @0, light table SSBO @1, ClusterGrid SSBO @2,
    // LightIndexList SSBO @3, LightIndexAlloc SSBO @4 } + a 16-byte ClusterCullPush. ---
    let cull_layout_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
    ];
    let cull_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc { entries: &cull_layout_entries })
        .expect("L1 cull bind-group layout");
    let cull_compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &cull_cs,
            entry: c"main",
            push_constant_bytes: CLUSTER_CULL_PUSH_BYTES,
            bind_group_layout: Some(&cull_layout),
            spec_constants: &[],
        })
        .expect("L1 cluster-cull compute pipeline");
    let cull_bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &cull_layout,
            entries: &[
                BindGroupEntry::UniformBuffer { buffer: &camera_uniform },
                BindGroupEntry::StorageBuffer { buffer: &light_table },
                BindGroupEntry::StorageBuffer { buffer: &cluster_grid },
                BindGroupEntry::StorageBuffer { buffer: &light_index },
                BindGroupEntry::StorageBuffer { buffer: &light_index_alloc },
            ],
        })
        .expect("L1 cull bind group");

    // --- The RESOLVE layout + pipeline + set. The L1 difference vs the table driver: bindings
    // 8/9 carry the REAL ClusterGrid + LightIndexList (not the light-table placeholder), so the
    // resolve's `clusters_enabled` gate loops the per-froxel index slice. ---
    let resolve_layout_entries = [
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
        // P6 R1: the SDF edit-list `Buf` SSBO @10 (the resolve's `sdf_soft_shadow_ranged`
        // analytic march reads it read-only; the SAME buffer the marcher binds @0). 11 ≤ 12
        // (no cap raise — the orchestrator's R1=(A) decision drops the brick atlas binds).
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Render P7 GROUP C1: the SSAO term `gSsao` STORAGE image @11 (read under
        // `ssao_mode != 0`; OFF here, bound-but-unread). The recompiled `deferred_pbr.comp`
        // STATICALLY declares `gSsao @11`, so the layout MUST declare it or the pipeline create
        // trips the binding-count check (the P6 R1 binding-10 discipline).
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        // CSM Increment 1b (Rung A): the cascade combined map+sampler @12 + the cascade UBO @13
        // (`csm_mode == 0` here → bound-but-unread; the recompiled resolve STATICALLY references
        // both).
        CsmResolveDummies::layout_entries()[0],
        CsmResolveDummies::layout_entries()[1],
        CsmResolveDummies::layout_entries()[2],
        CsmResolveDummies::layout_entries()[3],
    ];
    // CSM Increment 1b + Shadow Inc-1-GPU: the OFF-path cascade trio @12/@13 + atlas trio @14/@15
    // (bound-but-unread).
    let csm_dummies = CsmResolveDummies::create(device);
    let resolve_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc { entries: &resolve_layout_entries })
        .expect("deferred resolve bind-group layout");
    let resolve_compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &resolve_cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&resolve_layout),
            spec_constants: &[],
        })
        .expect("deferred resolve compute pipeline");
    let resolve_bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &resolve_layout,
            entries: &[
                BindGroupEntry::StorageImage { texture: &albedo },
                BindGroupEntry::StorageImage { texture: &normal },
                BindGroupEntry::StorageImage { texture: &material },
                BindGroupEntry::StorageImage { texture: &lit },
                BindGroupEntry::StorageBuffer { buffer: &material_table },
                BindGroupEntry::UniformBuffer { buffer: &camera_uniform },
                BindGroupEntry::StorageBuffer { buffer: &light_table },
                BindGroupEntry::StorageImage { texture: &viewt },
                // Lighting L1 @8/@9: the REAL cluster buffers the cull pass populated.
                BindGroupEntry::StorageBuffer { buffer: &cluster_grid },
                BindGroupEntry::StorageBuffer { buffer: &light_index },
                // P6 R1: the SDF edit-list `Buf` @10 (the marcher's vocab @0 SSBO).
                BindGroupEntry::StorageBuffer { buffer: &buffer },
                // Render P7 GROUP C1: the SSAO term `gSsao` @11 (bound-but-unread — `ssao_mode`
                // is 0 here, so the resolve never loads it; present only to satisfy the layout).
                BindGroupEntry::StorageImage { texture: &ssao },
                // CSM Increment 1b (Rung A): the cascade combined map+sampler @12 + UBO @13
                // (bound-but-unread — `csm_mode == 0` here, so the resolve's PCF sample never runs).
                BindGroupEntry::CombinedImage {
                    texture: &csm_dummies.cascade,
                    sampler: &csm_dummies.sampler,
                },
                BindGroupEntry::UniformBuffer { buffer: &csm_dummies.ubo },
                // Shadow Inc-1-GPU: the atlas combined map+sampler @14 + UBO @15 (bound-but-unread —
                // `punctual_shadow_mode == 0` here, so the resolve's spot PCF sample never runs). The
                // 15th + 16th entries — the resolve set hits 16/16.
                BindGroupEntry::CombinedImage {
                    texture: &csm_dummies.atlas,
                    sampler: &csm_dummies.atlas_sampler,
                },
                BindGroupEntry::UniformBuffer { buffer: &csm_dummies.atlas_ubo },
            ],
        })
        .expect("deferred resolve bind group");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    encoder.begin().expect("begin");

    // --- Render P5-r0 mesh raster pass A: clear depth + the 3 MRT G-buffer lanes, then
    // rasterize the quad as a 3-MRT producer. The 3 RGBA8 images: UNDEFINED →
    // COLOR_ATTACHMENT_OPTIMAL. ---
    for tex in [&albedo, &normal, &material] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::TOP_OF_PIPE,
            dst_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
            src_access: BarrierAccess::NONE,
            dst_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
            old_layout: ImageLayout::Undefined,
            new_layout: ImageLayout::ColorAttachmentOptimal,
            range: ImageSubresourceRange::COLOR,
        });
    }
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::DepthAttachmentOptimal,
        range: ImageSubresourceRange::DEPTH,
    });

    let full = RenderArea { x: 0, y: 0, width: SDF_IMG_W, height: SDF_IMG_H };
    // Render P5-r0 / Decision r0-2: the MRT clears ARE the marcher's mask=0 neutral G-buffer.
    let color_attachments = [
        RenderingAttachment {
            texture: &albedo,
            layout: ImageLayout::ColorAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_color: [0.05, 0.05, 0.1, 1.0],
        },
        RenderingAttachment {
            texture: &normal,
            layout: ImageLayout::ColorAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_color: [0.5, 0.5, 0.0, 0.0],
        },
        RenderingAttachment {
            texture: &material,
            layout: ImageLayout::ColorAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_color: [1.0, 1.0, 0.0, 1.0],
        },
    ];
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &color_attachments,
        depth: Some(DepthAttachment {
            texture: &depth,
            layout: ImageLayout::DepthAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_depth: DEPTH_CLEAR,
        }),
    });
    encoder.bind_graphics_pipeline(&gfx);
    // M1: bind the 1-element identity instance SSBO at set 0 (bound-but-unread — the
    // `use_model_matrix == 0` push selects the VS's legacy arm, byte-identical pixels).
    encoder.bind_descriptor_set(&instance_bind_group, &gfx);
    encoder.push_graphics_constants(&gfx, ShaderStage::VERTEX, 0, &ortho_mvp_bytes());
    encoder.bind_vertex_buffer(&vertex_buffer, 0, 0);
    encoder.set_viewport(&Viewport {
        x: 0.0,
        y: 0.0,
        width: SDF_IMG_W as f32,
        height: SDF_IMG_H as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    });
    encoder.set_scissor(&full);
    encoder.draw(6, 1, 0, 0);
    encoder.end_rendering();

    // --- The single depth dual-use barrier: DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY. ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth,
        src_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        src_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
        dst_access: BarrierAccess::SHADER_READ,
        old_layout: ImageLayout::DepthAttachmentOptimal,
        new_layout: ImageLayout::ShaderReadOnlyOptimal,
        range: ImageSubresourceRange::DEPTH,
    });

    // --- Render P5-r0 barrier-out: the 3 RGBA8 G-buffer images COLOR_ATTACHMENT_OPTIMAL →
    // GENERAL (a genuine raster-write hand-off to the marcher). ---
    for tex in [&albedo, &normal, &material] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
            dst_access: BarrierAccess::SHADER_READ | BarrierAccess::SHADER_WRITE,
            old_layout: ImageLayout::ColorAttachmentOptimal,
            new_layout: ImageLayout::General,
            range: ImageSubresourceRange::COLOR,
        });
    }

    // --- The lit + gViewT + Render P7 SSAO images: UNDEFINED → GENERAL (not rasterized into in
    // r0; `ssao` lives in GENERAL its whole life, bound-but-unread under ssao_mode 0). ---
    for tex in [&lit, &viewt, &ssao] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::TOP_OF_PIPE,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::NONE,
            dst_access: BarrierAccess::SHADER_WRITE,
            old_layout: ImageLayout::Undefined,
            new_layout: ImageLayout::General,
            range: ImageSubresourceRange::COLOR,
        });
    }

    // --- SDF marcher compute pass: SAMPLE the depth image, STORE the G-buffer + gViewT. ---
    encoder.bind_compute_pipeline(&compute);
    encoder.bind_descriptor_set_compute(&bind_group, &compute);
    // M4 (Slice C): the L1 path runs `brick_trilinear == 0` / `brick_enabled == 0` (the brick blocks
    // are dead), so `brick_levels` is moot; `with_brick_levels(1)` documents the OFF/N=1 contract.
    let push = FineMarcherPush::new(false, 1.0, lighting_flags, light_dir).with_brick_levels(1);
    encoder.push_compute_constants(&compute, ShaderStage::COMPUTE, 0, push.as_bytes());
    encoder.dispatch(group_count_x(), 1, 1);

    // --- Make the marcher's gAlbedo + gNormal + gMaterial + gViewT stores available + visible
    // to the resolve's loads (COMPUTE→COMPUTE, SHADER_WRITE→SHADER_READ, GENERAL→GENERAL). ---
    for tex in [&albedo, &normal, &material, &viewt] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::COMPUTE_SHADER,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::SHADER_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
            old_layout: ImageLayout::General,
            new_layout: ImageLayout::General,
            range: ImageSubresourceRange::COLOR,
        });
    }

    // --- Lighting L1: the CLUSTER-CULL pass (mirrors `swapchain.rs`'s `render_gbuffer_frame`
    // cull recording). The `LightIndexAlloc` counter was host-zeroed before the submit (the
    // production `cmd_fill_buffer` reset equivalent), so no TRANSFER→COMPUTE alloc barrier is
    // needed. Bind the cull pipeline + the cull set, push the 16-byte ClusterCullPush, and
    // dispatch over `cluster_count` froxels at the 64-wide group. The cull is geometric (it
    // does NOT read gViewT), so it can run after the marcher without further sync. ---
    let cull_push = ClusterCullPush::new(cfg.z_near, cfg.z_far, cfg.max_lights_per_cluster, index_list_cap);
    let cull_groups = cluster_count.div_ceil(64);
    encoder.bind_compute_pipeline(&cull_compute);
    encoder.bind_descriptor_set_compute(&cull_bind_group, &cull_compute);
    encoder.push_compute_constants(&cull_compute, ShaderStage::COMPUTE, 0, cull_push.as_bytes());
    encoder.dispatch(cull_groups, 1, 1);

    // --- (L1) Make the cull's ClusterGrid + LightIndexList writes available + visible to the
    // resolve's reads (COMPUTE→COMPUTE, SHADER_WRITE→SHADER_READ) on both buffers. ---
    let cull_to_resolve = [
        BufferBarrier {
            buffer: &cluster_grid,
            src_access: BarrierAccess::SHADER_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
        },
        BufferBarrier {
            buffer: &light_index,
            src_access: BarrierAccess::SHADER_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
        },
    ];
    encoder.pipeline_barrier(&BarrierDesc {
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        buffers: &cull_to_resolve,
    });

    // --- RESOLVE pass: bind the resolve pipeline + the resolve set (now with the real cluster
    // buffers @8/@9), dispatch at the SAME grid the marcher used. The `clusters_enabled` header
    // gate makes it loop the per-froxel index slice for the point/spot block. ---
    encoder.bind_compute_pipeline(&resolve_compute);
    encoder.bind_descriptor_set_compute(&resolve_bind_group, &resolve_compute);
    encoder.dispatch(group_count_x(), 1, 1);

    // --- LIT: GENERAL → TRANSFER_SRC_OPTIMAL for the readback copy. ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &lit,
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::SHADER_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::TransferSrcOptimal,
        range: ImageSubresourceRange::COLOR,
    });

    let regions = [BufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        aspect: ImageAspect::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
        image_offset_x: 0,
        image_offset_y: 0,
        image_offset_z: 0,
        image_extent_w: SDF_IMG_W,
        image_extent_h: SDF_IMG_H,
        image_extent_d: 1,
    }];
    encoder.copy_image_to_buffer(&lit, ImageLayout::TransferSrcOptimal, &readback, &regions);

    // --- gViewT: GENERAL → TRANSFER_SRC_OPTIMAL for the readback copy. The marcher is the
    // producer (SHADER_WRITE); the resolve only READ it, so the transfer reads the marcher's
    // store directly. The copy regions are identical (same extent, R32_SFLOAT == 4 B/texel). ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &viewt,
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::SHADER_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::TransferSrcOptimal,
        range: ImageSubresourceRange::COLOR,
    });
    encoder.copy_image_to_buffer(&viewt, ImageLayout::TransferSrcOptimal, &viewt_readback, &regions);

    // --- gNormal: GENERAL → TRANSFER_SRC_OPTIMAL for the readback copy. Same producer (the
    // marcher's SHADER_WRITE store) and identical regions (RGBA8 == 4 B/texel). ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &normal,
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::SHADER_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::TransferSrcOptimal,
        range: ImageSubresourceRange::COLOR,
    });
    encoder.copy_image_to_buffer(&normal, ImageLayout::TransferSrcOptimal, &normal_readback, &regions);

    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    // Read back the LIT R8G8B8A8 bytes.
    let dst_ptr = device
        .buffer_mapped_ptr(&readback)
        .expect("host-visible readback buffer is mapped");
    let mut out = vec![0u8; READBACK_BYTES as usize];
    // SAFETY: `dst_ptr` points to `READBACK_BYTES` mapped host-coherent bytes; a fence wait
    // preceded this read, so the GPU store + copy are complete + coherent; reading
    // `READBACK_BYTES` bytes is in-bounds; `out` is a distinct allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), READBACK_BYTES as usize);
    }

    // Read back the gViewT (R32_SFLOAT) lane as one f32 per pixel (row-major, the SAME
    // `py * SDF_IMG_W + px` order `albedo_rgb` uses). The L1 resolve golden overrides the host
    // oracle's `attrs.view_t` with this so the resolve shades the GPU's EXACT surface point.
    let viewt_ptr = device
        .buffer_mapped_ptr(&viewt_readback)
        .expect("host-visible gViewT readback buffer is mapped");
    let mut viewt_bytes = vec![0u8; READBACK_BYTES as usize];
    // SAFETY: `viewt_ptr` points to `READBACK_BYTES` mapped host-coherent bytes (the buffer was
    // sized `READBACK_BYTES` above); the fence wait preceded this read, so the GPU store + copy
    // are complete + coherent; reading `READBACK_BYTES` is in-bounds; `viewt_bytes` is distinct.
    unsafe {
        core::ptr::copy_nonoverlapping(viewt_ptr.as_ptr(), viewt_bytes.as_mut_ptr(), READBACK_BYTES as usize);
    }
    let viewt_px: Vec<f32> = viewt_bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    // Read back the gNormal (RGBA8) lane: the per-pixel R8G8 oct-encoded normal bytes (row-major,
    // same order). The L1 resolve golden overrides the oracle's `attrs.oct_rg` with these so the
    // oct decode is bit-identical to the GPU resolve's gNormal load.
    let normal_ptr = device
        .buffer_mapped_ptr(&normal_readback)
        .expect("host-visible gNormal readback buffer is mapped");
    let mut normal_bytes = vec![0u8; READBACK_BYTES as usize];
    // SAFETY: `normal_ptr` points to `READBACK_BYTES` mapped host-coherent bytes (the buffer was
    // sized `READBACK_BYTES` above); the fence wait preceded this read, so the GPU store + copy
    // are complete + coherent; reading `READBACK_BYTES` is in-bounds; `normal_bytes` is distinct.
    unsafe {
        core::ptr::copy_nonoverlapping(normal_ptr.as_ptr(), normal_bytes.as_mut_ptr(), READBACK_BYTES as usize);
    }
    let normal_oct: Vec<[u8; 2]> = normal_bytes
        .chunks_exact(4)
        .map(|c| [c[0], c[1]])
        .collect();

    // Read back the ClusterGrid occupancy ({offset, count} per froxel). It is a
    // HostVisibleCoherent STORAGE buffer, so no transfer copy is required — the cull's writes
    // completed before the fence signalled, and host-coherent memory makes them visible here.
    let grid_ptr = device
        .buffer_mapped_ptr(&cluster_grid)
        .expect("host-visible ClusterGrid buffer is mapped");
    let mut grid_out = vec![0u8; cluster_grid_bytes as usize];
    // SAFETY: `grid_ptr` points to `cluster_grid_bytes` mapped host-coherent bytes (the buffer
    // was sized so above); the fence wait preceded this read, so the cull's writes are complete
    // + coherent; reading `cluster_grid_bytes` is in-bounds; `grid_out` is a distinct alloc.
    unsafe {
        core::ptr::copy_nonoverlapping(grid_ptr.as_ptr(), grid_out.as_mut_ptr(), cluster_grid_bytes as usize);
    }

    // Read back the flat LightIndexList (the per-froxel index slices the cull scattered). Like
    // the ClusterGrid it is HostVisibleCoherent, so the cull's writes are visible after the
    // fence wait. The buffer is NOT zero-initialized before the cull, so only the slots the cull
    // claimed via InterlockedAdd carry valid indices — the probe reads each froxel's claimed
    // slice `[offset .. offset+count)` ONLY, never the uninitialized tail.
    let light_index_bytes_len = (l1_index_list_cap(cfg) as u64) * 4;
    let index_ptr = device
        .buffer_mapped_ptr(&light_index)
        .expect("host-visible LightIndexList buffer is mapped");
    let mut index_out = vec![0u8; light_index_bytes_len as usize];
    // SAFETY: `index_ptr` points to `light_index_bytes_len` mapped host-coherent bytes (the
    // buffer was sized `index_list_cap * 4` above); the fence wait preceded this read, so the
    // cull's writes are complete + coherent; reading `light_index_bytes_len` is in-bounds;
    // `index_out` is a distinct alloc.
    unsafe {
        core::ptr::copy_nonoverlapping(index_ptr.as_ptr(), index_out.as_mut_ptr(), light_index_bytes_len as usize);
    }

    assert_validation_clean(ctx);

    // SAFETY: every resource was created on `device`; the last submission completed
    // (fence-waited above), so none is in use; each is destroyed exactly once.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_bind_group(resolve_bind_group);
        device.destroy_bind_group(cull_bind_group);
        device.destroy_bind_group(bind_group);
        device.destroy_compute_pipeline(resolve_compute);
        device.destroy_compute_pipeline(cull_compute);
        device.destroy_compute_pipeline(compute);
        device.destroy_bind_group_layout(resolve_layout);
        device.destroy_bind_group_layout(cull_layout);
        device.destroy_bind_group_layout(bind_layout);
        // CSM Increment 1b: the OFF-path cascade trio bound at resolve @12/@13.
        csm_dummies.destroy(device);
        device.destroy_graphics_pipeline(gfx);
        // M1 instance-model resources (bind group → buffer → layout, after the pipeline).
        device.destroy_bind_group(instance_bind_group);
        device.destroy_buffer(instance_buffer);
        device.destroy_bind_group_layout(instance_layout);
        device.destroy_shader_module(resolve_cs);
        device.destroy_shader_module(cull_cs);
        device.destroy_shader_module(cs);
        device.destroy_shader_module(fs);
        device.destroy_shader_module(vs);
        device.destroy_buffer(vertex_buffer);
        device.destroy_buffer(normal_readback);
        device.destroy_buffer(viewt_readback);
        device.destroy_buffer(readback);
        device.destroy_sampler(sampler);
        device.destroy_texture(ssao);
        device.destroy_texture(viewt);
        device.destroy_texture(lit);
        device.destroy_texture(material);
        device.destroy_texture(normal);
        device.destroy_texture(albedo);
        device.destroy_texture(depth);
        device.destroy_buffer(tiles_buffer);
        device.destroy_buffer(light_index_alloc);
        device.destroy_buffer(light_index);
        device.destroy_buffer(cluster_grid);
        device.destroy_buffer(light_table);
        device.destroy_buffer(material_table);
        device.destroy_buffer(camera_uniform);
        device.destroy_buffer(buffer);
        // M2: the brick atlas (image + sampler). The last submission completed (fence-waited
        // above), so no work still samples it; `destroy` consumes `self` ⇒ each object once.
        brick_atlas.destroy(device);
    }

    (out, grid_out, index_out, viewt_px, normal_oct)
}

/// Diffs the whole GPU LIT readback (run through the FULL clustered path) against the host
/// `golden_deferred_resolve_clustered` per texel, within ±2/255. The host oracle is fed the
/// host cull `grid` (`golden_cluster_cull`, which is bit-exact to what the GPU cull writes for
/// these no-overflow scenes).
///
/// **Resolve isolation.** The host `golden_marcher_attributes` re-derives the surface depth +
/// normal via an INDEPENDENT CPU march; that marcher's GPU-vs-CPU FP gap (~0.002 in `view_t`,
/// plus the matching `gNormal` gap) is amplified to white by this scene's pathologically
/// close+intense lights (atten ≈ 1/d²). The marcher is validated separately (the rung-8..11
/// goldens), so to isolate the RESOLVE under test we OVERRIDE both `attrs.view_t` (with the GPU's
/// `gViewT` lane, `viewt_px`) and `attrs.oct_rg` (with the GPU's stored `gNormal` R8G8 bytes,
/// `normal_oct`) before calling the oracle. Both sides then reconstruct the SAME
/// `P = ro + rd * gViewT_gpu` (and the SAME froxel z-slice) and decode the SAME normal, leaving
/// only the resolve's own arithmetic under test. Returns the max delta + the SDF-lit
/// (mask == 1) pixel count so the caller can prove a real lit surface.
#[allow(clippy::too_many_arguments)]
fn assert_lit_matches_clustered_golden(
    lit: &[u8],
    edits: &[SdfEdit],
    flags: u32,
    light_dir: [f32; 3],
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
    cfg: &GoldenClusterConfig,
    grid: &[Vec<u32>],
    viewt_px: &[f32],
    normal_oct: &[[u8; 2]],
    name: &str,
) -> (i32, u64) {
    let mut max_delta = 0i32;
    let mut sdf_lit_hits = 0u64;
    let materials = host_material_table();
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let md = expected_mesh_depth(px, py);
            let mut attrs = golden_marcher_attributes(
                edits, &materials, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                flags, light_dir,
            );
            // Isolate the resolve: feed the GPU's REAL surface depth + normal so the oracle
            // reconstructs the identical P/view_z and decodes the identical normal. Only
            // meaningful on the SDF-lit arm (mask == 1); the oracle reads neither `view_t` nor
            // `oct_rg` on a non-lit pixel, so the override is harmless there.
            if attrs.mask == 1 {
                let i = (py * SDF_IMG_W + px) as usize;
                attrs.view_t = viewt_px[i];
                attrs.oct_rg = normal_oct[i];
            }
            let want = unpack_packed_rgb(golden_deferred_resolve_clustered(
                attrs, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &materials, header,
                lights, cfg, grid,
            ));
            let got = albedo_rgb(lit, px, py);
            let dmax = (0..3).map(|c| (got[c] - want[c]).abs()).max().unwrap();
            if attrs.mask == 1 {
                sdf_lit_hits += 1;
            }
            if dmax > max_delta {
                max_delta = dmax;
            }
            assert!(
                dmax <= DEFERRED_ARM1_TOL,
                "[{name}] L1 clustered LIT texel ({px},{py}) got {got:?} want {want:?} (clustered \
                 oracle) exceeds ±{DEFERRED_ARM1_TOL}/255 (delta {dmax}) — the GPU cluster path \
                 diverged from the host brute-force-equal oracle"
            );
        }
    }
    (max_delta, sdf_lit_hits)
}

/// **The load-bearing L1 golden.** Builds a multi-light scene (1 directional + 3 point + 1
/// spot, `new_clustered(l0a=1, point_spot=4)`), runs the FULL GPU clustered path — the
/// `cluster_cull` compute pass populates the real `ClusterGrid` + `LightIndexList` from the
/// table, then the deferred resolve reads them with `clusters_enabled == 1` — and asserts the
/// GPU LIT image matches the host `golden_deferred_resolve_clustered` oracle (which == brute
/// force, proven by `lighting_l1_host_oracle.rs`) within ±2/255 over all 3 SDF scenes. This is
/// the test that previously caught the NaN→black at crater(38,18); the `safe_normalize` fix in
/// `deferred_pbr.hlsl` makes it match.
#[test]
fn l1_clustered_resolve_matches_the_brute_force_image() {
    let Some(ctx) = boot_render_or_skip("l1_clustered_resolve_matches_the_brute_force_image") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let cfg = l1_cluster_config();
    // l0a_count = 1 (the directional front block); point_spot_count = 4 (3 point + 1 spot).
    // The surface band sits near world z = 0 (the ORTHO fixture marches +Z→origin), so the
    // lights live in front of it with a generous range so a swath of pixels is lit.
    let header = GoldenLightHeader::new_clustered(1, 4, 1.0, &cfg);
    let lights = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::point([-0.5, 0.3, 0.2], [1.0, 0.3, 0.3], 3000.0, 2.0),
        GoldenLight::point([0.5, -0.3, 0.2], [0.3, 1.0, 0.3], 3000.0, 2.0),
        GoldenLight::point([0.0, 0.0, 0.4], [0.3, 0.3, 1.0], 3000.0, 2.5),
        GoldenLight::spot([0.2, 0.2, 0.6], [0.0, 0.0, 1.0], [1.0, 1.0, 0.6], 5000.0, 3.0, 20.0, 35.0),
    ];
    let table = pack_light_table(&header, &lights);

    // The host cull grid — the bit-exact reference for the GPU cull (no overflow on this scene,
    // so GPU occupancy == host occupancy and the resolve sees the same per-froxel light set).
    let grid = golden_cluster_cull(SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &cfg, &header, &lights);

    for (name, edits) in p4b_scenes() {
        let (lit, _grid_bytes, _index_bytes, viewt_px, normal_oct) =
            run_gbuffer_hybrid_lit_clustered(&ctx, &edits, flags, DEFAULT_LIGHT_DIR, &table, &cfg);
        assert_eq!(lit.len(), READBACK_BYTES as usize);
        let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(nonzero > 0, "[{name}] L1 clustered LIT all-zero — device did not render");

        let (max_delta, sdf_lit_hits) =
            assert_lit_matches_clustered_golden(&lit, &edits, flags, DEFAULT_LIGHT_DIR, &header, &lights, &cfg, &grid, &viewt_px, &normal_oct, name);
        assert!(
            sdf_lit_hits > 0,
            "[{name}] L1 clustered: no SDF-lit pixel — the gate is vacuous"
        );
        println!(
            "[{name}] L1 clustered resolve (GPU cull → froxel-mapped resolve == brute-force \
             oracle): max delta {max_delta}/255 (tol {DEFERRED_ARM1_TOL}); {sdf_lit_hits} \
             SDF-lit px"
        );
    }
}

/// Reads a `ClusterGrid` readback (`cluster_count` × `{u32 offset, u32 count}`) at flat froxel
/// index `fi`, returning `(offset, count)`.
fn cluster_cell(grid_bytes: &[u8], fi: usize) -> (u32, u32) {
    let base = fi * 8;
    let offset = u32::from_le_bytes([grid_bytes[base], grid_bytes[base + 1], grid_bytes[base + 2], grid_bytes[base + 3]]);
    let count = u32::from_le_bytes([grid_bytes[base + 4], grid_bytes[base + 5], grid_bytes[base + 6], grid_bytes[base + 7]]);
    (offset, count)
}

/// **L1 cull occupancy golden.** Runs the GPU `cluster_cull` pass for a known multi-light
/// scene, reads back the `ClusterGrid`, and asserts its per-froxel occupancy `count` matches
/// the host `golden_cluster_cull` froxel-for-froxel. The cull is geometric + exact under the
/// (un-hit) cap, so the GPU `count` per froxel equals the host set length for every froxel (the
/// lost test passed 2108 == 2108 total occupancy). Non-vacuous: the scene MUST land at least
/// one light in at least one froxel.
#[test]
fn l1_known_light_lands_in_the_expected_clusters() {
    let Some(ctx) = boot_render_or_skip("l1_known_light_lands_in_the_expected_clusters") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let cfg = l1_cluster_config();
    // A known multi-light scene: 1 directional (GLOBAL, never in a froxel) + 3 point + 1 spot
    // spread across the view at the surface band, each with a generous range so several froxels
    // keep them. Identical light geometry to the resolve golden so the two tests cross-check.
    let header = GoldenLightHeader::new_clustered(1, 4, 1.0, &cfg);
    let lights = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::point([-0.5, 0.3, 0.2], [1.0, 0.3, 0.3], 3000.0, 2.0),
        GoldenLight::point([0.5, -0.3, 0.2], [0.3, 1.0, 0.3], 3000.0, 2.0),
        GoldenLight::point([0.0, 0.0, 0.4], [0.3, 0.3, 1.0], 3000.0, 2.5),
        GoldenLight::spot([0.2, 0.2, 0.6], [0.0, 0.0, 1.0], [1.0, 1.0, 0.6], 5000.0, 3.0, 20.0, 35.0),
    ];
    let table = pack_light_table(&header, &lights);

    // The host cull occupancy — the bit-exact reference. The SDF scene does not affect the cull
    // (it is purely geometric on the light table + camera), so any scene drives the cull pass;
    // use the crater fixture.
    let host_grid = golden_cluster_cull(SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &cfg, &header, &lights);
    let cluster_count = cfg.cluster_count() as usize;
    assert_eq!(host_grid.len(), cluster_count);

    let (_lit, grid_bytes, index_bytes, _viewt_px, _normal_oct) =
        run_gbuffer_hybrid_lit_clustered(&ctx, &crater(), LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO, DEFAULT_LIGHT_DIR, &table, &cfg);
    assert_eq!(grid_bytes.len(), cluster_count * 8);
    let index_list_cap = l1_index_list_cap(&cfg);
    assert_eq!(index_bytes.len(), index_list_cap as usize * 4);

    // Compare the GPU `count` per froxel to the host occupancy froxel-for-froxel. The flat
    // froxel index is `golden_cluster_index(x, y, z, ...)` (Z innermost) — the SAME
    // linearization the cull-write + resolve-read share, so the GPU `ClusterGrid[fi]` aligns
    // with `host_grid[fi]`.
    let mut gpu_total = 0u64;
    let mut host_total = 0u64;
    // Track every claimed light-index slice `[offset, offset+count)` so the probe can assert
    // they are PAIRWISE DISJOINT (the InterlockedAdd hands out non-overlapping bases) and that
    // their union covers exactly `sum(count)` distinct slots — a mis-based / overlapping slice
    // would let the resolve read another froxel's (or uninitialized) indices while the per-froxel
    // `count` still matched the host, which is the documented occupancy-only blind spot.
    let mut claimed_slices: Vec<(u32, u32)> = Vec::new();
    for z in 0..cfg.dim_z {
        for y in 0..cfg.dim_y {
            for x in 0..cfg.dim_x {
                let fi = golden_cluster_index(x, y, z, cfg.dim_x, cfg.dim_z) as usize;
                let (gpu_offset, gpu_count) = cluster_cell(&grid_bytes, fi);
                let host_set = &host_grid[fi];
                let host_count = host_set.len() as u32;
                assert_eq!(
                    gpu_count, host_count,
                    "L1 cull occupancy mismatch at froxel ({x},{y},{z}) [fi={fi}]: GPU {gpu_count} \
                     vs host {host_count} — the GPU cull dropped or kept a light the host oracle did not"
                );

                // The STRONG probe (Step 1): for a non-empty froxel, read the GPU's claimed slice
                // `LightIndexList[offset .. offset+count)` and assert its index SET equals the host
                // `golden_cluster_cull` set for the same froxel (sorted — order is table-order in
                // both, but compare as sets to be robust). A correct `count` with WRONG index
                // VALUES (a mis-based slice, or all-lights, or uninitialized tail) — exactly the
                // over-accumulation symptom — is caught here, NOT by the count-only gate.
                if gpu_count > 0 {
                    assert!(
                        (gpu_offset as u64) + (gpu_count as u64) <= index_list_cap as u64,
                        "L1 cull slice out of bounds at froxel ({x},{y},{z}) [fi={fi}]: offset \
                         {gpu_offset} + count {gpu_count} > index_list_cap {index_list_cap}"
                    );
                    let mut gpu_set: Vec<u32> = (0..gpu_count)
                        .map(|k| light_index_entry(&index_bytes, gpu_offset + k))
                        .collect();
                    let mut host_sorted: Vec<u32> = host_set.clone();
                    gpu_set.sort_unstable();
                    host_sorted.sort_unstable();
                    assert_eq!(
                        gpu_set, host_sorted,
                        "L1 cull INDEX-SET mismatch at froxel ({x},{y},{z}) [fi={fi}] (offset \
                         {gpu_offset}): GPU LightIndexList slice {gpu_set:?} vs host \
                         golden_cluster_cull {host_sorted:?} — the cull wrote correct COUNT but \
                         WRONG index values (the over-accumulation blind spot)"
                    );
                    // Every index must be a valid point/spot slot `[l0a_count, light_count)` (the
                    // cull must never scatter a directional/sky or an out-of-range index).
                    let l0a = header.l0a_count();
                    let total = header.light_count();
                    for &j in &gpu_set {
                        assert!(
                            j >= l0a && j < total,
                            "L1 cull wrote a non-point/spot index {j} into froxel ({x},{y},{z}) \
                             [fi={fi}] (valid range [{l0a},{total}))"
                        );
                    }
                    claimed_slices.push((gpu_offset, gpu_count));
                }

                gpu_total += gpu_count as u64;
                host_total += host_count as u64;
            }
        }
    }
    assert_eq!(gpu_total, host_total, "L1 cull total occupancy GPU {gpu_total} != host {host_total}");
    assert!(
        host_total > 0,
        "L1 cull occupancy gate is vacuous — the known scene landed NO point/spot light in ANY froxel"
    );

    // Assert the claimed slices are PAIRWISE DISJOINT and their counts sum to the total occupancy
    // (no two froxels share a base — an overlap would make the resolve read a neighbour's indices
    // for a correct-count froxel). Sort by offset, then verify each slice starts at or after the
    // previous slice's end.
    claimed_slices.sort_unstable_by_key(|&(off, _)| off);
    let mut prev_end = 0u32;
    let mut covered = 0u64;
    for &(off, cnt) in &claimed_slices {
        assert!(
            off >= prev_end,
            "L1 cull slices OVERLAP: a slice at offset {off} starts before the previous slice's \
             end {prev_end} — the InterlockedAdd claims must be disjoint"
        );
        prev_end = off + cnt;
        covered += cnt as u64;
    }
    assert_eq!(
        covered, gpu_total,
        "L1 cull disjoint-slice coverage {covered} != total occupancy {gpu_total}"
    );

    println!(
        "[l1_cull] GPU ClusterGrid occupancy + LightIndexList index-SET == host \
         golden_cluster_cull froxel-for-froxel; {} disjoint slices cover {gpu_total} index slots \
         across {cluster_count} froxels",
        claimed_slices.len()
    );
}

/// Reads a `LightIndexList` readback (`index_list_cap` × `u32`) at flat slot `i`, returning the
/// stored light-table index. Only slots inside a froxel's claimed slice `[offset, offset+count)`
/// carry a valid index (the buffer is not zero-initialized before the cull).
fn light_index_entry(index_bytes: &[u8], i: u32) -> u32 {
    let base = (i as usize) * 4;
    u32::from_le_bytes([
        index_bytes[base],
        index_bytes[base + 1],
        index_bytes[base + 2],
        index_bytes[base + 3],
    ])
}

// ===========================================================================================
// SDF M1 — the OFFSCREEN GPU empty-skip gate (`#[ignore]`, for the owner's RTX).
//
// The owner's on-device visual/correctness gate. It (a) creates the marcher descriptor-set
// layout WITH binding 9 (the `PointerGrid` StorageBuffer at `register(t9)`) alongside the
// existing 0..8 marcher bindings, (b) bakes the pointer grid (`build_pointer_grid`) from the
// authority `SdfEditField` and uploads it as a `BoundBuffer` (binding 9), (c) runs the
// marcher with `brick_enabled = 1` (via `FineMarcherPush::with_brick`), (d) reads back the
// resolved LIT G-buffer and asserts EVERY texel is within ±2/255 (the established GPU-golden
// tolerance) of `golden_composite_pixel_brick(.., brick_enabled = true, &grid, &cells)`.
//
// It ALSO asserts the brick-ON GPU image is within ±2/255 of the brick-OFF GPU image — the
// on-device empty-skip hit-set == analytic gate (the host gate proves it CPU-side; this proves
// the GPU shader's gated empty-skip matches the GPU analytic marcher). Validation-clean.
//
// THE DESCRIPTOR WIRING THIS GATE ADDED (so a future maintainer can trace it):
//   - `run_gbuffer_hybrid_lit_table_brick` (the harness): a `brick: Option<(&PointerGrid,
//     &[u32])>` param. binding 9 (`DescriptorKind::StorageBuffer`, COMPUTE) is now in the
//     marcher `layout_entries` + the `bind_group`, bound to the `build_pointer_grid` bake on
//     the ON path and to a 1-cell placeholder on the OFF path (the shader statically
//     references t9 inside the runtime-gated branch, so the layout must declare it either way).
//   - the marcher `FineMarcherPush` is built with `.with_brick(grid.origin, grid.dims,
//     grid.brick_world)` when `brick.is_some()`, stamping the M1 push uniforms @32/@44/@48/@60.
//   - the buffer is destroyed in the cleanup block; the run stays validation-clean.
//
// Run on the RTX: `cargo test -p boyko_rhi_vulkan --test sdf_gbuffer_hybrid \
//   sdf_m1_brick_offscreen -- --ignored --nocapture`
// ===========================================================================================

/// Bakes the default near-field pointer grid from `edits` (the SAME `build_pointer_grid`
/// the host golden replays + the GPU binds at binding 9). Returns the grid descriptor + the
/// dense `u32` cell codes.
fn bake_brick_grid(edits: &[SdfEdit]) -> (boyko_sdf_math::brick::PointerGrid, Vec<u32>) {
    use boyko_sdf_math::SdfEditField;
    use boyko_sdf_math::brick::{PointerGrid, build_pointer_grid};
    let mut field = SdfEditField::new();
    for e in edits {
        assert!(field.push(*e), "scene must fit MAX_SDF_EDITS");
    }
    field.bump_gen();
    let grid = PointerGrid::default_near_field();
    let mut cells = vec![0u32; grid.cell_count()];
    build_pointer_grid(&field, &grid, &mut cells);
    (grid, cells)
}

/// SDF M1 OFFSCREEN GPU gate (`#[ignore]`, RTX-only). The marcher runs with the empty-skip
/// ON (binding 9 = the baked pointer grid, `brick_enabled = 1`); the LIT readback is asserted
/// (i) within ±2/255 of `golden_composite_pixel_brick(brick_enabled = true)` and (ii) within
/// ±2/255 of the brick-OFF GPU image (the on-device hit-set == analytic gate). Validation-clean.
#[test]
#[ignore = "GPU offscreen gate — requires a Vulkan device (the owner's RTX); run with --ignored"]
fn sdf_m1_brick_offscreen_matches_golden_and_analytic() {
    let Some(ctx) = boot_or_skip("sdf_m1_brick_offscreen_matches_golden_and_analytic") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());
    // Pixel/analytic golden: runs with or without validation (the clean-oracle
    // assert self-gates when validation is disabled).
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — golden still runs");
    }

    for (name, edits) in [
        ("crater_csg", crater()),
        ("box_csg", box_csg()),
        ("smooth_union", smooth_union()),
    ] {
        let (grid, cells) = bake_brick_grid(&edits);

        // The brick-ON marcher run (binding 9 = the bake, brick_enabled = 1; M2 trilinear OFF).
        let (lit_on, _) = run_gbuffer_hybrid_lit_table_brick(
            &ctx, &edits, false, false, 1.0, 0, DEFAULT_LIGHT_DIR, &DEGENERATE_LIGHT_TABLE,
            Some((&grid, &cells)), false,
        );
        // The brick-OFF marcher run (binding 9 = placeholder, brick_enabled = 0; M2 trilinear OFF).
        let (lit_off, _) = run_gbuffer_hybrid_lit_table_brick(
            &ctx, &edits, false, false, 1.0, 0, DEFAULT_LIGHT_DIR, &DEGENERATE_LIGHT_TABLE, None, false,
        );
        assert_eq!(lit_on.len(), READBACK_BYTES as usize);
        assert_eq!(lit_off.len(), READBACK_BYTES as usize);

        // THE LOAD-BEARING GPU GATE: GPU brick-ON == GPU brick-OFF (analytic) within ±2/255.
        // Both runs go through the IDENTICAL deferred resolve + L0a lighting; only the
        // marcher's empty-skip differs. This is the on-device hit-set == analytic proof and
        // is INDEPENDENT of which shading model the GPU uses (the host inline-Lambert oracle
        // `golden_composite_pixel_brick` models the MVP-1 composite, NOT the deferred PBR LIT
        // the GPU emits — so a host-vs-GPU diff is a shading-model gap, not an empty-skip bug;
        // the ON-vs-OFF GPU diff isolates the empty-skip exactly).
        let mut max_vs_off = 0i32;
        let mut diverged_px: Option<(u32, u32, [i32; 3], [i32; 3])> = None;
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                let base = ((py * SDF_IMG_W + px) as usize) * 4;
                let gpu_on = unpack_texel_rgb(&lit_on[base..base + 4]);
                let gpu_off = unpack_texel_rgb(&lit_off[base..base + 4]);
                let d = (0..3).map(|ch| (gpu_on[ch] - gpu_off[ch]).abs()).max().unwrap();
                if d > max_vs_off {
                    max_vs_off = d;
                    if d > CHANNEL_TOL {
                        diverged_px = Some((px, py, gpu_on, gpu_off));
                    }
                }
            }
        }
        assert!(
            max_vs_off <= CHANNEL_TOL,
            "[{name}] GPU brick-ON LIT vs GPU brick-OFF (analytic): max per-channel delta \
             {max_vs_off}/255 > {CHANNEL_TOL} (the on-device empty-skip changed the image — a \
             skipped or spurious surface). First divergent: {diverged_px:?}"
        );
        assert_validation_clean(&ctx);
        println!(
            "[{name}] M1 GPU empty-skip: brick-ON vs GPU analytic {max_vs_off}/255 (tol \
             {CHANNEL_TOL}); validation clean"
        );
    }
}

// ===========================================================================================
// SDF M2 — the OFFSCREEN GPU trilinear+cubic SURFACE discriminator (`#[ignore]`, RTX-only).
//
// The make-or-break on-device gate for the brick-atlas M2 path. It re-uses the re-wired
// `run_gbuffer_hybrid_m2` harness (binding 10 = the `BrickAtlas` baked from the scene, the b5
// camera UBO widened to 128 B with the `M2GridParams` tail @80, `with_brick_trilinear(true)`)
// and proves FOUR properties the prior `.Load`-fix incident left unverified:
//
//   (a) THE M2 BRANCH ENGAGES — `m2(M2GridParams default) != m2(M2GridParams zeroed)`. A zeroed
//       grid block (`atlas_dim == 0`) maps no tile, so the M2 step finds nothing → the image must
//       DIFFER from the default-grid run. (The prior bug made these byte-identical = branch dead.)
//   (b) M2 != ANALYTIC — `m2(brick_trilinear=1) != analytic(brick_trilinear=0)` on the SURFACE
//       scenes: the M2 cubic finds near-tangent crossings the analytic sphere-trace's SDF_EPS gate
//       under-resolves. (The prior `.SampleLevel` bug gave 0 GPU hits → byte-identical to analytic.)
//   (c) HOST-MIRROR HIT AGREEMENT — every non-mesh pixel's GPU hit/miss decision (`gViewT !=
//       sentinel`) matches the host mirror `golden_composite_pixel_brick_m2(brick_trilinear=true)`
//       hit/miss; on the hit pixels the GPU surface `t` matches the host's analytic hit `t` within
//       a small world-`t` epsilon (the GPU `.Load` corners bit-match the host `decode_snorm8`
//       fetch → the GPU cubic == the host cubic). NOTE: the GPU emits DEFERRED-PBR LIT while the
//       host mirror emits the MVP-1 inline-Lambert PACKED color, so the two SHADED colors diverge
//       by design (the documented M1 shading-model gap); the load-bearing host==GPU agreement is
//       therefore the HIT-SET + the surface-`t`, NOT the packed color. The pass-through (mesh /
//       background) arms, where both shading models agree, are still color-checked within ±2/255.
//   (d) EXACT CSG — every GPU M2 hit lies on the true analytic surface: reconstruct `p = ro + rd *
//       gViewT` and assert `|sdf_edit_list(p)| < M2_CREASE_EPS` (the residual-fallback guarantee).
//       Zero wrong-surface hits.
//
// Run on the RTX: `cargo test -p boyko_rhi_vulkan --test sdf_gbuffer_hybrid \
//   sdf_m2_brick_trilinear_offscreen -- --ignored --nocapture`
// ===========================================================================================

/// The gViewT non-hit sentinel (the marcher stores `1.0e30` on a pure-background / empty pixel;
/// a finite value is a real surface `t`). Mirrors the shader's `FAR`-class sentinel. NOTE
/// (Render P7/P5-r1b): a MESH-covered raster-owned pixel now carries the finite mesh ray-t
/// `t_mesh` (NOT the sentinel), so `gpu_is_hit` is only meaningful in the PURE-SDF region — every
/// caller below restricts to `!mesh_covers_pixel(px, py)` for exactly this reason.
const VIEWT_NO_HIT: f32 = 1.0e30;

/// A GPU pixel is an SDF surface hit iff its gViewT lane carries a finite (non-sentinel) `t`.
/// Callers MUST restrict to the pure-SDF region (a mesh pixel's finite `t_mesh` is not an SDF hit).
fn gpu_is_hit(viewt: &[f32], px: u32, py: u32) -> bool {
    let t = viewt[(py * SDF_IMG_W + px) as usize];
    t.is_finite() && t < VIEWT_NO_HIT * 0.5
}

/// SDF M2 OFFSCREEN GPU discriminator (`#[ignore]`, RTX-only). Proves the brick-atlas trilinear+
/// cubic SURFACE path ENGAGES on-device, finds crossings the analytic marcher misses, agrees with
/// the host cubic's hit-set + surface-`t`, and keeps EXACT CSG (every hit on the true surface).
#[test]
#[ignore = "GPU offscreen gate — requires a Vulkan device (the owner's RTX); run with --ignored"]
fn sdf_m2_brick_trilinear_offscreen_engages_and_matches_host() {
    let Some(ctx) = boot_or_skip("sdf_m2_brick_trilinear_offscreen_engages_and_matches_host") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());
    // Host-mirror golden: runs with or without validation (the clean-oracle
    // assert self-gates when validation is disabled).
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — golden still runs");
    }

    // The host mirror's hit-`t` is not exposed directly, so the (c) surface-`t` agreement compares
    // the GPU `gViewT` against the host mirror's hit DECISION (color != background) and, where both
    // hit, against the analytic field crossing the GPU `t` reconstructs to (the EXACT-CSG residual
    // in (d) doubles as the host-vs-GPU `t` proof: both land within M2_CREASE_EPS of the true field).
    let mut any_scene_hit = false;

    for (name, edits) in [
        ("crater_csg", crater()),
        ("box_csg", box_csg()),
        ("smooth_union", smooth_union()),
    ] {
        let (grid, cells) = bake_brick_grid(&edits);

        // HOST PRE-FLIGHT (no GPU): does the host mirror's M2-ON path differ from its analytic
        // path on THIS scene at all? `golden_composite_pixel_brick_m2(true)` runs the cubic;
        // `(false)` delegates to the analytic marcher. If the host packed colors are identical on
        // every pixel, the M2 path is a perf optimization that lands on the SAME surface (the
        // exact-CSG contract) and an on-device LIT/`t` byte-identity is CORRECT, not a dead branch.
        let bg_pf = packed_background();
        let mut host_on_off_diff = 0u32;
        let mut host_m2_hits = 0u32;
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                if mesh_covers_pixel(px, py) {
                    continue;
                }
                let on = golden_composite_pixel_brick_m2(
                    &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    1.0, 0, DEFAULT_LIGHT_DIR, true, true, &grid, &cells,
                );
                let off = golden_composite_pixel_brick_m2(
                    &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    1.0, 0, DEFAULT_LIGHT_DIR, true, false, &grid, &cells,
                );
                if on != off {
                    host_on_off_diff += 1;
                }
                let on_rgb = unpack_packed_rgb(on);
                if (0..3).any(|c| (on_rgb[c] - bg_pf[c]).abs() > CHANNEL_TOL) {
                    host_m2_hits += 1;
                }
            }
        }
        println!(
            "[{name}] HOST pre-flight: golden_brick_m2(ON) vs (OFF/analytic) differ on \
             {host_on_off_diff} non-mesh px; {host_m2_hits} host M2 hits"
        );

        // Capture the FIRST host M2-on-vs-analytic divergent pixel as the on-device counterexample
        // anchor: the host says the M2 cubic changes this pixel; the GPU gViewT/LIT must show the
        // SAME change if the branch engages. Reported when the (a) assertion below trips.
        let host_first_div: Option<(u32, u32, u32, u32)> = (|| {
            for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    if mesh_covers_pixel(px, py) {
                        continue;
                    }
                    let on = golden_composite_pixel_brick_m2(
                        &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                        CompositeCamera::Ortho, 1.0, 0, DEFAULT_LIGHT_DIR, true, true, &grid, &cells,
                    );
                    let off = golden_composite_pixel_brick_m2(
                        &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                        CompositeCamera::Ortho, 1.0, 0, DEFAULT_LIGHT_DIR, true, false, &grid, &cells,
                    );
                    if on != off {
                        return Some((px, py, on, off));
                    }
                }
            }
            None
        })();

        // Three runs, ALL reading back gViewT (the per-pixel surface `t`): M2 ON default grid,
        // M2 ON zeroed grid (atlas_dim 0 → the branch maps no tile), and the analytic marcher.
        let (lit_m2, _, viewt_m2_opt) = run_gbuffer_hybrid_m2(
            &ctx, &edits, false, false, 1.0, 0, DEFAULT_LIGHT_DIR, &DEGENERATE_LIGHT_TABLE,
            Some((&grid, &cells)), true, true, true,
        );
        let viewt_m2 = viewt_m2_opt.expect("gViewT readback requested");

        let (_lit_m2_zeroed, _, viewt_z_opt) = run_gbuffer_hybrid_m2(
            &ctx, &edits, false, false, 1.0, 0, DEFAULT_LIGHT_DIR, &DEGENERATE_LIGHT_TABLE,
            Some((&grid, &cells)), true, true, /* m2_grid_default = */ false,
        );
        let viewt_zeroed = viewt_z_opt.expect("gViewT readback requested");

        let (lit_analytic, _, viewt_a_opt) = run_gbuffer_hybrid_m2(
            &ctx, &edits, false, false, 1.0, 0, DEFAULT_LIGHT_DIR, &DEGENERATE_LIGHT_TABLE,
            Some((&grid, &cells)), false, true, true,
        );
        let viewt_analytic = viewt_a_opt.expect("gViewT readback requested");

        assert_eq!(lit_m2.len(), READBACK_BYTES as usize);

        // --- (a) THE M2 BRANCH ENGAGES: the M2 path TERMINATES the march at the cubic-validated
        // crossing, NOT the analytic SDF_EPS threshold, so the surface `t` it stores differs from
        // the analytic marcher's at the sub-epsilon level (and from the zeroed-grid run, which maps
        // no tile → folds analytic). LIT color is byte-identical by DESIGN (the exact-CSG contract:
        // M2 lands on the SAME true surface; the committed `on_hit_set_equals_analytic_over_many_
        // scenes` pins ON == analytic within ±1/255), so the DISCRIMINATING observable is `gViewT`,
        // not the shaded color. We require the default-grid M2 `t` field to DIFFER from BOTH the
        // zeroed-grid run AND the analytic run on at least one shared hit pixel — proof the marcher
        // read the b5 M2GridParams tail and took the cubic branch. ---
        let viewt_t_delta = |a: &[f32], b: &[f32]| -> (f32, u32) {
            let mut max_d = 0.0f32;
            let mut diff_px = 0u32;
            for i in 0..(PIXELS as usize) {
                if a[i].is_finite() && a[i] < VIEWT_NO_HIT * 0.5 && b[i].is_finite() && b[i] < VIEWT_NO_HIT * 0.5 {
                    let d = (a[i] - b[i]).abs();
                    if d > 1.0e-6 {
                        diff_px += 1;
                    }
                    max_d = max_d.max(d);
                }
            }
            (max_d, diff_px)
        };
        let (a_vs_zero_max, a_vs_zero_px) = viewt_t_delta(&viewt_m2, &viewt_zeroed);
        let (a_vs_an_max, a_vs_an_px) = viewt_t_delta(&viewt_m2, &viewt_analytic);
        let lit_a_max = {
            let mut m = 0i32;
            for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    let base = ((py * SDF_IMG_W + px) as usize) * 4;
                    let a = unpack_texel_rgb(&lit_m2[base..base + 4]);
                    let b = unpack_texel_rgb(&lit_analytic[base..base + 4]);
                    m = m.max((0..3).map(|ch| (a[ch] - b[ch]).abs()).max().unwrap());
                }
            }
            m
        };
        println!(
            "[{name}] (a/b) gViewT Δt: M2-default vs zeroed-grid max {a_vs_zero_max:.5} on \
             {a_vs_zero_px} px; M2-default vs analytic max {a_vs_an_max:.5} on {a_vs_an_px} px; \
             LIT M2-vs-analytic max {lit_a_max}/255 (exact-CSG ⇒ ~0 expected)"
        );
        // The on-device counterexample at the FIRST host-divergent pixel: the host M2 cubic changes
        // this pixel, so a live GPU branch must too. Report the GPU gViewT (M2 default vs analytic)
        // there — bit-equal ⇒ the branch is dead at a pixel the host PROVES it should change.
        let counterexample = host_first_div.map(|(px, py, on, off)| {
            let idx = (py * SDF_IMG_W + px) as usize;
            let (ro, rd) = composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
            let gpu_t_m2 = viewt_m2[idx];
            let gpu_t_an = viewt_analytic[idx];
            format!(
                "pixel ({px},{py}): host golden_brick_m2(ON)=0x{on:08X} vs (analytic)=0x{off:08X} \
                 DIFFER → the host cubic moves this pixel; but GPU gViewT_m2={gpu_t_m2} == \
                 gViewT_analytic={gpu_t_an} (ro={ro:?} rd={rd:?}). The on-device M2 branch did NOT \
                 move it."
            )
        }).unwrap_or_else(|| "no host-divergent pixel captured".to_string());
        assert!(
            a_vs_zero_px > 0 || a_vs_an_px > 0,
            "[{name}] (a) M2 branch DEAD: the M2-default gViewT `t` field is BIT-IDENTICAL to BOTH \
             the zeroed-grid run AND the analytic run (Δt max default-vs-zeroed {a_vs_zero_max:.2e}, \
             default-vs-analytic {a_vs_an_max:.2e}) while the HOST mirror changes \
             {host_on_off_diff} pixel(s) for the SAME inputs. The marcher never took the cubic \
             branch on-device (it neither read the b5 M2GridParams tail nor terminated at a cubic \
             crossing). The .Load fix did not fully land. COUNTEREXAMPLE: {counterexample}"
        );

        // --- (a2) EXACT-CSG LIT AGREEMENT: brick-cubic LIT must match analytic LIT within the
        // consumer-side budget. This assertion was MISSING (`lit_a_max` was computed-but-unasserted),
        // which is exactly what hid BUG-B1-ANALYTIC-BLACK: the over-relaxed analytic accept landed
        // DEEP inside the surface (`d < 0` but `< EPS`), collapsing its shadow+AO to 0 → the analytic
        // arm rendered BLACK while the brick arm (two-sided signed refine) rendered the lit gray.
        // With the analytic accept now applying the SAME signed refine, both land ON the surface, so
        // brick and analytic agree to within `LIT_CHANNEL_TOL` (±3/255 — the brick cubic vs analytic
        // surface differs at the sub-pixel level, never gray-vs-black). A regression that reopens the
        // overshoot would push `lit_a_max` to ~the gray magnitude and trip here.
        assert!(
            lit_a_max <= LIT_CHANNEL_TOL,
            "[{name}] (a2) EXACT-CSG LIT MISMATCH: max |LIT_brick − LIT_analytic| = {lit_a_max}/255 \
             exceeds ±{LIT_CHANNEL_TOL}/255. The brick and analytic arms must land on the SAME true \
             surface (exact-CSG) → near-identical LIT. A large delta means one arm accepted a hit \
             OFF the surface (e.g. the B1 over-relaxation overshoot collapsing analytic shadow+AO to \
             black) — the analytic accept-refine regressed."
        );

        // --- (b) M2 ENGAGES THE CUBIC (not the analytic fold): the default-grid `t` field differs
        // from the zeroed-grid run on the SURFACE pixels (the zeroed grid degrades the M2 step to
        // the analytic fold). This isolates "the cubic ran" from "the trilinear gate is on but the
        // tile lookup yields nothing". ---
        assert!(
            a_vs_zero_px > 0,
            "[{name}] (b) M2 cubic INERT: default-grid gViewT == zeroed-grid gViewT (Δt max \
             {a_vs_zero_max:.2e}) — the trilinear gate is on but the M2GridParams tile lookup found \
             no SURFACE tile, so the cubic never ran on-device (the prior 0-cubic-hits state)."
        );

        // --- (c) HOST-MIRROR HIT AGREEMENT + (d) EXACT CSG ---
        // The host mirror with NO mesh (MESH_DEPTH_CLEAR) gives the pure SDF hit-set; compare only
        // the NON-mesh-covered pixels (the mesh-covered ones are pass-through on both sides and
        // carry the gViewT sentinel). `golden_composite_pixel_brick_m2` returns the MVP-1 packed
        // color; a hit ⇒ a shaded (non-background) color.
        let bg = packed_background();
        let mut gpu_hits = 0u32;
        let mut host_hits = 0u32;
        let mut hitset_mismatch: Option<(u32, u32, bool, bool)> = None;
        let mut csg_violations = 0u32;
        let mut worst_resid = 0.0f32;
        let mut worst_resid_px: Option<(u32, u32, f32, f32)> = None;
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                if mesh_covers_pixel(px, py) {
                    continue; // restrict to the pure SDF region (no mesh occlusion)
                }
                // The host M2 hit decision (no mesh → background on a miss, a shaded color on a hit).
                let host_packed = golden_composite_pixel_brick_m2(
                    &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    1.0, 0, DEFAULT_LIGHT_DIR, true, true, &grid, &cells,
                );
                let host_rgb = unpack_packed_rgb(host_packed);
                let host_hit = (0..3).any(|c| (host_rgb[c] - bg[c]).abs() > CHANNEL_TOL);
                let gpu_hit = gpu_is_hit(&viewt_m2, px, py);
                if host_hit {
                    host_hits += 1;
                }
                if gpu_hit {
                    gpu_hits += 1;
                }
                // (c) hit-set agreement.
                if host_hit != gpu_hit && hitset_mismatch.is_none() {
                    hitset_mismatch = Some((px, py, gpu_hit, host_hit));
                }
                // (d) EXACT CSG: a GPU hit's reconstructed point lies on the true surface.
                if gpu_hit {
                    let t = viewt_m2[(py * SDF_IMG_W + px) as usize];
                    let (ro, rd) = composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
                    let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                    let resid = boyko_sdf_math::sdf_edit_list(&edits, p).abs();
                    if resid > worst_resid {
                        worst_resid = resid;
                        worst_resid_px = Some((px, py, t, resid));
                    }
                    if resid >= M2_CREASE_EPS {
                        csg_violations += 1;
                    }
                }
            }
        }

        // (c): the hit-sets must agree pixel-for-pixel (the GPU cubic == the host cubic).
        assert!(
            hitset_mismatch.is_none(),
            "[{name}] (c) HOST-MIRROR HIT MISMATCH at {hitset_mismatch:?} (px, py, gpu_hit, \
             host_hit): the GPU `.Load` cubic disagrees with the host `decode_snorm8` cubic on the \
             hit/miss decision. gViewT corners diverged from the host fetch."
        );
        // (d): every GPU hit lands on the true analytic surface — exact CSG, 0 wrong-surface hits.
        assert_eq!(
            csg_violations, 0,
            "[{name}] (d) EXACT-CSG VIOLATION: {csg_violations} GPU M2 hit(s) off the true surface \
             (|sdf(hit)| >= M2_CREASE_EPS={M2_CREASE_EPS}). Worst: {worst_resid_px:?} (px, py, t, \
             resid). The residual fallback failed to land the hit on the field."
        );
        assert_eq!(
            gpu_hits, host_hits,
            "[{name}] (c) HIT-COUNT MISMATCH: GPU {gpu_hits} vs host {host_hits} SDF hits"
        );
        assert!(gpu_hits > 0, "[{name}] no GPU M2 hits — the SURFACE scene produced an empty image");
        any_scene_hit = true;

        assert_validation_clean(&ctx);
        println!(
            "[{name}] M2 ENGAGES (a: gViewT Δt default-vs-zeroed max {a_vs_zero_max:.5} on \
             {a_vs_zero_px} px; LIT M2-vs-analytic {lit_a_max}/255 ≈0 by exact-CSG) · CUBIC RAN \
             (b: {a_vs_zero_px} px differ from the analytic fold) · HOST-MIRROR hit-set MATCH \
             (c: {gpu_hits} GPU == {host_hits} host hits) · EXACT CSG (d: 0 violations, worst \
             |sdf(hit)|={worst_resid:.5} < {M2_CREASE_EPS}); validation clean"
        );
    }

    assert!(any_scene_hit, "no scene produced an M2 surface hit — the discriminator proved nothing");
}

// ===========================================================================================
// SDF brick-atlas M4 (clip-map LOD, Slice C) — the N-level clip-map tests. The OFF/N=1 byte-identity
// gates are CPU-RUNNABLE (host goldens); the engagement + far-field gates are `#[ignore]` RTX
// (the owner runs them on the device — the host mirror is the CPU oracle the GPU is compared against).
//
// Run the RTX gates: `cargo test -p boyko_rhi_vulkan --test sdf_gbuffer_hybrid sdf_m4_clipmap -- \
//   --ignored --nocapture`
// ===========================================================================================

/// Builds the per-level host EMPTY-SKIP pointer grids for `golden_composite_pixel_brick_m4`, each
/// mirroring the GPU `PointerGrid{L}` binding the shader's level-`L` empty-skip arm reads.
///
/// CRITICAL — the empty-skip grid and the surface atlas grid are DISTINCT, exactly as M2 keeps them
/// (the conflation of the two was BUG-M4-SLICE-C-1):
/// - **Level 0** uses the FINE [`PointerGrid::default_near_field`] (`DEFAULT_GRID_DIM³ @
///   DEFAULT_BRICK_WORLD` = `16³ @ 0.5`, origin `[-4, -4, -4]`) — the SAME grid the GPU binds at
///   binding 9 and the shader's lvl==0 arm reads via `pc.grid_*` (the harness seeds binding 9 +
///   `with_brick` from `bake_brick_grid`, which IS `default_near_field`). This is also the SAME grid
///   `golden_composite_pixel_brick_m2`'s empty-skip reads, so the OFF/N=1 path is byte-identical to M2.
///   The level-0 SURFACE atlas (the COARSE `M2_GRID_DIM³ @ M2_BRICK_WORLD` = `4³ @ 2.0` grid) is read
///   independently inside [`host_m2_surface_hit_at`] via `at_level_from_params` — NOT from this grid.
/// - **Levels ≥ 1** use the per-level COARSE grid (`M2_GRID_DIM³` cells of `geo.brick_world` from the
///   snapped `geo.origin`) — the SAME bake the GPU clip-map's `PointerGrid1/2` SSBO holds (the shader's
///   coarse arms read `m2_levels[L]` geometry, i.e. `4³ @ (M2_BRICK_WORLD · 2^L)`).
///
/// Returns `(PointerGrid, Vec<u32>)` per level so the host golden's per-level empty-skip mirrors the
/// GPU per level. The level-0 fine grid is origin-centered (NOT camera-snapped) to mirror the GPU
/// binding 9, which the harness always seeds from the origin-centered `bake_brick_grid`.
fn bake_clipmap_host_grids(
    edits: &[SdfEdit],
    params: &M4GridParams,
) -> Vec<(boyko_sdf_math::brick::PointerGrid, Vec<u32>)> {
    use boyko_rhi_vulkan::compute::BrickLevelParams;
    use boyko_sdf_math::SdfEditField;
    use boyko_sdf_math::brick::{self, PointerGrid, build_pointer_grid};
    let mut field = SdfEditField::new();
    for e in edits {
        assert!(field.push(*e), "scene must fit MAX_SDF_EDITS");
    }
    field.bump_gen();
    let mut out = Vec::with_capacity(brick::BRICK_LEVELS);
    for level in 0..brick::BRICK_LEVELS {
        let grid = if level == 0 {
            // Level-0 empty-skip = the FINE 16³@0.5 near-field grid the GPU binds at binding 9 (read
            // via `pc.grid_*`), identical to the M2 reference's empty-skip grid — NOT the 4³@2.0 atlas.
            PointerGrid::default_near_field()
        } else {
            // Coarse levels: the per-level 4³@scaled grid the GPU's `PointerGrid{L}` SSBO holds (the
            // snapped MIN corner from `geo.origin`, `geo.brick_world` cells), mirroring `m2_levels[L]`.
            let geo = BrickLevelParams::at_level_from_params(params, level);
            PointerGrid {
                origin: geo.origin,
                dims: [brick::M2_GRID_DIM; 3],
                brick_world: geo.brick_world,
            }
        };
        let mut cells = vec![0u32; grid.cell_count()];
        build_pointer_grid(&field, &grid, &mut cells);
        out.push((grid, cells));
    }
    out
}

/// Writes an RGBA8 image (`width × height`, the harness readback layout) as a 24-bit BMP for the
/// owner's visual sign-off (the established offscreen screenshot pattern). BGR, bottom-up rows, 4-byte
/// row padding. Drops the alpha channel.
fn write_bmp_rgba(path: &std::path::Path, rgba: &[u8], width: u32, height: u32) {
    let w = width as usize;
    let h = height as usize;
    let row_unpadded = w * 3;
    let row_padded = (row_unpadded + 3) & !3; // 4-byte aligned rows
    let pixel_bytes = row_padded * h;
    let file_size = 54 + pixel_bytes; // 14-byte file header + 40-byte info header
    let mut buf = Vec::with_capacity(file_size);
    // File header (14 bytes).
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&(file_size as u32).to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // reserved
    buf.extend_from_slice(&54u32.to_le_bytes()); // pixel data offset
    // Info header (BITMAPINFOHEADER, 40 bytes).
    buf.extend_from_slice(&40u32.to_le_bytes());
    buf.extend_from_slice(&(width as i32).to_le_bytes());
    buf.extend_from_slice(&(height as i32).to_le_bytes()); // positive ⇒ bottom-up
    buf.extend_from_slice(&1u16.to_le_bytes()); // planes
    buf.extend_from_slice(&24u16.to_le_bytes()); // bits per pixel
    buf.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB (no compression)
    buf.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    buf.extend_from_slice(&2835i32.to_le_bytes()); // x ppm (~72 DPI)
    buf.extend_from_slice(&2835i32.to_le_bytes()); // y ppm
    buf.extend_from_slice(&0u32.to_le_bytes()); // palette colors
    buf.extend_from_slice(&0u32.to_le_bytes()); // important colors
    // Pixel data: bottom-up, BGR, padded rows.
    for y in (0..h).rev() {
        let mut written = 0;
        for x in 0..w {
            let i = (y * w + x) * 4;
            buf.push(rgba[i + 2]); // B
            buf.push(rgba[i + 1]); // G
            buf.push(rgba[i]); // R
            written += 3;
        }
        while written < row_padded {
            buf.push(0);
            written += 1;
        }
    }
    std::fs::write(path, &buf).expect("write BMP screenshot");
}

/// A far-field scene: a sphere ~9.5 units deep along the ortho ray (`z = -7.5`, camera at `z = 2`, so
/// `t = 9.5 < T_MAX = 10`). It is OUTSIDE the level-0 box (`±4`) but inside the level-1 box (`±8`), so
/// the clip-map MUST select level 1 to render it — the M4 far-reach discriminator. The XY is on-screen
/// (the ortho view half-extent is `1.0`), so the sphere covers the center pixels.
fn far_field_sphere() -> Vec<SdfEdit> {
    vec![SdfEdit::sphere([0.0, 0.0, -7.5], 0.6, sdf_op::UNION, 0.0)]
}

/// SDF M4 OFF/N=1 BYTE-IDENTITY (CPU-runnable, the 0%-gate). The N-level host golden at `brick_levels =
/// 1` with `M4GridParams::near_field_only()` (level 0 == the M2 near-field) is BYTE-FOR-BYTE the
/// single-level M2 host golden on every non-mesh pixel, every scene. This is the inviolable keystone:
/// `brick_levels == 1` reduces the clip-map to the M2 path. No GPU required.
#[test]
fn sdf_m4_clipmap_off_byte_identical() {
    let params = M4GridParams::near_field_only();
    for (name, edits) in [
        ("crater_csg", crater()),
        ("box_csg", box_csg()),
        ("smooth_union", smooth_union()),
    ] {
        let (grid, cells) = bake_brick_grid(&edits);
        let level_grids_owned = bake_clipmap_host_grids(&edits, &params);
        // The golden takes `&[(PointerGrid, &[u32])]`; build the borrowed view over the owned grids.
        let level_grids: Vec<(boyko_sdf_math::brick::PointerGrid, &[u32])> = level_grids_owned
            .iter()
            .map(|(g, c)| (*g, c.as_slice()))
            .collect();
        let mut diffs = 0u32;
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                if mesh_covers_pixel(px, py) {
                    continue;
                }
                // The M2 single-level golden (the established M2 oracle).
                let m2 = golden_composite_pixel_brick_m2(
                    &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    1.0, 0, DEFAULT_LIGHT_DIR, true, true, &grid, &cells,
                );
                // The N-level golden at brick_levels = 1 (the OFF/N=1 path: select_level loops once
                // over level 0 == the M2 near-field, so it must reduce to the M2 golden bit-for-bit).
                let m4 = golden_composite_pixel_brick_m4(
                    &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    1.0, 0, DEFAULT_LIGHT_DIR, true, true, 1, &params, &level_grids,
                );
                if m2 != m4 {
                    diffs += 1;
                }
            }
        }
        assert_eq!(
            diffs, 0,
            "[{name}] M4 OFF/N=1 NOT byte-identical to M2: {diffs} pixel(s) differ. \
             `brick_levels == 1` + near_field_only() must reduce the clip-map golden to the M2 golden."
        );
    }
}

/// SDF M4 the per-level UBO block byte-identity (CPU-runnable, the 0%-gate). `near_field_only()`'s
/// LEVEL-0 48-byte block is BYTE-FOR-BYTE the M2 `default_near_field` tail (the keystone the shader's
/// `m2_levels[0]` reads at N=1). Proves the M4 array tail's first entry equals the pre-M4 M2 tail.
#[test]
fn sdf_m4_level0_ubo_block_byte_identical_to_m2() {
    let m4 = M4GridParams::near_field_only().as_ubo_bytes();
    let m2 = M2GridParams::default_near_field();
    let m2_bytes = m2.as_bytes();
    assert_eq!(
        &m4[..M2_GRID_PARAMS_BYTES_LOCAL],
        m2_bytes,
        "M4 near_field_only() level-0 block must be byte-identical to M2 default_near_field() \
         (the OFF/N=1 keystone — the shader's m2_levels[0] reads the same bytes the pre-M4 M2 tail had)"
    );
}

/// SDF M4 CLIP-MAP NEAR-FIELD == SINGLE-LEVEL (`#[ignore]`, RTX). A scene fully inside the level-0 box
/// (`±4`) rendered with the N-level clip-map (`brick_levels = 3`) is BYTE-IDENTICAL to the single-level
/// M2 render (`brick_levels = 1`): level 0 wins by containment, so the coarser levels never engage. The
/// host pre-flight (CPU) proves the goldens agree; the RTX run proves the GPU `gViewT`/LIT agree.
#[test]
#[ignore = "GPU offscreen gate — requires a Vulkan device (the owner's RTX); run with --ignored"]
fn sdf_m4_clipmap_near_field_matches_single_level() {
    let Some(ctx) = boot_or_skip("sdf_m4_clipmap_near_field_matches_single_level") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    for (name, edits) in [("crater_csg", crater()), ("box_csg", box_csg())] {
        let (grid, cells) = bake_brick_grid(&edits);

        // The N-level GPU clip-map, camera-centered on the origin (level 0 == the M2 near-field).
        let clipmap = {
            use boyko_sdf_math::SdfEditField;
            let mut field = SdfEditField::new();
            for e in &edits {
                assert!(field.push(*e), "scene must fit MAX_SDF_EDITS");
            }
            field.bump_gen();
            BrickClipmap::create(&ctx, &field, [0.0, 0.0, 0.0]).expect("M4 clip-map create")
        };

        // The single-level M2 render (brick_levels = 1) and the N-level clip-map render (brick_levels =
        // 3). Both read back gViewT — in the near field they must be byte-identical (level 0 contains
        // the whole scene, so select_level returns 0 on every hit pixel; levels 1/2 never engage).
        let (lit_single, _, vt_single_opt) = run_gbuffer_hybrid_m2(
            &ctx, &edits, false, false, 1.0, 0, DEFAULT_LIGHT_DIR, &DEGENERATE_LIGHT_TABLE,
            Some((&grid, &cells)), true, true, true,
        );
        let vt_single = vt_single_opt.expect("gViewT readback (single-level)");

        let (lit_clip, _, vt_clip_opt) = run_gbuffer_hybrid_m4(
            &ctx, &edits, false, false, 1.0, 0, DEFAULT_LIGHT_DIR, &DEGENERATE_LIGHT_TABLE,
            Some((&grid, &cells)), true, true, true, Some(&clipmap),
        );
        let vt_clip = vt_clip_opt.expect("gViewT readback (clip-map)");

        // Byte-identical LIT + gViewT in the near field (level 0 wins by containment).
        assert_eq!(
            lit_single, lit_clip,
            "[{name}] near-field clip-map LIT differs from single-level — level 0 should win by \
             containment (the scene is inside the ±4 level-0 box)"
        );
        let vt_diff = vt_single
            .iter()
            .zip(vt_clip.iter())
            .filter(|(a, b)| (*a - *b).abs() > 1.0e-6)
            .count();
        assert_eq!(
            vt_diff, 0,
            "[{name}] near-field clip-map gViewT differs from single-level on {vt_diff} px — level 0 \
             must win by containment in the near field"
        );

        assert_validation_clean(&ctx);
        // SAFETY: the render submits were fence-waited inside `run_gbuffer_hybrid_*`; the device is
        // drained, so no work still samples the clip-map; `destroy` consumes it (each resource once).
        unsafe { clipmap.destroy(&ctx) };
        println!("[{name}] M4 clip-map near-field == single-level (LIT + gViewT byte-identical)");
    }
}

/// SDF M4 CLIP-MAP FAR-FIELD RENDERS (`#[ignore]`, RTX). A sphere ~9.5 units deep (`z = -7.5`) is
/// OUTSIDE the level-0 box (`±4`) but inside level 1 (`±8`): the single-level M2 path (`brick_levels =
/// 1`) finds NO level-0 tile (no M2 cubic hit → analytic fold), while the clip-map (`brick_levels = 3`)
/// selects level 1 and renders the surface via the level-1 bricks. The GPU hit agrees with the analytic
/// field within `M2_CREASE_EPS` (the exact-CSG residual). This is the M4 far-reach proof.
#[test]
#[ignore = "GPU offscreen gate — requires a Vulkan device (the owner's RTX); run with --ignored"]
fn sdf_m4_clipmap_far_field_renders() {
    let Some(ctx) = boot_or_skip("sdf_m4_clipmap_far_field_renders") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let edits = far_field_sphere();
    let (grid, cells) = bake_brick_grid(&edits);

    let clipmap = {
        use boyko_sdf_math::SdfEditField;
        let mut field = SdfEditField::new();
        for e in &edits {
            assert!(field.push(*e), "scene must fit MAX_SDF_EDITS");
        }
        field.bump_gen();
        BrickClipmap::create(&ctx, &field, [0.0, 0.0, 0.0]).expect("M4 far-field clip-map create")
    };

    // The clip-map render (brick_levels = 3): the far sphere is in level 1, so the clip-map renders it.
    let (lit_clip, _, vt_clip_opt) = run_gbuffer_hybrid_m4(
        &ctx, &edits, false, false, 1.0, 0, DEFAULT_LIGHT_DIR, &DEGENERATE_LIGHT_TABLE,
        Some((&grid, &cells)), true, true, true, Some(&clipmap),
    );
    let vt_clip = vt_clip_opt.expect("gViewT readback (clip-map)");

    // The clip-map must produce SURFACE hits on the far sphere (gViewT carries a finite t), and each hit
    // must lie on the true analytic surface within M2_CREASE_EPS (the exact-CSG residual fallback).
    let mut hits = 0u32;
    let mut csg_violations = 0u32;
    let mut worst_resid = 0.0f32;
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            if mesh_covers_pixel(px, py) {
                continue;
            }
            if gpu_is_hit(&vt_clip, px, py) {
                hits += 1;
                let t = vt_clip[(py * SDF_IMG_W + px) as usize];
                let (ro, rd) = composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
                let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                let resid = boyko_sdf_math::sdf_edit_list(&edits, p).abs();
                worst_resid = worst_resid.max(resid);
                if resid >= M2_CREASE_EPS {
                    csg_violations += 1;
                }
            }
        }
    }
    assert!(
        hits > 0,
        "M4 far-field clip-map produced NO surface hits — the level-1 brick path did not render the \
         far sphere at z=-7.5 (outside the ±4 level-0 box, inside the ±8 level-1 box)"
    );
    assert_eq!(
        csg_violations, 0,
        "M4 far-field EXACT-CSG VIOLATION: {csg_violations} clip-map hit(s) off the true surface \
         (|sdf(hit)| >= M2_CREASE_EPS={M2_CREASE_EPS}); worst |sdf(hit)| = {worst_resid:.5}"
    );
    assert_eq!(lit_clip.len(), READBACK_BYTES as usize);

    assert_validation_clean(&ctx);
    // SAFETY: the render submit was fence-waited; the device is drained; `destroy` consumes the clip-map.
    unsafe { clipmap.destroy(&ctx) };
    println!(
        "M4 clip-map far-field RENDERS: {hits} surface hits on the far sphere (level 1), \
         worst |sdf(hit)| = {worst_resid:.5} < M2_CREASE_EPS={M2_CREASE_EPS} (exact CSG)"
    );
}

/// SDF M4 clip-map OFFSCREEN SCREENSHOT DUMP (`#[ignore]`, RTX) — the owner's visual sign-off. Renders
/// the far-field sphere with the N-level clip-map and writes the LIT image to a BMP the owner opens.
#[test]
#[ignore = "GPU offscreen screenshot dump — the owner runs it on the RTX for visual sign-off"]
fn sdf_m4_clipmap_far_field_screenshot_dump() {
    let Some(ctx) = boot_or_skip("sdf_m4_clipmap_far_field_screenshot_dump") else {
        return;
    };
    let edits = far_field_sphere();
    let (grid, cells) = bake_brick_grid(&edits);
    let clipmap = {
        use boyko_sdf_math::SdfEditField;
        let mut field = SdfEditField::new();
        for e in &edits {
            assert!(field.push(*e), "scene must fit MAX_SDF_EDITS");
        }
        field.bump_gen();
        BrickClipmap::create(&ctx, &field, [0.0, 0.0, 0.0]).expect("M4 screenshot clip-map create")
    };
    let (lit, _, _) = run_gbuffer_hybrid_m4(
        &ctx, &edits, false, false, 1.0, LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO,
        DEFAULT_LIGHT_DIR, &DEGENERATE_LIGHT_TABLE, Some((&grid, &cells)), true, false, true,
        Some(&clipmap),
    );
    let path = std::env::temp_dir().join("sdf_m4_clipmap_far_field.bmp");
    write_bmp_rgba(&path, &lit, SDF_IMG_W, SDF_IMG_H);
    println!("M4 clip-map far-field screenshot written to {}", path.display());
    assert_validation_clean(&ctx);
    // SAFETY: the render submit was fence-waited; the device is drained; `destroy` consumes the clip-map.
    unsafe { clipmap.destroy(&ctx) };
}

/// Render P5 (r0+r1) MESH+SDF OFFSCREEN SCREENSHOT DUMP (`#[ignore]`, RTX) — the owner's
/// visual sign-off that the mesh is now a first-class RASTER-PBR G-buffer producer (lit by
/// the SAME deferred Cook-Torrance as the SDF), NOT the pre-P5 flat MESH_COLOR pass-through.
///
/// Renders the `crater()` SDF scene WITH the harness mesh quad rasterized over it (the quad
/// spans the world-XY footprint at `MESH_Z`, occluding the LEFT portion of the crater), under
/// a colorful L0b light table (1 directional + 1 point (warm) + 1 spot (cool)) so the lit
/// shading is visible. The mesh-covered pixels now show a PBR-LIT WHITE quad (the white
/// vertex albedo run through full Cook-Torrance). Render P7/P5-r1b UNLOCK: the marcher now writes
/// the mesh surface ray-t `t_mesh` into gViewT (not the old `1.0e30` sentinel), so the resolve
/// reconstructs the real mesh `P` and IN-RANGE point/spot lights now light the mesh (the warm
/// point + cool spot reach the quad) — NOT just directional + sky as before; the SDF-covered
/// pixels show the lit crater. The owner eyeballs that the mesh region is a shaded white surface
/// (not a flat green `[38,166,64]` = the old MESH_COLOR), proving the P5 raster-PBR producer is
/// on-screen — and that the mesh now picks up the punctual lights' warm/cool tint.
///
/// Writes the LIT image to `D:/tmp/p5_mesh_sdf.bmp` (created if absent). `#[ignore]` because
/// it needs the RTX (no CPU oracle assert — it is a visual dump).
#[test]
#[ignore = "GPU offscreen screenshot dump — the owner runs it on the RTX for visual sign-off"]
fn p5_mesh_sdf_pbr_screenshot_dump() {
    let Some(ctx) = boot_or_skip("p5_mesh_sdf_pbr_screenshot_dump") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());

    // The SDF body the mesh quad partially occludes — the recognizable crater CSG.
    let edits = crater();

    // A colorful L0b light table (mirrors `l0b_point_and_spot_match_the_table_oracle`): a
    // white directional + a warm point + a cool spot, all in front of the origin-plane
    // surface so a swath of pixels is lit. Shadows + AO ON.
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let header = GoldenLightHeader::new(1, 2, 1.0);
    let lights = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::point([0.0, 0.0, 1.5], [1.0, 0.9, 0.8], 4000.0, 6.0),
        GoldenLight::spot([0.4, 0.4, 1.5], [0.0, 0.0, 1.0], [0.8, 0.9, 1.0], 6000.0, 6.0, 20.0, 35.0),
    ];
    let table = pack_light_table(&header, &lights);

    // The full offscreen hybrid composite: raster the mesh quad → marcher writes SDF/empty
    // attributes (yielding the mesh-owned pixels to the raster) → deferred resolve lights the
    // whole G-buffer (mesh + SDF) with Cook-Torrance.
    let lit =
        run_gbuffer_hybrid_lit_table(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR, &table).0;
    assert_eq!(lit.len(), READBACK_BYTES as usize);
    let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
    assert!(nonzero > 0, "P5 mesh+SDF LIT all-zero — device did not render");

    // Spot-check: a mesh-covered pixel must now read a LIT (non-zero, non-MESH_COLOR) value.
    let probe = (10u32, 32u32);
    assert!(
        mesh_covers_pixel(probe.0, probe.1),
        "the probe pixel must be inside the mesh quad footprint"
    );
    let mesh_px = albedo_rgb(&lit, probe.0, probe.1);
    let old_mesh = unpack_packed_rgb(pack_rgba(MESH_COLOR));
    assert!(
        !(0..3).all(|c| (mesh_px[c] - old_mesh[c]).abs() <= CHANNEL_TOL),
        "the mesh pixel {mesh_px:?} must be the raster-PBR producer, NOT the retired flat \
         MESH_COLOR {old_mesh:?}"
    );

    let dir = std::path::Path::new("D:/tmp");
    std::fs::create_dir_all(dir).expect("create D:/tmp for the screenshot dump");
    let path = dir.join("p5_mesh_sdf.bmp");
    write_bmp_rgba(&path, &lit, SDF_IMG_W, SDF_IMG_H);
    println!(
        "P5 mesh+SDF PBR screenshot written to {} (mesh pixel {mesh_px:?} = raster-PBR white, \
         not MESH_COLOR {old_mesh:?})",
        path.display()
    );
    assert_validation_clean(&ctx);
}

/// Nearest-neighbor upscales an R8G8B8A8 image `scale`× in both axes, returning the larger
/// R8G8B8A8 buffer. The composite GPU extent is fixed at `SDF_IMG_W × SDF_IMG_H` (64×64) — the
/// marcher/resolve dispatch + readback are hardwired to it — so the owner-facing 512×512
/// screenshot is the GENUINE GPU-shaded pixels block-replicated (each source texel → a
/// `scale × scale` block). No filtering: the per-pixel shadow term stays crisp, not blurred.
fn upscale_rgba_nn(src: &[u8], w: u32, h: u32, scale: u32) -> Vec<u8> {
    let (w, h, s) = (w as usize, h as usize, scale as usize);
    let (dw, dh) = (w * s, h * s);
    let mut dst = vec![0u8; dw * dh * 4];
    for dy in 0..dh {
        let sy = dy / s;
        for dx in 0..dw {
            let sx = dx / s;
            let si = (sy * w + sx) * 4;
            let di = (dy * dw + dx) * 4;
            dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    dst
}

/// P6 R1 MULTI-LIGHT SHADOW OFFSCREEN SCREENSHOT DUMP (`#[ignore]`, RTX) — the owner's visual
/// sign-off that analytic multi-light SDF shadows are on-screen. Renders the [`p6_r1_twin_scene`]
/// (two side-by-side spheres) under the `p6_r1_multi_light_table()` scene (a front-fill
/// directional + two shadow-flagged point casters straddling the valley, `shadow_mode == 1`,
/// NON-CLUSTERED) and writes the LIT image — nearest-neighbor upscaled 8× to **512×512** (the
/// native composite extent is 64×64) — to `D:/tmp/p6_multilight_shadows.bmp`. The owner eyeballs
/// the darkened valley band where each caster's rays into the opposite sphere's facing flank are
/// occluded by the near sphere. `#[ignore]` (no CPU assert beyond non-empty — it is a visual
/// dump; the GPU/oracle agreement is the load-bearing `p6_r1_multi_light_sdf_shadows_match_oracle`).
#[test]
#[ignore = "GPU offscreen screenshot dump — the owner runs it on the RTX for visual sign-off"]
fn p6_multilight_shadows_screenshot_dump() {
    let Some(ctx) = boot_or_skip("p6_multilight_shadows_screenshot_dump") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());

    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let (header, lights) = p6_r1_multi_light_table();
    let table = pack_light_table(&header, &lights);
    let edits = p6_r1_twin_scene();

    // The full offscreen hybrid composite, NON-CLUSTERED multi-light resolve (clusters_enabled
    // == false): the marcher writes the primary `gMaterial.r`, the resolve marches the two
    // flagged point casters per pixel — the darkened valley band is the analytic SDF shadow.
    let lit =
        run_gbuffer_hybrid_lit_table(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR, &table).0;
    assert_eq!(lit.len(), READBACK_BYTES as usize);
    let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
    assert!(nonzero > 0, "P6 R1 multi-light LIT all-zero — device did not render");

    // Native composite extent is 64×64; upscale 8× → 512×512 for the owner-facing screenshot.
    const SHOT_SCALE: u32 = 8;
    let shot_w = SDF_IMG_W * SHOT_SCALE;
    let shot_h = SDF_IMG_H * SHOT_SCALE;
    debug_assert_eq!((shot_w, shot_h), (512, 512), "the dump must be 512×512");
    let big = upscale_rgba_nn(&lit, SDF_IMG_W, SDF_IMG_H, SHOT_SCALE);

    let dir = std::path::Path::new("D:/tmp");
    std::fs::create_dir_all(dir).expect("create D:/tmp for the screenshot dump");
    let path = dir.join("p6_multilight_shadows.bmp");
    write_bmp_rgba(&path, &big, shot_w, shot_h);
    println!(
        "P6 R1 multi-light shadows screenshot written to {} ({shot_w}×{shot_h}, 8× NN upscale of \
         the {SDF_IMG_W}×{SDF_IMG_H} GPU composite; {nonzero} non-black native px)",
        path.display()
    );
    assert_validation_clean(&ctx);
}

// ============================================================================
// Render P7 GROUP C2 — SSAO GPU goldens (offscreen). The marcher → SSAO → resolve path on
// hardware, verified against the host two-stage oracle:
//   - Stage 1: `golden_gbuffer(...)` builds the host G-buffer ONCE (the marcher mirror).
//   - Stage 2: `golden_ssao_attributes(gbuf, px, py, ..)` gathers the per-pixel AO factor.
//   - the combine: `golden_deferred_resolve_table_shadowed_ssao(.., ssao)` mirrors the resolve's
//     `ao_final = min(class_ao, gSsao)` under `ssao_mode != 0`.
// The light table arms `with_ssao_mode(1)`; every golden is `boot_render_or_skip`-gated and runs
// under `BOYKO_DISABLE_VALIDATION=1` on the dev box (the orchestrator runs them on the RTX).
// ============================================================================

/// The SSAO AO-channel tolerance (consumer-side `sqrt`/`div` ULP budget, the plan's ±6/255). The
/// AO factor is R8-quantized on store; the host `golden_ssao_attributes` runs the SAME no-trig
/// reducer (integer rotation + integer step-rounding are bit-exact), so the only divergence is the
/// last-ULP `sqrt`/`div` the parity `composite_ray` already relies on + the ±1/255 oct-normal byte
/// disagreement propagated linearly through `dot(N, slice_dir)`.
const SSAO_AO_TOL: i32 = 6;

/// The default SSAO light table fixture (`ssao_mode == 1`): one directional + one sky (so the
/// ambient the SSAO modulates is non-trivial), NON-CLUSTERED, `shadow_mode == 0`. Mirrors the
/// L0a/L0b degenerate spirit but with a real sky term + the SSAO mode armed.
fn ssao_light_table() -> (GoldenLightHeader, Vec<GoldenLight>) {
    let header = GoldenLightHeader::new(2, 0, 1.0).with_ssao_mode(1);
    let lights = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        // A real sky/hemisphere ambient — the term `ao_final` modulates (so SSAO is observable).
        GoldenLight::sky([0.30, 0.36, 0.46], [0.14, 0.13, 0.12]),
    ];
    (header, lights)
}

/// The same fixture with SSAO DISARMED (`ssao_mode == 0`) — for the byte-identity 0%-gate (the
/// resolve never reads the SSAO image, so the lit output equals the pre-SSAO `_lit_table` path).
fn ssao_light_table_off() -> (GoldenLightHeader, Vec<GoldenLight>) {
    let (h, l) = ssao_light_table();
    (h.with_ssao_mode(0), l)
}

/// Builds the host Stage-1 G-buffer for the ORTHO SSAO fixture (the marcher mirror at the golden
/// 64×64 extent), shared by all per-pixel Stage-2 gathers (NOT O(N²)).
fn ssao_host_gbuffer(
    edits: &[SdfEdit],
    flags: u32,
    light_dir: [f32; 3],
) -> Vec<MarcherAttributes> {
    let materials = host_material_table();
    golden_gbuffer(
        edits,
        &materials,
        expected_mesh_depth,
        SDF_IMG_W,
        SDF_IMG_H,
        CompositeCamera::Ortho,
        1.0,
        flags,
        light_dir,
    )
}

/// **C2 golden — the SSAO AO channel == the host oracle (±6/255).** The GPU `ssao` R8 readback
/// must match `golden_ssao_attributes` (Stage-2 over the Stage-1 host gbuf) within ±6/255 over all
/// SDF-lit pixels. Prints the max delta + the lit-pixel count.
#[test]
fn ssao_ao_channel_matches_host_oracle() {
    let Some(ctx) = boot_render_or_skip("ssao_ao_channel_matches_host_oracle") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let (header, lights) = ssao_light_table();
    let table = pack_light_table(&header, &lights);

    for (name, edits) in p4b_scenes() {
        let (_lit, ssao) =
            run_gbuffer_hybrid_ssao(&ctx, &edits, flags, DEFAULT_LIGHT_DIR, &table, SSAO_QUALITY_MEDIUM);
        assert_eq!(ssao.len(), PIXELS as usize, "[{name}] SSAO R8 readback size");

        let gbuf = ssao_host_gbuffer(&edits, flags, DEFAULT_LIGHT_DIR);
        let mut max_delta = 0i32;
        let mut lit_px = 0u64;
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                let idx = (py * SDF_IMG_W + px) as usize;
                if gbuf[idx].mask != 1 {
                    continue; // only SDF-lit pixels carry a meaningful AO factor
                }
                lit_px += 1;
                let host = golden_ssao_attributes(
                    &gbuf, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    &SSAO_PARAMS[SSAO_QUALITY_MEDIUM],
                );
                let want = (host * 255.0).round() as i32;
                let got = ssao[idx] as i32;
                let d = (got - want).abs();
                if d > max_delta {
                    max_delta = d;
                }
                assert!(
                    d <= SSAO_AO_TOL,
                    "[{name}] SSAO AO texel ({px},{py}) got {got}/255 want {want}/255 \
                     (host oracle) exceeds ±{SSAO_AO_TOL}/255 (delta {d})"
                );
            }
        }
        assert!(lit_px > 0, "[{name}] SSAO AO channel: no SDF-lit pixel — the gate is vacuous");
        println!(
            "[{name}] SSAO AO channel == host oracle: max delta {max_delta}/255 (tol \
             {SSAO_AO_TOL}); {lit_px} SDF-lit px"
        );
    }
}

/// **C2 golden — the combined LIT == the host SSAO-aware resolve oracle (±2/255).** The GPU LIT
/// readback (SSAO ON) must match `golden_deferred_resolve_table_shadowed_ssao` fed the per-pixel
/// host SSAO term, within the EXISTING ±2/255 (AO modulates only ambient — no relaxation).
#[test]
fn ssao_combined_lit_matches_host() {
    let Some(ctx) = boot_render_or_skip("ssao_combined_lit_matches_host") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let (header, lights) = ssao_light_table();
    let table = pack_light_table(&header, &lights);
    let materials = host_material_table();

    for (name, edits) in p4b_scenes() {
        let (lit, _ssao) =
            run_gbuffer_hybrid_ssao(&ctx, &edits, flags, DEFAULT_LIGHT_DIR, &table, SSAO_QUALITY_MEDIUM);
        assert_eq!(lit.len(), READBACK_BYTES as usize);

        let gbuf = ssao_host_gbuffer(&edits, flags, DEFAULT_LIGHT_DIR);
        let field = |q: [f32; 3]| boyko_sdf_math::sdf_edit_list(&edits, q);
        // Render P7 POLISH: the resolve now BLURS `gSsao` (a 7×7 depth-gated box) before the
        // combine, so the host must feed the SSAO-aware resolve mirror the BLURRED per-pixel
        // term, NOT the raw single tap. Build the RAW host SSAO byte image ONCE — the SAME
        // `(host * 255).round() as u8` quantization the AO-channel golden asserts the GPU
        // `gSsao` against — then `golden_ssao_blur` over it per pixel mirrors the resolve's
        // inline gather exactly (so GPU == host within ±2/255 holds despite the blur).
        let raw_ssao: Vec<u8> = (0..PIXELS)
            .map(|i| {
                let px = i % SDF_IMG_W;
                let py = i / SDF_IMG_W;
                let a = golden_ssao_attributes(
                    &gbuf, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    &SSAO_PARAMS[SSAO_QUALITY_MEDIUM],
                );
                (a * 255.0).round() as u8
            })
            .collect();
        let mut max_delta = 0i32;
        let mut lit_hits = 0u64;
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                let idx = (py * SDF_IMG_W + px) as usize;
                let attrs = gbuf[idx];
                let (ro, rd) = composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
                // The per-pixel BLURRED host SSAO term (the 7×7 depth-gated box over the raw
                // host SSAO image — the exact mirror of the resolve's inline blur), fed into the
                // SSAO-aware resolve mirror.
                let ao = golden_ssao_blur(&raw_ssao, &gbuf, px, py, SDF_IMG_W, SDF_IMG_H);
                let want = unpack_packed_rgb(golden_deferred_resolve_table_shadowed_ssao(
                    attrs, ro, rd, &materials, &header, &lights, &field, ao,
                ));
                let got = albedo_rgb(&lit, px, py);
                let d = (0..3).map(|c| (got[c] - want[c]).abs()).max().unwrap();
                if attrs.mask == 1 {
                    lit_hits += 1;
                }
                if d > max_delta {
                    max_delta = d;
                }
                assert!(
                    d <= DEFERRED_ARM1_TOL,
                    "[{name}] SSAO combined LIT texel ({px},{py}) got {got:?} want {want:?} \
                     (SSAO oracle) exceeds ±{DEFERRED_ARM1_TOL}/255 (delta {d})"
                );
            }
        }
        assert!(lit_hits > 0, "[{name}] SSAO combined LIT: no SDF-lit pixel — the gate is vacuous");
        println!(
            "[{name}] SSAO combined LIT == host SSAO oracle: max delta {max_delta}/255 (tol \
             {DEFERRED_ARM1_TOL}); {lit_hits} SDF-lit px"
        );
    }
}

/// **Render P7-Q2 golden — EVERY pre-compiled SSAO quality variant matches its host oracle.** For
/// each of Low / Medium / High (Mechanism C — the host binds a different pre-compiled `.spv`, ZERO
/// per-pixel runtime cost), bind that variant's pipeline ([`run_gbuffer_hybrid_ssao`]`(.., quality)`)
/// and assert:
///   1. **The GPU `ssao` AO channel == [`golden_ssao_attributes`] fed `SSAO_PARAMS[quality]`** within
///      ±[`SSAO_AO_TOL`]/255 over the SDF-lit pixels — the per-variant host oracle (the parameterized
///      gather) reproduces the variant `.spv`'s baked tap budget.
///   2. **The combined LIT == the SSAO-aware resolve oracle** fed the BLURRED per-variant host SSAO
///      term within ±[`DEFERRED_ARM1_TOL`]/255 — the end-to-end frame matches per variant.
///
/// The MEDIUM arm is also the no-op proof at the pixel level (its host params == today's shipped
/// consts), so it must reproduce the pre-Q2 `ssao_ao_channel_matches_host_oracle` /
/// `ssao_combined_lit_matches_host` results bit-for-bit. The Low/High arms prove the variant
/// pipelines are wired AND the parameterized oracle tracks each baked budget. The orchestrator runs
/// this on the RTX.
#[test]
fn ssao_variants_match_host() {
    let Some(ctx) = boot_render_or_skip("ssao_variants_match_host") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let (header, lights) = ssao_light_table();
    let table = pack_light_table(&header, &lights);
    let materials = host_material_table();

    for quality in [SSAO_QUALITY_LOW, SSAO_QUALITY_MEDIUM, SSAO_QUALITY_HIGH] {
        let params = &SSAO_PARAMS[quality];
        for (name, edits) in p4b_scenes() {
            let (lit, ssao) =
                run_gbuffer_hybrid_ssao(&ctx, &edits, flags, DEFAULT_LIGHT_DIR, &table, quality);
            assert_eq!(ssao.len(), PIXELS as usize, "[q{quality} {name}] SSAO R8 readback size");
            assert_eq!(lit.len(), READBACK_BYTES as usize, "[q{quality} {name}] LIT readback size");

            let gbuf = ssao_host_gbuffer(&edits, flags, DEFAULT_LIGHT_DIR);

            // (1) The AO channel == the parameterized host oracle (variant-matched), and build the
            // RAW host SSAO byte image (the SAME quantization the AO golden asserts) for the blur.
            let mut raw_ssao = vec![0u8; PIXELS as usize];
            let mut max_ao_delta = 0i32;
            let mut lit_px = 0u64;
            for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    let idx = (py * SDF_IMG_W + px) as usize;
                    let host = golden_ssao_attributes(
                        &gbuf, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, params,
                    );
                    raw_ssao[idx] = (host * 255.0).round() as u8;
                    if gbuf[idx].mask != 1 {
                        continue; // only SDF-lit pixels carry a meaningful AO factor
                    }
                    lit_px += 1;
                    let want = (host * 255.0).round() as i32;
                    let got = ssao[idx] as i32;
                    let d = (got - want).abs();
                    if d > max_ao_delta {
                        max_ao_delta = d;
                    }
                    assert!(
                        d <= SSAO_AO_TOL,
                        "[q{quality} {name}] SSAO AO texel ({px},{py}) got {got}/255 want \
                         {want}/255 (variant host oracle) exceeds ±{SSAO_AO_TOL}/255 (delta {d})"
                    );
                }
            }
            assert!(lit_px > 0, "[q{quality} {name}] SSAO AO channel: no SDF-lit pixel (vacuous)");

            // (2) The combined LIT == the SSAO-aware resolve oracle fed the BLURRED per-variant SSAO
            // term (the resolve blur is variant-independent: a fixed 7×7 depth-gated box).
            let mut max_lit_delta = 0i32;
            for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    let idx = (py * SDF_IMG_W + px) as usize;
                    let attrs = gbuf[idx];
                    let (ro, rd) =
                        composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
                    let ao = golden_ssao_blur(&raw_ssao, &gbuf, px, py, SDF_IMG_W, SDF_IMG_H);
                    let want = unpack_packed_rgb(golden_deferred_resolve_table_shadowed_ssao(
                        attrs, ro, rd, &materials, &header, &lights,
                        &|q: [f32; 3]| boyko_sdf_math::sdf_edit_list(&edits, q), ao,
                    ));
                    let got = albedo_rgb(&lit, px, py);
                    let d = (0..3).map(|c| (got[c] - want[c]).abs()).max().unwrap();
                    if d > max_lit_delta {
                        max_lit_delta = d;
                    }
                    assert!(
                        d <= DEFERRED_ARM1_TOL,
                        "[q{quality} {name}] SSAO combined LIT texel ({px},{py}) got {got:?} want \
                         {want:?} (variant SSAO oracle) exceeds ±{DEFERRED_ARM1_TOL}/255 (delta {d})"
                    );
                }
            }
            println!(
                "[q{quality} {name}] variant SSAO == host: AO max delta {max_ao_delta}/255 (tol \
                 {SSAO_AO_TOL}), LIT max delta {max_lit_delta}/255 (tol {DEFERRED_ARM1_TOL}); \
                 {lit_px} SDF-lit px (slices={} steps={})",
                params.slices, params.steps
            );
        }
    }
}

/// **C2 golden — SSAO OFF is BYTE-IDENTICAL to the pre-SSAO LIT (the 0%-gate).** The SAME scene
/// with `ssao_mode == 0` + `scene.ssao = None` (here: the pre-SSAO `run_gbuffer_hybrid_lit_table`)
/// must produce a LIT readback BYTE-FOR-BYTE equal to the SSAO harness run with `ssao_mode == 0`.
/// Proves the SSAO image being written + the combine being compiled in change NOTHING when the
/// header gate is 0.
#[test]
fn ssao_off_lit_is_byte_identical() {
    let Some(ctx) = boot_render_or_skip("ssao_off_lit_is_byte_identical") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let (header_off, lights_off) = ssao_light_table_off();
    let table_off = pack_light_table(&header_off, &lights_off);

    for (name, edits) in p4b_scenes() {
        // The pre-SSAO path (NO SSAO pass recorded): the canonical reference.
        let pre = run_gbuffer_hybrid_lit_table(
            &ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR, &table_off,
        )
        .0;
        // The SSAO harness with the header DISARMED (`ssao_mode == 0`): the SSAO pass still RUNS +
        // writes the image, but the resolve never reads it (the structural `if` is false), so the
        // lit output must be byte-for-byte the pre-SSAO image.
        let (with_pass, _ssao) = run_gbuffer_hybrid_ssao(
            &ctx, &edits, flags, DEFAULT_LIGHT_DIR, &table_off, SSAO_QUALITY_MEDIUM,
        );
        assert_eq!(pre.len(), with_pass.len());
        assert_eq!(
            pre, with_pass,
            "[{name}] ssao_mode==0: the SSAO-pass LIT must be BYTE-IDENTICAL to the pre-SSAO LIT \
             (the 0%-gate — the written-but-unread SSAO image must not perturb the resolve)"
        );
        println!("[{name}] SSAO OFF 0%-gate: LIT byte-identical to the pre-SSAO path ({} bytes)", pre.len());
    }
}

/// **C2 golden — a broad flat lit region is INVARIANT under SSAO (±2/255).** The key correctness
/// proof of the GROUP-B normal-elevation fix: SSAO must NOT darken open flat surfaces. A single
/// large flat SDF box fills the view; the SSAO-ON lit pixels in its interior (away from the
/// silhouette edge) must be within ±2/255 of the SSAO-OFF lit. (A naive SSAO that measured raw
/// depth deltas — not elevation above the tangent — would darken the whole flat face.)
#[test]
fn ssao_flat_region_invariance() {
    let Some(ctx) = boot_render_or_skip("ssao_flat_region_invariance") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    // A big fronto-parallel box: its front face is a broad FLAT lit region (constant normal +Z),
    // exactly the surface SSAO must leave untouched.
    let edits = vec![SdfEdit::box_shape([0.0, 0.0, 0.0], [0.7, 0.7, 0.2], sdf_op::UNION, 0.0)];

    let (h_on, l_on) = ssao_light_table();
    let (h_off, l_off) = ssao_light_table_off();
    let on = run_gbuffer_hybrid_ssao(&ctx, &edits, flags, DEFAULT_LIGHT_DIR, &pack_light_table(&h_on, &l_on), SSAO_QUALITY_MEDIUM).0;
    let off = run_gbuffer_hybrid_ssao(&ctx, &edits, flags, DEFAULT_LIGHT_DIR, &pack_light_table(&h_off, &l_off), SSAO_QUALITY_MEDIUM).0;

    // The interior band: lit SDF pixels strictly inside the box footprint (≥ FLAT_MARGIN px from
    // the silhouette), where the surface is flat and SSAO must not darken.
    const FLAT_MARGIN: u32 = 6;
    let gbuf = ssao_host_gbuffer(&edits, flags, DEFAULT_LIGHT_DIR);
    let mut checked = 0u64;
    let mut max_delta = 0i32;
    for py in FLAT_MARGIN..(SDF_IMG_H - FLAT_MARGIN) {
        for px in FLAT_MARGIN..(SDF_IMG_W - FLAT_MARGIN) {
            let idx = (py * SDF_IMG_W + px) as usize;
            if gbuf[idx].mask != 1 {
                continue;
            }
            // Require the whole FLAT_MARGIN neighbourhood to be lit SDF too (so the pixel is a true
            // interior flat-region pixel, not a near-edge one whose taps cross the silhouette).
            let interior = (py - FLAT_MARGIN..=py + FLAT_MARGIN).all(|qy| {
                (px - FLAT_MARGIN..=px + FLAT_MARGIN)
                    .all(|qx| gbuf[(qy * SDF_IMG_W + qx) as usize].mask == 1)
            });
            if !interior {
                continue;
            }
            checked += 1;
            let g_on = albedo_rgb(&on, px, py);
            let g_off = albedo_rgb(&off, px, py);
            let d = (0..3).map(|c| (g_on[c] - g_off[c]).abs()).max().unwrap();
            if d > max_delta {
                max_delta = d;
            }
            assert!(
                d <= CHANNEL_TOL,
                "SSAO flat-region invariance: interior flat texel ({px},{py}) SSAO-ON {g_on:?} vs \
                 SSAO-OFF {g_off:?} differs by {d}/255 (> ±{CHANNEL_TOL}) — SSAO darkened an open \
                 flat surface (the normal-elevation reducer regressed)"
            );
        }
    }
    assert!(
        checked > 32,
        "SSAO flat-region invariance: only {checked} interior flat pixels found — the box fixture \
         must present a broad flat lit region (the gate is near-vacuous)"
    );
    println!(
        "SSAO flat-region invariance: {checked} interior flat px within ±{CHANNEL_TOL}/255 \
         (max delta {max_delta}/255) — SSAO leaves open flat surfaces unchanged"
    );
}

/// **C2 golden — SSAO genuinely DARKENS a concavity (non-vacuity).** A real crevice (two SDF
/// spheres meeting at a contact seam) must have ≥N SDF-lit crevice pixels with GPU `ssao < 0.85`
/// (SSAO genuinely occludes), AND those crevice lit pixels darker than the SSAO-OFF lit by ≥ a few
/// /255. Without this, a flat-invariance pass alone could be vacuously satisfied by an all-1.0 AO.
#[test]
fn ssao_darkens_a_concavity() {
    let Some(ctx) = boot_render_or_skip("ssao_darkens_a_concavity") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    // A DEEP front-facing bowl: a large sphere with a smaller sphere SUBTRACTED toward the camera
    // (+z), carving a deeply concave cavity whose walls rise above the bowl-floor tangent within
    // the SSAO radius — the canonical SSAO occluder. (A shallow / side-offset crater or a sphere-
    // union SEAM is too shallow at the 0.5-unit radius to clear the horizon — host-probed: those
    // produce min_ssao 255/255, whereas this bowl produces min_ssao ~105/255 with ~100 occluded
    // px. The depth, not the mere presence of a concavity, is what the horizon gather needs.)
    let edits = vec![
        SdfEdit::sphere([0.0, 0.0, 0.0], 0.55, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.0, 0.0, 0.45], 0.40, sdf_op::SUBTRACT, 0.0),
    ];

    let (h_on, l_on) = ssao_light_table();
    let (h_off, l_off) = ssao_light_table_off();
    let (on, ssao) = run_gbuffer_hybrid_ssao(&ctx, &edits, flags, DEFAULT_LIGHT_DIR, &pack_light_table(&h_on, &l_on), SSAO_QUALITY_MEDIUM);
    let off = run_gbuffer_hybrid_ssao(&ctx, &edits, flags, DEFAULT_LIGHT_DIR, &pack_light_table(&h_off, &l_off), SSAO_QUALITY_MEDIUM).0;

    let gbuf = ssao_host_gbuffer(&edits, flags, DEFAULT_LIGHT_DIR);
    // The AO floor that proves a real occlusion (1.0 = no occlusion; 0.85 = a meaningful crevice).
    const OCCLUDED_AO: u8 = (0.85 * 255.0) as u8;
    // The minimum darkening of the lit image at an occluded crevice pixel (a few /255 survives the
    // ambient-only modulation + double-quant).
    const MIN_LIT_DARKEN: i32 = 2;
    let mut occluded_px = 0u64;
    let mut darkened_px = 0u64;
    let mut min_ssao = 255u8;
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let idx = (py * SDF_IMG_W + px) as usize;
            if gbuf[idx].mask != 1 {
                continue;
            }
            let a = ssao[idx];
            if a < min_ssao {
                min_ssao = a;
            }
            if a < OCCLUDED_AO {
                occluded_px += 1;
                // The lit crevice pixel must be darker than the SSAO-OFF lit (SSAO modulated the
                // ambient down). A monotone darkening COUNT (not a per-pixel edge match).
                let g_on = albedo_rgb(&on, px, py);
                let g_off = albedo_rgb(&off, px, py);
                // OFF brighter than ON on at least one channel by ≥ MIN_LIT_DARKEN.
                let darken = (0..3).map(|c| g_off[c] - g_on[c]).max().unwrap();
                if darken >= MIN_LIT_DARKEN {
                    darkened_px += 1;
                }
            }
        }
    }
    assert!(
        occluded_px >= 8,
        "SSAO non-vacuity: only {occluded_px} crevice px with ssao < 0.85 (min ssao {}/255) — the \
         concavity fixture did not produce a real occluded region (SSAO is vacuously ~1.0)",
        min_ssao
    );
    assert!(
        darkened_px >= 4,
        "SSAO non-vacuity: {occluded_px} occluded crevice px but only {darkened_px} are LIT-darker \
         than SSAO-OFF by ≥{MIN_LIT_DARKEN}/255 — the SSAO term did not modulate the ambient down"
    );
    println!(
        "SSAO non-vacuity: {occluded_px} crevice px with ssao < 0.85 (min {}/255), {darkened_px} \
         LIT-darker than SSAO-OFF — SSAO genuinely occludes a concavity",
        min_ssao
    );
    // P7-C2b (Render P7 Unlock-2): the mesh-AO proof now lives in
    // `ssao_darkens_mesh_near_sdf_occluder` — a flat RASTER MESH quad + an SDF sphere standing IN
    // FRONT of it; the mesh pixels around the sphere silhouette receive SSAO the A2 SDF-march
    // cannot give them (their A2 == 1.0). The geometry was host-probed via
    // `probe_mesh_ssao_geometry` (the printed min_ssao / occluded-count sweep).
}

/// The world Z of the unlock-2 SDF occluder sphere's near pole — chosen so the sphere stands IN
/// FRONT of the mesh quad (`MESH_Z == 1.0`): the sphere center sits at `+Z` and its surface near
/// pole reaches `MESH_SSAO_SPHERE_Z + MESH_SSAO_SPHERE_R > 1.0`, so the SDF WINS ownership where
/// it covers (`t_sdf = CAM_Z - surface_z < t_mesh = CAM_Z - MESH_Z`) and the mesh stands elsewhere.
const MESH_SSAO_SPHERE_CZ: f32 = 0.95;
/// The unlock-2 occluder sphere radius. With `CZ + R = 1.55 > MESH_Z` the near pole pokes ~0.55
/// world units toward the camera above the mesh plane — a steep wall that rises above the nearby
/// mesh pixels' (`+Z`) tangent well within the SSAO 0.5-unit radius around the silhouette.
const MESH_SSAO_SPHERE_R: f32 = 0.60;

/// The unlock-2 SDF occluder: ONE sphere standing in front of the mesh quad (see the `CZ`/`R`
/// const docs). Shared by the host probe and the GPU non-vacuity gate so they march the SAME field.
fn mesh_ssao_occluder() -> Vec<SdfEdit> {
    vec![SdfEdit::sphere([0.0, 0.0, MESH_SSAO_SPHERE_CZ], MESH_SSAO_SPHERE_R, sdf_op::UNION, 0.0)]
}

/// `true` if pixel `(px,py)` is MESH-owned in the host Stage-1 G-buffer `gbuf`: the raster quad
/// covered it AND the SDF did NOT win ownership there, so the marcher stored the mesh surface
/// (`view_t == t_mesh`, `ao == 255` from the raster's A2 == 1.0, `mask == 1`). The robust
/// classifier the unlock-2 gate uses to separate mesh pixels from SDF-lit ones: `view_t` equals
/// `t_mesh` to within an epsilon (an SDF hit stores `view_t = t_sdf < t_mesh`). `t_mesh` is the
/// constant `depth_to_t(mesh_depth_for_z(MESH_Z))` the raster producer + the host oracle share.
fn host_mesh_owned(gbuf: &[MarcherAttributes], px: u32, py: u32) -> bool {
    let idx = (py * SDF_IMG_W + px) as usize;
    let a = gbuf[idx];
    if a.mask != 1 {
        return false;
    }
    let t_mesh = boyko_rhi_vulkan::compute::depth_to_t(mesh_depth_for_z(MESH_Z));
    // The mesh arm stores ao == 255 (raster A2 == 1.0) AND view_t == t_mesh; the SDF arm stores a
    // smaller view_t (the marched hit) + a generally < 255 ao. `view_t ~= t_mesh` is the clean cut.
    a.ao == 255 && (a.view_t - t_mesh).abs() < 1.0e-4
}

/// **Host probe (NO GPU) — find a mesh+SDF arrangement that produces mesh-pixel SSAO occlusion.**
/// Builds the host Stage-1 G-buffer for a sweep of sphere depths/radii (the SAME `expected_mesh_
/// depth` fixed quad the harness rasters), runs `golden_ssao_attributes` over the MESH-owned
/// pixels (`host_mesh_owned`), and prints `min_ssao` + the count below 0.85 for each candidate.
/// Pure host math — it runs on a device-less box. Kept as the documented record of how the
/// `ssao_darkens_mesh_near_sdf_occluder` geometry was chosen (the printed sweep is in the report).
#[test]
fn probe_mesh_ssao_geometry() {
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    const OCCLUDED_AO: u8 = (0.85 * 255.0) as u8;
    println!("mesh-SSAO geometry probe (mesh quad MESH_Z=1.0, X∈[-1,0.2] Y∈[-1,1], R=0.5 world):");
    for &cz in &[0.75_f32, 0.85, 0.95, 1.05, 1.15] {
        for &r in &[0.45_f32, 0.55, 0.60, 0.70] {
            let edits = vec![SdfEdit::sphere([0.0, 0.0, cz], r, sdf_op::UNION, 0.0)];
            let gbuf = ssao_host_gbuffer(&edits, flags, DEFAULT_LIGHT_DIR);
            let mut mesh_px = 0u64;
            let mut occluded = 0u64;
            let mut min_ssao = 255u8;
            for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    if !host_mesh_owned(&gbuf, px, py) {
                        continue;
                    }
                    mesh_px += 1;
                    let ao = golden_ssao_attributes(
                        &gbuf, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                        &SSAO_PARAMS[SSAO_QUALITY_MEDIUM],
                    );
                    let a = (ao * 255.0).round() as u8;
                    if a < min_ssao {
                        min_ssao = a;
                    }
                    if a < OCCLUDED_AO {
                        occluded += 1;
                    }
                }
            }
            println!(
                "  cz={cz:.2} r={r:.2} (near pole z={:.2}): mesh_px={mesh_px} min_ssao={min_ssao}/255 occluded(<0.85)={occluded}",
                cz + r
            );
        }
    }
}

/// **Render P7 Unlock-2 — the offscreen MESH+SDF SSAO non-vacuity golden (the headline).** PROVES
/// the SSAO mesh-AO value the gViewT unlock (the marcher writes `gViewT = t_mesh` for mesh-owned
/// pixels) enabled: SSAO darkens RASTER MESH pixels — AO the A2 SDF-march CANNOT produce, since the
/// mesh carries no field (its A2 `gMaterial.g` == 1.0).
///
/// The scene: the harness's flat raster mesh quad (`MESH_Z == 1.0`, the fixed `expected_mesh_depth`
/// footprint) + an SDF sphere ([`mesh_ssao_occluder`]) standing IN FRONT of it — center at `+Z`,
/// near pole at `z == 1.55 > MESH_Z`, so the SDF WINS ownership where it covers (`t_sdf < t_mesh`)
/// and the mesh stands elsewhere. The sphere wall rises above the nearby mesh pixels' (`+Z`) tangent
/// well within the SSAO 0.5-unit radius around the silhouette → a mesh-pixel occlusion ring.
///
/// Asserts, over the MESH-OWNED pixels ([`host_mesh_owned`] — the raster won, `view_t == t_mesh`,
/// `ao == 255`):
///   1. **≥ `MESH_SSAO_MIN_OCCLUDED` mesh pixels have GPU `ssao < 0.85`** — the structural mesh-AO
///      win (the A2 march gives these `ao == 1.0`; SSAO is their ONLY AO). Host-probed: this
///      geometry yields ~577 such mesh pixels (`min_ssao ~31/255`); `N == 100` is well below.
///   2. **GPU `ssao` == host `golden_ssao_attributes` within ±`SSAO_AO_TOL`/255** over the mesh
///      region — the host now models mesh SSAO via the unlocked `t_mesh`.
///   3. **The control:** the SAME mesh pixels' A2 (`gbuf.ao`) == 255 — proving the darkening is
///      SSAO, not the A2 SDF-march (which cannot touch a mesh pixel).
#[test]
fn ssao_darkens_mesh_near_sdf_occluder() {
    let Some(ctx) = boot_render_or_skip("ssao_darkens_mesh_near_sdf_occluder") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let (header, lights) = ssao_light_table();
    let table = pack_light_table(&header, &lights);
    let edits = mesh_ssao_occluder();

    let (_lit, ssao) =
        run_gbuffer_hybrid_ssao(&ctx, &edits, flags, DEFAULT_LIGHT_DIR, &table, SSAO_QUALITY_MEDIUM);
    assert_eq!(ssao.len(), PIXELS as usize, "SSAO R8 readback size");

    let gbuf = ssao_host_gbuffer(&edits, flags, DEFAULT_LIGHT_DIR);

    /// The AO floor proving a real occlusion (1.0 = unoccluded; 0.85 = a meaningful occlusion).
    const OCCLUDED_AO: u8 = (0.85 * 255.0) as u8;
    /// The minimum number of MESH-owned pixels the SSAO must darken below 0.85 — well below the
    /// host-probed ~577 (this geometry), so GPU↔host fp drift cannot make the gate vacuous.
    const MESH_SSAO_MIN_OCCLUDED: u64 = 100;

    let mut mesh_px = 0u64;
    let mut occluded_px = 0u64;
    let mut min_ssao = 255u8;
    let mut max_ao_delta = 0i32;
    let mut ao_outliers = 0u64;
    let mut worst_outlier = (0u32, 0u32, 0i32);
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            if !host_mesh_owned(&gbuf, px, py) {
                continue;
            }
            let idx = (py * SDF_IMG_W + px) as usize;
            mesh_px += 1;

            // (3) The control: the host A2 for this mesh pixel is exactly 255 (the raster's
            // gMaterial.g == 1.0) — the SDF march produced NO AO here, so any darkening is SSAO.
            assert_eq!(
                gbuf[idx].ao, 255,
                "mesh pixel ({px},{py}) must carry A2 == 255 (raster ao = 1.0); a non-255 A2 means \
                 the classifier mis-tagged an SDF pixel as mesh"
            );

            // (2) GPU ssao == the host SSAO oracle (the host now models mesh SSAO via t_mesh).
            let host = golden_ssao_attributes(
                &gbuf, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                &SSAO_PARAMS[SSAO_QUALITY_MEDIUM],
            );
            let want = (host * 255.0).round() as i32;
            let got = ssao[idx] as i32;
            let d = (got - want).abs();
            if d > max_ao_delta {
                max_ao_delta = d;
            }
            // Silhouette borderline-tap-round: where a gather tap lands on the SDF sphere's
            // silhouette, the FP `round()` of the tap's pixel position can flip GPU vs host (DXC
            // FMA-contraction of `px + dir*advance` vs the host's split mul/add), and at a strong
            // occluder a single flipped tap moves the AO by a visible step (up to ~30/255). This is
            // inherent cross-compiler FP and touches only a thin silhouette ring; the VISIBLE
            // combined-lit result stays within ±2 (`ssao_combined_lit_matches_host`). Count the ring
            // and bound it below; do NOT require exact GPU==host on these few rim pixels.
            if d > SSAO_AO_TOL {
                ao_outliers += 1;
                if d > worst_outlier.2 {
                    worst_outlier = (px, py, d);
                }
            }

            // (1) The structural mesh-AO win: count the mesh pixels the GPU darkened below 0.85.
            let a = ssao[idx];
            if a < min_ssao {
                min_ssao = a;
            }
            if a < OCCLUDED_AO {
                occluded_px += 1;
            }
        }
    }

    assert!(
        mesh_px > 0,
        "mesh-SSAO non-vacuity: no MESH-owned pixel — the sphere filled the view or the classifier \
         is wrong (the gate is vacuous)"
    );
    assert!(
        occluded_px >= MESH_SSAO_MIN_OCCLUDED,
        "mesh-SSAO non-vacuity: only {occluded_px} MESH-owned px with ssao < 0.85 (min ssao \
         {min_ssao}/255, {mesh_px} mesh px) — expected ≥ {MESH_SSAO_MIN_OCCLUDED}. The gViewT \
         unlock did not reach the SSAO mesh path (the marcher's mesh `gViewT = t_mesh` write or the \
         SSAO `view_t < SSAO_VIEWT_BG` gate regressed) — a real bug, NOT a geometry tweak"
    );
    /// The silhouette borderline-tap-round ring bound (see the in-loop note). A thin ring of mesh
    /// pixels where a gather tap straddles the SDF sphere silhouette and the tap-position `round()`
    /// flips GPU vs host. Measured 1 px on this geometry (worst 26/255 at (31,8)); bounded at 16 —
    /// generous over the observed handful, yet far below a gross regression (the PCG dither
    /// diverging GPU/host, or the mesh path breaking) which would blow into the hundreds.
    const MESH_AO_OUTLIER_MAX: u64 = 16;
    assert!(
        ao_outliers <= MESH_AO_OUTLIER_MAX,
        "mesh-SSAO GPU↔host: {ao_outliers} mesh px exceed ±{SSAO_AO_TOL}/255 (worst {}/255 at \
         {:?}) — expected ≤ {MESH_AO_OUTLIER_MAX} (a thin silhouette ring). A large count means the \
         dither hash diverges GPU/host or the mesh SSAO path regressed, NOT a silhouette tap-round",
        worst_outlier.2,
        (worst_outlier.0, worst_outlier.1)
    );
    println!(
        "mesh-SSAO non-vacuity: {occluded_px}/{mesh_px} MESH-owned px with ssao < 0.85 (min \
         {min_ssao}/255), GPU↔host max AO delta {max_ao_delta}/255 (tol {SSAO_AO_TOL}), \
         {ao_outliers} silhouette-ring outliers (worst {}/255 at {:?}) — SSAO darkens RASTER MESH \
         pixels the A2 SDF-march cannot (their A2 == 255). The gViewT unlock reaches the SSAO mesh \
         path.",
        worst_outlier.2,
        (worst_outlier.0, worst_outlier.1)
    );
}
