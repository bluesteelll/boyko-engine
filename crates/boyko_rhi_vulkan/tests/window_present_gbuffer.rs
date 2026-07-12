//! Render **P1c GPU gate** (scaffold) — the FIRST IMAGE-BASED HYBRID FRAME ON SCREEN.
//! The P1b shared-depth marcher (a real GPU-rasterized mesh's depth written into a
//! D32_SFLOAT IMAGE, SAMPLED directly by the SDF compute marcher, which STORES the
//! FINAL composite into an R8G8B8A8 ALBEDO storage image) is driven ON SCREEN and the
//! ALBEDO is present-blit (fullscreen-sample) into the windowed swapchain image, with
//! the validation layer as the soundness oracle. On ONE presented frame — before
//! present — the swapchain image is copied back into a host-visible staging buffer and
//! golden-asserted against the host composite truth ([`golden_composite_pixel`]).
//!
//! # The P1c graduation: image-based, NO depth→buffer copy
//!
//! This is the on-screen counterpart of the P1b OFFSCREEN driver
//! (`tests/sdf_gbuffer_hybrid.rs::run_gbuffer_hybrid`). Where the packed on-screen path
//! (`window_present_hybrid.rs`) copies the rasterized depth into a shared buffer
//! (`copy_image_to_buffer(depth)`) and the marcher reads it as a packed buffer, the P1c
//! path SAMPLES the depth IMAGE and STORES the composite into the ALBEDO IMAGE — the
//! per-frame depth→buffer copy is GONE. The single depth
//! `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` barrier replaces the packed
//! path's depth copy + its two transfer barriers; the descriptor sets are written ONCE
//! at [`Renderer::render_gbuffer_frame`]'s first sync (NO per-frame
//! `vkUpdateDescriptorSets`).
//!
//! Determinism (INVIOLABLE): the marcher (field eval + ray-gen + lighting + the albedo
//! composite) is BYTE-UNTOUCHED from P1b (the verbatim `sdf_gbuffer_composite_spirv`),
//! so the on-screen image-composite golden equals the packed on-screen composite within
//! `+/-2/255` per channel — the SAME host golden the packed path uses.
//!
//! # 1:1 top-left present (WSI-clamp safe)
//!
//! The composite is rendered at its NATIVE 64×64 size; [`Renderer::render_gbuffer_frame`]
//! present-blits it 1:1 in the swapchain image's TOP-LEFT (it clamps the present
//! viewport/scissor to `min(swapchain_extent, present_extent)`), so the per-texel golden
//! is exact regardless of a WSI `current_extent` clamp, as long as the swapchain is at
//! least 64×64 (the top-left sub-rect fits). The G-buffer + marcher dispatch are sized
//! to the 64×64 composite, NOT the (possibly wider) swapchain extent.
//!
//! # The discriminator texels (picked host-side, BEFORE any GPU run)
//!
//! The same three rung-10/P1b regions: a mesh-occludes-SDF texel (`MESH_COLOR`), an SDF
//! lit texel, and a background texel — each asserted color-close to
//! [`golden_composite_pixel`] within `+/-2/255`, accounting for the swapchain being
//! `B8G8R8A8` (the readback bytes are then BGRA; the golden is RGBA byte order).
//!
//! # SCAFFOLD STATUS — the GPU run is the TESTER's
//!
//! This file compiles + [`Renderer::render_gbuffer_frame`] records the full P1c stream,
//! but the golden GPU assertion is gated behind a graceful boot/WSI/format SKIP and a
//! `#[cfg(windows)]` (it needs a real RTX-3060 windowed device). The tester: run it on
//! the GPU, confirm the presented swapchain image matches the rung-10/P1b hybrid golden
//! within `+/-2/255`, confirm — by recording inspection — that NO
//! `copy_image_to_buffer(depth)` and NO per-frame `vkUpdateDescriptorSets` are in the
//! stream, and confirm validation + sync-validation are clean.

#![cfg(windows)]

mod common;
use common::*;

use core::ptr::NonNull;

use boyko_rhi::enums::{AddressMode, DescriptorKind, Filter, IndexType};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferUsage, CompareOp, ComputePipelineDesc, DepthBias, Format, CullMode, GraphicsPipelineDesc,
    ImageUsage, MemoryLocation, MipMode,
    PrimitiveTopology, QueryPoolDesc, RhiDevice, SamplerDesc, ShaderStage, TextureDesc,
    TextureDimension, VertexAttribute, VertexBufferLayout, VertexFormat,
};
use boyko_rhi_vulkan::compute::{B5_CAMERA_UBO_BYTES_M4, B5_CAMERA_UBO_BYTES_MESH_SDF, COMPOSITE_PUSH_CONSTANT_BYTES, CoarseMode, CompositePushConstants, csm_depth_fs_spirv, csm_depth_vs_spirv, punctual_depth_fs_spirv, punctual_depth_vs_spirv, EDITLIST_BUFFER_WORDS, GOLDEN_LIGHT_HEADER_BASE_WORDS, GOLDEN_LIGHT_KIND_DIRECTIONAL, GOLDEN_LIGHT_KIND_POINT, GOLDEN_LIGHT_KIND_SPOT, INTERP_INSTANCES_PUSH_BYTES, interp_instances_spirv, LOCAL_SIZE_X, M2_GRID_PARAMS_OFFSET, MESH_SDF_PARAMS_OFFSET, MeshSdfParams, MESH_DEPTH_CLEAR, SDF_CAMERA_Z, SDF_TRACE_T_MAX, SDF_VIEW_HALF_EXTENT, SdfEdit, TILE_BOUND_BYTES, CompositeCamera, encode_edit_list, deferred_pbr_spirv, composite_pixel_ray, DEFAULT_MARCHER_OMEGA, LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS, DEFAULT_LIGHT_DIR, mesh_depth_for_z, sdf_gbuffer_composite_spirv, sdf_op, sdf_ssao_spirv_variant, sdf_tile_cull_spirv, tile_grid_extent, SSAO_QUALITY_LOW, SSAO_QUALITY_MEDIUM, SSAO_QUALITY_HIGH};
use boyko_rhi_vulkan::goldens::{GoldenLight, GoldenLightHeader, golden_composite_pixel_ex, golden_deferred_resolve, golden_marcher_attributes, GoldenMaterial};
use boyko_rhi_vulkan::mesh_sdf_texture::MeshSdfTexture;
use boyko_sdf_math::mesh_sdf::{BakeMesh, MeshSdfField};
use boyko_rhi_vulkan::brick_atlas::BrickClipmap;
use boyko_rhi_vulkan::compute::sdf_probe_update_spirv;
use boyko_rhi_vulkan::ddgi::{
    DDGI_GRID_DIM_X, DDGI_GRID_DIM_Y, DDGI_GRID_DIM_Z, DDGI_PROBE_COUNT, DdgiAtlas,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::rhi_impl::{
    ComputePipeline, VulkanBindGroup, VulkanBindGroupLayout, VulkanGraphicsPipeline,
    VulkanQueryPool, VulkanSampler,
};
use boyko_rhi_vulkan::texture::{MAX_TEXTURE_LAYERS, VulkanTexture};
use boyko_rhi_vulkan::ffi::{
    VK_FORMAT_B8G8R8A8_UNORM, VK_FORMAT_R8G8B8A8_UNORM, VkExtent2D,
};
use boyko_rhi_vulkan::swapchain::{
    BrickActivation, CsmDepthActivation, DdgiUpdateActivation, FRAMES_IN_FLIGHT, FrameWriteToken,
    GBUFFER_IDENTITY_INSTANCE, GBUFFER_INSTANCE_MODEL_BYTES, GBUFFER_PUSH_BYTES, GBufferFrame,
    GBufferMeshDraw, GBufferScene, InterpActivation, PASS_COUNT, PunctualDepthActivation, Renderer,
    SsaoActivation, Surface, Swapchain, TimestampCollector,
};
use boyko_rhi_vulkan::window::{CapturedMsg, Window};

use boyko_sdf_math::brick::{BRICK_LEVELS, PointerGrid};

/// The composite extent — the size the G-buffer is allocated at, the marcher dispatches
/// at, and the depth-prepass rasterizes at, present-blit 1:1 into the swapchain's top-left.
///
/// DECOUPLED from the frozen `SDF_IMG_W`/`SDF_IMG_H` (64×64): the ORTHO camera maps
/// `u, v ∈ [-1, 1]` → world `[-SDF_VIEW_HALF_EXTENT, +SDF_VIEW_HALF_EXTENT]` *regardless of
/// resolution*, so a larger extent keeps the SAME framing (the r=0.5 sphere stays centered,
/// occupying the central ~half of the view, with the mesh quad over the left part) and only
/// raises the sample density. The golden is recomputed at this extent via the extent-aware
/// `golden_*` oracles (`golden_composite_pixel_ex` / `golden_marcher_attributes`), so it
/// re-blesses automatically — the frozen field, the offscreen tests, and the brick path are
/// all untouched (this test simply marches the same field at a finer grid). 512×512 is large
/// enough for the owner to evaluate the brick-ON vs analytic A/B by eye; the whole sphere is
/// visible with margin and the occluding quad is clearly distinguishable.
const COMPOSITE_W: u32 = 512;
const COMPOSITE_H: u32 = 512;

/// The window's client size — the swapchain is created at the composite extent so the
/// 1:1 top-left present fills the whole window. The WSI may clamp it wider/narrower.
const WIDTH: u32 = COMPOSITE_W;
const HEIGHT: u32 = COMPOSITE_H;

/// Total pixel count (the marcher's dispatch element count; the shader bounds `idx < count`).
const PIXELS: u32 = COMPOSITE_W * COMPOSITE_H;

/// Per-channel tolerance on the packed-RGBA bytes (identical to rung 9/10 / P1b): DXC
/// `mad`/`fma` rounding + the float→UNORM store + the sample round-trip make a bit-exact
/// match brittle; `+/-2/255` still proves the lit SDF / flat mesh / background apart.
const CHANNEL_TOL: i32 = 2;

/// The depth attachment's CLEAR value (the far plane). Must equal [`MESH_DEPTH_CLEAR`].
const DEPTH_CLEAR: f32 = MESH_DEPTH_CLEAR;

/// The brick-cache activation state the present STARTS in. `true` boots brick-ON (empty-skip +
/// trilinear/cubic surface cache + the 3-level clip-map) so the owner sees the activated path
/// immediately; the 'B' key flips it live for an A/B comparison against the analytic marcher. Flip
/// this to `false` to boot in the analytic (OFF) state instead. The brick path is RTX-verified
/// byte-identical to analytic in this small origin-centered scene, so the on-screen image must look
/// IDENTICAL either way (the toggle proves the brick render == analytic, just faster).
const BRICK_START_ON: bool = true;

/// Win32 `WM_KEYDOWN` (`0x0100`) — the message the toggle watches for in the captured input ring
/// (matched numerically; `boyko_rhi_vulkan::window`'s OS constants are private, but the renderer
/// captures the verbatim `(msg, wparam, lparam)` triple, and `wparam` is the virtual-key code).
const WM_KEYDOWN: u32 = 0x0100;

/// The virtual-key code for the 'B' key (`0x42`) — the brick A/B toggle.
const VK_B: usize = 0x42;

/// The mesh-raster G-buffer color format (Render P5-r0). MUST equal the recorder's
/// `GBUFFER_FORMAT` (`R8G8B8A8_UNORM`) so the mesh-MRT producer pipeline's 3 declared
/// color formats match the bound albedo/normal/material attachments.
const RASTER_COLOR_FORMAT: Format = Format::R8G8B8A8Unorm;

const VERTEX_STRIDE: u32 = core::mem::size_of::<Vertex>() as u32;
const _: () = assert!(VERTEX_STRIDE == 40, "Vertex must be tightly packed at 40 bytes");

/// The mesh-raster VERTEX push byte size. Must equal [`GBUFFER_PUSH_BYTES`] (88: the
/// 80-byte `{ view_proj; cam_eye }` block + the M1 `{ base_instance; use_model_matrix }`
/// tail). The `mvp` builders (`ortho_mvp_bytes` / `perspective_mvp_bytes`) write the first
/// 80 bytes exactly as before and append two zero `u32`s — `use_model_matrix == 0` selects
/// the VS's legacy arm (byte-identical pixels).
const MVP_BYTES: u32 = GBUFFER_PUSH_BYTES as u32;

/// Render P5-r0: the mesh-MRT G-buffer PRODUCER vertex SPIR-V (`gbuffer_mrt.vs.spv`).
/// Vertex layout: position (loc 0, offset 0) + color (loc 1, offset 24) + per-vertex world
/// normal (loc 2, offset 12); passes the LINEAR color + the per-vertex normal through. M1
/// grew the blob 1480 -> 3068 B (the `use_model_matrix` instanced-arm branch + the set-0
/// instance SSBO); M4 grew it 3068 -> 4480 B (the per-vertex inverse-transpose normal matrix +
/// the W4 degeneracy guard in the instanced arm). The legacy arm (`use_model_matrix == 0`)
/// rasterizes byte-identical pixels (it is untouched — the bit-identity gate).
static MRT_VS_SPV: SpirvBlob<4480> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/gbuffer_mrt.vs.spv"
)));

/// Render P5-r0: the mesh-MRT G-buffer PRODUCER fragment SPIR-V (`gbuffer_mrt.fs.spv`):
/// writes albedo/normal/material as 3 MRT in the marcher's exact encoding (mask=1) + the
/// marcher-aligned `SV_Depth` (euclidean under perspective, axial under ortho).
static MRT_FS_SPV: SpirvBlob<2252> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/gbuffer_mrt.fs.spv"
)));

/// The committed rung-5 fullscreen vertex SPIR-V (`fullscreen_sample.vs.spv`): a
/// fullscreen triangle generating positions + UVs from `SV_VertexID`.
static SAMPLE_VS_SPV: SpirvBlob<744> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/fullscreen_sample.vs.spv"
)));

/// The committed rung-5 fullscreen fragment SPIR-V (`fullscreen_sample.fs.spv`): samples
/// the bound `Texture2D` + `SamplerState` at the UV and outputs it.
static SAMPLE_FS_SPV: SpirvBlob<764> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/fullscreen_sample.fs.spv"
)));

/// `ceil(PIXELS / LOCAL_SIZE_X)` — the 1D compute dispatch group count.
fn group_count_x() -> u32 {
    PIXELS.div_ceil(LOCAL_SIZE_X)
}

/// CSM Increment 1b (Rung A): the cascade shadow-map resolution the demo renders into (a 2K
/// tile — the `CsmConfig` research default).
const CSM_SHADOW_DIM: u32 = 2048;
/// CSM Increment 3 (Rung B): the cascade-array cap — mirrors `boyko_rhi_vulkan::texture::
/// MAX_CASCADES` (the per-layer-view array bound + the resolve's `MAX_CASCADES`). Local alias so
/// the demo fit array + the host golden are sized from one source.
const CSM_MAX_CASCADES: usize = boyko_rhi_vulkan::texture::MAX_CASCADES;
/// CSM Increment 1b: the byte size of the host cascade UBO — a `ResolvedCsm` mirror (336 B:
/// `[CascadeData; 4]` + `active_count` + `csm_mode_word` + pad). The resolve reads
/// `gCascades[0].view_proj` from it; the depth pass pushes the SAME matrix.
const CSM_UBO_BYTES: u64 = 336;
/// CSM Increment 1b: the host-side normal-bias FACTOR — MUST equal the resolve shader's
/// `CSM_NORMAL_BIAS` (`deferred_pbr.hlsl`) so the host matrix golden reprojects EXACTLY as the
/// resolve does.
const CSM_NORMAL_BIAS: f32 = 2.0;

/// Shadow Phase 5 Inc-1-GPU: the sparse SPOT/POINT shadow-atlas resolution (a 512 tile — the
/// `boyko_render::shadow_atlas::SHADOW_DIM` default).
const SPOT_SHADOW_DIM: u32 = 512;
/// Shadow Phase 5 Inc-1-GPU: the atlas layer budget — mirrors `boyko_render::shadow_atlas::M_SLOTS`
/// (16) and the atlas texture's `array_layers`. The demo uses ONE layer (slot 0).
const SPOT_ATLAS_SLOTS: u32 = 16;
/// Shadow Phase 5 Inc-1-GPU: the byte size of the host atlas UBO — a `ResolvedShadowAtlas` mirror
/// (1296 B: `[FaceTransform; M_SLOTS]` + `active_layers` + `mode_word` + pad). The resolve reads
/// `gFaces[slot].view_proj` from it; the depth pass pushes the SAME matrix.
const SPOT_ATLAS_UBO_BYTES: u64 = 1296;
/// SDFDDGI I0: the byte size of the host DDGI grid UBO — a `ResolvedDdgi` mirror (48 B: `origin` +
/// `inv_spacing`/dims + `ddgi_mode_word` + pad). Zero-seeded, bound-but-unread at resolve binding 18
/// while the GI gate is OFF (the default on every golden present).
const DDGI_UBO_BYTES: u64 = 48;
/// SDFDDGI I4 — the probe-update Fibonacci ray count for the GI-ON showcase (`run_showcase_body_ddgi`):
/// the ray table length + the b6 UBO's `rays_per_probe`. 64 rays/probe (≤ the shader's `GI_MAX_RAYS`)
/// — the owner-locked derived-ray budget the I4 update pass converges under 3 ms at full grid.
const DDGI_UPDATE_RAYS: usize = 64;
/// Shadow Phase 5 Inc-1-GPU: the host-side spot normal-bias FACTOR — MUST equal the resolve shader's
/// `SPOT_SHADOW_NORMAL_BIAS` (`deferred_pbr.hlsl`) so the host spot matrix golden reprojects EXACTLY
/// as the resolve does.
const SPOT_SHADOW_NORMAL_BIAS: f32 = 0.02;

/// CSM Increment 1b (Rung A): the cascade resources a [`GBufferScene`] threads into the resolve
/// (binding 12/13 — ALWAYS bound) + the depth pass ([`GBufferScene::csm`] — `Some` on the demo).
/// The trio (a multi-layer D32 cascade texture, a PCF comparison sampler, a host-coherent
/// `ResolvedCsm`-shaped UBO) plus the depth-only pipeline. The 2 golden presents use only the
/// trio (the depth pipeline is built but the scene's `csm` is `None` — the bound-but-unread
/// 0%-gate); the `#[ignore]` demo wires `csm = Some(..)` and uploads a real `view_proj`.
struct CsmSceneResources {
    cascade: VulkanTexture,
    sampler: VulkanSampler,
    // The cascade UBO RING (one host-coherent slot per in-flight frame): the resolve binds
    // `ubo[frame_index]` @13 and the viewer writes that SAME slot per frame (a per-frame CSM
    // re-fit), so the sibling in-flight frame reads a DIFFERENT slot — the lock-free
    // write-after-read fix. A STATIC scene seeds every slot identically (byte-identical output).
    ubo: [BoundBuffer; FRAMES_IN_FLIGHT],
    depth_pipeline: VulkanGraphicsPipeline,
    depth_vs: boyko_rhi_vulkan::rhi_impl::VulkanShaderModule,
    depth_fs: boyko_rhi_vulkan::rhi_impl::VulkanShaderModule,
    // Shadow Phase 5 Inc-1-GPU: the sparse SPOT/POINT atlas trio (a 16-layer D32 atlas texture, a
    // PCF comparison sampler, a host-coherent `ResolvedShadowAtlas`-shaped UBO). ALWAYS supplied to
    // the resolve @14/@15 (bound-but-unread on the 2 golden presents, where the scene's
    // `atlas_punctual` is `None` / `punctual_shadow_mode == 0`); the `#[ignore]` spot demo wires
    // `atlas_punctual = Some(..)` + uploads a real spot `view_proj`. The depth pass reuses the SAME
    // `depth_pipeline` (SPOT uses NDC-z like a CSM cascade — `csm_depth.vs/fs` verbatim).
    atlas: VulkanTexture,
    atlas_sampler: VulkanSampler,
    atlas_ubo: BoundBuffer,
    // SDFDDGI I0: the DDGI grid UBO (single buffer — the grid is world-fixed, no per-FIF ring),
    // zero-seeded ⇒ `ddgi_mode_word == 0`, bound-but-unread at resolve binding 18 while the GI gate
    // is OFF (the default on every golden present).
    ddgi_ubo: BoundBuffer,
    // SDFDDGI I1: the REAL probe atlas — irradiance (`B10G11R11_UFLOAT`) + depth (`R16G16_SFLOAT`)
    // `Texture2DArray`s + the per-probe classification buffer + a dedicated LINEAR sampler,
    // boot-cleared + boot-transitioned to `SHADER_READ_ONLY_OPTIMAL`. Bound at resolve @16/@17
    // (severing the I0a CSM-cascade/comparison-sampler dummy) — bound-but-UNREAD on every golden
    // present (the GI gate is OFF), so the swap is byte-identical.
    ddgi_atlas: DdgiAtlas,
    // Shadow Phase 5 Inc-2 (POINT cube): the POINT depth-WRITE pipeline (`punctual_depth.vs/fs` —
    // the FS writes the linear radial distance `SV_Depth`) + its two shader modules. A SEPARATE
    // pipeline from `depth_pipeline` (the SPOT NDC-z path); the punctual depth pass binds it for
    // POINT-face layers. Bound-but-unbound on the 2 golden presents (`atlas_punctual` is `None`).
    point_depth_pipeline: VulkanGraphicsPipeline,
    point_depth_vs: boyko_rhi_vulkan::rhi_impl::VulkanShaderModule,
    point_depth_fs: boyko_rhi_vulkan::rhi_impl::VulkanShaderModule,
}

impl CsmSceneResources {
    /// Creates the cascade trio + the depth-only graphics pipeline. `instance_layout` is the
    /// SAME set-0 instance-SSBO bind-group layout the gbuffer raster pipeline uses (the depth VS
    /// reads `instances[base_instance + SV_InstanceID]`).
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
                // 4 layers (== MAX_CASCADES) so the 2D_ARRAY sample view exists; Rung A renders
                // only layer 0.
                array_layers: 4,
                mip_levels: 1,
                view_format: None,
            },
        )
        .expect("CSM cascade array texture");
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
        .expect("CSM PCF comparison sampler");
        // The cascade UBO RING (one host-coherent slot per in-flight frame): the lock-free
        // per-frame re-fit binds `ubo[frame_index]` and writes that SAME slot, so the sibling
        // in-flight frame reads a DIFFERENT slot (no write-after-read overlap).
        let ubo: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: CSM_UBO_BYTES,
                    usage: BufferUsage::UNIFORM,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("CSM cascade UBO (ResolvedCsm mirror)")
        });
        let depth_vs = RhiDevice::create_shader_module(device, csm_depth_vs_spirv())
            .expect("CSM depth VS module");
        let depth_fs = RhiDevice::create_shader_module(device, csm_depth_fs_spirv())
            .expect("CSM depth FS module");
        // The depth-only pipeline: EMPTY color_formats, D32 depth, FRONT cull (Rung A casts the
        // BACK faces so the receiver's front face is unbiased — the standard shadow-map config),
        // a slope+constant depth bias (the acne fix), the set-0 instance layout + the 88-byte
        // VERTEX push (the SAME shape the gbuffer raster pipeline declares).
        let attributes = [
            VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
            VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
            VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
        ];
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
                depth_bias: Some(DepthBias {
                    constant_factor: 0.0015,
                    slope_factor: 1.5,
                    clamp: 0.0,
                }),
            },
        )
        .expect("CSM depth-only graphics pipeline");
        // Shadow Phase 5 Inc-1-GPU: the atlas trio (16 layers == M_SLOTS so the 2D_ARRAY sample view
        // exists; the demo renders only slot 0).
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
                mip_levels: 1,
                view_format: None,
            },
        )
        .expect("shadow-atlas array texture");
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
        .expect("shadow-atlas PCF comparison sampler");
        let atlas_ubo = RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: SPOT_ATLAS_UBO_BYTES,
                usage: BufferUsage::UNIFORM,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("shadow-atlas UBO (ResolvedShadowAtlas mirror)");
        // SDFDDGI I0: the DDGI grid UBO — a SINGLE zero-seeded buffer (the grid is world-fixed, no
        // ring), bound-but-unread at resolve binding 18 (GI gate OFF on every golden present).
        let ddgi_ubo = RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: DDGI_UBO_BYTES,
                usage: BufferUsage::UNIFORM,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("SDFDDGI grid UBO (ResolvedDdgi mirror)");
        // SDFDDGI I1: the REAL probe atlas + classification buffer + LINEAR sampler. Boot-cleared +
        // boot-transitioned to SHADER_READ_ONLY_OPTIMAL inside `DdgiAtlas::create`. Bound at resolve
        // @16/@17 (replacing the I0a dummy) — bound-but-unread on every golden present (GI OFF).
        let ddgi_atlas = DdgiAtlas::create(device).expect("SDFDDGI probe atlas");
        // Shadow Phase 5 Inc-2 (POINT cube): the POINT depth-WRITE pipeline (`punctual_depth.vs/fs`).
        // Same EMPTY color_formats / D32 depth / FRONT cull / depth-bias / set-0 instance layout /
        // 88-byte push as the SPOT pipeline; the ONLY difference is the shader pair — the POINT FS
        // writes `SV_Depth = saturate(length(world - light_pos) * inv_range)` (the linear radial
        // distance). Because the FS writes depth it has no early-Z, but the pipeline state is
        // otherwise identical, so it reuses the SAME `attributes` + bias values.
        let point_depth_vs = RhiDevice::create_shader_module(device, punctual_depth_vs_spirv())
            .expect("punctual point depth VS module");
        let point_depth_fs = RhiDevice::create_shader_module(device, punctual_depth_fs_spirv())
            .expect("punctual point depth FS module");
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
                depth_bias: Some(DepthBias {
                    constant_factor: 0.0015,
                    slope_factor: 1.5,
                    clamp: 0.0,
                }),
            },
        )
        .expect("punctual point depth-write graphics pipeline");
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
            ddgi_ubo,
            ddgi_atlas,
            point_depth_pipeline,
            point_depth_vs,
            point_depth_fs,
        }
    }

    /// Shadow Phase 5 Inc-1-GPU: writes a `ResolvedShadowAtlas`-shaped image into the host-coherent
    /// atlas UBO from a [`SpotDemoFit`]: slot `s`'s `view_proj` (16 column-major floats) + the
    /// POINT-shared `light_pos`/`inv_range` lanes into `gFaces[s]`, then `active_layers`/`mode_word`
    /// in the trailing words. The trailing `[active_layers..M_SLOTS)` faces stay zero
    /// (bound-but-unread). The per-slot `view_proj` bytes are the SAME ones the depth pass stamps
    /// (the O1 single-matrix pin).
    fn upload_atlas(&self, device: &VulkanContext, fit: &SpotDemoFit) {
        let mut bytes = [0u8; SPOT_ATLAS_UBO_BYTES as usize];
        for (s, face) in fit.faces.iter().enumerate().take(fit.active_layers as usize) {
            let base = s * 80; // FaceTransform stride (== CascadeData stride)
            for (i, f) in face.view_proj.iter().enumerate() {
                bytes[base + i * 4..base + i * 4 + 4].copy_from_slice(&f.to_le_bytes());
            }
            // FaceTransform layout: view_proj @0 (64 B), light_pos @64 (12 B), inv_range @76, then
            // 8 B pad to the 80 B stride is part of the next 16-B cbuffer row (pad @64+12..80 here).
            bytes[base + 64..base + 68].copy_from_slice(&face.light_pos[0].to_le_bytes());
            bytes[base + 68..base + 72].copy_from_slice(&face.light_pos[1].to_le_bytes());
            bytes[base + 72..base + 76].copy_from_slice(&face.light_pos[2].to_le_bytes());
            bytes[base + 76..base + 80].copy_from_slice(&face.inv_range.to_le_bytes());
        }
        // After the 16 × 80-byte FaceTransform array (1280 B): active_layers @1280, mode_word @1284.
        bytes[1280..1284].copy_from_slice(&fit.active_layers.to_le_bytes());
        bytes[1284..1288].copy_from_slice(&1u32.to_le_bytes());
        let dst = RhiDevice::buffer_mapped_ptr(device, &self.atlas_ubo).expect("atlas UBO mapped");
        // SAFETY: `dst` points to `SPOT_ATLAS_UBO_BYTES` mapped host-coherent bytes (the UBO was
        // created at exactly that size); `bytes` is a distinct stack array of the same length;
        // host-coherent => the write is visible before the next submit reads the UBO.
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                dst.as_ptr(),
                SPOT_ATLAS_UBO_BYTES as usize,
            );
        }
    }

    /// CSM Increment 3 (Rung B): writes a `ResolvedCsm`-shaped image into the host-coherent UBO
    /// from a [`CsmDemoFit`]: each cascade `c`'s `view_proj` (16 column-major floats), `split_far`,
    /// and `texel_size` into `gCascades[c]`, then `active_count`/`csm_mode_word` in the header. The
    /// trailing `[active_count..4)` cascade slots stay zero (bound-but-unread; the resolve SELECT
    /// loops only `[0..active_count)`). The per-cascade `view_proj` bytes are the SAME ones the
    /// depth pass stamps (the O1 single-matrix pin).
    ///
    /// `slot` selects the RING slot to write (the in-flight frame index): the viewer writes the
    /// slot the upcoming present binds + reads, so the sibling in-flight frame reads a DIFFERENT
    /// slot (the lock-free write-after-read fix). A static scene seeds every slot identically.
    fn upload(&self, device: &VulkanContext, fit: &CsmDemoFit, slot: usize) {
        let mut bytes = [0u8; CSM_UBO_BYTES as usize];
        for (c, cascade) in fit.cascades.iter().enumerate().take(fit.active_count as usize) {
            let base = c * 80; // CascadeData stride
            for (i, f) in cascade.view_proj.iter().enumerate() {
                bytes[base + i * 4..base + i * 4 + 4].copy_from_slice(&f.to_le_bytes());
            }
            // CascadeData layout: view_proj @0 (64 B), split_far @64, texel_size @68, pad @72..80.
            bytes[base + 64..base + 68].copy_from_slice(&cascade.split_far.to_le_bytes());
            bytes[base + 68..base + 72].copy_from_slice(&cascade.texel_size.to_le_bytes());
        }
        // After the 4 × 80-byte CascadeData array (320 B): active_count @320, csm_mode_word @324.
        bytes[320..324].copy_from_slice(&fit.active_count.to_le_bytes());
        bytes[324..328].copy_from_slice(&1u32.to_le_bytes());
        let dst = RhiDevice::buffer_mapped_ptr(device, &self.ubo[slot]).expect("CSM UBO mapped");
        // SAFETY: `dst` points to `CSM_UBO_BYTES` mapped host-coherent bytes (the slot's UBO was
        // created at exactly that size); `bytes` is a distinct stack array of the same length;
        // host-coherent => the write is visible before the next submit reads the UBO. `slot` is a
        // valid ring index (`< FRAMES_IN_FLIGHT`), and that slot is per-frame-private — the sibling
        // in-flight frame binds + reads a DIFFERENT slot, so this write is race-free (no fence wait).
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst.as_ptr(), CSM_UBO_BYTES as usize);
        }
    }

    /// The cascade UBO ring (one slot per in-flight frame) for the [`GBufferScene::csm_cascade_ring`]
    /// field. The resolve binds slot `frame_index` @13; the viewer writes that same slot per frame.
    fn csm_ring(&self) -> &[BoundBuffer; FRAMES_IN_FLIGHT] {
        &self.ubo
    }

    /// Tears the resources down (reverse creation order).
    ///
    /// # Safety
    /// Each was created on `device`, its GPU work completed (the caller fence/idle-waited), and
    /// each is destroyed exactly once here.
    unsafe fn destroy(self, device: &VulkanContext) {
        // SAFETY: per the contract `device` is live and nothing references these resources.
        unsafe {
            RhiDevice::destroy_graphics_pipeline(device, self.point_depth_pipeline);
            RhiDevice::destroy_shader_module(device, self.point_depth_fs);
            RhiDevice::destroy_shader_module(device, self.point_depth_vs);
            // SDFDDGI I1: the probe atlas + classification buffer + LINEAR sampler (reverse creation
            // order — created after ddgi_ubo). `DdgiAtlas::destroy` is `unsafe` on the same
            // device-idle contract this block upholds.
            self.ddgi_atlas.destroy(device);
            // SDFDDGI I0: the single DDGI grid UBO (reverse creation order — created after atlas_ubo).
            RhiDevice::destroy_buffer(device, self.ddgi_ubo);
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

/// CSM Increment 1b (Rung A): the HOST↔SHADER MATRIX GOLDEN (the acne oracle). Given a cascade
/// `view_proj` (column-major, the SAME bytes the depth VS pushes + the resolve UBO carries) + a
/// world receiver point `P` + its normal `n` + the cascade `texel_size`, this reprojects `P` the
/// SAME way the depth VS (`mul(view_proj, float4(P,1))`) and the resolve's `csm_visibility`
/// (normal-offset + the Y-flipped NDC→UV) do — so the host can assert the two reprojections
/// agree (the depth-VS write and the resolve lookup cannot drift). Mirrors the resolve's
/// `csm_visibility` UV math byte-for-byte.
///
/// Returns `(uv_x, uv_y, ndc_z, in_bounds)`: the shadow-map UV, the receiver light-space depth,
/// and whether the lookup is inside the cascade footprint (else the resolve treats it as lit).
fn csm_host_project(
    view_proj: &[f32; 16],
    p: [f32; 3],
    n: [f32; 3],
    texel_size: f32,
) -> (f32, f32, f32, bool) {
    // Normal-offset the receiver (D6) — IDENTICAL to the resolve's `P + n * texel_size * BIAS`.
    let off = texel_size * CSM_NORMAL_BIAS;
    let pw = [p[0] + n[0] * off, p[1] + n[1] * off, p[2] + n[2] * off, 1.0];
    // Column-major `view_proj * pw`: column `c` is `view_proj[c*4 + r]`, so
    // `clip[r] = sum_c view_proj[c*4 + r] * pw[c]`.
    let mut clip = [0.0f32; 4];
    for (r, clip_r) in clip.iter_mut().enumerate() {
        let mut acc = 0.0f32;
        for (c, &pw_c) in pw.iter().enumerate() {
            acc += view_proj[c * 4 + r] * pw_c;
        }
        *clip_r = acc;
    }
    if clip[3] <= 0.0 {
        return (0.0, 0.0, 0.0, false);
    }
    let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
    let uv_x = ndc[0] * 0.5 + 0.5;
    let uv_y = 1.0 - (ndc[1] * 0.5 + 0.5); // Vulkan Y-flip (matches the resolve)
    let in_bounds = (0.0..=1.0).contains(&uv_x)
        && (0.0..=1.0).contains(&uv_y)
        && (0.0..=1.0).contains(&ndc[2]);
    (uv_x, uv_y, ndc[2], in_bounds)
}

/// CSM Increment 3 (Rung B): the host-side normal-bias band overlap PROPORTION — MUST equal the
/// resolve shader's `CSM_OVERLAP_PROPORTION` (`deferred_pbr.hlsl`) so the host select+blend golden
/// computes the SAME `band_t` the GPU does.
const CSM_OVERLAP_PROPORTION: f32 = 0.2;

/// CSM Increment 3 (Rung B): the HOST MIRROR of the resolve's `csm_visibility` SELECT + BLEND
/// control flow (the arithmetic that decides WHICH cascade(s) a `view_z` reads and the cross-fade
/// weight) — NOT the PCF sample itself (the GPU owns the texture). Given the fit + a receiver
/// `view_z`, returns `(selected, next, band_t, covered)`:
///   * `selected` — the cascade index the SELECT picks (the first `c` with `view_z < split_far[c]`),
///   * `next`     — `selected + 1` clamped to the last active cascade (the blend partner),
///   * `band_t`   — the cross-fade weight in `[0,1]` (0 outside the band / on the last cascade),
///   * `covered`  — `false` when `view_z` is past every active split (the resolve returns fully lit).
///
/// Byte-mirrors the shader's branch-light chain so the golden pins the SELECT boundaries to
/// `split_far` and the blend ramp to the band — the host and GPU cannot drift.
fn csm_host_select_blend(fit: &CsmDemoFit, view_z: f32) -> (usize, usize, f32, bool) {
    let active = fit.active_count as usize;
    // SELECT: the selected cascade = the COUNT of splits view_z has passed (the shader's
    // `sum_c step(split_far[c], view_z)`); `prev_split` latches the selected cascade's near edge.
    let mut selected = 0usize;
    let mut prev_split = 0.0f32;
    for cascade in fit.cascades.iter().take(active) {
        let far_c = cascade.split_far;
        if view_z >= far_c {
            prev_split = far_c;
            selected += 1;
        }
    }
    if selected >= active {
        // Past the last split — uncovered (the resolve returns fully lit). Clamp `selected` for the
        // returned index (the caller treats `covered == false` as authoritative).
        return (active.saturating_sub(1), active.saturating_sub(1), 0.0, false);
    }
    // BLEND: the trailing `overlap * range` of the selected cascade's view-z range.
    let far_sel = fit.cascades[selected].split_far;
    let range = (far_sel - prev_split).max(1.0e-4);
    let band_start = far_sel - CSM_OVERLAP_PROPORTION * range;
    let mut band_t = ((view_z - band_start) / (far_sel - band_start).max(1.0e-4)).clamp(0.0, 1.0);
    let has_next = selected + 1 < active;
    if !has_next {
        band_t = 0.0;
    }
    let next = (selected + 1).min(active.saturating_sub(1));
    (selected, next, band_t, true)
}

/// Maps the swapchain's `i32` `VkFormat` to "readback bytes are BGRA" (skips an
/// unsupported / SRGB format). Identical to the packed on-screen test.
fn swapchain_readback_is_bgra(vk_format: i32) -> Option<bool> {
    match vk_format {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(true),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(false),
        _ => None,
    }
}

/// The orthographic MVP push for the mesh-MRT vertex shader, uploaded COLUMN-MAJOR (the
/// VERIFIED transpose). Maps a fronto-parallel world vertex so the rasterized
/// `SV_Position.z` is the AXIAL `(CAM_Z - worldZ) / T_MAX` — the depth the fragment writes
/// back unchanged under ortho (`cam_mode == 0`), byte-identical to step 1. Bytes 64..80 are
/// the `cam_eye` push field: `[0, 0, 0, 0]` (mode 0 = ortho; the eye is unused since the
/// ortho fragment keeps `SV_Position.z`). Bytes 80..88 are the M1 instanced-arm selectors,
/// left zero (`base_instance == 0`, `use_model_matrix == 0` => the VS's legacy arm).
/// Mirrors the packed/P1b convention.
#[rustfmt::skip]
fn ortho_mvp_bytes() -> [u8; MVP_BYTES as usize] {
    let h = SDF_VIEW_HALF_EXTENT;
    let tmax = SDF_TRACE_T_MAX;
    let cam = SDF_CAMERA_Z;
    let mt: [f32; 16] = [
        1.0 / h, 0.0,      0.0,          0.0,
        0.0,     -1.0 / h, 0.0,          0.0,
        0.0,     0.0,      -1.0 / tmax,  0.0,
        0.0,     0.0,      cam / tmax,   1.0,
    ];
    // `[0u8; MVP_BYTES]` leaves the trailing cam_eye (bytes 64..80) at [0,0,0,0] => mode 0.
    let mut out = [0u8; MVP_BYTES as usize];
    for (i, f) in mt.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    out
}

/// The PERSPECTIVE MVP push (`proj * view`, column-major) FOLLOWED by the `cam_eye`
/// float4 (`xyz = eye`, `w = 1.0` = perspective mode), for the mesh-MRT vertex shader.
///
/// The leading 80-byte layout MUST match `gbuffer_mrt.vs.hlsl`'s `{ float4x4 view_proj;
/// float4 cam_eye }`; bytes 80..88 are the M1 instanced-arm selectors, left zero
/// (`base_instance == 0`, `use_model_matrix == 0` => the legacy arm). The `proj * view` is
/// built from the SAME eye / basis / fov / aspect the marcher's
/// perspective ray-gen (`ray_gen.hlsli`) + `CompositePushConstants::perspective` use, so a
/// mesh vertex projects to the SAME pixel the marcher's ray through that pixel reaches at
/// that world point (screen-space alignment is the load-bearing requirement).
///
/// Convention matched to the marcher (`ray_gen.hlsli` PERSPECTIVE arm):
///   * `view`  : a right-handed look-along-`forward` frame. The marcher builds the ray
///     direction as `forward + right*(ndc_x*aspect*tan) + up*(ndc_y*tan)` and marches from
///     `eye` along it; the equivalent view matrix rows are `right`, `up`, `-forward` with
///     the eye translation, mapping a world point to camera space where camera looks down
///     `-z_cam` (`z_cam = -dot(forward, P - eye)`, the positive depth in front).
///   * `proj`  : maps camera `x_cam / (z_cam * aspect * tan)` and `y_cam / (z_cam * tan)`
///     to clip x/y (the inverse of the marcher's NDC->dir scaling). The marcher flips
///     NDC-y (`ndc_y = -(...)`), so the projection negates the camera-up axis to land a
///     `+y` world point in the upper half of the image, matching the ortho `-1/h` row.
///   * depth   : Vulkan clip `z ∈ [0, w]`; the EXACT clip-z is IRRELEVANT to correctness
///     here because the FRAGMENT overwrites depth via `SV_Depth = length(eye_rel)/T_MAX`.
///     A simple `z_clip = z_cam`, `w_clip = z_cam` (=> SV_Position.z = 1, unused) keeps the
///     vertex in front of the near plane (`z_cam > 0`) so it is not clipped.
///
/// The mesh and the marcher therefore agree in screen x/y; the per-pixel mesh depth comes
/// from the fragment's euclidean `length(cam_eye - P)`, NOT from this matrix's z.
#[rustfmt::skip]
fn perspective_mvp_bytes(
    eye: [f32; 3],
    forward: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    fov_y_radians: f32,
    aspect: f32,
) -> [u8; MVP_BYTES as usize] {
    let tan = (fov_y_radians * 0.5).tan();
    // view: world -> camera. Rows are the basis; the camera looks down -forward, so
    // z_cam = -dot(forward, P - eye) = +depth in front. (right, up, forward) is right-handed.
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (rx, ry, rz) = (right[0], right[1], right[2]);
    let (ux, uy, uz) = (up[0], up[1], up[2]);
    let (fx, fy, fz) = (forward[0], forward[1], forward[2]);
    let tx = -dot(right, eye);
    let ty = -dot(up, eye);
    let tz = dot(forward, eye); // forward·eye; the in-front view depth is z_cam = forward·(P-eye) = forward·P - tz
    // proj * view, ROW-MAJOR math rows below; uploaded column-major (transposed) to match
    // `ortho_mvp_bytes`. clip.x = x_cam/(aspect*tan); clip.y = -y_cam/tan (flip to match the
    // marcher's `ndc_y = -(...)`); clip.z = clip.w = z_cam = forward·(P-eye) (POSITIVE in front, so
    // the perspective divide is well-defined). `forward` points INTO the scene (= the marcher's ray
    // direction `rd` in `ray_gen.hlsli`), so the basis row is `+forward`, NOT `-forward` — a flipped
    // sign here warps every vertex by a depth-dependent amount (`-2·forward·P`), which a flat quad
    // survives but a multi-depth cube cracks into black wedges (BUG fixed: was `[-fx,-fy,-fz,-tz]`).
    let sx = 1.0 / (aspect * tan);
    let sy = -1.0 / tan;
    // pv row r = proj_scale_r · view_row_r  (view_row = [basis | translation]).
    let pv: [[f32; 4]; 4] = [
        [sx * rx, sx * ry, sx * rz, sx * tx], // clip.x
        [sy * ux, sy * uy, sy * uz, sy * ty], // clip.y (flipped)
        [fx,      fy,      fz,      -tz     ], // clip.z = z_cam = forward·(P-eye) = forward·P - tz
        [fx,      fy,      fz,      -tz     ], // clip.w = z_cam (perspective divide)
    ];
    // Upload COLUMN-MAJOR: out[col*4 + row] holds pv[row][col] (the verified transpose).
    let mut out = [0u8; MVP_BYTES as usize];
    for col in 0..4 {
        for row in 0..4 {
            let b = pv[row][col].to_le_bytes();
            out[(col * 4 + row) * 4..(col * 4 + row) * 4 + 4].copy_from_slice(&b);
        }
    }
    // cam_eye push field (bytes 64..80): xyz = eye, w = 1.0 (perspective mode).
    let cam_eye = [eye[0], eye[1], eye[2], 1.0_f32];
    for (i, f) in cam_eye.iter().enumerate() {
        out[64 + i * 4..64 + i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    out
}

/// M1: creates the gbuffer raster pipeline's `set 0` per-instance model resources — a
/// 1-binding `StorageBuffer` layout (binding 0, VERTEX stage), a 1-element host-visible
/// instance SSBO seeded with the [`GBUFFER_IDENTITY_INSTANCE`] affine, and a bind group
/// pointing the layout at the buffer. The gbuffer VS statically references
/// `StructuredBuffer<InstanceModelCol> instances`, so the layout MUST be in the pipeline
/// layout and a valid buffer MUST be bound for every draw; the legacy merged draw
/// (`use_model_matrix == 0`) never reads it (bound-but-unread). The caller OWNS all three
/// and tears them down (`destroy_bind_group` → `destroy_buffer` → `destroy_bind_group_layout`).
fn create_identity_instance(
    device: &VulkanContext,
) -> (VulkanBindGroupLayout, BoundBuffer, VulkanBindGroup) {
    let layout = RhiDevice::create_bind_group_layout(
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
    .expect("M1 instance-model bind-group layout");
    let buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: GBUFFER_INSTANCE_MODEL_BYTES as u64,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("M1 identity instance SSBO");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &buffer)
            .expect("host-visible identity instance buffer is mapped");
        let mut bytes = [0u8; GBUFFER_INSTANCE_MODEL_BYTES];
        for (i, f) in GBUFFER_IDENTITY_INSTANCE.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
        }
        // SAFETY: `mapped` points to `GBUFFER_INSTANCE_MODEL_BYTES` (48) mapped host-coherent
        // bytes; `bytes` is exactly that length and copied in full, in-bounds. No GPU work is
        // in flight yet (the present loop follows), so the write is unsynchronized-safe.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
        }
    }
    let bind_group = RhiDevice::create_bind_group(
        device,
        &BindGroupDesc {
            layout: &layout,
            entries: &[BindGroupEntry::StorageBuffer { buffer: &buffer }],
        },
    )
    .expect("M1 identity instance bind group");
    (layout, buffer, bind_group)
}

/// Mesh foundation M2: builds the INSTANCED-arm per-instance model SSBO — an N-element
/// host-visible `StorageBuffer` of `InstanceModelCol` affines (`affines[i]` = instance `i`'s
/// 3x4 ROW-MAJOR model matrix, 12 `f32` = 48 B each, the SAME layout the M1 VS reads as
/// `instances[base_instance + SV_InstanceID]`) + a bind group on the SAME `layout` shape the
/// gbuffer pipeline declares at set 0. Unlike [`create_identity_instance`]'s 1-element dummy,
/// this holds REAL non-identity placements the `use_model_matrix == 1` arm transforms vertices
/// by. The caller OWNS the buffer + bind group and tears them down (bind group → buffer).
fn create_instance_buffer(
    device: &VulkanContext,
    layout: &VulkanBindGroupLayout,
    affines: &[[f32; 12]],
) -> (BoundBuffer, VulkanBindGroup) {
    assert!(!affines.is_empty(), "the instanced draw needs at least one instance affine");
    let total_bytes = affines.len() * GBUFFER_INSTANCE_MODEL_BYTES;
    let buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: total_bytes as u64,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("M2 N-instance model SSBO");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &buffer)
            .expect("host-visible instance buffer is mapped");
        let mut bytes = vec![0u8; total_bytes];
        for (inst, affine) in affines.iter().enumerate() {
            let base = inst * GBUFFER_INSTANCE_MODEL_BYTES;
            for (i, f) in affine.iter().enumerate() {
                bytes[base + i * 4..base + i * 4 + 4].copy_from_slice(&f.to_le_bytes());
            }
        }
        // SAFETY: `mapped` points to `total_bytes` mapped host-coherent bytes; `bytes` is
        // exactly that length and copied in full, in-bounds. No GPU work is in flight yet (the
        // present loop follows), so the write is unsynchronized-safe.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
        }
    }
    let bind_group = RhiDevice::create_bind_group(
        device,
        &BindGroupDesc {
            layout,
            entries: &[BindGroupEntry::StorageBuffer { buffer: &buffer }],
        },
    )
    .expect("M2 N-instance bind group");
    (buffer, bind_group)
}

/// Mesh foundation M2: a 3x4 ROW-MAJOR affine (the `InstanceModelCol` the SSBO carries) for a
/// uniform `scale` + a Y-axis rotation by `yaw` (radians) + a translation `t`. Row-major: row
/// `i`'s `.xyz` is the rotation/scale row, `.w` the translation component. Matches the host
/// mirror in `instanced_vs_host_mirror.rs` so the on-screen placement equals the CPU C2 gate.
fn instance_affine(yaw: f32, scale: f32, t: [f32; 3]) -> [f32; 12] {
    let (s, c) = yaw.sin_cos();
    [
        c * scale, 0.0, s * scale, t[0],
        0.0, scale, 0.0, t[1],
        -s * scale, 0.0, c * scale, t[2],
    ]
}

// ════════════════════════════════════════════════════════════════════════════
// Pillar B B3 — the interpolation compute PRE-PASS resources + inline TRS.
//
// `boyko_rhi_vulkan` cannot depend on `boyko_render` (which owns `GpuTransform3D` /
// `pack_gpu_transforms` / `gather_mesh_draw_pairs`) — `boyko_render` depends UPWARD on
// this crate, so a dev-dep would cycle. The B1 host mirror is therefore reproduced INLINE
// here (a few lines: the 96-byte `TransformPair` pack + a trivial Euler falling-box
// integrator), driving the REAL production interp GPU pass (`interp_instances.comp` +
// `InterpActivation` + the framegraph COMPUTE→VERTEX barrier). The eDSL byte-identity of
// the shader itself is proven by `tests/interp_edsl_sync.rs`; this file proves the wired
// GPU pass moves the drawn geometry per the interpolated pose.
// ════════════════════════════════════════════════════════════════════════════

/// The 96-byte `TransformPair` the interp pre-pass reads (byte-mirror of the B2 shader's
/// `TransformPair` / `boyko_render::GpuTransform3D`): `prev` TRS at byte 0, `curr` at 48.
/// Each TRS is `pos.xyzw`(pad w) @0, `rot.xyzw` quaternion @16, `scale.xyzw`(pad w) @32.
#[derive(Clone, Copy)]
struct InterpPair {
    prev: [f32; 12],
    curr: [f32; 12],
}

impl InterpPair {
    /// Serializes to the 96 packed bytes the pair SSBO stride declares (prev@0, curr@48).
    fn to_bytes(self) -> [u8; 96] {
        let mut out = [0u8; 96];
        write_trs(&mut out[0..48], &self.prev);
        write_trs(&mut out[48..96], &self.curr);
        out
    }
}

/// Decomposes a 3×4 ROW-MAJOR affine (an `InstanceModelCol`: `rows[i] = [Rx|Ry|Rz|Tx]`) into the
/// interp shader's `Trs` (`pos.xyzw` + unit-quaternion `rot.xyzw` + `scale.xyzw`), so the viewer's
/// hand-built room affines can be routed through the pair path (a still instance seeds `prev ==
/// curr` → the B2 keystone renders it bitwise-stable, reproducing its placement). The scale is the
/// per-axis COLUMN norm of the linear 3×3; the rotation is the quaternion of the normalized (pure-
/// rotation) 3×3; the translation is the last column. Handles the Y-rotation + (non)uniform scale
/// the room's `instance_affine{,_nonuniform}` produce (any pure T·R·S with a right-handed R).
fn trs_from_affine(a: &[f32; 12]) -> [f32; 12] {
    // Column vectors of the linear 3×3 (row-major: a[row*4 + col]).
    let col = |c: usize| [a[c], a[4 + c], a[8 + c]];
    let (c0, c1, c2) = (col(0), col(1), col(2));
    let norm = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let (sx, sy, sz) = (norm(c0), norm(c1), norm(c2));
    // Normalized rotation columns (guard a zero-scale axis with the identity basis vector).
    let unit = |v: [f32; 3], s: f32, fallback: [f32; 3]| {
        if s > 1e-6 { [v[0] / s, v[1] / s, v[2] / s] } else { fallback }
    };
    let r0 = unit(c0, sx, [1.0, 0.0, 0.0]);
    let r1 = unit(c1, sy, [0.0, 1.0, 0.0]);
    let r2 = unit(c2, sz, [0.0, 0.0, 1.0]);
    // Rotation-matrix → quaternion (Shepperd's method, the numerically stable branch pick). The
    // matrix columns r0/r1/r2 form R (m[row][col] = r{col}[row]).
    let m = [
        [r0[0], r1[0], r2[0]],
        [r0[1], r1[1], r2[1]],
        [r0[2], r1[2], r2[2]],
    ];
    let trace = m[0][0] + m[1][1] + m[2][2];
    let (qx, qy, qz, qw) = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        (
            (m[2][1] - m[1][2]) / s,
            (m[0][2] - m[2][0]) / s,
            (m[1][0] - m[0][1]) / s,
            0.25 * s,
        )
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        (
            0.25 * s,
            (m[0][1] + m[1][0]) / s,
            (m[0][2] + m[2][0]) / s,
            (m[2][1] - m[1][2]) / s,
        )
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        (
            (m[0][1] + m[1][0]) / s,
            0.25 * s,
            (m[1][2] + m[2][1]) / s,
            (m[0][2] - m[2][0]) / s,
        )
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        (
            (m[0][2] + m[2][0]) / s,
            (m[1][2] + m[2][1]) / s,
            0.25 * s,
            (m[1][0] - m[0][1]) / s,
        )
    };
    [a[3], a[7], a[11], 0.0, qx, qy, qz, qw, sx, sy, sz, 0.0]
}

/// Writes one 48-byte `Trs` (12 `f32`) into `dst`.
fn write_trs(dst: &mut [u8], trs: &[f32; 12]) {
    for (i, f) in trs.iter().enumerate() {
        dst[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
}

/// The built Pillar-B B3 interpolation pre-pass GPU resources: the compute pipeline + its
/// 2-binding set layout, and the FRAMES_IN_FLIGHT-ringed pair / draw SSBOs (frame-private,
/// like the G-buffer ring) with their per-slot bind groups. The host writes this frame's
/// `pairs[fi]`, the compute reads it + writes `draw[fi]`, and the raster VS reads `draw[fi]`
/// via `draw_bg[fi]` (bound as `GBufferScene::instance_bind_group`). The caller OWNS all of
/// it and tears it down via [`Self::destroy`].
struct InterpGpu {
    pipeline: ComputePipeline,
    layout: VulkanBindGroupLayout,
    /// FIF ring of pair SSBOs (host-written, COMPUTE-read); capacity = `count` × 96 B.
    pairs: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// FIF ring of out-slot SSBOs (host-written, COMPUTE-read); capacity = `count` × 4 B.
    /// This harness is ALL-interpolated (no static rows), so the out-slot lane is the
    /// IDENTITY `[0, 1, .., count-1]` — each dynamic instance writes its own ring index.
    out_slot: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// FIF ring of draw SSBOs (COMPUTE-written, VERTEX-read); capacity = `count` × 48 B.
    /// Refined-B: this IS the shared model-out ring the raster VS reads (the harness has
    /// no separate static rows to CPU-scatter).
    draw: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// FIF ring of interp bind groups { pairs[fi] @0, out_slot[fi] @1, draw[fi] @2 } on
    /// [`Self::layout`] — the model_out target is the SAME `draw` ring the raster VS reads.
    interp_bg: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// FIF ring of draw-read bind groups { draw[fi] @0 } on the gbuffer set-0 instance
    /// layout — the SAME shape the raster VS reads as `instances[...]`.
    draw_bg: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// The interpolated instance count (the dispatch bound + the push `count`).
    count: u32,
}

impl InterpGpu {
    /// Builds the interp pipeline + the FIF-ringed pair/draw SSBOs + their bind groups for a
    /// draw list of exactly `count` instances. `instance_layout` is the gbuffer set-0 layout
    /// (1 STORAGE buffer @0, VERTEX) the raster VS reads — the draw-read bind groups bind the
    /// draw SSBO on it, so passing `draw_bg[fi]` as `scene.instance_bind_group` needs no new
    /// pipeline. `count` must be ≥ 1.
    fn create(device: &VulkanContext, instance_layout: &VulkanBindGroupLayout, count: u32) -> Self {
        assert!(count >= 1, "the interp pass needs at least one instance");
        let cs = RhiDevice::create_shader_module(device, interp_instances_spirv())
            .expect("B3 interp compute shader module");
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
        .expect("B3 interp bind-group layout");
        let pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &cs,
                entry: c"main",
                push_constant_bytes: INTERP_INSTANCES_PUSH_BYTES,
                bind_group_layout: Some(&layout),
                spec_constants: &[],
            },
        )
        .expect("B3 interp compute pipeline");

        let pair_bytes = count as u64 * 96;
        let out_slot_bytes = count as u64 * 4;
        let draw_bytes = count as u64 * GBUFFER_INSTANCE_MODEL_BYTES as u64;
        let make_buf = |size: u64, what: &str| {
            RhiDevice::create_buffer(
                device,
                &BufferDesc { size, usage: BufferUsage::STORAGE, location: MemoryLocation::HostVisibleCoherent },
            )
            .unwrap_or_else(|e| panic!("B3 interp {what} SSBO: {e:?}"))
        };
        // Each ring slot is a distinct frame-private SSBO (host-coherent so the pairs write +
        // the readback of the draw output need no explicit flush).
        let pairs: [BoundBuffer; FRAMES_IN_FLIGHT] =
            core::array::from_fn(|_| make_buf(pair_bytes, "pairs"));
        let draw: [BoundBuffer; FRAMES_IN_FLIGHT] =
            core::array::from_fn(|_| make_buf(draw_bytes, "draw"));
        // The out-slot lane is the IDENTITY (all-interpolated harness): out_slot[i] = i, so
        // each thread writes its own ring index. Seed it once (it never changes).
        let out_slot: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            let b = make_buf(out_slot_bytes, "out_slot");
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .expect("host-visible interp out-slot SSBO is mapped");
            let identity: Vec<u32> = (0..count).collect();
            // SAFETY: `mapped` targets `count * 4` mapped host-coherent bytes; `identity` is
            // exactly `count` u32s, copied in full, in-bounds. Seeded at setup — no submission
            // references this slot yet.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    identity.as_ptr().cast::<u8>(),
                    mapped.as_ptr(),
                    out_slot_bytes as usize,
                );
            }
            b
        });
        let interp_bg: [VulkanBindGroup; FRAMES_IN_FLIGHT] = core::array::from_fn(|fi| {
            RhiDevice::create_bind_group(
                device,
                &BindGroupDesc {
                    layout: &layout,
                    entries: &[
                        BindGroupEntry::StorageBuffer { buffer: &pairs[fi] },
                        BindGroupEntry::StorageBuffer { buffer: &out_slot[fi] },
                        BindGroupEntry::StorageBuffer { buffer: &draw[fi] },
                    ],
                },
            )
            .expect("B3 interp bind group")
        });
        let draw_bg: [VulkanBindGroup; FRAMES_IN_FLIGHT] = core::array::from_fn(|fi| {
            RhiDevice::create_bind_group(
                device,
                &BindGroupDesc {
                    layout: instance_layout,
                    entries: &[BindGroupEntry::StorageBuffer { buffer: &draw[fi] }],
                },
            )
            .expect("B3 interp draw-read bind group")
        });
        Self { pipeline, layout, pairs, out_slot, draw, interp_bg, draw_bg, count }
    }

    /// Writes the `count` pairs into this frame slot's pair SSBO (host-coherent). Called each
    /// frame the pose changed (a substep ran or the count changed); the alpha slides every
    /// frame via the push constant regardless. `token` is the per-slot write proof, BORROWED
    /// (R0b: a mid-frame write — the caller keeps the token for the frame-ending submit) —
    /// the memcpy targets `token.slot()` and cannot precede that slot's fence wait.
    fn write_pairs(&self, device: &VulkanContext, token: &FrameWriteToken, pairs: &[InterpPair]) {
        let fi = token.slot();
        debug_assert_eq!(pairs.len(), self.count as usize, "pair count must equal the built count");
        let mapped = RhiDevice::buffer_mapped_ptr(device, &self.pairs[fi])
            .expect("host-visible interp pair SSBO is mapped");
        let mut bytes = vec![0u8; pairs.len() * 96];
        for (i, p) in pairs.iter().enumerate() {
            bytes[i * 96..(i + 1) * 96].copy_from_slice(&p.to_bytes());
        }
        // SAFETY: `mapped` points to `count * 96` mapped host-coherent bytes (the buffer this
        // slot was sized to); `bytes` is exactly that length and copied in full, in-bounds.
        // The `token` proves slot `fi` is host-writable: its in-flight fence was waited THIS
        // frame (`Renderer::wait_frame_in_flight`) or nothing submitted references the slot
        // yet (`forge_unfenced` at seeding) — the previous occupant no longer reads the buffer.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len()) };
    }

    /// The [`InterpActivation`] for this frame slot: the interp set (pairs@0 + out_slot@1 +
    /// model_out@2 for slot `fi`) + the instance count + this frame's overstep `alpha`. The
    /// model_out target is the `draw` ring (which the raster VS also reads — refined-B).
    fn activation(&self, fi: usize, alpha: f32) -> InterpActivation<'_> {
        InterpActivation {
            pipeline: &self.pipeline,
            interp_set: &self.interp_bg[fi],
            pair_buffer: &self.pairs[fi],
            out_slot_buffer: &self.out_slot[fi],
            model_out_buffer: &self.draw[fi],
            instance_count: self.count,
            alpha,
        }
    }

    /// Tears down every owned resource (bind groups → buffers → pipeline → layout), reverse
    /// dependency order. Call after the renderer is dropped (device idle).
    ///
    /// # Safety
    ///
    /// No submission may reference these resources (the renderer's `Drop` waited the device
    /// idle); each is destroyed exactly once.
    unsafe fn destroy(self, device: &VulkanContext) {
        // SAFETY: the caller guarantees device-idle + single teardown (this fn's contract).
        unsafe {
            for bg in self.draw_bg {
                RhiDevice::destroy_bind_group(device, bg);
            }
            for bg in self.interp_bg {
                RhiDevice::destroy_bind_group(device, bg);
            }
            for b in self.draw {
                RhiDevice::destroy_buffer(device, b);
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

/// Mesh foundation M4: a 3x4 row-major affine with a NON-UNIFORM per-axis scale `(sx, sy, sz)`
/// composed with a Y-axis rotation by `yaw` and a translation `t` — i.e. `T * R * S`, so the 3x3
/// linear part is `R * diag(s)` (a non-orthogonal basis). This is the placement the M4
/// inverse-transpose normal arm needs: under `sx != sy != sz` the naive `mul(m3, normal)` skews
/// normals off the surface, so the demo makes the inverse-transpose vs naive difference visible.
fn instance_affine_nonuniform(yaw: f32, s: [f32; 3], t: [f32; 3]) -> [f32; 12] {
    let (sin, cos) = yaw.sin_cos();
    // R (row-major) * diag(sx, sy, sz): scale COLUMN j of R by s[j].
    [
        cos * s[0], 0.0, sin * s[2], t[0],
        0.0, s[1], 0.0, t[1],
        -sin * s[0], 0.0, cos * s[2], t[2],
    ]
}

/// Emits one mesh quad face as two CCW triangles `(a, b, c)` + `(a, c, d)`, every vertex
/// carrying the supplied outward world `normal` `n` and `color`. `corners` are the four
/// quad corners in CCW order as seen from the `+n` side (matching [`quad_vertices`]'s
/// `bl, br, tr, tl` winding for the `+Z` face). Culling is OFF (`rhi_impl/device.rs`), so the
/// winding is cosmetic, but it is kept consistent for correctness.
fn mesh_quad(corners: [[f32; 3]; 4], n: [f32; 3], color: [f32; 4]) -> [Vertex; 6] {
    let [a, b, c, d] = corners;
    let v = |p: [f32; 3]| Vertex { position: p, normal: n, color };
    [v(a), v(b), v(c), v(a), v(c), v(d)]
}

/// A solid axis-aligned mesh box centered at `center` with per-axis half-extents `half`,
/// as 6 faces × 2 triangles = 36 vertices. Each face carries its outward axis normal
/// (`±X`, `±Y`, `±Z`), with its 4 corners ordered CCW as seen from outside the box. The
/// per-vertex normals feed the G-buffer normal target so the box's faces shade distinctly.
fn mesh_box(center: [f32; 3], half: [f32; 3], color: [f32; 4]) -> Vec<Vertex> {
    let [cx, cy, cz] = center;
    let [hx, hy, hz] = half;
    let (x0, x1) = (cx - hx, cx + hx);
    let (y0, y1) = (cy - hy, cy + hy);
    let (z0, z1) = (cz - hz, cz + hz);

    // Each face lists its 4 corners CCW from the outward normal's side.
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        // +Z (front): looking toward -Z, CCW = bl, br, tr, tl in the +Z plane.
        ([0.0, 0.0, 1.0], [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]]),
        // -Z (back): looking toward +Z, CCW winds the opposite way in X.
        ([0.0, 0.0, -1.0], [[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]]),
        // +X (right): looking toward -X, CCW in the +X plane.
        ([1.0, 0.0, 0.0], [[x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]]),
        // -X (left): looking toward +X.
        ([-1.0, 0.0, 0.0], [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]]),
        // +Y (top): looking toward -Y.
        ([0.0, 1.0, 0.0], [[x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]]),
        // -Y (bottom): looking toward +Y.
        ([0.0, -1.0, 0.0], [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]]),
    ];

    let mut verts = Vec::with_capacity(36);
    for (n, corners) in faces {
        verts.extend_from_slice(&mesh_quad(corners, n, color));
    }
    verts
}

/// Mesh foundation M2: a MODEL-SPACE unit-ish box centered at the ORIGIN with per-axis
/// half-extents `half`, returned as `(vertices, indices)` for an INDEXED draw — the form the
/// [`MeshRegistry`](boyko_render::MeshRegistry) stores and the instanced gbuffer arm draws.
/// Unlike [`mesh_box`] (which expands to 36 fully-duplicated triangle-list vertices for the
/// legacy non-indexed draw), this emits 24 UNIQUE vertices (4 per face, each face carrying its
/// outward axis normal so faces shade distinctly) + 36 indices (2 CCW triangles per face). The
/// box is at the origin; an instance affine places + orients it in world space (the model-space
/// contract). Index width is `u16`-sized (24 ≤ 65536), so the registry mints `Uint16` indices.
fn mesh_box_model(half: [f32; 3], color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let [hx, hy, hz] = half;
    let (x0, x1) = (-hx, hx);
    let (y0, y1) = (-hy, hy);
    let (z0, z1) = (-hz, hz);

    // Each face: outward normal + 4 corners CCW from outside (matching `mesh_box`'s winding).
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        ([0.0, 0.0, 1.0], [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]]),
        ([0.0, 0.0, -1.0], [[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]]),
        ([1.0, 0.0, 0.0], [[x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]]),
        ([-1.0, 0.0, 0.0], [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]]),
        ([0.0, 1.0, 0.0], [[x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]]),
        ([0.0, -1.0, 0.0], [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]]),
    ];

    let mut verts: Vec<Vertex> = Vec::with_capacity(24);
    let mut indices: Vec<u32> = Vec::with_capacity(36);
    for (n, corners) in faces {
        let base = verts.len() as u32;
        for c in corners {
            verts.push(Vertex { position: c, normal: n, color });
        }
        // Two CCW triangles (a, b, c) + (a, c, d) over this face's 4 corners.
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (verts, indices)
}

/// A procedural TRIANGULATED TORUS centered at `center`, major radius `r_major` (ring), minor
/// radius `r_minor` (tube), with `rings × sides` quads. The torus axis is +Z (the ring lies in the
/// XY plane facing the ORTHO camera) — an ASYMMETRIC shape (a hole + a ring) so its cast shadow
/// reads clearly on the floor. Returns BOTH the raster `Vertex` list (per-vertex outward normals for
/// the G-buffer) AND the `(positions, indices)` the MDF baker consumes (the SAME geometry — the
/// raster mesh IS its own shadow proxy). Used by the MDF Stage-2c demo.
#[allow(clippy::type_complexity)]
fn torus_mesh(
    center: [f32; 3],
    r_major: f32,
    r_minor: f32,
    rings: u32,
    sides: u32,
    color: [f32; 4],
) -> (Vec<Vertex>, Vec<[f32; 3]>, Vec<[u32; 3]>) {
    use core::f32::consts::PI;
    // The (positions, normals) lattice: `(rings+1) × (sides+1)` so the seam wraps cleanly.
    let row = sides + 1;
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(((rings + 1) * row) as usize);
    let mut nrm: Vec<[f32; 3]> = Vec::with_capacity(((rings + 1) * row) as usize);
    for i in 0..=rings {
        let u = 2.0 * PI * i as f32 / rings as f32; // around the ring (the major circle)
        let (su, cu) = (u.sin(), u.cos());
        for j in 0..=sides {
            let v = 2.0 * PI * j as f32 / sides as f32; // around the tube (the minor circle)
            let (sv, cv) = (v.sin(), v.cos());
            // The tube-center on the major circle (in the XY plane, axis +Z).
            let ring_x = r_major * cu;
            let ring_y = r_major * su;
            // The surface point: the tube offset by `r_minor` in the (radial, +Z) frame.
            let p = [
                center[0] + (ring_x + r_minor * cv * cu),
                center[1] + (ring_y + r_minor * cv * su),
                center[2] + r_minor * sv,
            ];
            // The outward normal = the surface point minus the tube center, normalized.
            let n = [cv * cu, cv * su, sv];
            pos.push(p);
            nrm.push(n);
        }
    }

    // The index list (two CCW triangles per quad) for BOTH the raster draw and the baker.
    let mut verts: Vec<Vertex> = Vec::with_capacity((rings * sides * 6) as usize);
    let mut indices: Vec<[u32; 3]> = Vec::with_capacity((rings * sides * 2) as usize);
    for i in 0..rings {
        for j in 0..sides {
            let a = i * row + j;
            let b = a + 1;
            let c = (i + 1) * row + j;
            let d = c + 1;
            indices.push([a, c, b]);
            indices.push([b, c, d]);
            for &k in &[a, c, b, b, c, d] {
                let k = k as usize;
                verts.push(Vertex { position: pos[k], normal: nrm[k], color });
            }
        }
    }
    (verts, pos, indices)
}

/// The ORTHO world-XY of pixel `(px, py)`'s ray at the COMPOSITE extent — the
/// extent-aware mirror of `compute::pixel_world_xy` (which is frozen to 64×64). The
/// arithmetic is byte-identical to the shader's / `composite_ray`'s ORTHO arm (`u`/`v`
/// → `* SDF_VIEW_HALF_EXTENT`), just parameterized on the live extent so the discriminator
/// picking + mesh-coverage host model track the 512×512 dispatch the marcher runs.
fn composite_pixel_world_xy(px: u32, py: u32) -> [f32; 2] {
    let u = (((px as f32) + 0.5) / (COMPOSITE_W as f32)) * 2.0 - 1.0;
    let v = -((((py as f32) + 0.5) / (COMPOSITE_H as f32)) * 2.0 - 1.0);
    [u * SDF_VIEW_HALF_EXTENT, v * SDF_VIEW_HALF_EXTENT]
}

/// Whether the SDF field is hit at pixel `(px, py)` IGNORING the mesh, at the COMPOSITE
/// extent. The extent-aware mirror of `compute::editlist_pixel_hits` (frozen to 64×64):
/// it asks the extent-aware marcher oracle for the attributes with NO mesh
/// (`mesh_depth == MESH_DEPTH_CLEAR`, so `t_mesh == +inf`) — then `mask == 1` is exactly
/// a pure SDF geometry hit. Lighting flags are irrelevant to the hit test.
fn composite_sdf_hits(edits: &[SdfEdit], px: u32, py: u32) -> bool {
    let materials = [GoldenMaterial::default()];
    let attrs = golden_marcher_attributes(
        edits,
        &materials,
        MESH_DEPTH_CLEAR,
        px,
        py,
        COMPOSITE_W,
        COMPOSITE_H,
        CompositeCamera::Ortho,
        DEFAULT_MARCHER_OMEGA,
        0,
        DEFAULT_LIGHT_DIR,
    );
    attrs.mask == 1
}

/// Whether pixel `(px, py)`'s orthographic ray passes through the mesh quad footprint
/// (the rasterizer's covered-pixel set, host-computable from the SAME camera mapping).
fn mesh_covers_pixel(px: u32, py: u32) -> bool {
    let [x, y] = composite_pixel_world_xy(px, py);
    (QUAD_X_MIN..=QUAD_X_MAX).contains(&x) && (QUAD_Y_MIN..=QUAD_Y_MAX).contains(&y)
}

/// The per-pixel mesh depth the GPU is expected to produce: the constant inside the
/// quad, the clear value outside.
fn expected_mesh_depth(px: u32, py: u32) -> f32 {
    if mesh_covers_pixel(px, py) {
        mesh_depth_for_z(MESH_Z)
    } else {
        DEPTH_CLEAR
    }
}

/// The base-sphere SDF scene (one union sphere, origin, r=0.5) — the recognizable SDF
/// body the mesh occludes (the packed/P1b `sphere_scene`).
fn sphere_scene() -> Vec<SdfEdit> {
    vec![SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0)]
}

/// PBR MVP-2: the std430 word-packing of a ONE-element material table holding the engine
/// default material (mid-gray dielectric: base 0.8/0.8/0.8/1, metallic 0, roughness 0.5,
/// reflectance 0.5, flags 0, emissive 0). 12 words = 48 B (mirrors `MaterialGpu`'s 3 vec4
/// lanes). The windowed scene's edits carry no material id, so every SDF hit picks id 0.
const DEFAULT_MATERIAL_TABLE: [u32; 12] = [
    0x3F4CCCCD, 0x3F4CCCCD, 0x3F4CCCCD, 0x3F800000, // base_color: 0.8, 0.8, 0.8, 1.0
    0x00000000, 0x3F000000, 0x3F000000, 0x00000000, // mrr: metallic 0, rough 0.5, refl 0.5, flags 0
    0x00000000, 0x00000000, 0x00000000, 0x00000000, // emissive: 0, 0, 0, 0
];

/// Lighting L0a: the std430 word-packing of the DEGENERATE light table — the 0%-gate
/// anchor that reproduces the resolve's old compiled-in `LIGHT_DIR`/`LIGHT_COLOR`/`SKY_*`
/// constants byte-for-byte. Layout `[LightHeaderGpu (16 words) || GpuLight[2] (24 words)]`
/// = 40 words = 160 B (mirrors `boyko_render::light` + `light_table.hlsli`):
///
/// - header: light_count 2, exposure 1.0, l0a_count 2 (1 dir + 1 sky), point_spot 0,
///   sky_diffuse/sky_spec = (0.10,0.10,0.12) (carried; the L0a resolve drives ambient
///   from the sky entity, these are unused by the resolve), cluster params 0.
/// - element 0 (DIRECTIONAL, kind 0): dir (0,0,1), range +inf, color (1,1,1) — matches
///   the old `LIGHT_DIR` / `LIGHT_COLOR` (illuminance 1.0).
/// - element 1 (SKY, kind 3): ground (0.10,0.10,0.12) in the pos lane, sky (0.10,0.10,0.12)
///   in the color lane — `sky == ground` ⇒ the hemisphere `lerp` folds to the old `SKY_*`.
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

/// Splits a packed `0xAABBGGRR` into `[r, g, b]` (the low three bytes).
fn unpack_rgb(packed: u32) -> [i32; 3] {
    [
        (packed & 0xFF) as i32,
        ((packed >> 8) & 0xFF) as i32,
        ((packed >> 16) & 0xFF) as i32,
    ]
}

/// Decodes one readback texel into `[r, g, b]`, applying the swapchain channel order.
fn readback_rgb(texel: [u8; 4], is_bgra: bool) -> [i32; 3] {
    if is_bgra {
        [texel[2] as i32, texel[1] as i32, texel[0] as i32]
    } else {
        [texel[0] as i32, texel[1] as i32, texel[2] as i32]
    }
}

/// `true` if a readback texel agrees with a golden packed `0xAABBGGRR` within
/// `CHANNEL_TOL` per RGB channel (swapchain-byte-order-aware).
fn readback_close(texel: [u8; 4], golden: u32, is_bgra: bool) -> bool {
    let g = readback_rgb(texel, is_bgra);
    let w = unpack_rgb(golden);
    (0..3).all(|c| (g[c] - w[c]).abs() <= CHANNEL_TOL)
}

/// Asserts a readback texel agrees with a golden packed color (swapchain-order-aware).
fn assert_readback_close(texel: [u8; 4], golden: u32, is_bgra: bool, label: &str) {
    assert!(
        readback_close(texel, golden, is_bgra),
        "{label}: readback {texel:02x?} (bgra={is_bgra}) != golden {golden:#010x} -> {:?} within +/-{CHANNEL_TOL}",
        unpack_rgb(golden),
    );
}

/// `true` if two golden packed colors agree within `CHANNEL_TOL` per RGB channel.
fn goldens_close(a: u32, b: u32) -> bool {
    let x = unpack_rgb(a);
    let y = unpack_rgb(b);
    (0..3).all(|c| (x[c] - y[c]).abs() <= CHANNEL_TOL)
}

/// The byte index of texel `(x, y)` in a tightly-packed 4-byte/texel readback of a
/// `w`-wide image.
fn texel_base(x: u32, y: u32, w: u32) -> usize {
    ((y * w + x) * 4) as usize
}

/// Scans for the first pixel matching `pred(sphere_hit, mesh_covered)` at the COMPOSITE
/// extent (using the extent-aware hit/coverage host models).
fn find_texel(edits: &[SdfEdit], pred: impl Fn(bool, bool) -> bool) -> Option<(u32, u32)> {
    for py in 0..COMPOSITE_H {
        for px in 0..COMPOSITE_W {
            let hit = composite_sdf_hits(edits, px, py);
            let covered = mesh_covers_pixel(px, py);
            if pred(hit, covered) {
                return Some((px, py));
            }
        }
    }
    None
}

/// Writes an `w × h` RGBA byte buffer as a 32-bpp top-down BI_RGB .bmp at `path`
/// (RGBA → the BMP's BGRA channel order; no row flip — `biHeight` is negative). Mirrors
/// the `boyko_render` test screenshot writer so the dump opens in any image viewer. The
/// caller passes an already-RGBA-normalized buffer (the swapchain R/B swap applied), so
/// the two dumps are byte-comparable regardless of the swapchain's native channel order.
fn write_bmp(path: &str, rgba: &[u8], w: u32, h: u32) -> std::io::Result<()> {
    debug_assert_eq!(
        rgba.len(),
        (w * h * 4) as usize,
        "invariant: BMP body is w*h*4 bytes"
    );
    let pixel_bytes = w * h * 4;
    let pixel_offset: u32 = 54; // 14-byte file header + 40-byte info header.
    let file_size = pixel_offset + pixel_bytes;

    let mut buf = Vec::with_capacity(file_size as usize);
    // --- BITMAPFILEHEADER (14 bytes) ---
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved1
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved2
    buf.extend_from_slice(&pixel_offset.to_le_bytes());
    // --- BITMAPINFOHEADER (40 bytes) ---
    buf.extend_from_slice(&40u32.to_le_bytes()); // biSize
    buf.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth
    buf.extend_from_slice(&(-(h as i32)).to_le_bytes()); // biHeight (negative => top-down)
    buf.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    buf.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    buf.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    buf.extend_from_slice(&pixel_bytes.to_le_bytes()); // biSizeImage
    buf.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    buf.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    // --- pixel data: RGBA -> BGRA (the ONLY channel swap; no row flip) ---
    for px in rgba.chunks_exact(4) {
        buf.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }

    std::fs::write(path, &buf)
}

/// Normalizes a swapchain readback (BGRA when `is_bgra`, else RGBA) into a contiguous
/// RGBA buffer — applying the SAME R/B handling as the golden assertion
/// ([`readback_rgb`]), so the two brick-ON / brick-OFF dumps are color-correct AND
/// byte-comparable to each other.
fn readback_to_rgba(readback: &[u8], w: u32, h: u32, is_bgra: bool) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    for (dst, src) in out.chunks_exact_mut(4).zip(readback.chunks_exact(4)) {
        let texel = [src[0], src[1], src[2], src[3]];
        let rgb = readback_rgb(texel, is_bgra);
        dst[0] = rgb[0] as u8;
        dst[1] = rgb[1] as u8;
        dst[2] = rgb[2] as u8;
        dst[3] = src[3];
    }
    out
}

/// The fixed dump path for the brick-ON (empty-skip + trilinear + clip-map) frame.
const BRICK_ON_BMP: &str = r"C:\Users\flint\AppData\Local\Temp\brick_on.bmp";
/// The fixed dump path for the brick-OFF (analytic marcher) frame.
const BRICK_OFF_BMP: &str = r"C:\Users\flint\AppData\Local\Temp\brick_off.bmp";

/// One booted windowed-present context, handed to a [`with_windowed_present`] body.
/// `window`/`ctx`/`surface` are BORROWED from `with_windowed_present`'s own stack frame
/// (which outlives the `body(..)` call and drops them in reverse-decl order — surface →
/// ctx → window — at its frame end, matching the former inline teardown tail). `swapchain`
/// and `renderer` are MOVED INTO the body so their `drop(..)` stays a real drop at its
/// original point: `renderer`'s `Drop` (`device_wait_idle`) fires before the body's
/// resource-destroy block, exactly as in the former inline code.
struct BootPresent<'a, 'ctx> {
    window: &'a mut Window,
    ctx: &'ctx VulkanContext,
    surface: &'a Surface<'ctx>,
    swapchain: Swapchain<'ctx>,
    renderer: Renderer<'ctx>,
    is_bgra: bool,
    swap_color_format: Format,
}

/// Boot the shared windowed-present topology (window + context + surface + swapchain +
/// renderer + swapchain-format detection) and run `body` against it. Any boot/format
/// failure prints a `SKIP {label}: …` line and returns WITHOUT calling `body` — the
/// pre-existing graceful-skip contract for a machine with no windowed device.
///
/// This is the single source of the prologue formerly inlined into every windowed golden
/// entry point. It is a strict superset of those prologues: the device-name / swapchain
/// diagnostics and the `image_count() >= 1` assert are stdout/soundness only and never
/// influence a rendered byte, so every caller's golden output is unchanged.
fn with_windowed_present(title: &str, label: &str, body: impl FnOnce(BootPresent<'_, '_>)) {
    let mut window = match Window::open(title, WIDTH, HEIGHT) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("SKIP {label}: cannot open a window ({e:?})");
            return;
        }
    };
    let ctx = match VulkanContext::boot(InstanceConfig { enable_validation: true, windowed: true }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP {label}: windowed Vulkan unavailable ({e:?})");
            return;
        }
    };
    println!("Vulkan device (windowed): {}", ctx.device_name());
    // Validation is the soundness oracle, NOT a render-output dependency: a context booted
    // with `BOYKO_DISABLE_VALIDATION` (the layer DLL crashes the MinGW process on this box)
    // still drives the pixel gate.
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — pixel gate still runs");
    }
    let caps = ctx.device_caps();
    assert!(
        caps.gbuffer_storage_format_ok,
        "a booted context must support STORAGE_IMAGE on the G-buffer format"
    );

    // SAFETY: `window` outlives the surface — both live on this stack frame and the surface is
    // dropped when this function returns, before `window`; its HWND/HINSTANCE stay live for the
    // surface's whole lifetime.
    let surface = match unsafe { Surface::new(&ctx, window.hinstance(), window.hwnd()) } {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP {label}: surface creation failed ({e:?})");
            return;
        }
    };
    let swapchain = match Swapchain::new(&ctx, &surface, window.width(), window.height()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP {label}: swapchain creation failed ({e:?})");
            return;
        }
    };
    assert!(swapchain.image_count() >= 1, "swapchain must expose >= 1 image");
    println!(
        "swapchain: {} images, extent {}x{}, format {}",
        swapchain.image_count(),
        swapchain.extent().width,
        swapchain.extent().height,
        swapchain.format()
    );

    if swapchain.extent().width < COMPOSITE_W || swapchain.extent().height < COMPOSITE_H {
        eprintln!(
            "SKIP {label}: swapchain extent {}x{} is smaller than the {COMPOSITE_W}x{COMPOSITE_H} composite",
            swapchain.extent().width,
            swapchain.extent().height,
        );
        return;
    }

    let Some(is_bgra) = swapchain_readback_is_bgra(swapchain.format()) else {
        eprintln!("SKIP {label}: swapchain format has no host-decodable UNORM byte order");
        return;
    };
    let Some(swap_color_format) = (match swapchain.format() {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(Format::B8G8R8A8Unorm),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(Format::R8G8B8A8Unorm),
        _ => None,
    }) else {
        eprintln!("SKIP {label}: swapchain format has no basic-slice Format variant");
        return;
    };

    let renderer =
        Renderer::new(&ctx, &surface, &swapchain).expect("renderer (command pool + sync) creation");

    body(BootPresent {
        window: &mut window,
        ctx: &ctx,
        surface: &surface,
        swapchain,
        renderer,
        is_bgra,
        swap_color_format,
    });
    // window, ctx, surface remain owned here and drop at this function's frame end in
    // reverse-decl order (surface → ctx → window), matching the former inline teardown tail.
}

#[test]
fn windowed_gbuffer_composite_present_is_validation_clean_and_renders_composite() {
    with_windowed_present(
        "boyko_rhi_vulkan gbuffer window",
        "windowed_gbuffer_present",
        body_windowed_gbuffer_composite,
    );
}

fn body_windowed_gbuffer_composite(bp: BootPresent<'_, '_>) {
    let BootPresent { window, ctx, surface, mut swapchain, mut renderer, is_bgra, swap_color_format } =
        bp;

    let device: &VulkanContext = ctx;
    let sdf = sphere_scene();

    // --- Pick the three discriminator texels host-side, BEFORE any GPU run. ---
    let (ax, ay) = find_texel(&sdf, |hit, covered| hit && covered)
        .expect("invariant: some pixel must be over BOTH the sphere and the quad (mesh-occludes-SDF)");
    let (bx, by) = find_texel(&sdf, |hit, covered| hit && !covered)
        .expect("invariant: some pixel must be over the sphere but NOT the quad (SDF)");
    let (dx, dy) = find_texel(&sdf, |hit, covered| !hit && !covered)
        .expect("invariant: some pixel must be over neither (background)");

    let depth_at = |px, py| expected_mesh_depth(px, py);
    // The mesh-occludes (a) + background (d) texels are mask == 0 PASS-THROUGH arms — the
    // resolve emits `base` byte-identically (the 0%-gate), so the old inline composite is
    // still the truth there. PBR MVP-2 only changed the SDF-LIT arm.
    // Live-computed at the COMPOSITE extent via the extent-aware ORTHO oracle, so the golden
    // re-blesses automatically at 512×512 (the frozen 64×64 `golden_composite_pixel` is the
    // `_ex` forwarder at `(SDF_IMG_W, SDF_IMG_H)`; here we forward at `(COMPOSITE_W, COMPOSITE_H)`).
    // Render P5: a_want (mesh-occludes) is now a RASTER-PBR producer (mask == 1) — computed below
    // alongside b_want via the PBR oracle, NOT the old flat MESH_COLOR pass-through.
    let d_want =
        golden_composite_pixel_ex(&sdf, depth_at(dx, dy), dx, dy, COMPOSITE_W, COMPOSITE_H, CompositeCamera::Ortho);
    // The SDF-LIT texel (b) is now FULL Cook-Torrance (the owner-acknowledged behavioral
    // change, PBR plan call F), NOT the old `base*vis` composite — so its golden comes from
    // the PBR oracle (`golden_deferred_resolve ∘ golden_marcher_attributes`) with the SAME
    // marcher params the windowed present uses (lighting ON, default light, DEFAULT omega).
    let materials = [GoldenMaterial::default()];
    let b_flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let b_attrs = golden_marcher_attributes(
        &sdf, &materials, depth_at(bx, by), bx, by, COMPOSITE_W, COMPOSITE_H, CompositeCamera::Ortho,
        DEFAULT_MARCHER_OMEGA, b_flags, DEFAULT_LIGHT_DIR,
    );
    let (_, b_rd) = composite_pixel_ray(bx, by, COMPOSITE_W, COMPOSITE_H, CompositeCamera::Ortho);
    let b_want = golden_deferred_resolve(b_attrs, b_rd, &materials);

    // Render P5: the mesh-occludes (a) texel is a raster-PBR producer (mask == 1) — model it
    // through the SAME PBR oracle as the SDF-lit texel (golden_marcher_attributes' has_mesh arm
    // emits the raster mesh attrs; golden_deferred_resolve runs full Cook-Torrance).
    let (_, a_rd) = composite_pixel_ray(ax, ay, COMPOSITE_W, COMPOSITE_H, CompositeCamera::Ortho);
    let a_attrs = golden_marcher_attributes(
        &sdf, &materials, depth_at(ax, ay), ax, ay, COMPOSITE_W, COMPOSITE_H, CompositeCamera::Ortho,
        DEFAULT_MARCHER_OMEGA, b_flags, DEFAULT_LIGHT_DIR,
    );
    let a_want = golden_deferred_resolve(a_attrs, a_rd, &materials);
    assert!(
        !goldens_close(a_want, b_want),
        "invariant: the raster-PBR mesh and the SDF lit color must differ beyond +/-{CHANNEL_TOL}"
    );
    assert!(
        !goldens_close(a_want, d_want),
        "invariant: MESH_COLOR and BACKGROUND must differ beyond +/-{CHANNEL_TOL}"
    );
    assert!(
        !goldens_close(b_want, d_want),
        "invariant: the SDF lit color and BACKGROUND must differ beyond +/-{CHANNEL_TOL}"
    );

    // === Build the P1c on-screen G-buffer scene's STATIC inputs (the GBufferScene). ===

    // The ONE SDF edit authority (principle 0): the field every brick resource bakes from. Built
    // once from the same `sdf` edits the marcher's edit-list carries, so the brick cache mirrors the
    // analytic field exactly (no parallel field store). `bump_gen()` marks it dirty-baked.
    let field = {
        use boyko_sdf_math::SdfEditField;
        let mut f = SdfEditField::new();
        for e in &sdf {
            assert!(f.push(*e), "windowed scene must fit MAX_SDF_EDITS");
        }
        f.bump_gen();
        f
    };

    // SDF brick-atlas campaign — the WINDOWED ACTIVATION. The full 3-level clip-map, baked from the
    // authority and centered at the WORLD ORIGIN (NOT the camera): the demo scene is small + fixed
    // (the sphere lives in ~[-0.5, 0.5]³, well inside level 0's [-4, 4]³ box), and the camera ORBITS
    // a fixed scene rather than translating through the world, so an origin-centered clip-map is
    // STATIC — no per-frame re-center (the toroidal camera-follow is campaign M5). Level 0 covers the
    // whole scene, so `brick_levels = 3` and `= 1` render the same here; the full 3-level path is
    // used to exercise exactly what the owner asked for (empty-skip + trilinear + clip-map LOD).
    //
    // `BrickClipmap::create` bakes every level's atlas + seeds every level's pointer grid + ends each
    // upload in SHADER_READ (the offscreen barrier discipline), so the cache is sample-ready before
    // the first present. The scene is static (the orbit moves only the camera, which is NOT in the
    // field), so no per-frame rebake is needed — a one-time startup bake suffices (an edit loop would
    // call `rebake_dirty_all` on the authority's `gen` change; there is none here).
    let clipmap = BrickClipmap::create(ctx, &field, [0.0, 0.0, 0.0])
        .expect("M4 brick clip-map (windowed activation) — create + bake every level + upload");

    // Level 0's empty-skip grid geometry (the marcher's `lvl == 0` arm indexes binding 9 with it).
    // The clip-map's level-0 grid IS the fine `default_near_field` (`16³ @ 0.5`, origin `[-4,-4,-4]`)
    // — see `brick_atlas::level_empty_skip_grid` — so the activation's `with_brick` uniforms come
    // from `PointerGrid::default_near_field` to match the bound binding-9 SSBO + the host oracle.
    let level0_grid = PointerGrid::default_near_field();
    let brick_on = BrickActivation {
        grid_origin: level0_grid.origin,
        grid_dims: level0_grid.dims,
        brick_world: level0_grid.brick_world,
        levels: BRICK_LEVELS as u32,
    };

    // The edit-list StorageBuffer (binding 0), host-seeded ONCE. Over-allocated to the
    // full `EDITLIST_BUFFER_WORDS` (the encoder debug-asserts that size); the marcher
    // only reads the header + edit array.
    let edit_list = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, &sdf);
        let mapped = RhiDevice::buffer_mapped_ptr(device, &edit_list)
            .expect("host-visible edit-list buffer is mapped");
        write_words(mapped, &header);
    }

    // The camera/extent UNIFORM buffer (binding 5), host-seeded ONCE at the golden 64×64
    // ORTHO extent (the composite size — NOT the swapchain extent) for bit-exact rays.
    //
    // SDF brick-atlas M4 (clip-map LOD, Slice C): the b5 UBO is sized to `B5_CAMERA_UBO_BYTES_M4`
    // (224 B) — the 80-byte camera block + the `BRICK_LEVELS`-level `M4GridParams` array tail at
    // `M2_GRID_PARAMS_OFFSET` (80). The widened marcher cbuffer declares 224 B. The tail holds the
    // ACTIVATED clip-map's baked per-level params (`clipmap.params()`); the brick-ON 'B' toggle reads
    // them across all 3 levels, the OFF path (`brick_levels = 1`) reads only level 0.
    // The camera/extent UBO RING (binding 5): one host-coherent slot per in-flight frame. Every
    // slot is seeded IDENTICALLY here and never rewritten in this offscreen path, so the output
    // stays byte-identical to the pre-ring single-buffer version; the ring only matters for the
    // interactive viewer (which writes `camera_ring[frame_index]` per frame, the lock-free fix).
    let camera_ring: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
        RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: B5_CAMERA_UBO_BYTES_M4 as u64,
                usage: BufferUsage::UNIFORM,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("camera uniform buffer")
    });
    {
        let pc = CompositePushConstants::ortho(COMPOSITE_W, COMPOSITE_H);
        assert_eq!(pc.count, PIXELS);
        let bytes = pc.as_bytes();
        debug_assert_eq!(bytes.len(), M2_GRID_PARAMS_OFFSET, "camera block must be 80 B (offset of the M4 tail)");
        // The M4 array tail at offset 80: the clip-map's baked per-level snapped origins (the values
        // the level atlases were baked at — `M4GridParams::camera_centered([0,0,0])`). The marcher's
        // clip-map ladder reads `m2_levels[0..brick_levels]` from here; on the brick-ON path (the 'B'
        // toggle, `brick_levels = 3`) it samples real per-level params, and on the OFF path
        // (`brick_levels = 1`) it reads only level 0 — which, origin-centered, equals the M2 near-field.
        let m4 = *clipmap.params();
        let m4_bytes = m4.as_ubo_bytes();
        debug_assert_eq!(M2_GRID_PARAMS_OFFSET + m4_bytes.len(), B5_CAMERA_UBO_BYTES_M4);
        for slot in &camera_ring {
            let mapped = RhiDevice::buffer_mapped_ptr(device, slot)
                .expect("host-visible uniform buffer is mapped");
            // SAFETY: `mapped` points to `B5_CAMERA_UBO_BYTES_M4` (224) mapped host-coherent bytes; the
            // 80-byte camera block is written at offset 0 and the (224-80)-byte M4 tail at offset 80 —
            // together exactly 224 in-bounds bytes, disjoint. No GPU work is in flight yet (the present
            // loop follows), so the writes are unsynchronized-safe. Every ring slot is seeded with the
            // SAME bytes (byte-identical to the pre-ring single buffer).
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
                core::ptr::copy_nonoverlapping(
                    m4_bytes.as_ptr(),
                    mapped.as_ptr().add(M2_GRID_PARAMS_OFFSET),
                    m4_bytes.len(),
                );
            }
        }
    }

    // P4b: the coarse-cull tile StorageBuffer (vocab binding 6), sized to the full tile
    // grid at the COMPOSITE extent (NOT the swapchain extent — the marcher dispatches +
    // the camera UBO `count` are sized to the 64×64 composite). The windowed path runs
    // the marcher with the coarse cull gated OFF (coarse_enabled=0), so its contents are
    // never read — but the marcher shader DECLARES binding 6, so a VALID descriptor must
    // be bound. Allocated once; bound (borrowed) into the vocabulary set; never written.
    let (tw, th) = tile_grid_extent(COMPOSITE_W, COMPOSITE_H);
    let tiles_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (tw as u64) * (th as u64) * (TILE_BOUND_BYTES as u64),
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("P4b coarse-cull tile-bound storage buffer (vocab binding 6)");

    // PBR MVP-2: the material table SSBO (vocab binding 7 + resolve binding 4). The windowed
    // scene's edits carry no material id (center.w == 0), so every SDF hit picks material 0 —
    // the default mid-gray dielectric. One 48-B element (12 words; mirrors MaterialGpu).
    let material_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (DEFAULT_MATERIAL_TABLE.len() as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("PBR material table storage buffer (vocab binding 7 / resolve binding 4)");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &material_table)
            .expect("host-visible material table is mapped");
        write_words(mapped, &DEFAULT_MATERIAL_TABLE);
    }

    // The brick bindings 9..=14 are the ACTIVATED clip-map's REAL per-level resources (created above
    // from the authority): level `L`'s empty-skip pointer grid at @9/@11/@13 and its brick atlas +
    // sampler at @10/@12/@14. This REPLACES the prior "single atlas duplicated at every level slot"
    // OFF scaffold with the genuine 3-level cache — the SAME binding discipline the offscreen
    // RTX-verified `run_gbuffer_hybrid_m4` uses. The descriptors are static (the clip-map is baked
    // once + origin-centered, never re-snapped), so they are written ONCE into the vocabulary set;
    // the per-frame 'B' toggle flips only the push gates. On the OFF push (`brick_levels = 1`) the
    // marcher reads only level 0's bindings (9/10) — bound-but-unread above that.

    // Lighting L0a: the light table SSBO (resolve binding 6). For this test the table is
    // seeded host-visible with the DEGENERATE table (the 0%-gate anchor); a production
    // path would mint it DEVICE-LOCAL (TRANSFER_DST | STORAGE) and seed via
    // `upload_initial`, then re-upload on-change via the async recorder (rung L0-r0). The
    // resolve reads the header (count + exposure) + the table.
    let light_table_bytes = (DEGENERATE_LIGHT_TABLE.len() as u64) * 4;
    let light_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: light_table_bytes,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("Lighting-L0 light table storage buffer (resolve binding 6)");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_table)
            .expect("host-visible light table is mapped");
        write_words(mapped, &DEGENERATE_LIGHT_TABLE);
    }
    // The host-coherent STAGING source for the async re-upload (rung L0-r0). Seeded with
    // the SAME degenerate table; the windowed present path runs with `light_dirty == false`
    // (the static-scene 0%-gate: the recorder records NO copy), so this is the dormant
    // source kept valid for the dirty-frame path.
    let light_staging = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: light_table_bytes,
            usage: BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("Lighting-L0 light table staging buffer (rung L0-r0)");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_staging)
            .expect("host-visible light staging is mapped");
        write_words(mapped, &DEGENERATE_LIGHT_TABLE);
    }

    // The mesh quad's vertex buffer (host-visible).
    let vertices = quad_vertices();
    let vertex_bytes = core::mem::size_of_val(&vertices) as u64;
    let vertex_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible vertex buffer");
    {
        let vb_ptr = RhiDevice::buffer_mapped_ptr(device, &vertex_buffer)
            .expect("host-visible vertex buffer is mapped");
        // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes;
        // `vertices` is a distinct stack array of `vertex_bytes` bytes; the write
        // completes before any submit references the buffer (host-coherent: no flush).
        unsafe {
            core::ptr::copy_nonoverlapping(
                vertices.as_ptr().cast::<u8>(),
                vb_ptr.as_ptr(),
                vertex_bytes as usize,
            );
        }
    }

    // The depth sampler (bound at vocab binding 1; ignored by the marcher's `.Load`).
    let depth_sampler = RhiDevice::create_sampler(device, &SamplerDesc::default())
        .expect("depth sampler (ignored by .Load)");
    // The present-blit sampler (nearest/clamp for a 1:1 albedo sample).
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
    .expect("present nearest/clamp sampler");

    // The mesh-MRT G-buffer producer graphics pipeline (Render P5-r0): rung-3 vertex
    // layout + 64-byte VERTEX MVP + 3 G-buffer color formats + a declared depth format.
    let vs = RhiDevice::create_shader_module(device, MRT_VS_SPV.as_words())
        .expect("mesh-MRT vertex shader module");
    let fs = RhiDevice::create_shader_module(device, MRT_FS_SPV.as_words())
        .expect("mesh-MRT fragment shader module");
    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
    ];
    // M1: the per-instance model SSBO layout + 1-element identity dummy + its bind group.
    // The gbuffer VS statically references `StructuredBuffer<InstanceModelCol> instances` at
    // set 0 binding 0, so the pipeline layout MUST declare it and every draw MUST bind a valid
    // buffer; the legacy merged draw (`use_model_matrix == 0`) never reads it (bound-but-unread).
    let (instance_layout, instance_buffer, instance_bind_group) = create_identity_instance(device);
    let raster_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            // Render P5-r0: 3 MRT color formats = the G-buffer RGBA8 lanes; the production
            // `record_gbuffer` binds albedo/normal/material as the 3 MRT attachments.
            color_formats: &[RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT],
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
        },
    )
    .expect("depth-prepass graphics pipeline");

    // The P1b marcher: the vocabulary bind-group layout + the compute pipeline.
    let cs = RhiDevice::create_shader_module(device, sdf_gbuffer_composite_spirv())
        .expect("P1b G-buffer marcher compute shader module");
    // P4b: binding 6 = the coarse-cull tile StorageBuffer. The marcher shader DECLARES
    // it unconditionally, so the layout must carry it (and a valid buffer must be bound)
    // even though the windowed path runs with the coarse cull gated OFF (coarse_enabled=0).
    let vocab_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // PBR MVP-2: the material table SSBO @7 (the marcher fetches `base_color`).
        BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Lighting L0b: the gViewT STORAGE image @8 (the marcher stores the surface `t`).
        BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        // M1: the empty-skip `PointerGrid` SSBO @9. The recompiled marcher SPIR-V statically
        // references `StructuredBuffer<uint> PointerGrid : register(t9)` inside the
        // runtime-gated empty-skip branch (DXC does NOT dead-strip it despite `brick_enabled`),
        // so the layout MUST declare binding 9 — a VALID StorageBuffer descriptor must be bound
        // even though the windowed path runs the skip OFF (`brick_enabled == 0`), or
        // `vkCreateComputePipelines` / `vkCmdDispatch` trip VUID-…-layout-07988 / -08114.
        BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // M2: the brick-atlas combined image+sampler @10. The recompiled marcher SPIR-V
        // statically references `Texture3D BrickAtlas : register(t10)` +
        // `SamplerState BrickSampler : register(s10)` (collapsed to ONE combined descriptor by
        // DXC) inside the runtime-gated `brick_trilinear` branch (NOT dead-stripped despite the
        // gate), so the layout MUST declare binding 10 — a VALID combined image+sampler must be
        // bound even though the windowed path runs the trilinear path OFF (`brick_trilinear == 0`,
        // bound-but-unread, byte-identical output), or the layout VUIDs trip (the M1 binding-9
        // lesson at the next slot).
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        // M4 clip-map LOD (Slice C): the LEVEL-1 + LEVEL-2 brick bindings. The recompiled marcher
        // SPIR-V statically references `PointerGrid1`@t11 + `BrickAtlas1`@t12 + `PointerGrid2`@t13 +
        // `BrickAtlas2`@t14 inside the runtime level branch-ladder, so the layout MUST declare all four
        // — bound-but-unread on the windowed OFF/N=1 path (`brick_levels == 1` takes only the lvl==0 arm).
        // 6 brick bindings total (9..=14) under the 16-binding cap.
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 12, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 13, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 14, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        // MDF Stage-2c: the dedicated dense mesh-SDF shadow-caster texture @15 (the 16th / last vocab
        // entry under the 16-binding cap). The recompiled marcher SPIR-V statically references
        // `MeshSdf`@t15 + `MeshSdfSampler`@s15 inside the runtime-gated `mesh_sdf_enabled` branch, so
        // the layout MUST declare binding 15 — a VALID combined image+sampler must be bound even on
        // the OFF path (`mesh_sdf_enabled == false` → bound-but-unread, byte-identical output).
        BindGroupLayoutEntry { binding: 15, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
    ];
    let vocab_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &vocab_entries },
    )
    .expect("P1b vocabulary bind-group layout");
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
    .expect("P1b G-buffer marcher compute pipeline");

    // The deferred RESOLVE (`deferred_pbr.comp`): 8 bindings (≤ 12) { gAlbedo @0, gNormal
    // @1, gMaterial @2, lit @3 (STORAGE images), the material SSBO @4, the camera UBO @5,
    // the Lighting-L0 light table SSBO @6, the Lighting-L0b gViewT STORAGE image @7 }. The
    // resolve reads the extent + the per-pixel view direction from the camera UBO, the
    // lights from the table (L0a), and (L0b) `gViewT` to reconstruct `P` for point/spot.
    let resolve_cs = RhiDevice::create_shader_module(device, deferred_pbr_spirv())
        .expect("deferred resolve compute shader module");
    let resolve_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Lighting L0b: the gViewT STORAGE image @7 (the resolve reads it under `mask == 1`).
        BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        // Lighting L1: the ClusterGrid @8 + LightIndexList @9 SSBOs (read on the cluster path;
        // L1 is OFF here, so they bind the light table as a harmless valid placeholder).
        BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // P6 R1: the SDF edit-list `Buf` SSBO @10 (the resolve's `sdf_soft_shadow_ranged`
        // analytic march reads it read-only; the SAME buffer the marcher binds @0).
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Render P7: the SSAO term `gSsao` STORAGE image @11 (the resolve reads it under
        // `ssao_mode != 0`; OFF here, so it is a bound-but-unread descriptor). The production
        // `GBufferTargets` binds the SSAO image at @11, so the resolve layout MUST declare it or
        // bind-group creation trips the entry-count check (the P6 R1 binding-10 discipline).
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        // CSM Increment 1b (Rung A): the cascade combined map+sampler @12 + the cascade UBO @13.
        // The production `GBufferTargets::create` binds the scene's cascade trio at @12/@13, so the
        // resolve layout MUST declare them (the recompiled resolve STATICALLY references `gCsm` +
        // `CsmCascades`). 14 bindings ≤ the 16-binding cap. `csm_mode == 0` on the golden presents
        // → bound-but-unread (the 0%-gate).
        BindGroupLayoutEntry { binding: 12, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 13, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        // Shadow Phase 5 Inc-1-GPU: the sparse spot/point shadow-atlas combined map+sampler @14 + the
        // atlas UBO @15. The production `GBufferTargets::create` binds the scene's atlas trio at
        // @14/@15, so the resolve layout MUST declare them (the recompiled resolve STATICALLY
        // references `gShadowAtlas` + `ShadowAtlas`). 16 bindings == the 16-binding cap (16/16);
        // `punctual_shadow_mode == 0` on the golden presents → bound-but-unread (the 0%-gate).
        BindGroupLayoutEntry { binding: 14, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 15, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        // SDFDDGI I0: the DDGI probe-irradiance combined image @16 + depth combined image @17 + the
        // `ResolvedDdgi` grid UBO @18 (bound-but-unread; the recompiled resolve STATICALLY references
        // `gDdgiIrr`/`gDdgiDepth`/`ResolvedDdgi`, so the layout MUST declare them). Exact-fill 19/19.
        BindGroupLayoutEntry { binding: 16, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 17, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 18, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        // Textured-PBR T6a (the critic's C1 fix): the SOFTWARE-ONLY `gPbr` STORAGE image @19.
        // `GBufferTargets::create` now allocates `gPbr` UNCONDITIONALLY (both feature legs) and
        // `DeferredSets::build`'s software resolve-set loop appends it past the shared 19 —
        // the layout MUST declare it too, or `create_bind_group`'s entry-count check trips (P1a).
        BindGroupLayoutEntry { binding: 19, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
    ];
    let resolve_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &resolve_entries },
    )
    .expect("deferred resolve bind-group layout");
    let resolve_pipeline = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &resolve_cs,
            entry: c"main",
            // The resolve shader pushes NO constants, but `create_compute_pipeline` requires
            // a non-empty (multiple-of-4) push range; declare the shared range (unused).
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&resolve_layout),
            spec_constants: &[],
        },
    )
    .expect("deferred resolve compute pipeline");

    // CSM Increment 1b (Rung A): the cascade trio + depth-only pipeline. This golden present is the
    // OFF path (`csm: None`, `csm_mode == 0`), so the trio is bound-but-unread at resolve @12/@13.
    let csm = CsmSceneResources::create(device, &instance_layout);

    // The present-blit: one COMBINED_IMAGE_SAMPLER layout + the fullscreen-sample pipeline.
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
    .expect("present-blit bind-group layout (one COMBINED_IMAGE_SAMPLER)");
    let sample_vs = RhiDevice::create_shader_module(device, SAMPLE_VS_SPV.as_words())
        .expect("fullscreen vertex shader module");
    let sample_fs = RhiDevice::create_shader_module(device, SAMPLE_FS_SPV.as_words())
        .expect("fullscreen fragment shader module");
    let present_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &sample_vs,
            vertex_entry: c"main",
            fragment_module: &sample_fs,
            fragment_entry: c"main",
            color_formats: &[swap_color_format],
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
    .expect("present-blit fullscreen-sample pipeline (swapchain color format)");

    // The shader modules are consumed by pipeline creation; destroy them now.
    // SAFETY: every module was created on `ctx` above + is no longer needed once its
    // pipeline is created (the pipeline holds its own compiled code); each is destroyed
    // exactly once.
    unsafe {
        RhiDevice::destroy_shader_module(device, sample_fs);
        RhiDevice::destroy_shader_module(device, sample_vs);
        RhiDevice::destroy_shader_module(device, resolve_cs);
        RhiDevice::destroy_shader_module(device, cs);
        RhiDevice::destroy_shader_module(device, fs);
        RhiDevice::destroy_shader_module(device, vs);
    }

    let mvp = ortho_mvp_bytes();
    let mut scene = GBufferScene {
        raster_pipeline: &raster_pipeline,
        vertex_buffer: &vertex_buffer,
        vertex_count: vertices.len() as u32,
        mvp,
        // M1: the legacy merged draw binds the 1-element identity instance SSBO at set 0
        // (bound-but-unread — the `use_model_matrix == 0` push selects the VS's legacy arm).
        instance_bind_group: &instance_bind_group,
        marcher: &marcher,
        vocab_layout: &vocab_layout,
        edit_list: &edit_list,
        camera_ring: &camera_ring,
        tiles_buffer: &tiles_buffer,
        // Brick bindings 9..=14: the ACTIVATED clip-map's REAL per-level resources. Level 0's grid +
        // atlas at @9/@10, level 1 at @11/@12, level 2 at @13/@14 — the genuine 3-level cache (NOT the
        // old "level-0 duplicated" OFF scaffold). The marcher samples level `L`'s grid/atlas on the
        // ON push; on the OFF push (`brick_levels = 1`) it reads only level 0 (9/10), with 11..14
        // bound-but-unread.
        pointer_grid: clipmap.grid_buffer(0),
        atlas: clipmap.atlas(0).texture(),
        atlas_sampler: clipmap.sampler(0),
        level_grids: [clipmap.grid_buffer(1), clipmap.grid_buffer(2)],
        level_atlases: [clipmap.atlas(1).texture(), clipmap.atlas(2).texture()],
        level_atlas_samplers: [clipmap.sampler(1), clipmap.sampler(2)],
        // MDF Stage-2c (binding 15): a non-MDF scene binds the brick atlas (level 0) as a benign
        // placeholder + gates the mesh-shadow path OFF — the texture is bound-but-unread (the R2
        // contract: a VALID descriptor must be bound, the read is gated by `mesh_sdf_enabled`).
        mesh_sdf: clipmap.atlas(0).texture(),
        mesh_sdf_sampler: clipmap.sampler(0),
        mesh_sdf_enabled: false,
        depth_sampler: &depth_sampler,
        material_table: &material_table,
        light_table: &light_table,
        light_staging: &light_staging,
        light_upload_bytes: light_table_bytes,
        // Static-scene 0%-gate: the table is seeded once (host-visible above); no
        // on-change re-upload this run, so the recorder records NO copy/barrier (the
        // command stream is byte-identical to before L0-r0).
        light_dirty: false,
        // Lighting L1 is OFF for the on-screen demo (no cluster cull wired): the cull
        // pipeline + cluster SSBOs are absent, so the recorder skips the cull pass entirely
        // and the resolve's `clusters_enabled` header gate (0) loops the flat table — the L1
        // OFF / 0%-gate. The resolve set's @8/@9 bind the light table as a harmless valid
        // placeholder (never read on the OFF path; see GBufferTargets::create).
        cluster_cull: None,
        cull_layout: None,
        cluster_grid: None,
        light_index: None,
        light_index_alloc: None,
        cluster_cull_push: [0u8; 16],
        cluster_count: 0,
        resolve_pipeline: &resolve_pipeline,
        resolve_layout: &resolve_layout,
        #[cfg(feature = "hwrt")]
        resolve_pipeline_hwrt: None,
        #[cfg(feature = "hwrt")]
        resolve_layout_hwrt: None,
        #[cfg(feature = "hwrt")]
        resolve_tlas_hwrt: None,
        // Rung 1b: the HWRT resolve is OFF in this harness (`resolve_tlas_hwrt: None`), so the
        // shadow-params UBO ring is bound by NO set — a benign valid placeholder (the whole cascade
        // UBO ring, a per-FIF `[BoundBuffer; FRAMES_IN_FLIGHT]`, host-coherent + >= 16 B/slot)
        // satisfies the field type without ever being read.
        #[cfg(feature = "hwrt")]
        ray_shadow_ubo: csm.csm_ring(),
        present_pipeline: &present_pipeline,
        present_layout: &present_layout,
        present_sampler: &present_sampler,
        dispatch_group_count_x: group_count_x(),
        // The brick A/B toggle's STARTING state (flipped live by the 'B' key in the present loop).
        // `Some(brick_on)` boots the empty-skip + trilinear/cubic surface cache + 3-level clip-map ON;
        // `None` boots the analytic (OFF) path. RTX-verified byte-identical in this scene, so either
        // start looks the same on screen.
        brick: if BRICK_START_ON { Some(brick_on) } else { None },
        // P0 coarse tile-cull: OFF for this existing golden present (the 0%-gate — NO coarse
        // dispatch / barrier recorded, `coarse_enabled == 0`, byte-identical to the pre-P0 stream).
        // The dedicated `p0_windowed_coarse_cull_matches_uncull` test drives the ON vs OFF readback.
        coarse: None,
        // The on-screen present's coarse-cull mode (a don't-care here since `coarse == None`):
        // `EmptySkipOnly` is the lit-transparent on-screen cull (EMPTY-skip only, no `near_t` seed).
        coarse_mode: CoarseMode::EmptySkipOnly,
        // The on-screen demo renders with soft shadows (A1) + AO (A2) — its existing lighting
        // validation is unchanged (byte-identical push to the pre-`lighting_flags`-field stream).
        lighting_flags: LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO,
        // The legacy head-on shadow direction (`[0,0,1]`) — byte-identical to the pre-`light_dir`-
        // field marcher push (this golden present asserts the existing stream).
        light_dir: DEFAULT_LIGHT_DIR,
        // Render P7: SSAO OFF (the default) — NO SSAO pass recorded, byte-identical to the pre-P7
        // stream (the 0%-gate). These golden/cull-comparison presents assert the existing stream.
        ssao: None,
        // M3: the LEGACY merged draw (no instanced mesh — an EMPTY batch slice) —
        // `record_gbuffer` keeps `vkCmdDraw(vertex_count, 1, 0, 0)`, byte-identical to the
        // pre-M2 stream.
        mesh_draw: &[],
        // CSM Increment 1b (Rung A): the cascade trio bound at resolve @12/@13 (ALWAYS), the depth
        // pass OFF (`csm: None`). On this golden present `csm_mode == 0`, so the resolve's PCF
        // sample never runs and the trio is bound-but-unread (the 0%-gate — byte-identical pixels).
        csm_cascade_texture: &csm.cascade,
        csm_compare_sampler: &csm.sampler,
        csm_cascade_ring: csm.csm_ring(),
        csm: None,
        // Shadow Phase 5 Inc-1-GPU: the atlas trio bound at resolve @14/@15 (ALWAYS), the punctual
        // depth pass OFF (`atlas_punctual: None`). On this golden present `punctual_shadow_mode == 0`,
        // so the resolve's spot PCF sample never runs and the trio is bound-but-unread (the 0%-gate —
        // byte-identical pixels).
        shadow_atlas_texture: &csm.atlas,
        shadow_atlas_sampler: &csm.atlas_sampler,
        shadow_atlas_ubo: &csm.atlas_ubo,
        // SDFDDGI I1: the 3 DDGI resolve bindings (@16/@17/@18) now bind the REAL probe atlas. The GI
        // gate is OFF on every golden present (LightBuf word-7 bit 4 == 0), so the resolve's probe
        // sample never runs and all three are bound-but-unread (the 0%-gate — byte-identical pixels).
        // I1 severs the I0a dummy: the irradiance/depth atlases are the dedicated
        // `B10G11R11_UFLOAT`/`R16G16_SFLOAT` `Texture2DArray`s, each sampled with a dedicated LINEAR
        // (non-comparison) sampler — closing the VUID trap (the old CSM COMPARISON sampler on a
        // non-Dref SampleLevel was UB). The grid UBO is the dedicated zeroed `ddgi_ubo`.
        ddgi_irr_texture: csm.ddgi_atlas.irradiance(),
        ddgi_irr_sampler: csm.ddgi_atlas.sampler(),
        ddgi_depth_texture: csm.ddgi_atlas.depth(),
        ddgi_depth_sampler: csm.ddgi_atlas.sampler(),
        ddgi_grid_ubo: &csm.ddgi_ubo,
        // SDFDDGI I2: the probe-update pass is OFF in these harness scenes (the GI-OFF 0%-gate).
        // `ddgi_update = None` ⇒ no update RDG pass / dispatch / barrier is recorded. The
        // classification / ray-table / update-UBO handles are supplied so the RDG sink can resolve
        // them (unread while the pass is off); the ray-table + update-UBO reuse the bound-but-unread
        // `ddgi_ubo` as a placeholder buffer (never read on the OFF path — this harness does not arm
        // the update pass; a bench/host that arms it supplies real dedicated buffers).
        ddgi_update: None,
        ddgi_classification: csm.ddgi_atlas.classification(),
        ddgi_ray_table: &csm.ddgi_ubo,
        ddgi_update_ubo: &csm.ddgi_ubo,
        atlas_punctual: None,
        // Pillar B B3: the interpolation pre-pass is OFF for every dump/offscreen golden — the
        // raster VS reads the hand-affine SSBO directly (byte-identical command stream + pixels).
        interp: None,
        // HW-RT rung R0: GPU timing OFF (byte-identical command stream).
        gpu_timing: None,
        // HW-RT rung R2a-3: the per-frame TLAS pack + build OFF (byte-identical command stream).
        #[cfg(feature = "hwrt")]
        tlas: None,
        // HW-RT rung 3a: the spatial (à-trous) RT soft-shadow denoise OFF (byte-identical).
        #[cfg(feature = "hwrt")]
        shadow: None,
        // HW-RT rung 3a: the STABLE denoise-set-build signals — all OFF in this harness (no denoise
        // sets built; byte-identical).
        #[cfg(feature = "hwrt")]
        resolve_layout_denoise_hwrt: None,
        #[cfg(feature = "hwrt")]
        atrous_layout_denoise_hwrt: None,
        #[cfg(feature = "hwrt")]
        shadow_denoise_enabled: false,
        #[cfg(feature = "hwrt")]
        shadow_denoise_final_is_vis2: false,
        // Rung-3b step 5a: the temporal-MV mesh path — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        temporal_enabled: false,
        #[cfg(feature = "hwrt")]
        raster_pipeline_mv: None,
        #[cfg(feature = "hwrt")]
        mv_bind_group: None,
        // F8-mv: the combined MV+PM mesh path — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        raster_pipeline_mvpm: None,
        #[cfg(feature = "hwrt")]
        mvpm_bind_group: None,
        // Rung-3b step 5b: the SDF motion-vector VIS path — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        vis_mv_pipeline: None,
        #[cfg(feature = "hwrt")]
        vis_mv_layout: None,
        #[cfg(feature = "hwrt")]
        motion_cam_ubo_ring: None,
        // Rung-3b step 6: the temporal reproject layout — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        temporal_layout: None,
        // Asset-streaming plan F8: PER_INSTANCE_MATERIAL is OFF in this low-level RHI harness
        // (no ECS gather / material store exists here) — byte-identical to the pre-F8 stream.
        pm_enabled: false,
        raster_pipeline_pm: None,
        pm_bind_group: None,
        // Textured-PBR T6c: TEXTURED is OFF in this low-level RHI harness (no ECS gather /
        // texture asset store exists here) — byte-identical to the pre-T6c stream.
        tex_enabled: false,
        raster_pipeline_tex: None,
        tex_bind_group: None,
        bindless_set: None,
    };

    // The composite's native size — drives the G-buffer alloc + the 1:1 top-left present.
    let present_extent = VkExtent2D { width: COMPOSITE_W, height: COMPOSITE_H };

    // A host-visible staging buffer sized for one full swapchain image (4 B/texel).
    let staging_size = (swapchain.extent().width * swapchain.extent().height * 4) as u64;
    let staging = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: staging_size,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible readback staging buffer");
    let alloc_extent = swapchain.extent();

    let mut frame = GBufferFrame::new();

    // === DIAGNOSTIC: dump TWO comparable offscreen frames — brick ON vs brick OFF — so the
    //     orchestrator can open them side-by-side and confirm the brick-ON render matches the
    //     analytic (OFF) render (the owner reported the sphere seems to "disappear" toggling). ===
    //
    // Both frames are rendered from the IDENTICAL camera / scene / edit-list, differing ONLY by
    // `scene.brick` (Some(brick_on) vs None — the gate the marcher push carries). Each is read back
    // through the SAME staging buffer with the SAME R/B handling as the golden capture
    // (`readback_to_rgba`), then written as a 32-bpp BMP — so the two files are byte-comparable.
    //
    // COHERENCY: `render_gbuffer_frame` issues the swapchain→staging copy in the readback frame's
    // submit, but the host read is only coherent once that frame slot's fence has been WAITED again
    // (it is waited at the START of each `render_gbuffer_frame`). The engine keeps `FRAMES_IN_FLIGHT`
    // (== 2) slots, so rendering `DRAIN_FRAMES` (3 > 2) further frames after the readback frame
    // guarantees the readback slot's fence was re-waited — exactly the discipline the existing golden
    // relies on. A swapchain recreate (`Ok(false)`) on the readback frame skips that dump gracefully.
    const DRAIN_FRAMES: u32 = 3;
    // Captures `scene`/`renderer`/`swapchain`/`frame`/`window` mutably + the device/surface/staging
    // immutably; takes only the brick state + dump path. NLL ends these borrows after the last call,
    // before the interactive loop reuses them. (A free `fn` would have to name `BoundBuffer` +
    // re-thread eight references; the capturing closure keeps the call sites trivial.)
    let mut dump_brick_ab = |brick_state: Option<BrickActivation>, path: &str| {
        if !window.pump_events() {
            return; // The window was closed before the dump — skip it cleanly.
        }
        window.refresh_size();
        let live = swapchain.extent();
        if live.width != alloc_extent.width || live.height != alloc_extent.height {
            eprintln!("NOTE brick dump: extent changed before the dump frame — skipping {path}");
            return;
        }

        scene.brick = brick_state;
        let clear = [0.0_f32, 0.0, 0.0, 1.0];

        // The readback frame (requests the swapchain→staging copy).
        let token = renderer
            .wait_frame_in_flight()
            .expect("invariant: the frame slot fence wait precedes the submit");
        // SAFETY: identical contract to the interactive loop's `render_gbuffer_frame` below —
        // `ctx`/`surface`/`swapchain`/`renderer` share one device; every `scene` resource is live;
        // `present_extent` + `scene.dispatch_group_count_x` + the camera UBO `count` cover the
        // composite extent; `staging` is host-visible and ≥ one swapchain image in bytes.
        let presented = unsafe {
            renderer.render_gbuffer_frame(
                token, ctx, surface, &mut swapchain, &scene, &mut frame,
                window.width(), window.height(), clear, present_extent, Some(&staging),
            )
        }
        .unwrap_or_else(|e| panic!("brick dump frame ({path}) failed: {e:?}"));
        if !presented {
            eprintln!("NOTE brick dump: swapchain recreated on the readback frame — skipping {path}");
            return;
        }
        let dump_extent = swapchain.extent();

        // Drain frames so the readback slot's fence is waited (staging becomes coherent).
        for _ in 0..DRAIN_FRAMES {
            if !window.pump_events() {
                break;
            }
            window.refresh_size();
            let token = renderer
                .wait_frame_in_flight()
                .expect("invariant: the frame slot fence wait precedes the submit");
            // SAFETY: same contract; no readback requested on the drain frames.
            let _ = unsafe {
                renderer.render_gbuffer_frame(
                    token, ctx, surface, &mut swapchain, &scene, &mut frame,
                    window.width(), window.height(), clear, present_extent, None,
                )
            }
            .unwrap_or_else(|e| panic!("brick dump drain frame ({path}) failed: {e:?}"));
        }

        // Read back the staged swapchain image, normalize to RGBA, write the BMP.
        let w = dump_extent.width;
        let h = dump_extent.height;
        let byte_count = (w * h * 4) as usize;
        let dst_ptr = RhiDevice::buffer_mapped_ptr(device, &staging)
            .expect("host-visible staging buffer is mapped");
        let mut raw = vec![0u8; byte_count];
        // SAFETY: `dst_ptr` points to `staging_size` (≥ `byte_count`) mapped host-coherent bytes;
        // the readback frame's copy completed before this read (its slot fence was re-waited by the
        // drain frames above); `raw` is a distinct, non-overlapping alloc.
        unsafe { core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), raw.as_mut_ptr(), byte_count) };
        let rgba = readback_to_rgba(&raw, w, h, is_bgra);
        match write_bmp(path, &rgba, w, h) {
            Ok(()) => println!("brick dump -> {path} ({w}x{h})"),
            Err(e) => eprintln!("NOTE brick dump: failed to write {path}: {e:?}"),
        }
    };

    dump_brick_ab(Some(brick_on), BRICK_ON_BMP);
    dump_brick_ab(None, BRICK_OFF_BMP);
    // The closure is not used again; NLL ends its `&mut scene`/`renderer`/`swapchain`/`frame`/`window`
    // borrows here, so the interactive loop below freely reuses them.

    // Restore the boot brick state for the interactive loop + the live golden capture.
    scene.brick = if BRICK_START_ON { Some(brick_on) } else { None };

    // --- Present the image-based composite; request the swapchain-image readback on ONE
    //     presented frame. The loop runs up to `MAX_FRAMES` (so CI / a headless run always
    //     terminates) but ALSO exits the moment the window is closed, so the owner can watch +
    //     toggle the brick path live and close the window to end the run. ---
    //
    // Brick A/B TOGGLE: each frame the captured input ring is drained; a 'B' WM_KEYDOWN flips
    // `scene.brick` between ON (`Some(brick_on)` — empty-skip + trilinear/cubic + 3-level clip-map)
    // and OFF (`None` — the analytic marcher). The gates live entirely in the per-frame marcher
    // push, so the flip costs nothing but a different push byte image — no re-record, no re-bind.
    // The owner confirms the brick render looks IDENTICAL to analytic (RTX-verified byte-identical
    // in this scene) and is faster (empty-space-skip).
    //
    // The frame cap. Under `cargo test` (CI / the tester) the loop must terminate fast + record the
    // golden, so the DEFAULT is a short bounded run (`CI_FRAMES`). The owner runs it interactively by
    // setting `BOYKO_WINDOW_FRAMES` (e.g. a large count) — then the loop runs that many frames (or
    // until the window is closed), long enough to watch + toggle the brick A/B live. Either way the
    // golden readback frame (`i == 2`) renders before the cap, in the `BRICK_START_ON` state (brick-ON
    // is byte-identical to analytic, so the +/-2/255 golden holds regardless of the start state).
    const CI_FRAMES: u32 = 5;
    let max_frames: u32 = std::env::var("BOYKO_WINDOW_FRAMES")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(CI_FRAMES);
    let clear = [0.0_f32, 0.0, 0.0, 1.0];
    let mut readback_done = false;
    let mut readback_extent = swapchain.extent();
    for i in 0..max_frames {
        if !window.pump_events() {
            break; // The window was closed — end the interactive run cleanly.
        }
        window.refresh_size();

        // Drain captured input; a 'B' key-down toggles the brick A/B state for the NEXT dispatch.
        window.drain_input(|msg| {
            if let CapturedMsg::Raw { msg: wm, wparam, .. } = msg
                && wm == WM_KEYDOWN
                && wparam == VK_B
            {
                scene.brick = match scene.brick {
                    Some(_) => None,
                    None => Some(brick_on),
                };
                println!(
                    "brick toggle -> {}",
                    if scene.brick.is_some() { "ON (empty-skip + trilinear + clip-map)" } else { "OFF (analytic)" }
                );
            }
        });

        // Request the readback on a single steady frame, only while the live extent
        // still matches the staging-buffer size (a resize simply skips the golden).
        let live = swapchain.extent();
        let extent_stable = live.width == alloc_extent.width && live.height == alloc_extent.height;
        let want_readback = i == 2 && !readback_done && extent_stable;
        let rb = if want_readback { Some(&staging) } else { None };

        // The golden discriminator-texel assertion (below) compares the readback against the ANALYTIC
        // host golden within ±2/255. The M1 empty-skip is verified ±2/255 of analytic, but the M2
        // trilinear+cubic SURFACE crossing is validated by the exact-CSG hit residual (M2_CREASE_EPS),
        // NOT by ±2/255 lit-color identity to the analytic marcher — the cubic can shift the surface
        // `t` (and thus the shaded color) slightly. So force the GOLDEN-CAPTURE frame to render OFF
        // (analytic), then restore the live brick state for the next frame. This keeps the CI golden
        // deterministic (analytic == analytic) while leaving the boot/interactive state owner-driven.
        let restore_brick = scene.brick;
        if want_readback {
            scene.brick = None;
        }

        let token = renderer
            .wait_frame_in_flight()
            .expect("invariant: the frame slot fence wait precedes the submit");
        // SAFETY: `ctx`/`surface`/`swapchain` are live + created on the same device as
        // `renderer`; every `scene` resource is live on this device; `edit_list` /
        // `camera_uniform` were host-seeded once + are never written again (the marcher
        // only reads them); `frame`'s targets are synced to `present_extent` by the call;
        // `scene.dispatch_group_count_x` + the camera UBO's `count` were sized to the
        // composite extent; a `Some(rb)` staging buffer is host-visible + `staging_size`
        // (>= one swapchain image) bytes.
        let presented = unsafe {
            renderer.render_gbuffer_frame(
                token,
                ctx,
                surface,
                &mut swapchain,
                &scene,
                &mut frame,
                window.width(),
                window.height(),
                clear,
                present_extent,
                rb,
            )
        }
        .unwrap_or_else(|e| panic!("gbuffer present frame {i} failed: {e:?}"));

        // Restore the live brick state the golden frame may have forced OFF.
        scene.brick = restore_brick;

        if want_readback && presented {
            readback_done = true;
            readback_extent = swapchain.extent();
        }
    }

    // The oracle: a clean windowed image-based present records zero validation messages.
    // Gated on `validation_enabled()` so the composite pixel golden below still runs under
    // `BOYKO_DISABLE_VALIDATION` (no messenger is created when validation is off).
    if ctx.validation_enabled() {
        let state = ctx
            .debug_state()
            .expect("validation enabled => a debug-messenger state is present");
        assert_eq!(
            state.total(),
            0,
            "validation layer reported {} message(s) during the windowed G-buffer present — \
             see the [vk-validation] log",
            state.total()
        );
    }

    // The golden: if a readback frame presented, the three discriminator texels must
    // match the host composite truth (swapchain byte-order-aware) — PROVING the
    // image-based composite reached the swapchain image with correct colors, equal to
    // the packed on-screen composite within +/-2/255.
    if readback_done {
        let w = readback_extent.width;
        let h = readback_extent.height;
        let dst_ptr = RhiDevice::buffer_mapped_ptr(device, &staging)
            .expect("host-visible staging buffer is mapped");
        let byte_count = (w * h * 4) as usize;
        let mut out = vec![0u8; byte_count];
        // SAFETY: `dst_ptr` points to `staging_size` (>= `byte_count`) mapped
        // host-coherent bytes; the readback frame's submit completed before this read
        // (the renderer fence-waits the frame slot at the START of each subsequent
        // `render_gbuffer_frame`, and frames followed frame 2, so frame 2's copy is
        // complete + coherent); `out` is a distinct, non-overlapping alloc.
        unsafe {
            core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), byte_count);
        }

        let read_texel = |px: u32, py: u32| -> [u8; 4] {
            let b = texel_base(px, py, w);
            [out[b], out[b + 1], out[b + 2], out[b + 3]]
        };

        let a_got = read_texel(ax, ay);
        let b_got = read_texel(bx, by);
        let d_got = read_texel(dx, dy);

        assert_readback_close(a_got, a_want, is_bgra, "mesh-occludes-SDF texel (raster-PBR)");
        assert_readback_close(b_got, b_want, is_bgra, "SDF texel (lit color)");
        assert_readback_close(d_got, d_want, is_bgra, "background texel");

        // The occlusion actually changed the on-screen pixel: the mesh-occludes-SDF
        // texel and the SDF texel are BOTH over the sphere, yet must differ.
        let a_rgb = readback_rgb(a_got, is_bgra);
        let b_rgb = readback_rgb(b_got, is_bgra);
        assert!(
            (0..3).any(|c| (a_rgb[c] - b_rgb[c]).abs() > CHANNEL_TOL),
            "the on-screen mesh-occluded texel {a_got:02x?} and the SDF-visible texel {b_got:02x?} \
             (both over the sphere) must differ — proving the image-based hybrid composite (not a clear) reached the screen"
        );
    } else {
        eprintln!(
            "NOTE windowed_gbuffer_present: no readback frame presented (swapchain kept recreating); \
             validation was still asserted clean across all frames"
        );
    }

    // Clean reverse-order teardown: renderer (waits idle) → the per-extent G-buffer
    // frame → the scene's static resources → swapchain → surface → window.
    drop(renderer);
    // SAFETY: the renderer was dropped above (its `Drop` waits the device idle), so no
    // submission references these resources; `ctx` is still alive; each is destroyed
    // exactly once, in reverse dependency order.
    unsafe {
        frame.destroy(ctx);
        RhiDevice::destroy_buffer(device, staging);
        // CSM Increment 1b: the cascade trio + depth pipeline.
        csm.destroy(ctx);
        RhiDevice::destroy_graphics_pipeline(device, present_pipeline);
        RhiDevice::destroy_bind_group_layout(device, present_layout);
        RhiDevice::destroy_compute_pipeline(device, resolve_pipeline);
        RhiDevice::destroy_bind_group_layout(device, resolve_layout);
        RhiDevice::destroy_compute_pipeline(device, marcher);
        RhiDevice::destroy_bind_group_layout(device, vocab_layout);
        RhiDevice::destroy_graphics_pipeline(device, raster_pipeline);
        // M1 instance-model resources (bind group → buffer → layout, after the pipeline).
        RhiDevice::destroy_bind_group(device, instance_bind_group);
        RhiDevice::destroy_buffer(device, instance_buffer);
        RhiDevice::destroy_bind_group_layout(device, instance_layout);
        RhiDevice::destroy_sampler(device, present_sampler);
        RhiDevice::destroy_sampler(device, depth_sampler);
        RhiDevice::destroy_buffer(device, vertex_buffer);
        RhiDevice::destroy_buffer(device, tiles_buffer);
        // The brick clip-map (every level's atlas image + sampler + pointer-grid SSBO). The renderer
        // was dropped above (waits idle), so no submission still samples it; `ctx` is alive; the
        // by-value `destroy` moves each level's resources out once.
        clipmap.destroy(ctx);
        RhiDevice::destroy_buffer(device, light_staging);
        RhiDevice::destroy_buffer(device, light_table);
        RhiDevice::destroy_buffer(device, material_table);
        for slot in camera_ring {
            RhiDevice::destroy_buffer(device, slot);
        }
        RhiDevice::destroy_buffer(device, edit_list);
    }
    drop(swapchain);
    // surface / ctx / window are owned by `with_windowed_present` and dropped in-order at its frame end.
}

/// **Render P0 GPU gate — the windowed EMPTY-SKIP-ONLY coarse cull is LIT-TRANSPARENT.**
///
/// Drives the WINDOWED present path (the same `Renderer::render_gbuffer_frame` 3-pass) through a
/// swapchain-image readback TWICE from the IDENTICAL camera / scene / edit-list AT THE REAL ON-SCREEN
/// LIT FLAGS (`LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO`), differing ONLY by [`GBufferScene::coarse`]:
///
/// - `coarse = None` (the OFF / 0%-gate path): NO coarse dispatch + NO `tiles_buffer` barrier are
///   recorded, `coarse_enabled == 0` — the pre-P0 windowed command stream, byte-for-byte.
/// - `coarse = Some(&coarse_compute)` with `coarse_mode = EmptySkipOnly` (the lit-transparent ON
///   path): the P4b coarse-cull pass runs BEFORE the marcher (one invocation per 8×8 tile writes a
///   `TileBound` into vocab binding 6), and the marcher reads them with `coarse_enabled == 2` — it
///   SKIPS EMPTY tiles only, WITHOUT seeding `near_t` on the surface tiles.
///
/// # Why EmptySkipOnly (mode 2), not Full (mode 1)
///
/// The empty-tile skip is provably image-identical lit+unlit (an empty tile has no surface). The
/// FULL mode (1) additionally seeds the march at the tile's conservative `near_t` on a NON-empty
/// tile; fed into the B1 over-relaxed march that seed latches a different grazing tangent on the
/// silhouette (a shifted normal → a shifted AO/shadow), so the LIT cull-ON image gains a 16–32/255
/// rim — the FULL cull is NOT lit-transparent. EmptySkipOnly drops the seed, so it is transparent
/// UNDER LIGHTING by construction. This test asserts that: the ON readback MUST equal the OFF
/// readback within the goldens' per-channel tolerance ([`CHANNEL_TOL`], `+/-2/255`) AT THE LIT FLAGS
/// — proving the on-screen cull adds NO visible rim. (The FULL-mode image-transparency contract
/// remains the UNLIT offscreen golden `sdf_gbuffer_hybrid::p4b_cull_on_conservative_within_tol_of_cull_off`.)
///
/// The brick path is held OFF (`brick = None`) on BOTH frames so the comparison isolates the cull.
/// The test also asserts the validation layer is clean across the ON path (the recorder's new coarse
/// dispatch + barrier are sound).
///
/// `#[ignore]`: needs a real RTX windowed device. The orchestrator runs it on the GPU; CPU `cargo
/// test` skips it (the harness still compiles it, proving the OFF caller + the new `coarse` field +
/// the coarse-pipeline creation type-check).
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU"]
fn p0_windowed_coarse_cull_matches_uncull() {
    with_windowed_present(
        "boyko_rhi_vulkan P0 coarse-cull window",
        "p0_windowed_coarse_cull",
        body_p0_coarse_cull,
    );
}

fn body_p0_coarse_cull(bp: BootPresent<'_, '_>) {
    let BootPresent { window, ctx, surface, mut swapchain, mut renderer, is_bgra, swap_color_format } =
        bp;

    let device: &VulkanContext = ctx;
    let sdf = sphere_scene();

    // --- The edit-list SSBO (binding 0), host-seeded ONCE. ---
    let edit_list = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, &sdf);
        let mapped = RhiDevice::buffer_mapped_ptr(device, &edit_list)
            .expect("host-visible edit-list buffer is mapped");
        write_words(mapped, &header);
    }

    // --- The camera/extent UBO (binding 5), host-seeded ONCE at the COMPOSITE ORTHO extent. The
    // M4 tail is zero here (brick is held OFF on both readback frames, so the marcher never reads
    // the per-level params; binding 9..=14 still need VALID descriptors below). ---
    // The camera/extent UBO RING (binding 5): one host-coherent slot per in-flight frame, every
    // slot seeded IDENTICALLY + never rewritten on these two readback frames (byte-identical to the
    // pre-ring single buffer); the ring only matters for the interactive viewer's per-frame writes.
    let camera_ring: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
        RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: B5_CAMERA_UBO_BYTES_M4 as u64,
                usage: BufferUsage::UNIFORM,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("camera uniform buffer")
    });
    {
        let pc = CompositePushConstants::ortho(COMPOSITE_W, COMPOSITE_H);
        assert_eq!(pc.count, PIXELS);
        let bytes = pc.as_bytes();
        debug_assert_eq!(bytes.len(), M2_GRID_PARAMS_OFFSET, "camera block must be 80 B");
        for slot in &camera_ring {
            let mapped = RhiDevice::buffer_mapped_ptr(device, slot)
                .expect("host-visible uniform buffer is mapped");
            // SAFETY: `mapped` points to `B5_CAMERA_UBO_BYTES_M4` (224) mapped host-coherent bytes; the
            // 80-byte camera block is written at offset 0 (the M4 tail stays zero — brick is OFF). No
            // GPU work is in flight yet, so the host write is unsynchronized-safe. Every ring slot is
            // seeded with the SAME bytes (byte-identical to the pre-ring single buffer).
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
            }
        }
    }

    // --- The P4b coarse-cull tile StorageBuffer (vocab binding 6), sized to the full tile grid at
    // the COMPOSITE extent. On the OFF frame it is bound-but-unread; on the ON frame the coarse
    // pass WRITES it and the marcher READS it. ---
    let (tw, th) = tile_grid_extent(COMPOSITE_W, COMPOSITE_H);
    let tiles_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (tw as u64) * (th as u64) * (TILE_BOUND_BYTES as u64),
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("P4b coarse-cull tile-bound storage buffer (vocab binding 6)");

    // --- The PBR material table SSBO (vocab binding 7 + resolve binding 4). ---
    let material_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (DEFAULT_MATERIAL_TABLE.len() as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("PBR material table storage buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &material_table)
            .expect("host-visible material table is mapped");
        write_words(mapped, &DEFAULT_MATERIAL_TABLE);
    }

    // --- The brick clip-map: the brick path is held OFF on both readback frames, but the marcher
    // SPIR-V statically references bindings 9..=14 past the runtime gate, so VALID descriptors must
    // be bound. The real clip-map supplies them (`brick = None` keeps them bound-but-unread). ---
    let field = {
        use boyko_sdf_math::SdfEditField;
        let mut f = SdfEditField::new();
        for e in &sdf {
            assert!(f.push(*e), "P0 cull scene must fit MAX_SDF_EDITS");
        }
        f.bump_gen();
        f
    };
    let clipmap = BrickClipmap::create(ctx, &field, [0.0, 0.0, 0.0])
        .expect("brick clip-map (P0 cull scene) — create + bake + upload");

    // --- The Lighting-L0 light table SSBO (resolve binding 6) + its staging source. ---
    let light_table_bytes = (DEGENERATE_LIGHT_TABLE.len() as u64) * 4;
    let light_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: light_table_bytes,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("Lighting-L0 light table storage buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_table)
            .expect("host-visible light table is mapped");
        write_words(mapped, &DEGENERATE_LIGHT_TABLE);
    }
    let light_staging = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: light_table_bytes,
            usage: BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("Lighting-L0 light table staging buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_staging)
            .expect("host-visible light staging is mapped");
        write_words(mapped, &DEGENERATE_LIGHT_TABLE);
    }

    // --- The mesh quad's vertex buffer. ---
    let vertices = quad_vertices();
    let vertex_bytes = core::mem::size_of_val(&vertices) as u64;
    let vertex_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible vertex buffer");
    {
        let vb_ptr = RhiDevice::buffer_mapped_ptr(device, &vertex_buffer)
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

    let depth_sampler = RhiDevice::create_sampler(device, &SamplerDesc::default())
        .expect("depth sampler (ignored by .Load)");
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
    .expect("present nearest/clamp sampler");

    // --- The mesh-MRT G-buffer producer graphics pipeline (Render P5-r0). ---
    let vs = RhiDevice::create_shader_module(device, MRT_VS_SPV.as_words())
        .expect("mesh-MRT vertex shader module");
    let fs = RhiDevice::create_shader_module(device, MRT_FS_SPV.as_words())
        .expect("mesh-MRT fragment shader module");
    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
    ];
    // M1: the per-instance model SSBO layout + 1-element identity dummy + its bind group.
    // The gbuffer VS statically references `StructuredBuffer<InstanceModelCol> instances` at
    // set 0 binding 0, so the pipeline layout MUST declare it and every draw MUST bind a valid
    // buffer; the legacy merged draw (`use_model_matrix == 0`) never reads it (bound-but-unread).
    let (instance_layout, instance_buffer, instance_bind_group) = create_identity_instance(device);
    let raster_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            // Render P5-r0: 3 MRT color formats = the G-buffer RGBA8 lanes; the production
            // `record_gbuffer` binds albedo/normal/material as the 3 MRT attachments.
            color_formats: &[RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT],
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
        },
    )
    .expect("depth-prepass graphics pipeline");

    // --- The P1b marcher: the vocabulary layout + the marcher pipeline. The SAME layout is shared
    // by the coarse-cull pipeline below (the cull shader declares only a subset — valid). ---
    let cs = RhiDevice::create_shader_module(device, sdf_gbuffer_composite_spirv())
        .expect("P1b G-buffer marcher compute shader module");
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
        // MDF Stage-2c: the dedicated dense mesh-SDF shadow-caster texture @15 (the 16th / last vocab
        // entry under the 16-binding cap). The recompiled marcher SPIR-V statically references
        // `MeshSdf`@t15 + `MeshSdfSampler`@s15 inside the runtime-gated `mesh_sdf_enabled` branch, so
        // the layout MUST declare binding 15 — a VALID combined image+sampler must be bound even on
        // the OFF path (`mesh_sdf_enabled == false` → bound-but-unread, byte-identical output).
        BindGroupLayoutEntry { binding: 15, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
    ];
    let vocab_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &vocab_entries },
    )
    .expect("P1b vocabulary bind-group layout");
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
    .expect("P1b G-buffer marcher compute pipeline");

    // --- Render P0: the COARSE-CULL pipeline, created against the SAME vocabulary layout (the
    // offscreen `run_gbuffer_hybrid_ex` discipline — the cull shader declares only a subset of the
    // vocab bindings, so sharing the full layout is valid). ---
    let coarse_cs = RhiDevice::create_shader_module(device, sdf_tile_cull_spirv())
        .expect("P4b coarse-cull compute shader module");
    let coarse_compute = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &coarse_cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&vocab_layout),
            spec_constants: &[],
        },
    )
    .expect("P4b coarse-cull compute pipeline (shared vocab layout)");

    // --- The deferred RESOLVE pipeline. ---
    let resolve_cs = RhiDevice::create_shader_module(device, deferred_pbr_spirv())
        .expect("deferred resolve compute shader module");
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
        // Render P6 R1: the deferred resolve binds the SDF edit-list `Buf` at binding 10 (the
        // sdf_soft_shadow_ranged march reads it). The production `record_gbuffer` binds it, so the
        // resolve layout MUST declare it or bind-group creation trips the entry-count check.
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Render P7: the SSAO term `gSsao` STORAGE image @11 (read under `ssao_mode != 0`; OFF
        // here, bound-but-unread). The production `GBufferTargets` binds the SSAO image at @11,
        // so the resolve layout MUST declare it (the P6 R1 binding-10 discipline).
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        // CSM Increment 1b (Rung A): the cascade combined map+sampler @12 + the cascade UBO @13.
        // The production `GBufferTargets::create` binds the scene's cascade trio at @12/@13, so the
        // resolve layout MUST declare them (the recompiled resolve STATICALLY references `gCsm` +
        // `CsmCascades`). 14 bindings ≤ the 16-binding cap. `csm_mode == 0` on the golden presents
        // → bound-but-unread (the 0%-gate).
        BindGroupLayoutEntry { binding: 12, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 13, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        // Shadow Phase 5 Inc-1-GPU: the sparse spot/point shadow-atlas combined map+sampler @14 + the
        // atlas UBO @15. The production `GBufferTargets::create` binds the scene's atlas trio at
        // @14/@15, so the resolve layout MUST declare them (the recompiled resolve STATICALLY
        // references `gShadowAtlas` + `ShadowAtlas`). 16 bindings == the 16-binding cap (16/16);
        // `punctual_shadow_mode == 0` on the golden presents → bound-but-unread (the 0%-gate).
        BindGroupLayoutEntry { binding: 14, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 15, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        // SDFDDGI I0: the DDGI probe-irradiance combined image @16 + depth combined image @17 + the
        // `ResolvedDdgi` grid UBO @18 (bound-but-unread; the recompiled resolve STATICALLY references
        // `gDdgiIrr`/`gDdgiDepth`/`ResolvedDdgi`, so the layout MUST declare them). Exact-fill 19/19.
        BindGroupLayoutEntry { binding: 16, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 17, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 18, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        // Textured-PBR T6a (the critic's C1 fix): the SOFTWARE-ONLY `gPbr` STORAGE image @19.
        // `GBufferTargets::create` now allocates `gPbr` UNCONDITIONALLY (both feature legs) and
        // `DeferredSets::build`'s software resolve-set loop appends it past the shared 19 —
        // the layout MUST declare it too, or `create_bind_group`'s entry-count check trips (P1a).
        BindGroupLayoutEntry { binding: 19, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
    ];
    let resolve_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &resolve_entries },
    )
    .expect("deferred resolve bind-group layout");
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
    .expect("deferred resolve compute pipeline");

    // --- The present-blit pipeline. ---
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
    .expect("present-blit bind-group layout");
    let sample_vs = RhiDevice::create_shader_module(device, SAMPLE_VS_SPV.as_words())
        .expect("fullscreen vertex shader module");
    let sample_fs = RhiDevice::create_shader_module(device, SAMPLE_FS_SPV.as_words())
        .expect("fullscreen fragment shader module");
    let present_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &sample_vs,
            vertex_entry: c"main",
            fragment_module: &sample_fs,
            fragment_entry: c"main",
            color_formats: &[swap_color_format],
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
    .expect("present-blit fullscreen-sample pipeline");

    // The shader modules are consumed by pipeline creation; destroy them now.
    // SAFETY: every module was created on `ctx` above + is no longer needed once its pipeline is
    // created; each is destroyed exactly once.
    unsafe {
        RhiDevice::destroy_shader_module(device, sample_fs);
        RhiDevice::destroy_shader_module(device, sample_vs);
        RhiDevice::destroy_shader_module(device, resolve_cs);
        RhiDevice::destroy_shader_module(device, coarse_cs);
        RhiDevice::destroy_shader_module(device, cs);
        RhiDevice::destroy_shader_module(device, fs);
        RhiDevice::destroy_shader_module(device, vs);
    }

    // CSM Increment 1b (Rung A): the cascade trio + depth-only pipeline (OFF path — `csm: None`,
    // bound-but-unread at resolve @12/@13 on this cull-comparison present).
    let csm = CsmSceneResources::create(device, &instance_layout);

    let mvp = ortho_mvp_bytes();
    let mut scene = GBufferScene {
        raster_pipeline: &raster_pipeline,
        vertex_buffer: &vertex_buffer,
        vertex_count: vertices.len() as u32,
        mvp,
        // M1: the legacy merged draw binds the 1-element identity instance SSBO at set 0
        // (bound-but-unread — the `use_model_matrix == 0` push selects the VS's legacy arm).
        instance_bind_group: &instance_bind_group,
        marcher: &marcher,
        vocab_layout: &vocab_layout,
        edit_list: &edit_list,
        camera_ring: &camera_ring,
        tiles_buffer: &tiles_buffer,
        pointer_grid: clipmap.grid_buffer(0),
        atlas: clipmap.atlas(0).texture(),
        atlas_sampler: clipmap.sampler(0),
        level_grids: [clipmap.grid_buffer(1), clipmap.grid_buffer(2)],
        level_atlases: [clipmap.atlas(1).texture(), clipmap.atlas(2).texture()],
        level_atlas_samplers: [clipmap.sampler(1), clipmap.sampler(2)],
        // MDF Stage-2c (binding 15): a non-MDF scene binds the brick atlas (level 0) as a benign
        // placeholder + gates the mesh-shadow path OFF — the texture is bound-but-unread (the R2
        // contract: a VALID descriptor must be bound, the read is gated by `mesh_sdf_enabled`).
        mesh_sdf: clipmap.atlas(0).texture(),
        mesh_sdf_sampler: clipmap.sampler(0),
        mesh_sdf_enabled: false,
        depth_sampler: &depth_sampler,
        material_table: &material_table,
        light_table: &light_table,
        light_staging: &light_staging,
        light_upload_bytes: light_table_bytes,
        light_dirty: false,
        cluster_cull: None,
        cull_layout: None,
        cluster_grid: None,
        light_index: None,
        light_index_alloc: None,
        cluster_cull_push: [0u8; 16],
        cluster_count: 0,
        resolve_pipeline: &resolve_pipeline,
        resolve_layout: &resolve_layout,
        #[cfg(feature = "hwrt")]
        resolve_pipeline_hwrt: None,
        #[cfg(feature = "hwrt")]
        resolve_layout_hwrt: None,
        #[cfg(feature = "hwrt")]
        resolve_tlas_hwrt: None,
        // Rung 1b: the HWRT resolve is OFF in this harness (`resolve_tlas_hwrt: None`), so the
        // shadow-params UBO ring is bound by NO set — a benign valid placeholder (the whole cascade
        // UBO ring, a per-FIF `[BoundBuffer; FRAMES_IN_FLIGHT]`, host-coherent + >= 16 B/slot)
        // satisfies the field type without ever being read.
        #[cfg(feature = "hwrt")]
        ray_shadow_ubo: csm.csm_ring(),
        present_pipeline: &present_pipeline,
        present_layout: &present_layout,
        present_sampler: &present_sampler,
        dispatch_group_count_x: group_count_x(),
        // The brick path is held OFF on BOTH frames so the cull-on-vs-off comparison is isolated.
        brick: None,
        // The cull gate, flipped per readback frame below (None then Some(&coarse_compute)).
        coarse: None,
        // EmptySkipOnly (mode 2) — the LIT-TRANSPARENT cull: EMPTY-skip only, NO `near_t` seed.
        // The empty-tile skip is provably image-identical lit+unlit (an empty tile has no surface);
        // dropping the `near_t` seed on the few NON-empty surface tiles removes the grazing-silhouette
        // AO/shadow rim the FULL mode's seed latches (a shifted grazing tangent → a shifted normal →
        // a shifted AO/shadow). So this mode is transparent UNDER LIGHTING — which is exactly what
        // this test now proves (it renders at the real on-screen lit flags, NOT `0`).
        coarse_mode: CoarseMode::EmptySkipOnly,
        // Lighting ON — the REAL on-screen flags (A1 soft shadows + A2 AO). The previous P0 test set
        // `lighting_flags == 0` to dodge the FULL-mode `near_t` rim (the lit cull-transparency
        // invariant was un-shipped). EmptySkipOnly is lit-transparent BY CONSTRUCTION (no seed → no
        // rim), so the cull-ON vs cull-OFF comparison is now asserted at the real lit flags — proving
        // the on-screen cull adds NO visible rim.
        lighting_flags: LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO,
        // The legacy head-on shadow direction (`[0,0,1]`): the cull-ON vs cull-OFF comparison must
        // hold the marcher push fixed, so this stays the pre-`light_dir`-field default.
        light_dir: DEFAULT_LIGHT_DIR,
        // Render P7: SSAO OFF (the default) — NO SSAO pass recorded, byte-identical to the pre-P7
        // stream (the 0%-gate). These golden/cull-comparison presents assert the existing stream.
        ssao: None,
        // M3: the LEGACY merged draw (no instanced mesh — an EMPTY batch slice) —
        // `record_gbuffer` keeps `vkCmdDraw(vertex_count, 1, 0, 0)`, byte-identical to the
        // pre-M2 stream.
        mesh_draw: &[],
        // CSM Increment 1b (Rung A): the cascade trio bound at resolve @12/@13 (ALWAYS), the depth
        // pass OFF (`csm: None`) — bound-but-unread on this cull-comparison present (the 0%-gate).
        csm_cascade_texture: &csm.cascade,
        csm_compare_sampler: &csm.sampler,
        csm_cascade_ring: csm.csm_ring(),
        csm: None,
        // Shadow Phase 5 Inc-1-GPU: the atlas trio bound at resolve @14/@15 (ALWAYS), the punctual
        // depth pass OFF (`atlas_punctual: None`). On this golden present `punctual_shadow_mode == 0`,
        // so the resolve's spot PCF sample never runs and the trio is bound-but-unread (the 0%-gate —
        // byte-identical pixels).
        shadow_atlas_texture: &csm.atlas,
        shadow_atlas_sampler: &csm.atlas_sampler,
        shadow_atlas_ubo: &csm.atlas_ubo,
        // SDFDDGI I1: the 3 DDGI resolve bindings (@16/@17/@18) now bind the REAL probe atlas. The GI
        // gate is OFF on every golden present (LightBuf word-7 bit 4 == 0), so the resolve's probe
        // sample never runs and all three are bound-but-unread (the 0%-gate — byte-identical pixels).
        // I1 severs the I0a dummy: the irradiance/depth atlases are the dedicated
        // `B10G11R11_UFLOAT`/`R16G16_SFLOAT` `Texture2DArray`s, each sampled with a dedicated LINEAR
        // (non-comparison) sampler — closing the VUID trap (the old CSM COMPARISON sampler on a
        // non-Dref SampleLevel was UB). The grid UBO is the dedicated zeroed `ddgi_ubo`.
        ddgi_irr_texture: csm.ddgi_atlas.irradiance(),
        ddgi_irr_sampler: csm.ddgi_atlas.sampler(),
        ddgi_depth_texture: csm.ddgi_atlas.depth(),
        ddgi_depth_sampler: csm.ddgi_atlas.sampler(),
        ddgi_grid_ubo: &csm.ddgi_ubo,
        // SDFDDGI I2: the probe-update pass is OFF in these harness scenes (the GI-OFF 0%-gate).
        // `ddgi_update = None` ⇒ no update RDG pass / dispatch / barrier is recorded. The
        // classification / ray-table / update-UBO handles are supplied so the RDG sink can resolve
        // them (unread while the pass is off); the ray-table + update-UBO reuse the bound-but-unread
        // `ddgi_ubo` as a placeholder buffer (never read on the OFF path — this harness does not arm
        // the update pass; a bench/host that arms it supplies real dedicated buffers).
        ddgi_update: None,
        ddgi_classification: csm.ddgi_atlas.classification(),
        ddgi_ray_table: &csm.ddgi_ubo,
        ddgi_update_ubo: &csm.ddgi_ubo,
        atlas_punctual: None,
        // Pillar B B3: the interpolation pre-pass is OFF for every dump/offscreen golden.
        interp: None,
        // HW-RT rung R0: GPU timing OFF (byte-identical command stream).
        gpu_timing: None,
        // HW-RT rung R2a-3: the per-frame TLAS pack + build OFF (byte-identical command stream).
        #[cfg(feature = "hwrt")]
        tlas: None,
        // HW-RT rung 3a: the spatial (à-trous) RT soft-shadow denoise OFF (byte-identical).
        #[cfg(feature = "hwrt")]
        shadow: None,
        // HW-RT rung 3a: the STABLE denoise-set-build signals — all OFF in this harness (no denoise
        // sets built; byte-identical).
        #[cfg(feature = "hwrt")]
        resolve_layout_denoise_hwrt: None,
        #[cfg(feature = "hwrt")]
        atrous_layout_denoise_hwrt: None,
        #[cfg(feature = "hwrt")]
        shadow_denoise_enabled: false,
        #[cfg(feature = "hwrt")]
        shadow_denoise_final_is_vis2: false,
        // Rung-3b step 5a: the temporal-MV mesh path — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        temporal_enabled: false,
        #[cfg(feature = "hwrt")]
        raster_pipeline_mv: None,
        #[cfg(feature = "hwrt")]
        mv_bind_group: None,
        // F8-mv: the combined MV+PM mesh path — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        raster_pipeline_mvpm: None,
        #[cfg(feature = "hwrt")]
        mvpm_bind_group: None,
        // Rung-3b step 5b: the SDF motion-vector VIS path — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        vis_mv_pipeline: None,
        #[cfg(feature = "hwrt")]
        vis_mv_layout: None,
        #[cfg(feature = "hwrt")]
        motion_cam_ubo_ring: None,
        // Rung-3b step 6: the temporal reproject layout — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        temporal_layout: None,
        // Asset-streaming plan F8: PER_INSTANCE_MATERIAL is OFF in this low-level RHI harness
        // (no ECS gather / material store exists here) — byte-identical to the pre-F8 stream.
        pm_enabled: false,
        raster_pipeline_pm: None,
        pm_bind_group: None,
        // Textured-PBR T6c: TEXTURED is OFF in this low-level RHI harness (no ECS gather /
        // texture asset store exists here) — byte-identical to the pre-T6c stream.
        tex_enabled: false,
        raster_pipeline_tex: None,
        tex_bind_group: None,
        bindless_set: None,
    };

    let present_extent = VkExtent2D { width: COMPOSITE_W, height: COMPOSITE_H };
    let staging_size = (swapchain.extent().width * swapchain.extent().height * 4) as u64;
    let staging = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: staging_size,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible readback staging buffer");
    let alloc_extent = swapchain.extent();
    let mut frame = GBufferFrame::new();

    // Render ONE readback frame at the current `scene.coarse` state + drain so the staging buffer is
    // coherent, then copy it out as an RGBA frame (the SAME R/B normalization the goldens use). The
    // closure mirrors the existing `dump_brick_ab` readback/drain discipline (FRAMES_IN_FLIGHT==2,
    // 3 drain frames re-wait the readback slot's fence). Returns `None` if the swapchain recreated
    // (a resize), in which case the comparison is skipped gracefully.
    const DRAIN_FRAMES: u32 = 3;
    let coarse_pipeline_ref: &boyko_rhi_vulkan::rhi_impl::ComputePipeline = &coarse_compute;
    let mut readback_rgba = |cull_on: bool| -> Option<(Vec<u8>, u32, u32)> {
        if !window.pump_events() {
            return None;
        }
        window.refresh_size();
        let live = swapchain.extent();
        if live.width != alloc_extent.width || live.height != alloc_extent.height {
            eprintln!("NOTE p0 cull: extent changed before the readback frame — skipping");
            return None;
        }

        scene.coarse = if cull_on { Some(coarse_pipeline_ref) } else { None };
        let clear = [0.0_f32, 0.0, 0.0, 1.0];

        let token = renderer
            .wait_frame_in_flight()
            .expect("invariant: the frame slot fence wait precedes the submit");
        // SAFETY: `ctx`/`surface`/`swapchain`/`renderer` share one device; every `scene` resource
        // is live; `present_extent` + `scene.dispatch_group_count_x` + the camera UBO `count` cover
        // the composite extent; `staging` is host-visible and ≥ one swapchain image in bytes.
        let presented = unsafe {
            renderer.render_gbuffer_frame(
                token, ctx, surface, &mut swapchain, &scene, &mut frame,
                window.width(), window.height(), clear, present_extent, Some(&staging),
            )
        }
        .unwrap_or_else(|e| panic!("p0 cull readback frame (cull_on={cull_on}) failed: {e:?}"));
        if !presented {
            eprintln!("NOTE p0 cull: swapchain recreated on the readback frame — skipping");
            return None;
        }
        let extent = swapchain.extent();

        for _ in 0..DRAIN_FRAMES {
            if !window.pump_events() {
                break;
            }
            window.refresh_size();
            let token = renderer
                .wait_frame_in_flight()
                .expect("invariant: the frame slot fence wait precedes the submit");
            // SAFETY: same contract; no readback requested on the drain frames.
            let _ = unsafe {
                renderer.render_gbuffer_frame(
                    token, ctx, surface, &mut swapchain, &scene, &mut frame,
                    window.width(), window.height(), clear, present_extent, None,
                )
            }
            .unwrap_or_else(|e| panic!("p0 cull drain frame (cull_on={cull_on}) failed: {e:?}"));
        }

        let w = extent.width;
        let h = extent.height;
        let byte_count = (w * h * 4) as usize;
        let dst_ptr = RhiDevice::buffer_mapped_ptr(device, &staging)
            .expect("host-visible staging buffer is mapped");
        let mut raw = vec![0u8; byte_count];
        // SAFETY: `dst_ptr` points to `staging_size` (≥ `byte_count`) mapped host-coherent bytes;
        // the readback frame's copy completed before this read (its slot fence was re-waited by the
        // drain frames); `raw` is a distinct, non-overlapping alloc.
        unsafe { core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), raw.as_mut_ptr(), byte_count) };
        Some((readback_to_rgba(&raw, w, h, is_bgra), w, h))
    };

    let off = readback_rgba(false);
    let on = readback_rgba(true);

    // The validation oracle: the ON path's new coarse dispatch + barrier are sound (zero messages
    // across all frames recorded by the two readbacks). Gated on `validation_enabled()` so the
    // pixel gate below still runs under `BOYKO_DISABLE_VALIDATION` (no messenger when off).
    if ctx.validation_enabled() {
        let state = ctx
            .debug_state()
            .expect("validation enabled => a debug-messenger state is present");
        assert_eq!(
            state.total(),
            0,
            "validation layer reported {} message(s) during the P0 coarse-cull present — \
             see the [vk-validation] log",
            state.total()
        );
    }

    // The pixel gate: cull-ON must equal cull-OFF within +/-CHANNEL_TOL per RGB channel (the cull
    // is a PERF optimization — same surface, fewer marches). Both frames are already RGBA-normalized
    // (the swapchain R/B swap applied), so they are byte-comparable per channel.
    match (off, on) {
        (Some((off_rgba, ow, oh)), Some((on_rgba, nw, nh))) => {
            assert_eq!((ow, oh), (nw, nh), "cull-ON and cull-OFF readback extents must match");
            assert_eq!(
                off_rgba.len(),
                on_rgba.len(),
                "cull-ON and cull-OFF readback byte lengths must match"
            );
            let mut mismatches = 0usize;
            let mut worst = (0u32, 0u32, 0i32);
            for (i, (o, n)) in off_rgba.chunks_exact(4).zip(on_rgba.chunks_exact(4)).enumerate() {
                let mut bad = false;
                for c in 0..3 {
                    let d = (o[c] as i32 - n[c] as i32).abs();
                    if d > CHANNEL_TOL {
                        bad = true;
                        if d > worst.2 {
                            let px = (i as u32) % ow;
                            let py = (i as u32) / ow;
                            worst = (px, py, d);
                        }
                    }
                }
                if bad {
                    mismatches += 1;
                }
            }
            assert_eq!(
                mismatches, 0,
                "P0 coarse cull changed {mismatches} pixel(s) beyond +/-{CHANNEL_TOL} (worst delta \
                 {} at ({}, {})) — the cull must skip empty tiles only, NOT alter the surface",
                worst.2, worst.0, worst.1,
            );
            println!("p0_windowed_coarse_cull: cull-ON == cull-OFF across {ow}x{oh} (0 mismatches)");
        }
        _ => {
            eprintln!(
                "NOTE p0_windowed_coarse_cull: a readback frame did not present (swapchain kept \
                 recreating); validation was still asserted clean"
            );
        }
    }

    drop(renderer);
    // SAFETY: the renderer was dropped above (its `Drop` waits the device idle), so no submission
    // references these resources; `ctx` is still alive; each is destroyed exactly once, in reverse
    // dependency order.
    unsafe {
        frame.destroy(ctx);
        RhiDevice::destroy_buffer(device, staging);
        // CSM Increment 1b: the cascade trio + depth pipeline.
        csm.destroy(ctx);
        RhiDevice::destroy_graphics_pipeline(device, present_pipeline);
        RhiDevice::destroy_bind_group_layout(device, present_layout);
        RhiDevice::destroy_compute_pipeline(device, resolve_pipeline);
        RhiDevice::destroy_bind_group_layout(device, resolve_layout);
        RhiDevice::destroy_compute_pipeline(device, coarse_compute);
        RhiDevice::destroy_compute_pipeline(device, marcher);
        RhiDevice::destroy_bind_group_layout(device, vocab_layout);
        RhiDevice::destroy_graphics_pipeline(device, raster_pipeline);
        // M1 instance-model resources (bind group → buffer → layout, after the pipeline).
        RhiDevice::destroy_bind_group(device, instance_bind_group);
        RhiDevice::destroy_buffer(device, instance_buffer);
        RhiDevice::destroy_bind_group_layout(device, instance_layout);
        RhiDevice::destroy_sampler(device, present_sampler);
        RhiDevice::destroy_sampler(device, depth_sampler);
        RhiDevice::destroy_buffer(device, vertex_buffer);
        RhiDevice::destroy_buffer(device, tiles_buffer);
        clipmap.destroy(ctx);
        RhiDevice::destroy_buffer(device, light_staging);
        RhiDevice::destroy_buffer(device, light_table);
        RhiDevice::destroy_buffer(device, material_table);
        for slot in camera_ring {
            RhiDevice::destroy_buffer(device, slot);
        }
        RhiDevice::destroy_buffer(device, edit_list);
    }
    drop(swapchain);
    // surface / ctx / window are owned by `with_windowed_present` and dropped in-order at its frame end.
}

// ============================================================================
// Engine showcase — a CRISP 512×512-NATIVE multi-light SDF-shadow screenshot.
//
// This drives the EXACT production windowed present (`Renderer::render_gbuffer_frame`,
// the same 3-pass raster-MRT → marcher → deferred-resolve → present-blit) at the native
// `COMPOSITE_W`×`COMPOSITE_H` (512×512) extent and dumps ONE true-resolution BMP (no
// upscale) so the owner can judge the render. Unlike the offscreen screenshot tests
// (hardwired to 64×64, then 8× upscaled → blocky), this is the windowed path that renders
// at the full composite extent.
//
// The scene is the P5 hybrid (a raster-PBR mesh + an SDF body) lit by P6 R1 multi-light
// SDF shadows: a neutral primary directional plus TWO shadow-flagged POINT casters of
// DISTINCT colors (a warm orange and a cool blue). The light table is `shadow_mode == 1`
// + NON-CLUSTERED — exactly the path `p6_r1_multi_light_sdf_shadows_match_oracle` validates
// offscreen, here re-seeded into the windowed `light_table` SSBO (no production-code change:
// the resolve reads `shadow_mode`/`casts_sdf_shadow` from the table header/elements, and the
// windowed `record_gbuffer` already binds the SDF edit-list at resolve binding 10, so the
// per-caster `sdf_soft_shadow_ranged` march runs on hardware).
// ============================================================================

/// The fixed dump path for the 512-native engine showcase frame.
const SHOWCASE_BMP: &str = r"D:\tmp\engine_showcase_512.bmp";

/// The fixed dump path for the 512-native engine SSAO showcase frame (the SAME scene with SSAO
/// ON, dumped under an SSAO-labelled path the orchestrator converts + shows the owner).
const SSAO_BMP: &str = r"D:\tmp\engine_ssao_512.bmp";

/// Mesh foundation M2 — the fixed dump path for the FIRST REAL instanced perspective frame:
/// N boxes drawn through the `use_model_matrix == 1` instanced arm (one registered model-space
/// mesh + an N-affine instance SSBO) co-scened with an SDF sphere so the depth-ownership between
/// the instanced raster boxes and the SDF sphere is the C2 visual proof.
const INSTANCED_PERSP_BMP: &str = r"D:\tmp\engine_instanced_persp.bmp";

/// Mesh foundation M3 — the multi-mesh instanced screenshot path. TWO distinct meshes (a
/// small `u16`-indexed box + a higher-poly `u32`-indexed box), each at several distinct
/// transforms, drawn through the recorder's BATCH LOOP: mesh A at `base_instance == 0`, mesh
/// B at a NONZERO `base_instance` (the C1 GPU proof). If mesh B's instances render at mesh A's
/// positions, the `base_instance` mechanism is broken and the screenshot shows it.
const MULTIMESH_PERSP_BMP: &str = r"D:\tmp\engine_multimesh.bmp";

/// Mesh foundation M4 — the NON-UNIFORM-scale normal screenshot path. Boxes squashed flat and
/// stretched tall, drawn through the `use_model_matrix == 1` instanced arm under directional
/// light. With the M4 inverse-transpose normal the lit shading on the stretched/squashed faces
/// is correct; with the old `mul(m3, normal)` the same faces are over-bright / wrongly shaded.
const NONUNIFORM_NORMALS_BMP: &str = r"D:\tmp\engine_nonuniform_normals.bmp";

/// **The GRAND flagship showcase** — the BMP path for the single-frame scene that combines the
/// MAXIMUM of the engine's shipped rendering: HYBRID SDF+raster mesh in one room, a warm
/// directional sun driving BOTH the CSM hardware cascaded shadow (on the raster boxes) AND the
/// marcher's analytic SDF soft shadow (on the SDF spheres), a cool point light driving the omni
/// POINT cube hardware shadow (on the raster boxes), instancing, and PBR — all in a clean,
/// understandable room. The orchestrator runs the GPU test + converts the BMP for the owner.
const GRAND_SHOWCASE_BMP: &str = r"D:\tmp\engine_grand_showcase.bmp";

/// The showcase sun direction (`L`, the un-normalized "direction TO the light"): upper-LEFT and
/// slightly toward the camera, ~57° elevation. Used BOTH as the marcher's `scene.light_dir` (the
/// A1 cast-shadow march direction) AND the primary directional in [`showcase_light_table`] — they
/// MUST match so the shadow the marcher bakes into `gMaterial.r` lands where the resolve lights
/// from. With this `+y`-dominant `L` the up-facing floor is well-lit (NoL ≈ 0.85) and the
/// sphere/box throw a clear elongated shadow back-and-right across the floor, visible to the
/// down-looking camera.
const SHOWCASE_SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// The showcase SDF body: a clean, realistic studio scene — a wide flat **floor slab** with a
/// **sphere**, a **cube**, and a smaller **sphere** RESTING on it (each primitive's base sits at
/// the slab's top face, `y = -0.5`). The perspective camera ([`showcase_camera`]) looks down at the
/// floor from the front, the warm directional sun ([`SHOWCASE_SUN_DIR`]) rakes from the upper-left,
/// and the marcher's A1 soft shadow casts each body's shadow ACROSS the floor — the floor-and-cast-
/// shadow composition a head-on ortho twin-sphere scene cannot show. Mid-gray dielectric (material
/// slot 0) throughout, so the shape reads from lighting + shadow, not color.
fn showcase_sdf_scene() -> Vec<SdfEdit> {
    vec![
        // The floor: a wide thin slab centered below the origin; its top face is at y = -0.5.
        SdfEdit::box_shape([0.0, -1.0, 0.0], [5.0, 0.5, 4.0], sdf_op::UNION, 0.0),
        // The hero sphere, resting on the floor (center y = -0.5 + r), a touch toward the camera.
        SdfEdit::sphere([0.0, 0.0, 0.2], 0.50, sdf_op::UNION, 0.0),
        // A cube to the left, resting on the floor (center y = -0.5 + half).
        SdfEdit::box_shape([-1.30, -0.18, -0.40], [0.32, 0.32, 0.32], sdf_op::UNION, 0.0),
        // A smaller sphere to the right, resting on the floor.
        SdfEdit::sphere([1.30, -0.22, -0.30], 0.28, sdf_op::UNION, 0.0),
    ]
}

/// The showcase perspective camera: eye in FRONT and ABOVE the scene (`+Z`, `+Y`), looking DOWN at
/// the floor and the hero sphere — so the floor recedes, the bodies sit on it, and their cast
/// shadows are visible. 50° vertical FOV. The basis is the standard non-rolled right-handed frame
/// (`right = [1,0,0]`, `up = right × forward`). The whole scene sits within the marcher's
/// `SDF_TRACE_T_MAX` (≈10) ray range so the finite floor's far edge renders against the dark
/// background.
fn showcase_camera() -> CompositePushConstants {
    // eye = [0, 1.9, 4.0], target = [0, -0.15, -0.30] → forward = normalize(target - eye).
    let forward = [0.0_f32, -0.43035, -0.90266];
    let right = [1.0_f32, 0.0, 0.0];
    let up = [0.0_f32, 0.90266, -0.43035]; // right × forward (unit)
    CompositePushConstants::perspective(
        [0.0, 1.9, 4.0],
        forward,
        right,
        up,
        core::f32::consts::FRAC_PI_3 * 5.0 / 6.0, // 50° vertical FOV (π/3 · 5/6)
        COMPOSITE_W,
        COMPOSITE_H,
    )
}

/// The showcase raster mesh is DEGENERATE (zero-area): the realistic showcase is ALL-SDF — the
/// floor is an SDF slab marched by the perspective camera, so a raster floor (which the harness
/// projects with the ORTHO `ortho_mvp_bytes` MVP) would land in the wrong place and double the
/// floor. Six identical vertices ⇒ two zero-area triangles ⇒ NO fragments ⇒ the raster pass only
/// clears the depth attachment to far (`MESH_DEPTH_CLEAR`), so `has_mesh == false` for every pixel
/// and the marcher OWNS the whole frame (the SDF floor + bodies).
fn showcase_quad_vertices() -> Vec<Vertex> {
    let v = Vertex { position: [0.0, 0.0, 0.0], normal: [0.0, 0.0, 1.0], color: [1.0, 1.0, 1.0, 1.0] };
    vec![v, v, v, v, v, v]
}

/// The showcase light table: a single warm-white directional **sun** (the PRIMARY directional —
/// its visibility reads the marcher's `gMaterial.r`, i.e. the A1 soft shadow the marcher marched
/// toward [`SHOWCASE_SUN_DIR`], so the bodies cast a real shadow across the floor) + a soft
/// **sky/hemisphere ambient** fill that lifts the shadowed floor off pure black and tints it
/// cool (sky) vs the warm sun — the warm-key/cool-fill contrast of a realistic render. NON-
/// CLUSTERED, `shadow_mode == 0` (no per-caster march — the cast shadow is the primary
/// directional's `gMaterial.r`). `l0a_count == 2` (sun + sky), `point_spot_count == 0`.
fn showcase_light_table() -> (GoldenLightHeader, Vec<GoldenLight>) {
    let header = GoldenLightHeader::new(2, 0, 1.0);
    let lights = vec![
        // The sun: warm white, raking from the upper-left ([`SHOWCASE_SUN_DIR`] — the SAME vector
        // as `scene.light_dir`, so the marched cast shadow matches the lit direction). Illuminance
        // tuned so the mid-gray floor lights to ~0.7 without clipping to white.
        GoldenLight::directional(SHOWCASE_SUN_DIR, [1.0, 0.96, 0.90], 2.8),
        // Sky/hemisphere ambient: a cool-blue sky over a warm-dark ground, so the cast shadow is a
        // readable cool gray (not black) and the contact AO still darkens it.
        GoldenLight::sky([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]),
    ];
    (header, lights)
}

/// The marcher's cast-shadow direction (`L`, direction TO the light) = the FIRST DIRECTIONAL light in
/// the showcase's table, so the marched cast shadow lands where the resolve lights from (single
/// source: the light table — no separate hardcoded sun vector to drift). Every existing showcase puts
/// its `SHOWCASE_SUN_DIR` directional at element 0, so this is byte-identical to the prior
/// `light_dir: SHOWCASE_SUN_DIR`; the MDF demo swaps in its own angled directional and this tracks it.
/// Falls back to `SHOWCASE_SUN_DIR` if a table somehow carries no directional.
fn marcher_light_dir(lights: &[GoldenLight]) -> [f32; 3] {
    lights
        .iter()
        .find(|l| l.kind() == GOLDEN_LIGHT_KIND_DIRECTIONAL)
        .map(|l| [l.dir_kind[0], l.dir_kind[1], l.dir_kind[2]])
        .unwrap_or(SHOWCASE_SUN_DIR)
}

/// Packs a `GoldenLightHeader` + `GoldenLight[]` into the std430 light-table SSBO word stream
/// (`[header (16 words) || GpuLight[] (12 words each)]`) the resolve reads at binding 6.
/// Host mirror of `boyko_render::light`'s packing; identical to the offscreen test's
/// `pack_light_table`.
fn pack_showcase_light_table(header: &GoldenLightHeader, lights: &[GoldenLight]) -> Vec<u32> {
    let mut words = vec![0u32; GOLDEN_LIGHT_HEADER_BASE_WORDS + lights.len() * 12];
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

/// Reads back a BI_RGB BMP's `biWidth` / `biHeight` (`biHeight` is negative for the top-down
/// images [`write_bmp`] emits, so the magnitude is the height). Returns `None` if the file is
/// not a `"BM"` 54-byte-header BMP. Used to VERIFY the dumped showcase is 512×512 native.
fn read_bmp_dimensions(bytes: &[u8]) -> Option<(i32, i32)> {
    if bytes.len() < 54 || &bytes[0..2] != b"BM" {
        return None;
    }
    let w = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
    let h = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
    Some((w, h.abs()))
}

/// The fixed dump path for the 512-native engine MESH-floor SSAO showcase frame (Render P7
/// Unlock-2): a flat RASTER MESH quad + an SDF sphere standing in front, SSAO ON — the mesh
/// (A2 == 1.0) visibly receives the sphere's contact AO, which the A2 SDF-march cannot give it.
const SSAO_MESH_BMP: &str = r"D:\tmp\engine_ssao_mesh_512.bmp";

/// Render P7-Q2 — the SSAO quality-LADDER BMP dump paths (the SAME mesh+SDF scene rendered with SSAO
/// OFF / Low / Medium / High, so the orchestrator converts + shows the owner the quality ladder).
const SSAO_LADDER_OFF_BMP: &str = r"D:\tmp\engine_ssao_off.bmp";
const SSAO_LADDER_LOW_BMP: &str = r"D:\tmp\engine_ssao_low.bmp";
const SSAO_LADDER_MEDIUM_BMP: &str = r"D:\tmp\engine_ssao_medium.bmp";
const SSAO_LADDER_HIGH_BMP: &str = r"D:\tmp\engine_ssao_high.bmp";

/// The fixed dump path for the ORTHO HYBRID-ROOM showcase (multi-object mesh + SDF, step 1 of
/// the hybrid-mesh-room build): a mesh backdrop wall + mesh cubes in front + SDF bodies casting
/// the marcher's analytic shadow/AO onto the mesh. The orchestrator runs the GPU test + converts.
const HYBRID_BMP: &str = r"D:\tmp\engine_hybrid_room.bmp";

/// Render Shadow Phase 1 — the capsule-character screenshot path: a coarse 6-capsule
/// humanoid (a character capsule-proxy) standing on the mesh floor in front of the back
/// wall, casting the marcher's analytic SDF shadow onto the mesh as a readable humanoid
/// silhouette. The orchestrator runs the GPU test + converts the BMP.
const CAPSULE_CHARACTER_BMP: &str = r"D:\tmp\engine_capsule_character.bmp";

/// Render Shadow Phase 3 — the Screen-Space Contact Shadows (SSCS) A/B screenshot paths: the
/// SAME capsule character feet-on-floor scene, dumped with `contact_shadow_mode` OFF (the A/B
/// reference + the 0%-gate visual proof) and ON (the contact-shadow tightening visible where
/// the feet meet the floor). The orchestrator runs the GPU test + converts the BMPs.
const CONTACT_SHADOW_OFF_BMP: &str = r"D:\tmp\engine_contact_shadow_off.bmp";
const CONTACT_SHADOW_ON_BMP: &str = r"D:\tmp\engine_contact_shadow.bmp";

/// MDF Stage-2c — the Mesh-Distance-Field shadow screenshot path: a PROCEDURAL static TORUS (an
/// asymmetric shape — a ring with a hole, so its cast shadow reads) RASTER-rendered standing in
/// front of the mesh floor, baked into the dedicated dense `MeshSdfTexture`, casting its baked MDF
/// soft shadow onto the raster floor (the visible mesh IS its own invisible shadow proxy). The
/// orchestrator runs the GPU test + converts the BMP.
const MDF_SHADOW_BMP: &str = r"D:\tmp\engine_mdf_shadow.bmp";

/// The per-showcase variable scene: the SDF edit list, the marcher/resolve camera push, the light
/// table (header + elements), and the RASTER MESH (vertices + MVP). The shared [`run_showcase_dump`]
/// body holds everything else (pipelines, barriers, the dump tail) constant. Built by the per-test
/// builders so the all-SDF perspective showcase and the ORTHO mesh-floor SSAO showcase share ONE
/// recorder/dump body without duplicating its ~400 lines.
struct ShowcaseConfig {
    /// The SDF edit list (the marcher field + the resolve per-caster shadow march).
    sdf: Vec<SdfEdit>,
    /// The marcher + resolve + SSAO camera push (perspective for the all-SDF showcase; ORTHO for
    /// the mesh-floor showcase, whose `md * T_MAX == t_mesh` decode the raster MVP below matches).
    camera: CompositePushConstants,
    /// The light table (already `ssao_mode`-armed by the builder).
    light_header: GoldenLightHeader,
    /// The light table elements (directional + sky [+ point/spot]).
    light_elems: Vec<GoldenLight>,
    /// The raster mesh vertices (DEGENERATE zero-area for the all-SDF showcase; a real floor quad
    /// for the mesh-floor showcase; an arbitrary multi-object mesh for the hybrid room). A `Vec`
    /// so any vertex count is supported — the draw is length-driven (`vertex_count == len`).
    vertices: Vec<Vertex>,
    /// The raster MVP push (the ORTHO `ortho_mvp_bytes` — its `(CAM_Z - z)/T_MAX` depth is the
    /// convention the marcher's `t_mesh = md * T_MAX` ownership/gViewT decode reconstructs exactly).
    mvp: [u8; MVP_BYTES as usize],
    /// Render P7-Q2 — the SSAO state: `Some(quality)` records the SSAO pass binding that variant's
    /// pre-compiled `.spv` (an `SSAO_QUALITY_*` index) AND arms `ssao_mode == 1`; `None` is SSAO OFF
    /// (no SSAO pass recorded — `scene.ssao = None` — AND `ssao_mode == 0`, the byte-identical 0%-gate
    /// reference for the ladder's `_off` frame). The builder sets the `ssao_mode` on `light_header`.
    ssao_quality: Option<usize>,
    /// MDF Stage-2c — the mesh-distance-field SHADOW caster. `None` = no MDF (every existing
    /// showcase): the marcher binds the brick atlas as a binding-15 placeholder + gates the
    /// mesh-shadow path OFF (byte-identical). `Some((positions, indices))` = a static raster mesh
    /// whose dense SDF is baked + uploaded into the [`crate::mesh_sdf_texture::MeshSdfTexture`] @15;
    /// `run_showcase_dump` writes the [`MeshSdfParams`] b5 UBO tail + arms `mesh_sdf_enabled` so the
    /// mesh casts its baked MDF soft shadow onto the raster floor. The mesh is the SAME geometry the
    /// `vertices` raster draw renders (the visible mesh IS its own invisible shadow proxy).
    mesh_sdf: Option<MeshSdfCaster>,
    /// Mesh foundation M2 — the OPTIONAL instanced-arm draw. `None` (every pre-M2 showcase): the
    /// `vertices`/`mvp` LEGACY merged draw runs (`scene.mesh_draw = None`, byte-identical). `Some`
    /// switches pass A to an INSTANCED INDEXED draw of ONE registered model-space mesh placed by N
    /// non-identity affines (`scene.mesh_draw = Some(GBufferMeshDraw)`); `run_showcase_dump`
    /// registers the mesh in a `MeshRegistry` + uploads the instance SSBO. The `mvp` MUST then carry
    /// `use_model_matrix == 1` (the caller sets byte 84). The `vertices` field stays a (degenerate)
    /// legacy buffer the recorder no longer draws on this path.
    instanced: Option<InstancedMesh>,
    /// CSM Increment 1b (Rung A) — the OPTIONAL cascade shadow demo. `None` (every existing
    /// showcase): the CSM depth pass is OFF + `csm_mode == 0` (the 0%-gate; the cascade trio is
    /// bound-but-unread). `Some(fit)` arms the depth pass: `run_showcase_dump` uploads `fit` into the
    /// cascade UBO, pushes its `view_proj` into the depth pass, sets `csm_mode == 1` on the light
    /// header, and the resolve `min`-combines the EXACT raster-mesh hard shadow onto the floor. The
    /// casters are the showcase's instanced batches (so `instanced` MUST be `Some` for a visible
    /// caster). Rung A is a SINGLE cascade (`c == 0`).
    csm: Option<CsmDemoFit>,
    /// Shadow Phase 5 Inc-1-GPU — the OPTIONAL sparse SPOT shadow demo. `None` (every existing
    /// showcase): the punctual depth pass is OFF + `punctual_shadow_mode == 0` (the 0%-gate; the
    /// atlas trio is bound-but-unread). `Some(fit)` arms the depth pass: `run_showcase_dump` uploads
    /// `fit` into the atlas UBO, pushes its `view_proj` into the depth pass, sets
    /// `punctual_shadow_mode == 1` on the light header (+ the spot's `dir_kind.w` slot=0 packed into
    /// the light table), and the resolve MULTIPLIES the EXACT raster-mesh hard shadow into the spot's
    /// contribution INSIDE the cone. The casters are the showcase's instanced batches (so `instanced`
    /// MUST be `Some` for a visible caster). Inc 1 is a SINGLE spot (slot 0).
    spot_atlas: Option<SpotDemoFit>,
}

/// Shadow Phase 5 Inc-1-GPU — one fitted atlas face in a [`SpotDemoFit`]: the column-major
/// world→light-clip `view_proj` (the SAME bytes the depth pass pushes + the resolve UBO holds, the
/// O1 single-matrix pin), plus the POINT-shared `light_pos`/`inv_range` lanes (unused by the SPOT
/// NDC-z compare). Mirrors `boyko_render::FaceTransform`.
#[derive(Clone, Copy)]
struct SpotFace {
    /// Column-major `perspective · light_view` (world → light clip), 16 floats.
    view_proj: [f32; 16],
    /// World-space light position (Inc-2 POINT cube; unused by the SPOT NDC-z compare).
    light_pos: [f32; 3],
    /// Reciprocal of the light range (Inc-2 POINT; unused by SPOT).
    inv_range: f32,
    /// Shadow Phase 5 Inc-2: the per-layer TYPE — `true` = a POINT cube face (the depth pass binds
    /// the `point_depth_pipeline` + stamps the `cam_eye@64` `light_pos`/`inv_range` lane), `false` =
    /// a SPOT face (the NDC-z `depth_pipeline`). A SPOT fit is all-`false`; a POINT fit is six
    /// contiguous `true` faces.
    is_point: bool,
}

/// Shadow Phase 5 Inc-1-GPU — the atlas FIT a [`ShowcaseConfig`] carries: up to
/// [`SPOT_ATLAS_SLOTS`] fitted faces + the active count. A real app derives these from
/// `boyko_render::resolve_shadow_atlas`; the demo hand-fits ONE spot face (slot 0). Inc 1 is
/// `active_layers == 1`.
#[derive(Clone, Copy)]
struct SpotDemoFit {
    /// The fitted atlas faces; only `[0..active_layers)` are valid (the rest zeroed,
    /// bound-but-unread).
    faces: [SpotFace; SPOT_ATLAS_SLOTS as usize],
    /// The number of valid atlas layers (`1..=SPOT_ATLAS_SLOTS`) — mirrors
    /// `ResolvedShadowAtlas::active_layers`.
    active_layers: u32,
}

/// CSM Increment 3 (Rung B) — one hand-fitted cascade in a [`CsmDemoFit`]: the column-major
/// world→light-clip `view_proj` (the SAME bytes the depth VS pushes + the resolve UBO holds, the
/// O1 single-matrix pin), the VIEW-SPACE `split_far` (the cascade-SELECT boundary), and the
/// world-space `texel_size` (the resolve's normal-bias scale). Mirrors `boyko_render::CascadeData`.
#[derive(Clone, Copy)]
struct CsmDemoCascade {
    /// Column-major `ortho · light_view` (world → light clip), 16 floats.
    view_proj: [f32; 16],
    /// The VIEW-SPACE far distance of this cascade — the SELECT boundary (`view_z < split_far`).
    split_far: f32,
    /// The world-space size of one shadow texel (the resolve's normal-bias scale).
    texel_size: f32,
}

/// CSM Increment 3 (Rung B) — the N-cascade FIT a [`ShowcaseConfig`] carries: up to
/// [`CSM_MAX_CASCADES`] hand-fitted cascades + the active count. A real app derives these from
/// `boyko_render::resolve_csm`; the demo hand-fits N split planes covering the floor's near→far
/// range so casters at increasing distance land in cascades 0, 1, 2 (the SELECT) and the cascade
/// boundaries cross-fade (the BLEND). Rung A is `active_count == 1` (the original single cascade).
#[derive(Clone, Copy)]
struct CsmDemoFit {
    /// The fitted cascades; only `[0..active_count)` are valid (the rest zeroed, bound-but-unread).
    cascades: [CsmDemoCascade; CSM_MAX_CASCADES],
    /// The number of valid cascades (`1..=CSM_MAX_CASCADES`) — mirrors `ResolvedCsm::active_count`.
    active_count: u32,
}

/// Mesh foundation M2/M3 — the instanced-draw spec a [`ShowcaseConfig`] carries: a LIST of
/// model-space meshes, each registered into a [`MeshRegistry`]-shaped table and drawn by its
/// own `affines` non-identity 3x4 ROW-MAJOR instance matrices through the
/// `use_model_matrix == 1` arm. M2 carries ONE mesh (a 1-batch list); M3 carries ≥2 distinct
/// meshes to exercise the recorder's batch loop + the nonzero `base_instance` (the C1 GPU
/// proof) + mixed `u16`/`u32` index width (O3).
struct InstancedMesh {
    /// The per-mesh draw specs, in registration order. The shared instance ring is built by
    /// concatenating each mesh's affines in this order (mesh 0 at base 0, mesh 1 at base
    /// `meshes[0].affines.len()`, …), so mesh `k`'s `base_instance` is the prefix-sum of the
    /// prior meshes' instance counts — NONZERO for every mesh after the first.
    meshes: Vec<InstancedMeshEntry>,
    /// Batch indices into [`meshes`](Self::meshes) that are RECEIVER-ONLY — visible + shadowed in
    /// the main pass but EXCLUDED from the shadow DEPTH passes (they do not cast). Use for a room
    /// shell (floor/walls) so it does not stamp a spurious shadow over the scene. Empty = every
    /// mesh casts (the prior all-casters behavior, byte-identical).
    non_casters: Vec<usize>,
}

/// One mesh in an [`InstancedMesh`] batch list: its model-space geometry + the per-instance
/// affines that place its copies. The host mirrors the M3 gather's per-mesh bucket: the build
/// concatenates `affines` into the shared ring at this mesh's prefix-sum offset.
struct InstancedMeshEntry {
    /// The model-space mesh vertices (registered into the mesh table).
    vertices: Vec<Vertex>,
    /// The mesh's triangle index list (`0..vertices.len()`).
    indices: Vec<u32>,
    /// This mesh's per-instance 3x4 row-major affines (one [`InstanceModelCol`] each); the
    /// mesh's batch issues `instance_count == affines.len()` instances at its `base_instance`.
    affines: Vec<[f32; 12]>,
}

/// Mesh foundation M3 — the BUILT GPU resources for an [`InstancedMesh`] batch list: the
/// per-mesh draw entries + the ONE shared instance SSBO (the concatenated ring) the recorder
/// binds once. The `run_showcase_dump` builds it inline (no `boyko_render` dep), borrows
/// `GBufferMeshDraw`s from `batches` for the scene, and tears it down explicitly.
struct InstancedGpu {
    /// The per-mesh GPU draw entries, in registration order (mesh 0's `base_instance == 0`,
    /// mesh 1's `== meshes[0].instance_count`, …). The recorder issues one indexed draw each.
    batches: Vec<InstancedGpuBatch>,
    /// The ONE shared N-instance model SSBO holding every mesh's affines concatenated; the
    /// VS indexes it by `base_instance + SV_InstanceID`. Bound once via `instance_bind_group`.
    instance_ssbo: BoundBuffer,
    /// The shared SSBO's bind group on the gbuffer set-0 layout (bound once for the batch list).
    instance_bind_group: VulkanBindGroup,
}

/// One built per-mesh draw entry in an [`InstancedGpu`]: the mesh's GPU buffers + the
/// `GBufferMeshDraw` metadata (`index_count` / `index_type` / `base_instance` /
/// `instance_count`). The scene's `mesh_draw` slice is built by borrowing these.
struct InstancedGpuBatch {
    /// The mesh's model-space vertex buffer.
    vertex_buffer: BoundBuffer,
    /// The mesh's index buffer (`index_type`-wide).
    index_buffer: BoundBuffer,
    /// The mesh's index count.
    index_count: u32,
    /// The mesh's bound index width as the agnostic `i32` (O3 mixed `Uint16`/`Uint32`).
    index_type: i32,
    /// This mesh's bucket start in the shared ring (the prefix-sum offset — NONZERO for every
    /// mesh after the first, the C1 GPU proof).
    base_instance: u32,
    /// This mesh's instance count (its bucket length).
    instance_count: u32,
    /// Whether this batch CASTS shadows (false = receiver-only: rasterized + shadowed in the main pass, skipped in the shadow depth passes). Mirrors the source `InstancedMesh::non_casters`.
    casts_shadow: bool,
}

/// A static mesh's `(positions, triangle_indices)` — the MDF Stage-2c shadow-caster geometry the
/// dense `MeshSdfTexture` baker consumes (a `type` alias so the `ShowcaseConfig` field + `torus_mesh`
/// return stay readable — clippy::type_complexity).
type MeshSdfCaster = (Vec<[f32; 3]>, Vec<[u32; 3]>);

/// The default all-SDF perspective showcase config (the historical [`run_showcase_dump`] scene):
/// the SDF floor + bodies, the down-looking [`showcase_camera`], the multi-light table, and the
/// DEGENERATE zero-area raster mesh (so the marcher owns the whole frame). `ssao_quality`:
/// `Some(SSAO_QUALITY_*)` arms the SSAO pass at that variant; `None` is SSAO OFF (the 0%-gate
/// reference — `ssao_mode == 0`, no SSAO pass).
fn showcase_config(ssao_quality: Option<usize>) -> ShowcaseConfig {
    let (light_header, light_elems) = showcase_light_table();
    let ssao_mode = if ssao_quality.is_some() { 1 } else { 0 };
    ShowcaseConfig {
        sdf: showcase_sdf_scene(),
        camera: showcase_camera(),
        light_header: light_header.with_ssao_mode(ssao_mode),
        light_elems,
        vertices: showcase_quad_vertices(),
        mvp: ortho_mvp_bytes(),
        ssao_quality,
        mesh_sdf: None,
        // M2: no instanced mesh — the legacy merged draw runs (byte-identical).
        instanced: None,
        // CSM Increment 1b: OFF (the 0%-gate — no cascade depth pass, `csm_mode == 0`).
        csm: None,
        // Shadow Phase 5 Inc-1-GPU: no sparse spot shadow (the 0%-gate — no punctual depth pass,
        // `punctual_shadow_mode == 0`).
        spot_atlas: None,
    }
}

/// **Engine showcase — a CRISP 512×512-NATIVE multi-light SDF-shadow screenshot.**
///
/// Renders the production windowed present (the raster-PBR mesh + the SDF twin-sphere body)
/// at the native 512×512 composite extent, lit by 1 directional + 2 shadow-flagged colored
/// point casters (`shadow_mode == 1`, NON-CLUSTERED), reads back the 512 frame, and writes a
/// TRUE 512×512 24-bit BMP to [`SHOWCASE_BMP`] — NO upscaling. Verifies the dumped BMP header
/// is 512×512. The orchestrator converts it to PNG + opens it for the owner.
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1` so the
/// (broken-on-this-box) validation layer does not crash the process; the screenshot is the
/// deliverable, not a golden assertion.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the screenshot"]
fn engine_showcase_512_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine showcase 512",
        SHOWCASE_BMP,
        showcase_config(Some(SSAO_QUALITY_MEDIUM)),
        false,
    );
}

/// **Engine SSAO showcase — the SAME crisp 512×512-native scene WITH SSAO ON, dumped to
/// [`SSAO_BMP`].** Identical to [`engine_showcase_512_screenshot_dump`] (the showcase already
/// arms `ssao_mode == 1` + `scene.ssao = Some(..)`, so the SSAO contact-crease darkening is in the
/// frame) — this sibling writes the SSAO-labelled BMP path the orchestrator converts + shows the
/// owner for the SSAO A/B visual sign-off.
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the SSAO screenshot"]
fn engine_ssao_512_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine SSAO 512",
        SSAO_BMP,
        showcase_config(Some(SSAO_QUALITY_MEDIUM)),
        false,
    );
}

/// **Render P7 Unlock-2 — engine MESH-FLOOR SSAO showcase (the visual).** A REAL RASTER MESH quad
/// floor (the ORTHO [`quad_vertices`] at `MESH_Z == 1.0`, whose A2 `gMaterial.g` == 1.0 — the
/// raster has no analytic SDF AO) + an SDF sphere standing IN FRONT of it ([`mesh_ssao_sphere`],
/// near pole at `z == 1.55 > MESH_Z`), lit + SSAO ON. The sphere casts CONTACT AO onto the mesh
/// around its silhouette — darkening the A2 SDF-march CANNOT produce on the mesh (its A2 == 1.0,
/// so SSAO is its only AO). Dumps a TRUE 512×512 BMP to [`SSAO_MESH_BMP`].
///
/// This is the ORTHO camera (matching the offscreen non-vacuity gate
/// `ssao_darkens_mesh_near_sdf_occluder`), so the raster MVP's `(CAM_Z - z)/T_MAX` depth is exactly
/// the convention the marcher's `t_mesh = md * T_MAX` ownership + gViewT decode reconstructs — no
/// perspective-MVP-vs-ray-gen alignment is needed. (The full PERSPECTIVE mesh-floor MVP is deferred:
/// the marcher decodes mesh depth as `md * T_MAX`, a LINEAR-in-ray-distance convention a standard
/// perspective projection's nonlinear NDC depth does not satisfy, so a perspective mesh floor would
/// need a custom depth-writing VS or a marcher decode change — both out of this pass's scope. The
/// ORTHO mesh floor delivers the same mesh-receives-SSAO visual with an exactly aligned gate.)
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the mesh-floor SSAO screenshot"]
fn engine_ssao_mesh_512_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine SSAO mesh floor 512",
        SSAO_MESH_BMP,
        mesh_ssao_config(Some(SSAO_QUALITY_MEDIUM)),
        false,
    );
}

/// **Render P7-Q2 — engine SSAO QUALITY-LADDER screenshot dump (the visual oracle).** Renders the
/// SAME mesh+SDF SSAO scene ([`mesh_ssao_config`] — the raster mesh quad floor + the SDF sphere in
/// front) FOUR times and dumps a TRUE 512×512 BMP per quality so the orchestrator converts + shows
/// the owner the ladder:
///   - [`SSAO_LADDER_OFF_BMP`] — SSAO OFF (`scene.ssao = None`, `ssao_mode == 0`, the 0%-gate frame).
///   - [`SSAO_LADDER_LOW_BMP`] — the Low variant pipeline (2×3×2 = 12 taps).
///   - [`SSAO_LADDER_MEDIUM_BMP`] — the Medium variant (2×4×2 = 16 taps; == today's shipped path).
///   - [`SSAO_LADDER_HIGH_BMP`] — the High variant (3×6×2 = 36 taps).
///
/// Each is a fresh windowed render (`run_showcase_dump` boots + tears down its own device per call),
/// so the ladder is four independent frames the owner compares side-by-side (the mesh contact-AO ring
/// sharpens / spreads with the tap budget; OFF is the no-AO baseline).
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the SSAO quality ladder"]
fn engine_ssao_ladder_off_dump() {
    // ONE window/context per process: a windowed boot only survives the FIRST showcase dump in a
    // process (later boots hit "swapchain kept recreating"), so each ladder rung is its OWN test —
    // the orchestrator runs them in separate processes.
    run_showcase_dump("boyko_engine SSAO ladder OFF", SSAO_LADDER_OFF_BMP, mesh_ssao_config(None), false);
}

/// SSAO ladder rung — LOW (2x3). See [`engine_ssao_ladder_off_dump`] for the one-per-process note.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU"]
fn engine_ssao_ladder_low_dump() {
    run_showcase_dump("boyko_engine SSAO ladder LOW", SSAO_LADDER_LOW_BMP, mesh_ssao_config(Some(SSAO_QUALITY_LOW)), false);
}

/// SSAO ladder rung — MEDIUM (2x4, == today). See [`engine_ssao_ladder_off_dump`].
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU"]
fn engine_ssao_ladder_medium_dump() {
    run_showcase_dump("boyko_engine SSAO ladder MEDIUM", SSAO_LADDER_MEDIUM_BMP, mesh_ssao_config(Some(SSAO_QUALITY_MEDIUM)), false);
}

/// SSAO ladder rung — HIGH (3x6). See [`engine_ssao_ladder_off_dump`].
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU"]
fn engine_ssao_ladder_high_dump() {
    run_showcase_dump("boyko_engine SSAO ladder HIGH", SSAO_LADDER_HIGH_BMP, mesh_ssao_config(Some(SSAO_QUALITY_HIGH)), false);
}

/// **Hybrid-room screenshot dump (step 1 of the hybrid-mesh-room build).** Renders the ORTHO
/// hybrid room ([`hybrid_room_config`]: an arbitrary multi-object mesh — a backdrop wall + 3 cubes
/// with per-vertex face normals — plus several SDF bodies standing in front so the marcher's
/// analytic shadows + AO fall on the mesh) at the native 512×512 composite extent and writes a
/// TRUE 512 BMP to [`HYBRID_BMP`]. Proves the multi-object-mesh + per-vertex-normal infra on the
/// PROVEN ORTHO path (the orchestrator adds the perspective camera in step 2).
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the hybrid-room screenshot"]
fn engine_hybrid_room_512_screenshot_dump() {
    run_showcase_dump("boyko_engine hybrid room 512", HYBRID_BMP, hybrid_room_config(), false);
}

/// The unlock-2 SDF occluder sphere for the mesh-floor showcase: ONE sphere standing in FRONT of
/// the mesh quad (`MESH_Z == 1.0`) — center at `+Z`, near pole at `z == 1.55 > MESH_Z`, so the SDF
/// wins ownership where it covers and the mesh stands elsewhere (the SAME geometry as the offscreen
/// `ssao_darkens_mesh_near_sdf_occluder` gate, lifted to the 512 composite).
fn mesh_ssao_sphere() -> Vec<SdfEdit> {
    vec![SdfEdit::sphere([0.0, 0.0, 0.95], 0.60, sdf_op::UNION, 0.0)]
}

/// The ORTHO mesh-floor SSAO showcase config (Render P7 Unlock-2): the real raster mesh quad floor,
/// the SDF sphere in front, the ORTHO camera, and the SSAO-armed showcase light table.
/// `ssao_quality`: `Some(SSAO_QUALITY_*)` arms the SSAO pass at that variant; `None` is SSAO OFF.
fn mesh_ssao_config(ssao_quality: Option<usize>) -> ShowcaseConfig {
    let (light_header, light_elems) = showcase_light_table();
    let ssao_mode = if ssao_quality.is_some() { 1 } else { 0 };
    ShowcaseConfig {
        sdf: mesh_ssao_sphere(),
        camera: CompositePushConstants::ortho(COMPOSITE_W, COMPOSITE_H),
        light_header: light_header.with_ssao_mode(ssao_mode),
        light_elems,
        // A REAL floor quad (NOT the degenerate all-SDF mesh): its A2 == 1.0, so the contact AO is
        // pure SSAO. The ORTHO MVP lands it exactly where the marcher's `t_mesh` decode expects.
        vertices: quad_vertices().to_vec(),
        mvp: ortho_mvp_bytes(),
        ssao_quality,
        mesh_sdf: None,
        // M2: no instanced mesh — the legacy merged draw runs (byte-identical).
        instanced: None,
        // CSM Increment 1b: OFF (the 0%-gate — no cascade depth pass, `csm_mode == 0`).
        csm: None,
        // Shadow Phase 5 Inc-1-GPU: no sparse spot shadow (the 0%-gate — no punctual depth pass,
        // `punctual_shadow_mode == 0`).
        spot_atlas: None,
    }
}

// === The 3D hybrid room — PERSPECTIVE step-2 named consts (orchestrator-tunable). ===
// All positions are world-space, y-up. The mesh floor (y = 0) + back wall (z = -4) +
// 2 mesh cubes RESTING on the floor form the room; 3 SDF bodies rest on the floor in
// front and cast the marcher's analytic shadow/AO onto the mesh.

/// The room camera EYE (world). Above + in front, looking down into the room. Pulled back
/// 2.5 units along -forward from the original `[0, 3.2, 4.5]` (owner: the framing was too
/// tight) — moving along the view axis keeps `ROOM_CAM_FORWARD`/`RIGHT`/`UP` unchanged (the
/// orthonormal-basis assert in `room_camera` still holds) and the CSM cascade fit reads this
/// same eye, so the cascades follow the wider frustum automatically.
const ROOM_CAM_EYE: [f32; 3] = [0.0, 4.128478, 6.821193];
/// The room camera LOOK-AT target (world).
const ROOM_CAM_TARGET: [f32; 3] = [0.0, 0.8, -1.5];
/// The room camera vertical FOV (radians) — 50°.
const ROOM_CAM_FOV_Y: f32 = 50.0 * core::f32::consts::PI / 180.0;
/// The room camera right-handed basis, precomputed from EYE/TARGET (verified orthonormal by
/// the `debug_assert!` in [`room_camera`]). forward = normalize(target - eye);
/// right = normalize(cross(forward, +Y)); up = cross(right, forward).
const ROOM_CAM_FORWARD: [f32; 3] = [0.0, -0.371391, -0.928477];
const ROOM_CAM_RIGHT: [f32; 3] = [1.0, 0.0, 0.0];
const ROOM_CAM_UP: [f32; 3] = [0.0, 0.928477, -0.371391];

/// The 2 mesh cubes resting on the floor: center / half-extent / color. Each bottom face sits a
/// hair (0.01) ABOVE the floor plane (y=0) — a coplanar bottom would Z-fight the floor under
/// `LESS` with no depth bias (the jagged contact line). 0.01 is sub-pixel at this camera distance.
const ROOM_CUBE_A: ([f32; 3], [f32; 3], [f32; 4]) =
    ([-1.6, 0.51, -1.5], [0.5, 0.5, 0.5], [0.80, 0.34, 0.28, 1.0]); // warm terracotta
const ROOM_CUBE_B: ([f32; 3], [f32; 3], [f32; 4]) =
    ([1.4, 0.36, -2.2], [0.35, 0.35, 0.35], [0.28, 0.46, 0.78, 1.0]); // cool blue

/// The 3 SDF bodies resting on the floor (center.y = radius / half-height): a hero sphere,
/// a smaller sphere, and a box. HARD unions (4th arg `0.0` = SMOOTHNESS, mid-gray material 0).
const ROOM_SDF_SPHERE_A: ([f32; 3], f32) = ([0.0, 0.7, -1.0], 0.7);
const ROOM_SDF_SPHERE_B: ([f32; 3], f32) = ([1.5, 0.5, -0.5], 0.5);
const ROOM_SDF_BOX: ([f32; 3], [f32; 3]) = ([-1.2, 0.51, 0.2], [0.5, 0.5, 0.5]); // bottom 0.01 above the floor: a coplanar bottom Z-fought the mesh floor (the "strange front shadow")

/// The mesh floor + wall colors (neutral grays; the wall a touch different so the corner reads).
const ROOM_FLOOR_COLOR: [f32; 4] = [0.55, 0.55, 0.57, 1.0];
const ROOM_WALL_COLOR: [f32; 4] = [0.45, 0.46, 0.50, 1.0];

/// The room camera push: a PERSPECTIVE [`CompositePushConstants`] matching the
/// [`perspective_mvp_bytes`] the raster mesh uses (same eye / basis / fov / aspect). The
/// `debug_assert!` guards the precomputed basis (unit, orthogonal, right-handed) against an
/// edit of the EYE/TARGET consts that forgets to recompute the basis.
fn room_camera() -> CompositePushConstants {
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (f, r, u) = (ROOM_CAM_FORWARD, ROOM_CAM_RIGHT, ROOM_CAM_UP);
    // The precomputed `forward` must equal normalize(TARGET - EYE) (guards an EYE/TARGET edit
    // that forgets to recompute), and the basis must be orthonormal + right-handed.
    let raw = [
        ROOM_CAM_TARGET[0] - ROOM_CAM_EYE[0],
        ROOM_CAM_TARGET[1] - ROOM_CAM_EYE[1],
        ROOM_CAM_TARGET[2] - ROOM_CAM_EYE[2],
    ];
    let inv_len = 1.0 / dot(raw, raw).sqrt();
    let fwd = [raw[0] * inv_len, raw[1] * inv_len, raw[2] * inv_len];
    debug_assert!(
        (f[0] - fwd[0]).abs() < 1e-3
            && (f[1] - fwd[1]).abs() < 1e-3
            && (f[2] - fwd[2]).abs() < 1e-3
            && (dot(f, f) - 1.0).abs() < 1e-3
            && (dot(r, r) - 1.0).abs() < 1e-3
            && (dot(u, u) - 1.0).abs() < 1e-3
            && dot(f, r).abs() < 1e-3
            && dot(f, u).abs() < 1e-3
            && dot(r, u).abs() < 1e-3,
        "invariant: ROOM_CAM_* basis must be orthonormal + match normalize(TARGET - EYE) \
         (recompute the basis consts after editing EYE/TARGET)"
    );
    CompositePushConstants::perspective(
        ROOM_CAM_EYE,
        ROOM_CAM_FORWARD,
        ROOM_CAM_RIGHT,
        ROOM_CAM_UP,
        ROOM_CAM_FOV_Y,
        COMPOSITE_W,
        COMPOSITE_H,
    )
}

/// The 3D-room SDF bodies (step 2, PERSPECTIVE): a sphere + a smaller sphere + a box, each
/// RESTING on the mesh floor (`center.y == radius / half-height`), standing in the room so the
/// marcher's analytic soft shadow + contact AO fall on the mesh floor, wall, and cubes. HARD
/// unions (4th arg `0.0` is SMOOTHNESS, not a material — the bodies are mid-gray material 0).
fn hybrid_room_sdf_scene() -> Vec<SdfEdit> {
    // SHADOW-CASTER PROXY for a mesh cube: a HARD-union SDF box at the cube's exact center,
    // shrunk by `PROXY_MARGIN` per axis. It is NEVER rendered — the raster mesh cube (larger) wins
    // the marcher ownership at every shared pixel (the mesh surface is `PROXY_MARGIN` in FRONT of
    // the proxy), so the visible cube stays polygonal. But the proxy IS in the FROZEN field, so
    // every OTHER surface's analytic soft-shadow march toward the light hits it → the mesh cube
    // casts a clean SDF shadow onto the floor/wall. This is the MAX-PERF mesh-shadow path in an
    // SDF-first engine: it piggybacks the already-running march (+1 edit each) instead of adding a
    // whole separate shadow-map pass. The `SHADOW_NORMAL_BIAS` lift keeps the cube's OWN lit faces
    // clear of self-shadow (the march starts outside the proxy and travels away from it).
    const PROXY_MARGIN: f32 = 0.02;
    let proxy = |center: [f32; 3], half: [f32; 3]| {
        SdfEdit::box_shape(
            center,
            [half[0] - PROXY_MARGIN, half[1] - PROXY_MARGIN, half[2] - PROXY_MARGIN],
            sdf_op::UNION,
            0.0,
        )
    };
    vec![
        SdfEdit::sphere(ROOM_SDF_SPHERE_A.0, ROOM_SDF_SPHERE_A.1, sdf_op::UNION, 0.0),
        SdfEdit::sphere(ROOM_SDF_SPHERE_B.0, ROOM_SDF_SPHERE_B.1, sdf_op::UNION, 0.0),
        SdfEdit::box_shape(ROOM_SDF_BOX.0, ROOM_SDF_BOX.1, sdf_op::UNION, 0.0),
        // Invisible shadow-caster proxies under the 2 mesh cubes (≤ MAX_SDF_EDITS: 5 edits total).
        proxy(ROOM_CUBE_A.0, ROOM_CUBE_A.1),
        proxy(ROOM_CUBE_B.0, ROOM_CUBE_B.1),
    ]
}

/// The 3D-room MESH geometry (step 2, PERSPECTIVE): a horizontal FLOOR quad at y = 0 (outward
/// normal `+Y`), a vertical BACK-WALL quad at z = -4 (outward normal `+Z`), and 2 mesh CUBES
/// resting on the floor (distinct positions / sizes / colors). All concatenated into one
/// `Vec<Vertex>` — the draw is length-driven. The cubes + floor + wall carry real per-vertex
/// face normals for the G-buffer; the SDF bodies ([`hybrid_room_sdf_scene`]) shadow them.
fn hybrid_room_mesh() -> Vec<Vertex> {
    let mut verts = Vec::new();

    // Floor: a horizontal quad at y = 0 spanning x[-3,3] z[-4,1], outward normal +Y. Corners
    // CCW as seen from +Y (above): looking down the -Y axis.
    verts.extend_from_slice(&mesh_quad(
        [[-3.0, 0.0, 1.0], [3.0, 0.0, 1.0], [3.0, 0.0, -4.0], [-3.0, 0.0, -4.0]],
        [0.0, 1.0, 0.0],
        ROOM_FLOOR_COLOR,
    ));

    // Back wall: a vertical quad at z = -4 spanning x[-3,3] y[0,4], outward normal +Z. Corners
    // CCW as seen from +Z (in front of the wall).
    verts.extend_from_slice(&mesh_quad(
        [[-3.0, 0.0, -4.0], [3.0, 0.0, -4.0], [3.0, 4.0, -4.0], [-3.0, 4.0, -4.0]],
        [0.0, 0.0, 1.0],
        ROOM_WALL_COLOR,
    ));

    // 2 cubes resting on the floor (bottom at y = 0).
    verts.extend(mesh_box(ROOM_CUBE_A.0, ROOM_CUBE_A.1, ROOM_CUBE_A.2));
    verts.extend(mesh_box(ROOM_CUBE_B.0, ROOM_CUBE_B.1, ROOM_CUBE_B.2));

    verts
}

/// **Hybrid-room showcase — a PERSPECTIVE 3D room (step 2 of the hybrid-mesh-room build).**
/// A real 3D room: a mesh FLOOR + BACK WALL + 2 mesh cubes ([`hybrid_room_mesh`]) under a
/// PERSPECTIVE camera ([`room_camera`], matched by the [`perspective_mvp_bytes`] raster MVP),
/// with 3 SDF bodies ([`hybrid_room_sdf_scene`]) resting on the floor that cast the marcher's
/// analytic SHADOWS + AO onto the mesh. Analytic path: `ssao_quality: None`, `lighting_flags`
/// SHADOWS|AO (set by [`run_showcase_dump`]'s shared body), 1 directional sun ([`SHOWCASE_SUN_DIR`])
/// + 1 dim sky.
fn hybrid_room_config() -> ShowcaseConfig {
    let header = GoldenLightHeader::new(2, 0, 1.0).with_ssao_mode(0);
    let lights = vec![
        // The sun: the recorder's hardcoded `SHOWCASE_SUN_DIR` so the marcher's shadow march
        // matches the resolve's primary directional. Strong illuminance.
        GoldenLight::directional(SHOWCASE_SUN_DIR, [1.0, 0.97, 0.92], 3.0),
        // A dim neutral sky/hemisphere fill so the shadowed floor reads off pure black.
        GoldenLight::sky([0.05, 0.05, 0.05], [0.05, 0.05, 0.05]),
    ];
    ShowcaseConfig {
        sdf: hybrid_room_sdf_scene(),
        camera: room_camera(),
        light_header: header,
        light_elems: lights,
        vertices: hybrid_room_mesh(),
        mvp: perspective_mvp_bytes(
            ROOM_CAM_EYE,
            ROOM_CAM_FORWARD,
            ROOM_CAM_RIGHT,
            ROOM_CAM_UP,
            ROOM_CAM_FOV_Y,
            COMPOSITE_W as f32 / COMPOSITE_H as f32,
        ),
        ssao_quality: None,
        mesh_sdf: None,
        // M2: no instanced mesh — the legacy merged draw runs (byte-identical).
        instanced: None,
        // CSM Increment 1b: OFF (the 0%-gate — no cascade depth pass, `csm_mode == 0`).
        csm: None,
        // Shadow Phase 5 Inc-1-GPU: no sparse spot shadow (the 0%-gate — no punctual depth pass,
        // `punctual_shadow_mode == 0`).
        spot_atlas: None,
    }
}

// === Mesh foundation M2 — the FIRST REAL instanced perspective draw. ===

/// The perspective MVP with the M1 instanced-arm selector ARMED (`use_model_matrix == 1`, push
/// byte 84). [`perspective_mvp_bytes`] writes the first 80 bytes (the `proj*view` + `cam_eye`,
/// `cam_eye.w == 1` = perspective mode) and leaves bytes 80..88 zero; this flips byte 84 so the
/// VS reads `instances[0 + SV_InstanceID]` and transforms each vertex by its per-instance affine.
fn instanced_room_mvp_bytes() -> [u8; MVP_BYTES as usize] {
    let mut bytes = perspective_mvp_bytes(
        ROOM_CAM_EYE,
        ROOM_CAM_FORWARD,
        ROOM_CAM_RIGHT,
        ROOM_CAM_UP,
        ROOM_CAM_FOV_Y,
        COMPOSITE_W as f32 / COMPOSITE_H as f32,
    );
    // byte 80..84 = base_instance (0), byte 84 = use_model_matrix (1).
    bytes[84] = 1;
    bytes
}

/// **Mesh foundation M2 — the FIRST REAL instanced perspective config.** ONE registered
/// MODEL-SPACE box ([`mesh_box_model`]) drawn through the `use_model_matrix == 1` instanced arm
/// by FOUR non-identity affines at DISTINCT world positions + depths + yaws (so perspective
/// foreshortening AND per-instance depth-ownership are exercised), CO-SCENED with an SDF sphere
/// resting on the floor under the [`room_camera`] perspective. The C2 proof: if the instanced
/// VS+FS depth were wrong, the raster boxes would punch through / be wrongly occluded by the SDF
/// sphere — here the depth between the four instanced boxes and the SDF sphere reads correctly.
///
/// The `vertices` legacy field is the DEGENERATE zero-area mesh (the recorder draws the instanced
/// mesh instead, NOT this), so the legacy non-indexed path produces no fragments even though a
/// valid vertex buffer is bound. `lighting_flags` SHADOWS|AO is set by the shared body.
fn instanced_persp_config() -> ShowcaseConfig {
    // One directional sun (the marcher's shadow march matches the resolve's primary) + a dim sky.
    let header = GoldenLightHeader::new(2, 0, 1.0).with_ssao_mode(0);
    let lights = vec![
        GoldenLight::directional(SHOWCASE_SUN_DIR, [1.0, 0.97, 0.92], 3.0),
        GoldenLight::sky([0.05, 0.05, 0.05], [0.05, 0.05, 0.05]),
    ];

    // The co-scene SDF: a hero sphere resting on the floor (the marcher owns it; the depth
    // ownership between it and the instanced raster boxes is the C2 visual proof).
    let sdf = vec![SdfEdit::sphere([1.4, 0.6, -0.6], 0.6, sdf_op::UNION, 0.0)];

    // The model-space unit box (half-extent 0.4) registered ONCE; four instances place it.
    let (verts, indices) = mesh_box_model([0.4, 0.4, 0.4], [0.82, 0.45, 0.30, 1.0]);

    // FOUR non-identity affines: distinct X, distinct Z (depth), distinct yaw + a slight scale
    // spread, all resting near the floor so the boxes + the SDF sphere share the room.
    let affines = vec![
        instance_affine(0.0, 1.0, [-1.8, 0.42, -0.4]),
        instance_affine(0.5, 0.8, [-0.6, 0.36, -1.6]),
        instance_affine(-0.4, 1.2, [0.5, 0.50, -2.8]),
        instance_affine(0.9, 0.9, [-1.2, 0.40, -3.6]),
    ];

    ShowcaseConfig {
        sdf,
        camera: room_camera(),
        light_header: header,
        light_elems: lights,
        // The degenerate legacy mesh (the recorder draws the instanced mesh below instead).
        vertices: showcase_quad_vertices(),
        // The instanced-arm MVP (`use_model_matrix == 1`).
        mvp: instanced_room_mvp_bytes(),
        ssao_quality: None,
        mesh_sdf: None,
        instanced: Some(InstancedMesh {
            meshes: vec![InstancedMeshEntry { vertices: verts, indices, affines }],
            non_casters: vec![],
        }),
        // CSM Increment 1b: OFF for this showcase (the 0%-gate — no cascade depth pass).
        csm: None,
        // Shadow Phase 5 Inc-1-GPU: no sparse spot shadow (the 0%-gate — no punctual depth pass,
        // `punctual_shadow_mode == 0`).
        spot_atlas: None,
    }
}

/// **Mesh foundation M2 — the FIRST REAL instanced perspective screenshot.** Drives the
/// instanced gbuffer arm (`use_model_matrix == 1`) for real: four boxes placed by per-instance
/// model matrices under the perspective room camera, co-scened with an SDF sphere so the
/// depth-ownership between the instanced raster boxes and the SDF surface is visible. Dumps a
/// TRUE 512×512 BMP to [`INSTANCED_PERSP_BMP`] for the owner's RTX visual sign-off.
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the instanced screenshot"]
fn engine_instanced_persp_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine instanced perspective 512",
        INSTANCED_PERSP_BMP,
        instanced_persp_config(),
        false,
    );
}

// === CSM Increment 1b (Rung A) — the single-cascade hardware shadow demo + the matrix golden. ===

/// The CSM demo's BMP dump path (the owner's RTX visual oracle).
const CSM_SHADOW_BMP: &str = "D:\\tmp\\engine_csm_shadow.bmp";
/// CSM Increment 3 (Rung B): the N-cascade demo's BMP dump path (the owner's RTX visual oracle —
/// the smooth multi-distance cascade transition).
const CSM_CASCADES_BMP: &str = "D:\\tmp\\engine_csm_cascades.bmp";

/// CSM Increment 1b (Rung A): the demo's hand-fitted cascade — a world-space box covering both the
/// floor and the caster, plus the world-space texel size derived from it at the shadow resolution.
/// A real app derives these from `boyko_render::resolve_csm`; the demo fixes them so the host
/// matrix golden and the GPU lookup share ONE known fit.
const CSM_DEMO_HALF_EXTENT: f32 = 4.0; // the ortho half-width (world units) covering the scene
const CSM_DEMO_NEAR: f32 = 0.1;
const CSM_DEMO_FAR: f32 = 20.0;

/// CSM Increment 1b (Rung A): builds the demo cascade's COLUMN-MAJOR world→light-clip `view_proj`
/// (`ortho · light_view`) for the sun `sun_dir` (direction TO the light) looking at `center`. The
/// light eye is pulled back along `sun_dir`; the ortho box is `[-h,h]²` × `[near,far]` in Vulkan
/// `[0,1]` depth. The SAME helper feeds the GPU UBO/push (`run_showcase_dump`) AND the host matrix
/// golden, so the depth-VS reprojection and the resolve lookup are pinned to ONE matrix.
///
/// Returns the 16 column-major floats (upload-ready) — the byte layout the depth VS push (`@0`) +
/// the resolve `CsmCascades` cbuffer expect.
fn csm_demo_view_proj(sun_dir: [f32; 3], center: [f32; 3]) -> [f32; 16] {
    // Rung A delegates to the parameterized Rung-B builder at the demo's fixed footprint/z-range.
    csm_cascade_view_proj(sun_dir, center, CSM_DEMO_HALF_EXTENT, CSM_DEMO_NEAR, CSM_DEMO_FAR)
}

/// Shadow Phase 5 Inc-1-GPU: the spot shadow near plane (view-space) — mirrors
/// `boyko_render::shadow_atlas::SPOT_SHADOW_NEAR`.
const SPOT_DEMO_NEAR: f32 = 0.05;

/// Shadow Phase 5 Inc-1-GPU: builds the demo SPOT's COLUMN-MAJOR world→light-clip `view_proj`
/// (`perspective · light_view`) for a spot at `eye` shining along `axis` (the world direction the
/// light points), full FOV `2·outer_rad`, near [`SPOT_DEMO_NEAR`], far `range`. The look-at uses a
/// right-handed convention with a +Y world-up (swapped to +Z when nearly collinear). The SAME helper
/// feeds the GPU UBO/push AND the host spot matrix golden, so the depth-pass reprojection and the
/// resolve lookup are pinned to ONE matrix.
///
/// Mirrors `boyko_render::shadow_atlas::spot_face`'s `view_proj` math but in plain `[f32; 16]`
/// arrays (no `boyko_render`/`boyko_math` dep here), and emits Vulkan `[0,1]` depth with the
/// engine's framebuffer Y-flip (`clip.y = -y_proj`), the SAME convention `csm_cascade_view_proj`
/// uses (so the resolve's shared Y-flipped NDC→UV addresses the right texel).
fn spot_demo_view_proj(eye: [f32; 3], axis: [f32; 3], outer_rad: f32, range: f32) -> [f32; 16] {
    let norm = |v: [f32; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    };
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

    // The look direction is the cone axis (the world direction the light shines along).
    let fwd = norm(axis);
    let up_hint = if dot(fwd, [0.0, 1.0, 0.0]).abs() > 0.99 {
        [0.0, 0.0, 1.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let right = norm(cross(up_hint, fwd));
    let up = cross(fwd, right);
    // light_view rows = basis; translation = -dot(basis, eye). z_light = forward·(P-eye) (POSITIVE
    // into the scene), matching `csm_cascade_view_proj`'s view convention.
    let tx = -dot(right, eye);
    let ty = -dot(up, eye);
    let tz = -dot(fwd, eye);

    // Perspective proj (Vulkan [0,1] depth, square aspect): full FOV = 2·outer.
    let near = SPOT_DEMO_NEAR;
    let far = range.max(SPOT_DEMO_NEAR + 1.0e-3);
    let f = 1.0 / (outer_rad).tan(); // cot(half_fov); half_fov = outer_rad
    // Standard RH perspective (Vulkan [0,1]): clip.x = f/aspect·x, clip.y = -f·y (Y-flip to match
    // the engine framebuffer), clip.z = far/(far-near)·z - far·near/(far-near), clip.w = z.
    // aspect == 1 (square map). pv[row][col] = proj_row · light_view.
    let zr = far - near;
    let pv: [[f32; 4]; 4] = [
        [f * right[0], f * right[1], f * right[2], f * tx],
        [-f * up[0], -f * up[1], -f * up[2], -f * ty],
        [
            (far / zr) * fwd[0],
            (far / zr) * fwd[1],
            (far / zr) * fwd[2],
            (far / zr) * tz - far * near / zr,
        ],
        [fwd[0], fwd[1], fwd[2], tz],
    ];
    // Upload COLUMN-MAJOR: out[col*4 + row] = pv[row][col].
    let mut out = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = pv[row][col];
        }
    }
    out
}

/// Shadow Phase 5 Inc-2 (POINT cube): builds ONE cube FACE's COLUMN-MAJOR world→light-clip
/// `view_proj` for a point at `eye` looking down `dir`, full FOV 90° (`π/2`), near
/// [`SPOT_DEMO_NEAR`], far `range`. Mirrors `boyko_render::shadow_atlas::point_faces`'s per-face
/// math (a right-handed look-at + a Vulkan-[0,1] perspective with the engine Y-flip) — the SAME
/// convention `spot_demo_view_proj` uses, just at a 90° FOV down an explicit axis. The depth-pass
/// raster footprint comes from this matrix; the STORED depth is the FS's linear radial distance.
fn point_face_view_proj(eye: [f32; 3], dir: [f32; 3], range: f32) -> [f32; 16] {
    // The point cube uses a 90° full FOV per face (outer half-angle = 45°). Delegate to the SAME
    // perspective+look-at builder the spot path uses, so the host fit and the resolve agree.
    spot_demo_view_proj(eye, dir, core::f32::consts::FRAC_PI_4, range)
}

/// Shadow Phase 5 Inc-1-GPU: a one-spot [`SpotDemoFit`] (slot 0) from a fitted spot `view_proj` +
/// the spot's world position + range. The trailing `[1..SPOT_ATLAS_SLOTS)` faces stay zero
/// (bound-but-unread).
fn spot_single_face(view_proj: [f32; 16], light_pos: [f32; 3], range: f32) -> SpotDemoFit {
    let mut faces = [SPOT_FACE_ZERO; SPOT_ATLAS_SLOTS as usize];
    faces[0] = SpotFace {
        view_proj,
        light_pos,
        inv_range: if range > 0.0 { range.recip() } else { 0.0 },
        is_point: false,
    };
    SpotDemoFit { faces, active_layers: 1 }
}

/// Shadow Phase 5 Inc-2 (POINT cube): a zeroed [`SpotFace`] for the unused trailing atlas slots
/// (bound-but-unread; `active_layers` bounds the valid prefix).
const SPOT_FACE_ZERO: SpotFace = SpotFace {
    view_proj: [0.0; 16],
    light_pos: [0.0; 3],
    inv_range: 0.0,
    is_point: false,
};

/// Shadow Phase 5 Inc-2 (POINT cube): builds a six-face POINT cube [`SpotDemoFit`] (slot base 0)
/// from the light world position + range. Mirrors `boyko_render::shadow_atlas::point_faces`'s
/// `view_proj` math (the `[+X, -X, +Y, -Y, +Z, -Z]` 90°-FOV faces) but in plain `[f32; 16]` arrays.
/// The depth pass renders the casters into layers `0..6` with each face's `view_proj` + the shared
/// `light_pos`/`inv_range` (stamped into the FS's `cam_eye@64` lane); the resolve major-axis-selects
/// among the six. The trailing `[6..SPOT_ATLAS_SLOTS)` faces stay zero (bound-but-unread).
fn point_cube_fit(light_pos: [f32; 3], range: f32) -> SpotDemoFit {
    let mut faces = [SPOT_FACE_ZERO; SPOT_ATLAS_SLOTS as usize];
    let inv_range = if range > 0.0 { range.recip() } else { 0.0 };
    // The six cube-face look directions, in the host fit order `[+X, -X, +Y, -Y, +Z, -Z]`.
    let dirs: [[f32; 3]; 6] = [
        [1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
    ];
    for (i, dir) in dirs.iter().enumerate() {
        faces[i] = SpotFace {
            view_proj: point_face_view_proj(light_pos, *dir, range),
            light_pos,
            inv_range,
            is_point: true,
        };
    }
    SpotDemoFit { faces, active_layers: 6 }
}

/// Shadow Phase 5 Inc-1-GPU: the HOST↔SHADER SPOT MATRIX GOLDEN (the acne oracle). Given a spot
/// `view_proj` (column-major, the SAME bytes the depth pass pushes + the resolve UBO carries) + a
/// world receiver point `P` + its normal `n`, this reprojects `P` the SAME way the depth pass
/// (`mul(view_proj, float4(P,1))`) and the resolve's `spot_atlas_visibility` (normal-offset by
/// `n * SPOT_SHADOW_NORMAL_BIAS` + the Y-flipped NDC→UV) do — so the host can assert the two
/// reprojections agree (the depth write and the resolve lookup cannot drift). Mirrors the resolve's
/// `spot_atlas_visibility` UV math byte-for-byte. Returns `(uv_x, uv_y, ndc_z, in_bounds)`.
fn spot_host_project(view_proj: &[f32; 16], p: [f32; 3], n: [f32; 3]) -> (f32, f32, f32, bool) {
    // Normal-offset the receiver — IDENTICAL to the resolve's `P + n * SPOT_SHADOW_NORMAL_BIAS`.
    let off = SPOT_SHADOW_NORMAL_BIAS;
    let pw = [p[0] + n[0] * off, p[1] + n[1] * off, p[2] + n[2] * off, 1.0];
    // Column-major `view_proj * pw`: `clip[r] = sum_c view_proj[c*4 + r] * pw[c]`.
    let mut clip = [0.0f32; 4];
    for (r, clip_r) in clip.iter_mut().enumerate() {
        let mut acc = 0.0f32;
        for (c, &pw_c) in pw.iter().enumerate() {
            acc += view_proj[c * 4 + r] * pw_c;
        }
        *clip_r = acc;
    }
    if clip[3] <= 0.0 {
        return (0.0, 0.0, 0.0, false);
    }
    let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
    let uv_x = ndc[0] * 0.5 + 0.5;
    let uv_y = 1.0 - (ndc[1] * 0.5 + 0.5); // Vulkan Y-flip (matches the resolve)
    let in_bounds = (0.0..=1.0).contains(&uv_x)
        && (0.0..=1.0).contains(&uv_y)
        && (0.0..=1.0).contains(&ndc[2]);
    (uv_x, uv_y, ndc[2], in_bounds)
}

/// CSM shimmer fix: the orthonormal LIGHT basis `(right, up, fwd = -sun)` for a sun `sun_dir`
/// (direction TO the light) — computed EXACTLY as [`csm_cascade_view_proj`] derives it, so the
/// per-cascade texel SNAP can project the cascade center onto the SAME right/up axes the matrix
/// is built from (snapping on any other basis would not land the origin on a shadow-map texel).
///
/// `sun = norm(sun_dir)`; `fwd = -sun`; `up_hint = [0,1,0]` unless `fwd` is nearly collinear with it
/// (then `[0,0,1]` — the W5 alt-up guard); `right = norm(cross(up_hint, fwd))`; `up = cross(fwd, right)`.
fn csm_light_basis(sun_dir: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let norm = |v: [f32; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    };
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

    let sun = norm(sun_dir);
    let fwd = [-sun[0], -sun[1], -sun[2]]; // = -sun (the light looks back toward the scene)
    // Right/up via a world-up hint (swap when nearly collinear — the W5 alt-up guard).
    let up_hint = if dot(fwd, [0.0, 1.0, 0.0]).abs() > 0.99 {
        [0.0, 0.0, 1.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let right = norm(cross(up_hint, fwd));
    let up = cross(fwd, right);
    (right, up, fwd)
}

/// CSM Increment 3 (Rung B): builds ONE cascade's COLUMN-MAJOR world→light-clip `view_proj`
/// (`ortho · light_view`) for the sun `sun_dir` (direction TO the light) looking at `center`, with
/// an orthographic half-extent `half` and light-space z-range `[z_near, z_far]` (Vulkan `[0,1]`
/// depth). The light eye is pulled back along `sun_dir` by `z_far*0.5`. Generalizes the Rung-A fit
/// (which fixed `half`/`z` to demo constants) so each cascade can size its own footprint. Returns
/// the 16 column-major floats (the byte layout the depth VS push `@0` + the resolve cbuffer expect).
fn csm_cascade_view_proj(
    sun_dir: [f32; 3],
    center: [f32; 3],
    half: f32,
    z_near: f32,
    z_far: f32,
) -> [f32; 16] {
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

    // The light basis (right, up, fwd = -sun) — the SAME derivation the texel snap uses, so the
    // snap projects `center` onto EXACTLY the axes this matrix is built from.
    let (right, up, fwd) = csm_light_basis(sun_dir);
    let sun = [-fwd[0], -fwd[1], -fwd[2]]; // norm(sun_dir)
    // The light looks back along -sun (from the sun toward the scene). Eye pulled back along sun.
    let pullback = z_far * 0.5;
    let eye = [
        center[0] + sun[0] * pullback,
        center[1] + sun[1] * pullback,
        center[2] + sun[2] * pullback,
    ];
    // light_view rows = basis; translation = -dot(basis, eye). z_light = forward·(P-eye) (POSITIVE
    // into the scene), matching `perspective_mvp_bytes`'s view convention.
    let tx = -dot(right, eye);
    let ty = -dot(up, eye);
    let tz = -dot(fwd, eye);
    // Ortho proj (Vulkan [0,1] depth): clip.x = x/h, clip.y = -y/h (Y-flip to match the engine's
    // framebuffer convention, the SAME flip `perspective_mvp_bytes` + the resolve apply),
    // clip.z = (z - near)/(far - near), clip.w = 1.
    let inv_h = 1.0 / half;
    let zr = z_far - z_near;
    // pv[row][col] = ortho_row · light_view_row.
    let pv: [[f32; 4]; 4] = [
        [inv_h * right[0], inv_h * right[1], inv_h * right[2], inv_h * tx],
        [-inv_h * up[0], -inv_h * up[1], -inv_h * up[2], -inv_h * ty],
        [fwd[0] / zr, fwd[1] / zr, fwd[2] / zr, (tz - z_near) / zr],
        [0.0, 0.0, 0.0, 1.0],
    ];
    // Upload COLUMN-MAJOR: out[col*4 + row] = pv[row][col] (the verified transpose).
    let mut out = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = pv[row][col];
        }
    }
    out
}

/// CSM Increment 3 (Rung B): the PSSM split distance for split `idx` of `n` (`idx ∈ 1..=n`) — the
/// HOST mirror of `boyko_render::csm_config::pssm_split`: `λ·log + (1−λ)·uniform`, `log =
/// near·(far/near)^(idx/n)`, `uniform = near + (far−near)·(idx/n)`. `idx == n` returns `far`.
fn csm_pssm_split(near: f32, far: f32, lambda: f32, idx: usize, n: f32) -> f32 {
    let t = idx as f32 / n;
    let log = near * (far / near).powf(t);
    let uniform = near + (far - near) * t;
    lambda * log + (1.0 - lambda) * uniform
}

/// CSM Increment 3 (Rung B): hand-fits `count` cascades over the camera frustum's `[near, far]`
/// VIEW-Z range, PSSM-partitioned (mirrors `resolve_csm`). For each cascade the view-z slice
/// `[near_i, split_i]` is bounded by a world-space sphere (center = mean of the 8 slice corners,
/// radius = max corner distance); the cascade's ortho footprint = the sphere diameter, its
/// `texel_size = diameter / CSM_SHADOW_DIM`, its `split_far = split_i` (the SELECT boundary). A
/// real app calls `boyko_render::resolve_csm`; the demo inlines it (no `boyko_render` dep here).
///
/// `eye`/`fwd`/`right`/`up` are the ORTHONORMAL camera basis (forward normalized); `fov_y` radians,
/// `aspect = W/H`. Cascades `[0..count)` are valid; the rest are zeroed.
#[allow(clippy::too_many_arguments)]
fn csm_demo_cascades(
    sun_dir: [f32; 3],
    eye: [f32; 3],
    fwd: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    fov_y: f32,
    aspect: f32,
    near: f32,
    far: f32,
    count: usize,
) -> CsmDemoFit {
    debug_assert!(
        (1..=CSM_MAX_CASCADES).contains(&count),
        "cascade count must be in 1..=MAX_CASCADES"
    );
    let n = count as f32;
    let half_tan = (fov_y * 0.5).tan();
    // The 4 corner ray directions (world space) at the frustum's NDC corners — scaled by view-z to
    // reach the slice planes. Matches `ray_gen`'s perspective dir combine (`fwd + right*aspect*tan*x
    // + up*tan*y`), so the cascade footprint covers exactly what the camera sees.
    let corner_dir = |sx: f32, sy: f32| {
        [
            fwd[0] + right[0] * (sx * aspect * half_tan) + up[0] * (sy * half_tan),
            fwd[1] + right[1] * (sx * aspect * half_tan) + up[1] * (sy * half_tan),
            fwd[2] + right[2] * (sx * aspect * half_tan) + up[2] * (sy * half_tan),
        ]
    };
    let corners = [
        corner_dir(-1.0, -1.0),
        corner_dir(1.0, -1.0),
        corner_dir(-1.0, 1.0),
        corner_dir(1.0, 1.0),
    ];

    let mut cascades = [CsmDemoCascade {
        view_proj: [0.0; 16],
        split_far: 0.0,
        texel_size: 0.0,
    }; CSM_MAX_CASCADES];

    let lambda = 0.5; // PSSM blend (mirror the CsmConfig research default)
    let mut near_i = near;
    for (i, slot) in cascades.iter_mut().enumerate().take(count) {
        let split_i = csm_pssm_split(near, far, lambda, i + 1, n);
        // The 8 world-space slice corners (near plane at `near_i`, far at `split_i`). A corner at
        // view-z `z` is `eye + dir * z` where `dir` already has unit forward component (`fwd` is
        // normalized and orthonormal to right/up), so `dot(dir, fwd) == 1` and `dir*z` lands on the
        // z-plane.
        let mut pts = [[0.0f32; 3]; 8];
        for (k, d) in corners.iter().enumerate() {
            for (p, &z) in [near_i, split_i].iter().enumerate() {
                pts[k * 2 + p] = [eye[0] + d[0] * z, eye[1] + d[1] * z, eye[2] + d[2] * z];
            }
        }
        // Bounding sphere: center = mean, radius = max corner distance (the resolve's sphere fit).
        let mut center = [0.0f32; 3];
        for p in &pts {
            center[0] += p[0];
            center[1] += p[1];
            center[2] += p[2];
        }
        center = [center[0] / 8.0, center[1] / 8.0, center[2] / 8.0];
        let mut radius = 0.0f32;
        for p in &pts {
            let dx = p[0] - center[0];
            let dy = p[1] - center[1];
            let dz = p[2] - center[2];
            radius = radius.max((dx * dx + dy * dy + dz * dz).sqrt());
        }
        let diameter = (2.0 * radius).max(1.0);
        let half = diameter * 0.5;
        let texel_size = diameter / CSM_SHADOW_DIM as f32;
        // TEXEL SNAP (shadow-shimmer fix): snap the cascade center onto the shadow-map texel grid in
        // the LIGHT plane. The bounding-sphere RADIUS is rotation-invariant (a sphere), so under
        // camera motion `texel_size` is constant frame-to-frame and ONLY `center` translates; quantizing
        // its light-plane (right/up) coordinates to whole texels keeps each shadow texel mapping to the
        // same world footprint between frames, killing the edge-crawl an unsnapped per-frame re-fit causes.
        let (right, up, _fwd) = csm_light_basis(sun_dir);
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let cx = dot(right, center);
        let cy = dot(up, center);
        let cxs = (cx / texel_size).floor() * texel_size;
        let cys = (cy / texel_size).floor() * texel_size;
        let dx = cxs - cx;
        let dy = cys - cy;
        let center = [
            center[0] + right[0] * dx + up[0] * dy,
            center[1] + right[1] * dx + up[1] * dy,
            center[2] + right[2] * dx + up[2] * dy,
        ];
        // The light z-range spans the sphere plus the pullback margin (the eye is pulled back
        // `z_far*0.5`); a generous far keeps the caster inside the ortho box.
        let z_far = diameter * 2.0;
        *slot = CsmDemoCascade {
            view_proj: csm_cascade_view_proj(sun_dir, center, half, CSM_DEMO_NEAR, z_far),
            split_far: split_i,
            texel_size,
        };
        near_i = split_i;
    }

    CsmDemoFit { cascades, active_count: count as u32 }
}

/// CSM Increment 1b (Rung A): the demo cascade's world-space texel size (the ortho footprint
/// `2h` spread over the shadow resolution) — the resolve's normal-bias scale.
fn csm_demo_texel_size() -> f32 {
    (2.0 * CSM_DEMO_HALF_EXTENT) / (CSM_SHADOW_DIM as f32)
}

/// CSM Increment 3 (Rung B): packs ONE Rung-A cascade into a `[CsmDemoCascade; MAX]` (slot 0; the
/// rest zeroed) so the single-cascade demo + the host golden reuse the N-cascade plumbing. The
/// `split_far` is set large enough that the lone cascade always SELECTs (Rung A had no select).
fn csm_single_cascade(
    view_proj: [f32; 16],
    texel_size: f32,
    split_far: f32,
) -> [CsmDemoCascade; CSM_MAX_CASCADES] {
    let mut cascades = [CsmDemoCascade {
        view_proj: [0.0; 16],
        split_far: 0.0,
        texel_size: 0.0,
    }; CSM_MAX_CASCADES];
    cascades[0] = CsmDemoCascade { view_proj, split_far, texel_size };
    cascades
}

/// **CSM Increment 1b (Rung A) — the single-cascade hardware shadow showcase config.** An
/// ASYMMETRIC raster box (NOT a sphere — the owner-eval pattern; a marker so the orientation reads)
/// standing on a raster floor, a LEVEL-ish perspective camera, ONE directional sun, `csm_mode` ON.
/// NO SDF/MDF in the scene (the floor's `gMaterial.r == 1`, so `min(1, csm_vis) == csm_vis` — the
/// CSM shadow shows; the caster has no SDF/MDF twin, C2). The box's instanced batch IS the CSM
/// caster: the depth pass renders it from the sun POV into cascade layer 0, and the resolve casts
/// its EXACT hard shadow onto the floor.
fn csm_shadow_config() -> ShowcaseConfig {
    // ONE directional sun (the cascade is fit to it) + a dim sky for ambient fill.
    let header = GoldenLightHeader::new(2, 0, 1.0).with_ssao_mode(0);
    let lights = vec![
        GoldenLight::directional(SHOWCASE_SUN_DIR, [1.0, 0.97, 0.92], 3.0),
        GoldenLight::sky([0.08, 0.08, 0.10], [0.08, 0.08, 0.10]),
    ];

    // NO SDF: the marcher owns no surface (the raster floor + box own every lit pixel; the floor's
    // mask=1 makes `min(1, csm_vis) == csm_vis`). A single degenerate far edit keeps the edit-list
    // valid (the marcher finds no hit — the background clears).
    let sdf = vec![SdfEdit::sphere([0.0, -1000.0, 0.0], 0.01, sdf_op::UNION, 0.0)];

    // The CASTER: an ASYMMETRIC box (a tall slab, distinct X/Y/Z extents) standing on the floor,
    // with a small marker cube on top so the cast shadow's orientation reads. ONE instanced mesh
    // entry, two affines (the slab + the marker), so the depth pass + the resolve both see them.
    let (slab_v, slab_i) = mesh_box_model([0.35, 0.9, 0.55], [0.82, 0.42, 0.30, 1.0]);
    let affines = vec![
        instance_affine(0.4, 1.0, [0.0, 0.9, -1.2]),  // the slab, yawed 0.4 rad
        instance_affine(0.4, 0.28, [0.0, 1.95, -1.2]), // a small marker cube on top
    ];

    // The raster FLOOR: a wide flat box (a thin slab) at y≈0 spanning the scene, so the cast shadow
    // lands on a real raster surface (mask=1).
    let (floor_v, floor_i) = mesh_box_model([4.0, 0.05, 4.0], ROOM_FLOOR_COLOR);
    let floor_affine = instance_affine(0.0, 1.0, [0.0, -0.05, -1.0]);

    ShowcaseConfig {
        sdf,
        camera: room_camera(),
        light_header: header,
        light_elems: lights,
        vertices: showcase_quad_vertices(), // degenerate legacy mesh (the instanced arm draws)
        mvp: instanced_room_mvp_bytes(),
        ssao_quality: None,
        mesh_sdf: None,
        instanced: Some(InstancedMesh {
            meshes: vec![
                // The floor mesh (batch 0, base 0) + the caster slab+marker (batch 1, nonzero base).
                InstancedMeshEntry { vertices: floor_v, indices: floor_i, affines: vec![floor_affine] },
                InstancedMeshEntry { vertices: slab_v, indices: slab_i, affines },
            ],
            non_casters: vec![],
        }),
        // CSM ON (Rung A): ONE cascade fit to the sun, looking at the scene center near the caster.
        // `split_far` is large (covers the whole scene) so the single cascade always SELECTs.
        csm: Some(CsmDemoFit {
            cascades: csm_single_cascade(
                csm_demo_view_proj(SHOWCASE_SUN_DIR, [0.0, 0.5, -1.2]),
                csm_demo_texel_size(),
                CSM_DEMO_FAR,
            ),
            active_count: 1,
        }),
        // This is the CSM (directional) demo — the sparse SPOT path stays OFF here.
        spot_atlas: None,
    }
}

/// **CSM Increment 1b (Rung A) — the single-cascade hardware shadow screenshot.** Drives the
/// cascade DEPTH pass (the asymmetric box rendered from the sun POV into cascade layer 0) + the
/// resolve `min`-combine, dumping a TRUE 512×512 BMP to [`CSM_SHADOW_BMP`] for the owner's RTX
/// visual sign-off (the deliverable: the box's EXACT hard shadow on the raster floor).
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the CSM shadow screenshot"]
fn engine_csm_shadow_512_screenshot_dump() {
    run_showcase_dump("boyko_engine CSM shadow 512", CSM_SHADOW_BMP, csm_shadow_config(), false);
}

/// **CSM Increment 1b (Rung A) — the HOST↔SHADER MATRIX GOLDEN (the acne oracle).** Asserts the
/// host reprojection ([`csm_host_project`], the mirror of the resolve's `csm_visibility` UV math)
/// AGREES with a direct column-major `view_proj · P` projection of a KNOWN caster point — so the
/// depth-VS write and the resolve lookup (which read the SAME `view_proj` bytes) cannot drift. A
/// point UNDER the cascade footprint maps inside `[0,1]²`; a point far outside maps out of bounds.
#[test]
fn csm_matrix_golden_host_projection_agrees() {
    let view_proj = csm_demo_view_proj(SHOWCASE_SUN_DIR, [0.0, 0.5, -1.2]);
    let texel = csm_demo_texel_size();

    // A receiver on the floor directly under the caster: in-bounds, depth in [0,1].
    let p_floor = [0.0, 0.0, -1.2];
    let n_up = [0.0, 1.0, 0.0];
    let (uv_x, uv_y, ndc_z, in_bounds) = csm_host_project(&view_proj, p_floor, n_up, texel);
    assert!(
        in_bounds,
        "the floor point under the caster must project inside the cascade footprint \
         (uv = ({uv_x}, {uv_y}), ndc_z = {ndc_z})"
    );
    assert!((0.0..=1.0).contains(&ndc_z), "the receiver depth must be in the Vulkan [0,1] range");

    // The normal-offset (D6) MUST move the lookup off the surface: a biased point differs from the
    // un-biased projection (the acne oracle — the bias is applied, not a no-op). The cascade's
    // look_at uses up == world-up, so world-y is PERPENDICULAR to the light's right axis — a +y
    // normal-offset (n_up) therefore perturbs uv_y / depth, NOT uv_x. Assert the lookup moves in ANY
    // component (a texel_size==0 no-op would leave all three unchanged and correctly fail here).
    let (uv_x0, uv_y0, ndc_z0, _) = csm_host_project(&view_proj, p_floor, [0.0, 0.0, 0.0], texel);
    assert!(
        (uv_x - uv_x0).abs() > f32::EPSILON
            || (uv_y - uv_y0).abs() > f32::EPSILON
            || (ndc_z - ndc_z0).abs() > f32::EPSILON,
        "the normal-offset bias must perturb the lookup (acne oracle)"
    );

    // A point far outside the cascade footprint projects out of bounds (the resolve treats it lit).
    let p_far = [100.0, 0.0, -1.2];
    let (_, _, _, far_in_bounds) = csm_host_project(&view_proj, p_far, n_up, texel);
    assert!(!far_in_bounds, "a point far outside the cascade box must project out of bounds");

    // The COLUMN-MAJOR transpose pin: a direct `view_proj · P` (no bias) must reproduce the same
    // clip the host helper computes internally — the depth VS uses this EXACT product.
    let pw = [p_floor[0], p_floor[1], p_floor[2], 1.0];
    let mut clip = [0.0f32; 4];
    for (r, clip_r) in clip.iter_mut().enumerate() {
        let mut acc = 0.0f32;
        for (c, &pw_c) in pw.iter().enumerate() {
            acc += view_proj[c * 4 + r] * pw_c;
        }
        *clip_r = acc;
    }
    assert!(clip[3] > 0.0, "the ortho clip.w must be positive (the depth VS divides by it)");
    let direct_uv_x = (clip[0] / clip[3]) * 0.5 + 0.5;
    let (uv_x_nobias, _, _, _) = csm_host_project(&view_proj, p_floor, [0.0, 0.0, 0.0], texel);
    assert!(
        (direct_uv_x - uv_x_nobias).abs() < 1e-5,
        "the direct column-major product and the host helper must agree (the majorness pin): \
         {direct_uv_x} vs {uv_x_nobias}"
    );
}

// === Shadow Phase 5 Inc-1-GPU — the sparse SPOT hardware-shadow demo + the spot matrix golden. ===

/// Shadow Phase 5 Inc-1-GPU: the BMP the spot-shadow demo dumps for the owner's RTX visual sign-off.
const SPOT_SHADOW_BMP: &str = "D:\\tmp\\engine_spot_shadow.bmp";

/// Shadow Phase 5 Inc-1-GPU: the demo SPOT's world position (the cone apex / perspective eye) —
/// above-and-to-the-side of the caster so the cone covers the box + the floor under it.
const SPOT_DEMO_POS: [f32; 3] = [1.3, 3.0, 0.4];
/// Shadow Phase 5 Inc-1-GPU: the point the spot aims at (the scene center near the caster) — the
/// cone axis is `normalize(SPOT_DEMO_TARGET - SPOT_DEMO_POS)`.
const SPOT_DEMO_TARGET: [f32; 3] = [0.0, 0.4, -1.2];
/// Shadow Phase 5 Inc-1-GPU: the spot's outer cone HALF-angle (degrees) — wide enough to cover the
/// caster + a floor patch, narrow enough that the shadow stays INSIDE the cone (no shadow outside).
const SPOT_DEMO_OUTER_DEG: f32 = 28.0;
/// Shadow Phase 5 Inc-1-GPU: the spot's inner cone half-angle (degrees) — the falloff start.
const SPOT_DEMO_INNER_DEG: f32 = 20.0;
/// Shadow Phase 5 Inc-1-GPU: the spot's range (the cone far plane / cull radius).
const SPOT_DEMO_RANGE: f32 = 8.0;

/// Shadow Phase 5 Inc-1-GPU: the cone axis (the world direction the spot shines along) =
/// `normalize(SPOT_DEMO_TARGET - SPOT_DEMO_POS)`. Used for both the perspective fit + the
/// `GoldenLight::spot` direction (which expects "direction TO the light" = `-axis`).
fn spot_demo_axis() -> [f32; 3] {
    let d = [
        SPOT_DEMO_TARGET[0] - SPOT_DEMO_POS[0],
        SPOT_DEMO_TARGET[1] - SPOT_DEMO_POS[1],
        SPOT_DEMO_TARGET[2] - SPOT_DEMO_POS[2],
    ];
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    [d[0] / l, d[1] / l, d[2] / l]
}

/// Shadow Phase 5 Inc-1-GPU: the SPOT-shadow showcase. ONE spot light (with its `dir_kind.w` slot
/// packed to 0 via `with_atlas_slot`) + a dim sky for fill, an ASYMMETRIC raster box caster + a
/// marker cube on a raster floor. The spot casts the box's EXACT silhouette shadow onto the floor
/// INSIDE the cone (no shadow outside the cone).
fn spot_shadow_config() -> ShowcaseConfig {
    // ONE spot (the atlas is fit to it; slot 0) + a dim sky for ambient fill. The spot's
    // `dir_kind.w` carries atlas slot 0 (`with_atlas_slot(0)`) so the resolve reads
    // `light_atlas_slot(L.kind) == 0` and samples `gFaces[0]`. The spot illuminates the scene and
    // is the ONLY shadow-casting punctual.
    let header = GoldenLightHeader::new(1, 1, 1.0).with_ssao_mode(0);
    let axis = spot_demo_axis();
    // The table LAYOUT is [L0a (directional/sky)..., L0b (point/spot)...]: with l0a_count=1 the sky
    // MUST be element 0 (the L0a block) and the spot element 1 (the L0b clustered punctual). The
    // reversed order ([spot, sky]) made the resolve read the spot as the L0a light + the sky as the
    // punctual → the spot was never clustered → the scene rendered black.
    let lights = vec![
        // L0a: a dim sky for ambient fill.
        GoldenLight::sky([0.06, 0.06, 0.08], [0.05, 0.05, 0.06]),
        // L0b: the spot. The resolve's cone test is `dot(-l, L.dir)` (in-cone when L.dir aligns with
        // light->surface), so L.dir is the SHINE direction = `axis` (NOT -axis). Its `dir_kind.w`
        // carries atlas slot 0 (`with_atlas_slot(0)`) so the resolve samples `gFaces[0]`.
        GoldenLight::spot(
            SPOT_DEMO_POS,
            axis,
            [1.0, 0.96, 0.88],
            220.0,
            SPOT_DEMO_RANGE,
            SPOT_DEMO_INNER_DEG,
            SPOT_DEMO_OUTER_DEG,
        )
        .with_atlas_slot(0),
    ];

    // NO SDF: the raster floor + box own every lit pixel (the floor's mask=1 makes the spot's
    // multiply land on a real surface). A single degenerate far edit keeps the edit-list valid.
    let sdf = vec![SdfEdit::sphere([0.0, -1000.0, 0.0], 0.01, sdf_op::UNION, 0.0)];

    // The CASTER: an ASYMMETRIC box (a tall slab, distinct X/Y/Z extents) standing on the floor,
    // with a small marker cube on top so the cast shadow's orientation reads. ONE instanced mesh
    // entry, two affines (the slab + the marker), so the depth pass + the resolve both see them.
    let (slab_v, slab_i) = mesh_box_model([0.35, 0.9, 0.55], [0.82, 0.42, 0.30, 1.0]);
    let affines = vec![
        instance_affine(0.4, 1.0, [0.0, 0.9, -1.2]),  // the slab, yawed 0.4 rad
        instance_affine(0.4, 0.28, [0.0, 1.95, -1.2]), // a small marker cube on top
    ];

    // The raster FLOOR: a wide flat box (a thin slab) at y≈0 spanning the scene.
    let (floor_v, floor_i) = mesh_box_model([4.0, 0.05, 4.0], ROOM_FLOOR_COLOR);
    let floor_affine = instance_affine(0.0, 1.0, [0.0, -0.05, -1.0]);

    ShowcaseConfig {
        sdf,
        camera: room_camera(),
        light_header: header,
        light_elems: lights,
        vertices: showcase_quad_vertices(),
        mvp: instanced_room_mvp_bytes(),
        ssao_quality: None,
        mesh_sdf: None,
        instanced: Some(InstancedMesh {
            meshes: vec![
                InstancedMeshEntry { vertices: floor_v, indices: floor_i, affines: vec![floor_affine] },
                InstancedMeshEntry { vertices: slab_v, indices: slab_i, affines },
            ],
            non_casters: vec![],
        }),
        // CSM OFF (this is the SPOT demo — the directional cascade path stays off).
        csm: None,
        // SPOT ON (Inc 1): ONE atlas face fit to the spot (slot 0), looking from the apex along the
        // cone axis, FOV `2·outer`. The resolve multiplies its PCF sample into the spot's contribution.
        spot_atlas: Some(spot_single_face(
            spot_demo_view_proj(SPOT_DEMO_POS, axis, SPOT_DEMO_OUTER_DEG.to_radians(), SPOT_DEMO_RANGE),
            SPOT_DEMO_POS,
            SPOT_DEMO_RANGE,
        )),
    }
}

/// **Shadow Phase 5 Inc-1-GPU — the sparse SPOT hardware shadow screenshot.** Drives the spot DEPTH
/// pass (the asymmetric box rendered from the spot POV into atlas layer 0) + the resolve per-spot
/// multiply, dumping a TRUE 512×512 BMP to [`SPOT_SHADOW_BMP`] for the owner's RTX visual sign-off
/// (the deliverable: the box's EXACT hard shadow on the raster floor, contained inside the cone).
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the spot shadow screenshot"]
fn engine_spot_shadow_512_screenshot_dump() {
    run_showcase_dump("boyko_engine spot shadow 512", SPOT_SHADOW_BMP, spot_shadow_config(), false);
}

/// **Shadow Phase 5 Inc-1-GPU — the HOST↔SHADER SPOT MATRIX GOLDEN (the acne oracle).** Asserts the
/// host reprojection ([`spot_host_project`], the mirror of the resolve's `spot_atlas_visibility` UV
/// math) AGREES with a direct column-major `view_proj · P` projection of a KNOWN caster point — so
/// the depth-pass write and the resolve lookup (which read the SAME `view_proj` bytes) cannot drift.
/// A point under the spot cone maps inside `[0,1]²`; a point far outside maps out of bounds.
#[test]
fn spot_matrix_golden_host_projection_agrees() {
    let axis = spot_demo_axis();
    let view_proj =
        spot_demo_view_proj(SPOT_DEMO_POS, axis, SPOT_DEMO_OUTER_DEG.to_radians(), SPOT_DEMO_RANGE);

    // A receiver on the floor under the spot's aim point: in-bounds, depth in [0,1].
    let p_floor = SPOT_DEMO_TARGET;
    let n_up = [0.0, 1.0, 0.0];
    let (uv_x, uv_y, ndc_z, in_bounds) = spot_host_project(&view_proj, p_floor, n_up);
    assert!(
        in_bounds,
        "the floor point under the spot must project inside the cone footprint \
         (uv = ({uv_x}, {uv_y}), ndc_z = {ndc_z})"
    );
    assert!((0.0..=1.0).contains(&ndc_z), "the receiver depth must be in the Vulkan [0,1] range");

    // The normal-offset bias MUST move the lookup off the surface (the acne oracle — the bias is
    // applied, not a no-op): a biased point differs from the un-biased projection in SOME component.
    let (uv_x0, uv_y0, ndc_z0, _) = spot_host_project(&view_proj, p_floor, [0.0, 0.0, 0.0]);
    assert!(
        (uv_x - uv_x0).abs() > f32::EPSILON
            || (uv_y - uv_y0).abs() > f32::EPSILON
            || (ndc_z - ndc_z0).abs() > f32::EPSILON,
        "the normal-offset bias must perturb the lookup (acne oracle)"
    );

    // A point far outside the cone footprint projects out of bounds (the resolve treats it lit).
    let p_far = [100.0, 0.0, -1.2];
    let (_, _, _, far_in_bounds) = spot_host_project(&view_proj, p_far, n_up);
    assert!(!far_in_bounds, "a point far outside the spot cone must project out of bounds");

    // The COLUMN-MAJOR transpose pin: a direct `view_proj · P` (no bias) must reproduce the same
    // clip the host helper computes internally — the depth pass uses this EXACT product.
    let pw = [p_floor[0], p_floor[1], p_floor[2], 1.0];
    let mut clip = [0.0f32; 4];
    for (r, clip_r) in clip.iter_mut().enumerate() {
        let mut acc = 0.0f32;
        for (c, &pw_c) in pw.iter().enumerate() {
            acc += view_proj[c * 4 + r] * pw_c;
        }
        *clip_r = acc;
    }
    assert!(clip[3] > 0.0, "the perspective clip.w must be positive (the depth pass divides by it)");
    let direct_uv_x = (clip[0] / clip[3]) * 0.5 + 0.5;
    let (uv_x_nobias, _, _, _) = spot_host_project(&view_proj, p_floor, [0.0, 0.0, 0.0]);
    assert!(
        (direct_uv_x - uv_x_nobias).abs() < 1e-5,
        "the direct column-major product and the host helper must agree (the majorness pin): \
         {direct_uv_x} vs {uv_x_nobias}"
    );

    // The atlas-slot pack/unpack pin: `GoldenLight::spot(..).with_atlas_slot(0)` round-trips through
    // `atlas_slot()` to 0 (the demo packs slot 0; the resolve reads `light_atlas_slot(L.kind)`), and
    // the kind tag survives (still SPOT). A `GOLDEN_SLOT_NONE` light has the casts bit clear.
    let spot = GoldenLight::spot(
        SPOT_DEMO_POS,
        [-axis[0], -axis[1], -axis[2]],
        [1.0, 1.0, 1.0],
        100.0,
        SPOT_DEMO_RANGE,
        SPOT_DEMO_INNER_DEG,
        SPOT_DEMO_OUTER_DEG,
    )
    .with_atlas_slot(0);
    assert_eq!(spot.atlas_slot(), 0, "with_atlas_slot(0) must round-trip through atlas_slot()");
    assert_eq!(spot.kind(), GOLDEN_LIGHT_KIND_SPOT, "the kind tag must survive the slot pack");
    assert!(spot.casts_sdf_shadow(), "a real slot must set the casts-shadow bit");
}

// === Shadow Phase 5 Increment 2 (POINT cube) — the OMNI point shadow demo + the host golden. =====

/// Shadow Phase 5 Inc-2 (POINT cube): the BMP dump path for the omni point shadow screenshot.
const POINT_SHADOW_BMP: &str = "D:\\tmp\\engine_point_shadow.bmp";

/// Shadow Phase 5 Inc-2 (POINT cube): the point light world position — the SCENE CENTER, slightly
/// above the floor, so its omni shadow of the central caster falls on the surrounding walls (every
/// cube face exercised).
const POINT_DEMO_POS: [f32; 3] = [0.0, 1.4, -1.0];
/// Shadow Phase 5 Inc-2: the point's range (the cube far plane / cull radius) — wide enough to
/// cover the floor + the four walls around the caster.
const POINT_DEMO_RANGE: f32 = 9.0;

/// Shadow Phase 5 Inc-2 (POINT cube): the OMNI point-shadow showcase. ONE point light (its
/// `dir_kind.w` slot packed to the cube BASE 0 via `with_atlas_slot`) + a dim sky for fill, an
/// ASYMMETRIC raster slab caster at the scene center RINGED by four walls + a floor. The point
/// casts the slab's silhouette onto MULTIPLE walls (all six cube faces are produced; the visible
/// walls show the omni shadow). NO SDF.
fn point_shadow_config() -> ShowcaseConfig {
    // The table LAYOUT is [L0a (sky)..., L0b (point)...]: sky at element 0 (L0a), the point at
    // element 1 (the L0b clustered punctual). The point's `dir_kind.w` carries the cube slot BASE 0
    // (`with_atlas_slot(0)`) so the resolve reads `light_atlas_slot(L.kind) == 0` then major-axis-
    // selects `gFaces[0..6]`.
    let header = GoldenLightHeader::new(1, 1, 1.0).with_ssao_mode(0);
    let lights = vec![
        // L0a: a dim sky for ambient fill.
        GoldenLight::sky([0.05, 0.05, 0.07], [0.04, 0.04, 0.05]),
        // L0b: the omni point at the scene center, packing cube slot base 0.
        GoldenLight::point(POINT_DEMO_POS, [1.0, 0.95, 0.88], 320.0, POINT_DEMO_RANGE)
            .with_atlas_slot(0),
    ];

    // NO SDF: a single degenerate far edit keeps the edit-list valid; the raster floor/walls/caster
    // own every lit pixel.
    let sdf = vec![SdfEdit::sphere([0.0, -1000.0, 0.0], 0.01, sdf_op::UNION, 0.0)];

    // The CASTER: an ASYMMETRIC slab (distinct X/Y/Z extents) standing at the scene center, just
    // BELOW the point light, with a small marker cube on top so the cast shadow's orientation reads.
    // ONE instanced mesh entry, two affines (the slab + the marker).
    let (slab_v, slab_i) = mesh_box_model([0.30, 0.55, 0.18], [0.85, 0.40, 0.28, 1.0]);
    let caster_affines = vec![
        instance_affine(0.5, 1.0, [0.0, 0.55, -1.0]),  // the slab, yawed 0.5 rad
        instance_affine(0.5, 0.25, [0.0, 1.15, -1.0]), // a small marker cube on top
    ];

    // The raster FLOOR: a wide flat box (a thin slab) at y≈0 spanning the room.
    let (floor_v, floor_i) = mesh_box_model([4.0, 0.05, 4.0], ROOM_FLOOR_COLOR);
    let floor_affine = instance_affine(0.0, 1.0, [0.0, -0.05, -1.0]);

    // FOUR WALLS ringing the caster (thin vertical slabs) at ±X and ±Z around the scene center, so
    // the point's omni shadow lands on whichever walls the marcher's camera can see. Each is one
    // instanced entry (its own non-uniform affine via `mesh_box_model`'s pre-scaled half-extents).
    let (wall_back_v, wall_back_i) = mesh_box_model([3.2, 2.2, 0.06], ROOM_WALL_COLOR);
    let wall_back_affine = instance_affine(0.0, 1.0, [0.0, 2.2, -4.0]); // far -Z wall
    let (wall_left_v, wall_left_i) = mesh_box_model([0.06, 2.2, 3.2], ROOM_WALL_COLOR);
    let wall_left_affine = instance_affine(0.0, 1.0, [-3.4, 2.2, -1.0]); // -X wall
    let (wall_right_v, wall_right_i) = mesh_box_model([0.06, 2.2, 3.2], ROOM_WALL_COLOR);
    let wall_right_affine = instance_affine(0.0, 1.0, [3.4, 2.2, -1.0]); // +X wall

    ShowcaseConfig {
        sdf,
        camera: room_camera(),
        light_header: header,
        light_elems: lights,
        vertices: showcase_quad_vertices(),
        mvp: instanced_room_mvp_bytes(),
        ssao_quality: None,
        mesh_sdf: None,
        instanced: Some(InstancedMesh {
            meshes: vec![
                InstancedMeshEntry { vertices: floor_v, indices: floor_i, affines: vec![floor_affine] },
                InstancedMeshEntry { vertices: wall_back_v, indices: wall_back_i, affines: vec![wall_back_affine] },
                InstancedMeshEntry { vertices: wall_left_v, indices: wall_left_i, affines: vec![wall_left_affine] },
                InstancedMeshEntry { vertices: wall_right_v, indices: wall_right_i, affines: vec![wall_right_affine] },
                InstancedMeshEntry { vertices: slab_v, indices: slab_i, affines: caster_affines },
            ],
            non_casters: vec![],
        }),
        // CSM OFF (this is the POINT demo).
        csm: None,
        // POINT ON (Inc 2): a six-face cube fit to the point (slot base 0), the standard
        // `[+X, -X, +Y, -Y, +Z, -Z]` 90°-FOV faces. The resolve major-axis-selects + does the
        // linear-distance compare; the depth pass renders each face into layers 0..6.
        spot_atlas: Some(point_cube_fit(POINT_DEMO_POS, POINT_DEMO_RANGE)),
    }
}

/// **Shadow Phase 5 Inc-2 (POINT cube) — the omni point hardware shadow screenshot.** Drives the
/// six-face POINT cube DEPTH pass (the asymmetric slab rendered from the point POV into atlas layers
/// 0..6 with the linear-distance FS) + the resolve per-point major-axis face-select + distance
/// compare, dumping a TRUE 512×512 BMP to [`POINT_SHADOW_BMP`] for the owner's RTX visual sign-off
/// (the deliverable: the slab's OMNI shadow on the surrounding walls).
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the point shadow screenshot"]
fn engine_point_shadow_512_screenshot_dump() {
    run_showcase_dump("boyko_engine point shadow 512", POINT_SHADOW_BMP, point_shadow_config(), false);
}

/// Shadow Phase 5 Inc-2 (POINT cube): the HOST↔SHADER POINT FACE-SELECT + LINEAR-DISTANCE GOLDEN
/// (the cube oracle, the mirror of the resolve's `punctual_atlas_visibility`). Given the light
/// position + range + a world receiver point `P`, this picks the SAME cube face the resolve's
/// major-axis select picks and computes the SAME normalized radial distance `ref`, so the depth-FS
/// write and the resolve lookup (which share `light_pos`/`inv_range`) cannot drift. Returns
/// `(face, uv_x, uv_y, ref)`: the selected cube face index `[0,6)` (host order `[+X,-X,+Y,-Y,+Z,-Z]`),
/// the resolve's per-face UV (`uvc = (right.d, -(up.d))` + the engine Y-flip), and the receiver's
/// normalized radial distance — so a golden can also assert the UV equals the depth pass's
/// `view_proj`-rasterized UV (the no-drift pin).
fn point_host_project(light_pos: [f32; 3], range: f32, p: [f32; 3]) -> (u32, f32, f32, f32) {
    let inv_range = if range > 0.0 { range.recip() } else { 0.0 };
    let dir = [p[0] - light_pos[0], p[1] - light_pos[1], p[2] - light_pos[2]];
    let a = [dir[0].abs(), dir[1].abs(), dir[2].abs()];
    // Major-axis face select + the per-face (sc, tc) minor coords — IDENTICAL to the resolve's pick.
    let (face, ma, uvc): (u32, f32, [f32; 2]) = if a[0] >= a[1] && a[0] >= a[2] {
        if dir[0] >= 0.0 {
            (0, a[0], [-dir[2], -dir[1]])
        } else {
            (1, a[0], [dir[2], -dir[1]])
        }
    } else if a[1] >= a[0] && a[1] >= a[2] {
        if dir[1] >= 0.0 {
            (2, a[1], [-dir[0], -dir[2]])
        } else {
            (3, a[1], [dir[0], -dir[2]])
        }
    } else if dir[2] >= 0.0 {
        (4, a[2], [dir[0], -dir[1]])
    } else {
        (5, a[2], [-dir[0], -dir[1]])
    };
    let inv_ma = if ma > 1e-8 { 1.0 / ma } else { 0.0 };
    let uv_x = uvc[0] * inv_ma * 0.5 + 0.5;
    let uv_y = 1.0 - (uvc[1] * inv_ma * 0.5 + 0.5);
    let dist = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    let r = (dist * inv_range).clamp(0.0, 1.0);
    (face, uv_x, uv_y, r)
}

/// **Shadow Phase 5 Inc-2 (POINT cube) — the HOST↔SHADER POINT MATRIX/FACE GOLDEN.** Asserts the
/// host face-select + linear-distance mirror ([`point_host_project`]) picks the EXPECTED cube face
/// for a known (light, receiver) pair AND that the per-face `view_proj` (`point_face_view_proj`,
/// the bytes the depth pass pushes + the resolve UBO carries) projects the matching axis sample
/// in-bounds — so the depth-pass write and the resolve lookup cannot drift.
#[test]
fn point_matrix_golden_face_select_and_distance_agree() {
    let lp = POINT_DEMO_POS;
    let range = POINT_DEMO_RANGE;

    // A receiver straight UP from the light maps to the +Y face (2); its normalized distance is
    // `dist / range`, in [0,1] for a receiver inside the range.
    let up = [lp[0], lp[1] + 2.0, lp[2]];
    let (face_up, _, _, ref_up) = point_host_project(lp, range, up);
    assert_eq!(face_up, 2, "a receiver straight up must select the +Y cube face");
    assert!((ref_up - 2.0 / range).abs() < 1e-5, "the +Y ref must be dist/range");
    assert!((0.0..=1.0).contains(&ref_up), "the ref must be in the [0,1] depth range");

    // A receiver along -X selects the -X face (1); along +Z selects the +Z face (4).
    let (face_xn, _, _, _) = point_host_project(lp, range, [lp[0] - 3.0, lp[1] + 0.1, lp[2]]);
    assert_eq!(face_xn, 1, "a receiver toward -X must select the -X cube face");
    let (face_zp, _, _, _) = point_host_project(lp, range, [lp[0], lp[1] + 0.1, lp[2] + 3.0]);
    assert_eq!(face_zp, 4, "a receiver toward +Z must select the +Z cube face");

    // The NO-DRIFT PIN: for an OFF-AXIS receiver (a non-trivial UV), the resolve's per-face UV
    // (`point_host_project`, the mirror of `punctual_atlas_visibility`) MUST equal the UV the depth
    // pass rasterized this point at through the SAME face `view_proj` (the cube oracle). A drift in
    // the face-select sign convention vs the look-at basis would make the two disagree → a
    // wrong-texel shadow. Sample a point biased toward +X and a little +Y/+Z (still +X-major).
    let fit = point_cube_fit(lp, range);
    let off = [lp[0] + 2.0, lp[1] + 0.6, lp[2] + 0.4];
    let (face_off, uvx_off, uvy_off, _) = point_host_project(lp, range, off);
    assert_eq!(face_off, 0, "the +X-major off-axis sample must select the +X face");
    // The depth pass's rasterized UV: project `off` through face 0's view_proj, divide, Y-flip.
    let vp0 = &fit.faces[face_off as usize].view_proj;
    let pw_off = [off[0], off[1], off[2], 1.0];
    let mut clip_off = [0.0f32; 4];
    for (r, clip_r) in clip_off.iter_mut().enumerate() {
        let mut acc = 0.0f32;
        for (c, &pw_c) in pw_off.iter().enumerate() {
            acc += vp0[c * 4 + r] * pw_c;
        }
        *clip_r = acc;
    }
    assert!(clip_off[3] > 0.0, "the off-axis +X sample must be in front of the light");
    let rast_uv_x = (clip_off[0] / clip_off[3]) * 0.5 + 0.5;
    let rast_uv_y = 1.0 - ((clip_off[1] / clip_off[3]) * 0.5 + 0.5);
    assert!(
        (uvx_off - rast_uv_x).abs() < 1e-4 && (uvy_off - rast_uv_y).abs() < 1e-4,
        "the resolve UV ({uvx_off}, {uvy_off}) must equal the depth-pass rasterized UV \
         ({rast_uv_x}, {rast_uv_y}) — the cube face-select / look-at basis cannot drift"
    );

    // The selected face's `view_proj` must project the on-axis sample in front of the eye
    // (clip.w > 0) and inside the NDC box — the depth pass rendered that face with this matrix.
    // +Y face (index 2): a point 2 units up from the light projects in-bounds in face 2.
    let vp = &fit.faces[2].view_proj;
    let pw = [up[0], up[1], up[2], 1.0];
    let mut clip = [0.0f32; 4];
    for (r, clip_r) in clip.iter_mut().enumerate() {
        let mut acc = 0.0f32;
        for (c, &pw_c) in pw.iter().enumerate() {
            acc += vp[c * 4 + r] * pw_c;
        }
        *clip_r = acc;
    }
    assert!(clip[3] > 0.0, "the +Y face sample must be in front of the light (w > 0)");
    let ndc_x = clip[0] / clip[3];
    let ndc_y = clip[1] / clip[3];
    assert!(ndc_x.abs() <= 1.0 + 1e-4, "the +Y face sample x must be in NDC bounds, got {ndc_x}");
    assert!(ndc_y.abs() <= 1.0 + 1e-4, "the +Y face sample y must be in NDC bounds, got {ndc_y}");

    // The atlas-slot pack pin: a point with `with_atlas_slot(0)` round-trips its cube BASE 0 and
    // keeps the POINT kind tag + the casts bit.
    let point = GoldenLight::point(lp, [1.0, 1.0, 1.0], 100.0, range).with_atlas_slot(0);
    assert_eq!(point.atlas_slot(), 0, "the point cube base 0 must round-trip");
    assert_eq!(point.kind(), GOLDEN_LIGHT_KIND_POINT, "the POINT kind tag must survive the slot pack");
    assert!(point.casts_sdf_shadow(), "a real cube slot must set the casts-shadow bit");
}

// === The GRAND flagship showcase — the MAXIMUM-capability single-frame scene. ============
//
// One clean room that combines, in ONE frame, the engine's full shipped rendering stack:
//   - HYBRID: SDF spheres (sphere-traced by the marcher) + raster mesh boxes (instanced gbuffer
//     arm) co-rendered with correct depth ownership in the same room.
//   - The WARM directional SUN drives TWO shadow systems at once: the marcher's ANALYTIC SDF soft
//     shadow on the SDF spheres + floor (`gMaterial.r`, A1, the SDF-native path) AND the hardware
//     CSM cascaded shadow on the raster boxes (`csm_mode == 1`, the cascade depth pass).
//   - A COOL point light drives the omni POINT-cube HARDWARE shadow on the raster boxes
//     (`punctual_shadow_mode == 1`, the six-face atlas depth pass).
//   - The SAME instanced caster batches feed BOTH hardware depth passes (build-once-consume-N).
//   - PBR + the sky/hemisphere ambient fill so nothing is pure black.
//
// `run_showcase_dump` records BOTH depth passes independently: `with_csm_mode(cfg.csm.is_some())`
// + `with_punctual_shadow_mode(cfg.spot_atlas.is_some())` arm the two header bits, and
// `cfg.csm.map(..)` / `cfg.spot_atlas.map(..)` build the two `Option` activations the recorder
// renders separately into the cascade texture (@12/@13) + the atlas texture (@14/@15). Both
// resolve bindings are ALWAYS bound; ON makes the resolve `min`-combine the cascade onto the sun's
// visibility and MULTIPLY the cube into the point's contribution.

/// The grand showcase room's cool POINT light — above and to the RIGHT of the raster mesh casters,
/// so its omni cube shadow of the boxes lands on the floor + the back/side walls. Cool blue to
/// contrast the warm sun (so the two shadow systems read as distinct lights).
const GRAND_POINT_POS: [f32; 3] = [2.0, 2.6, -0.8];
/// The grand showcase point light's range (the cube far plane / cull radius) — wide enough to
/// cover the mesh casters + the surrounding walls/floor.
const GRAND_POINT_RANGE: f32 = 9.0;
/// The grand showcase's CSM cascade count (3) + the shadow distance the inline PSSM fit partitions
/// over the camera frustum's near→far view-z (mirrors `csm_cascades_config`).
const GRAND_CSM_COUNT: usize = 3;
const GRAND_CSM_FAR: f32 = 16.0;
/// The grand showcase's mesh-box materials — a WARM terracotta for the raster casters so the
/// raster-vs-SDF split reads (the SDF spheres are a light dielectric); a mid-gray floor/wall.
const GRAND_BOX_COLOR: [f32; 4] = [0.82, 0.42, 0.30, 1.0];

/// **The GRAND flagship showcase config — the MAXIMUM-capability single-frame scene.** A clean
/// room (raster floor + back wall + two short side walls) with TWO SDF spheres on the LEFT
/// (sphere-traced; the sun's analytic SDF soft shadow falls on them + the floor) and TWO raster
/// mesh boxes on the RIGHT (an asymmetric box + a tall slab; the CSM + point HARDWARE casters),
/// under the perspective [`room_camera`]. Lights `[sun, sky, point]` with `new(2, 1, ..)`. BOTH
/// `csm` (3 PSSM cascades fit to the sun) AND `spot_atlas` (the point's six-face cube fit) are
/// `Some`, so `run_showcase_dump` records both hardware depth passes; the resolve `min`-combines
/// the cascade onto the sun and multiplies the cube into the point.
fn grand_showcase_config() -> ShowcaseConfig {
    // The LIGHT TABLE order is [L0a directional, L0a sky, L0b point]: `new(2, 1, ..)` = 2 L0a + 1
    // L0b. The punctual MUST come AFTER the L0a lights or the resolve mis-reads it (the spot-demo
    // lesson). The point's `dir_kind.w` carries the cube atlas slot base 0 (`with_atlas_slot(0)`).
    let header = GoldenLightHeader::new(2, 1, 1.0).with_ssao_mode(0);
    let lights = vec![
        // L0a[0]: the WARM directional SUN — drives the CSM hardware cascades (the raster boxes)
        // AND the marcher's analytic SDF shadow (the SDF spheres; `light_dir == SHOWCASE_SUN_DIR`).
        GoldenLight::directional(SHOWCASE_SUN_DIR, [1.0, 0.95, 0.88], 3.0),
        // L0a[1]: a soft cool-sky / warm-ground hemisphere ambient so the shadows read off black.
        GoldenLight::sky([0.22, 0.27, 0.36], [0.10, 0.09, 0.08]),
        // L0b[0]: the COOL point light — drives the omni POINT cube hardware shadow on the boxes.
        // Packs cube atlas slot base 0 to match the `point_cube_fit` faces below.
        GoldenLight::point(GRAND_POINT_POS, [0.55, 0.70, 1.0], 300.0, GRAND_POINT_RANGE)
            .with_atlas_slot(0),
    ];

    // LEFT — TWO SDF spheres resting on the floor (the marcher sphere-traces them; the sun's A1
    // analytic soft shadow falls on them + the floor). 2 edits ≤ MAX_SDF_EDITS. A light dielectric
    // (material 0) so the raster-vs-SDF split reads.
    let sdf = vec![
        SdfEdit::sphere([-1.8, 0.7, -1.0], 0.7, sdf_op::UNION, 0.0),
        SdfEdit::sphere([-0.9, 0.4, -0.1], 0.4, sdf_op::UNION, 0.0),
    ];

    // RIGHT — TWO raster mesh boxes (the CSM + point HARDWARE casters): an asymmetric box + a tall
    // slab. They are distinct model-space meshes (distinct half-extents), each its own instanced
    // batch. The SAME batches feed BOTH the CSM cascade depth pass AND the point cube depth pass.
    let (box_v, box_i) = mesh_box_model([0.45, 0.45, 0.45], GRAND_BOX_COLOR);
    let box_affines = vec![instance_affine(0.5, 1.0, [1.6, 0.46, -1.2])]; // the box, yawed
    // The tall SLAB is a separately scaled box (a non-uniform stretch) so the two casters differ.
    let (slab_v, slab_i) = mesh_box_model([0.30, 0.95, 0.40], GRAND_BOX_COLOR);
    let slab_affines = vec![instance_affine(-0.3, 1.0, [2.0, 0.95, -2.6])];

    // The ROOM — a raster floor + a back wall + two short side walls, all instanced mesh boxes so
    // BOTH the CSM cascade shadow + the point cube shadow land on real raster surfaces (mask=1).
    let (floor_v, floor_i) = mesh_box_model([5.0, 0.05, 5.0], ROOM_FLOOR_COLOR);
    let floor_affine = instance_affine(0.0, 1.0, [0.0, -0.05, -1.5]);
    let (wall_back_v, wall_back_i) = mesh_box_model([5.0, 2.6, 0.06], ROOM_WALL_COLOR);
    let wall_back_affine = instance_affine(0.0, 1.0, [0.0, 2.6, -5.0]); // far -Z wall
    let (wall_left_v, wall_left_i) = mesh_box_model([0.06, 2.6, 3.5], ROOM_WALL_COLOR);
    let wall_left_affine = instance_affine(0.0, 1.0, [-4.5, 2.6, -1.5]); // -X wall
    let (wall_right_v, wall_right_i) = mesh_box_model([0.06, 2.6, 3.5], ROOM_WALL_COLOR);
    let wall_right_affine = instance_affine(0.0, 1.0, [4.5, 2.6, -1.5]); // +X wall

    ShowcaseConfig {
        sdf,
        camera: room_camera(),
        light_header: header,
        light_elems: lights,
        // The degenerate legacy mesh (the recorder draws the instanced batches instead).
        vertices: showcase_quad_vertices(),
        // The instanced-arm MVP (`use_model_matrix == 1`, push byte 84).
        mvp: instanced_room_mvp_bytes(),
        // PBR-lit, no screen-space SSAO pass (the SDF analytic AO + the two hardware shadows are
        // the deliverable; SSAO is orthogonal and adds noise to a clean read).
        ssao_quality: None,
        mesh_sdf: None,
        instanced: Some(InstancedMesh {
            meshes: vec![
                // The room shells (batches 0..4, each a single affine) + the two raster casters.
                InstancedMeshEntry { vertices: floor_v, indices: floor_i, affines: vec![floor_affine] },
                InstancedMeshEntry { vertices: wall_back_v, indices: wall_back_i, affines: vec![wall_back_affine] },
                InstancedMeshEntry { vertices: wall_left_v, indices: wall_left_i, affines: vec![wall_left_affine] },
                InstancedMeshEntry { vertices: wall_right_v, indices: wall_right_i, affines: vec![wall_right_affine] },
                InstancedMeshEntry { vertices: box_v, indices: box_i, affines: box_affines },
                InstancedMeshEntry { vertices: slab_v, indices: slab_i, affines: slab_affines },
            ],
            // RECEIVER-ONLY: the room shell — floor (batch 0) + the 3 walls (1,2,3). The floor MUST
            // be excluded too: a flat floor casts no real shadow (nothing is below it) but, left in
            // the depth maps, it stamps its own top face as the nearest occluder and SELF-shadows
            // (acne / a dim wash, worst under the omni POINT 2.6 units straight above it).
            non_casters: vec![0, 1, 2, 3],
        }),
        // CSM ON: 3 PSSM cascades fit to the sun over the camera frustum's near→far range. The
        // casters are the SAME instanced batches above (build-once-consume-N: the cascade depth
        // pass renders them from the sun POV; the resolve `min`-combines the hard shadow).
        csm: Some(csm_demo_cascades(
            SHOWCASE_SUN_DIR,
            ROOM_CAM_EYE,
            ROOM_CAM_FORWARD,
            ROOM_CAM_RIGHT,
            ROOM_CAM_UP,
            ROOM_CAM_FOV_Y,
            COMPOSITE_W as f32 / COMPOSITE_H as f32,
            CSM_DEMO_NEAR,
            GRAND_CSM_FAR,
            GRAND_CSM_COUNT,
        )),
        // POINT ON: the six-face cube fit to the point (slot base 0). The depth pass renders the
        // SAME instanced batches into atlas layers 0..6 from the point POV; the resolve major-axis-
        // selects + does the linear-distance compare, multiplying the cube into the point's term.
        spot_atlas: Some(point_cube_fit(GRAND_POINT_POS, GRAND_POINT_RANGE)),
    }
}

/// **The GRAND flagship showcase screenshot.** Renders the single-frame maximum-capability room —
/// HYBRID SDF spheres + raster mesh boxes, the warm sun driving BOTH the marcher's analytic SDF
/// shadow (on the spheres) AND the CSM hardware cascade shadow (on the boxes), the cool point light
/// driving the omni POINT cube hardware shadow (on the boxes), instancing + PBR + sky ambient — at
/// the native 512×512 composite extent, dumping a TRUE 512×512 BMP to [`GRAND_SHOWCASE_BMP`] for the
/// owner's RTX visual sign-off. Both hardware depth passes run before the resolve; both resolve
/// bindings sample.
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the grand showcase screenshot"]
fn engine_grand_showcase_512_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine grand showcase 512",
        GRAND_SHOWCASE_BMP,
        grand_showcase_config(),
        false,
    );
}

/// The GRAND flagship showcase screenshot with SDFDDGI **GI ON** — the FIRST render (rung I4) that
/// arms the live probe-update pass AND the resolve's GI-injection gate. The warm sun drives the
/// probe update, whose converged indirect irradiance the resolve injects onto the two SDF spheres
/// (the only `is_sdf_lit` geometry in [`grand_showcase_config`]). Dumps to [`GRAND_SHOWCASE_DDGI_BMP`]
/// for the owner's RTX visual sign-off. See [`run_showcase_body_ddgi`] for the deltas from the
/// GI-OFF golden body.
const GRAND_SHOWCASE_DDGI_BMP: &str = r"D:\tmp\engine_grand_showcase_ddgi.bmp";

/// **The GRAND flagship showcase — SDFDDGI I4, dynamic diffuse GI ON.** Renders the same
/// maximum-capability room as [`engine_grand_showcase_512_screenshot_dump`], but arms the live
/// DDGI probe-update pass + the resolve's word-7 bit-4 GI-injection gate and converges the probe
/// atlas over [`run_showcase_body_ddgi`]'s ramp before the readback frame, so the two SDF spheres
/// pick up the sun-driven indirect bounce. Dumps a TRUE 512×512 BMP to [`GRAND_SHOWCASE_DDGI_BMP`].
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the DDGI GI-ON showcase screenshot"]
fn engine_grand_showcase_512_ddgi_screenshot_dump() {
    with_windowed_present("boyko_engine grand showcase DDGI 512", "engine_showcase_512", |bp| {
        // `gpu_timing = None`: ZERO extra commands, byte-identical to the pre-R0 golden.
        run_showcase_body_ddgi(bp, GRAND_SHOWCASE_DDGI_BMP, grand_showcase_config(), false, None)
    });
}

// === Shared fly-camera basis (VIEWER_INITIAL_PITCH + vadd/vscale/vcross/vnorm) used by the scripted shadow-diagnostic harnesses. The interactive viewer moved to boyko_app examples/showcase.rs (a fly-able host mixed scene). ===

/// The interactive fly-camera's initial pitch (radians). At `yaw == 0` the FPS basis maps
/// `forward == [sin(yaw)·cos(pitch), sin(pitch), -cos(yaw)·cos(pitch)]`, so this pitch reproduces
/// [`ROOM_CAM_FORWARD`] (`[0, -0.371, -0.928]`) — the same down-into-the-room framing the static
/// showcase uses, so the viewer opens on the familiar shot before the owner flies off it.
const VIEWER_INITIAL_PITCH: f32 = -0.3805;

#[inline]
fn vadd(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
fn vsub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn vscale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
#[inline]
fn vcross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// Normalizes `a`, or returns `[0, 0, 0]` for a (near-)zero vector — used for the WASD move
/// vector (no input ⇒ no motion) and for the basis vectors (a degenerate cross stays zero).
#[inline]
fn vnorm_or_zero(a: [f32; 3]) -> [f32; 3] {
    let len2 = a[0] * a[0] + a[1] * a[1] + a[2] * a[2];
    if len2 > 1e-12 {
        vscale(a, 1.0 / len2.sqrt())
    } else {
        [0.0, 0.0, 0.0]
    }
}

/// Packs a [`GoldenMaterial`] into the 12-word (`3×vec4` std430) table element the marcher's
/// `MaterialGpu` reads: lane 0 `base_color`, lane 1 `mrr`, lane 2 `emissive` — all LINEAR, each
/// `f32` bitcast to its `u32` word.
#[inline]
fn pack_material(m: GoldenMaterial) -> [u32; 12] {
    let mut w = [0u32; 12];
    for c in 0..4 {
        w[c] = m.base_color[c].to_bits();
        w[4 + c] = m.mrr[c].to_bits();
        w[8 + c] = m.emissive[c].to_bits();
    }
    w
}

/// The showcase / interactive-viewer material table (3 slots): slot 0 = the default gray dielectric
/// (every SDF + mesh surface), slot 1 = a COOL-BLUE EMISSIVE (the point-light marker GLOWS), slot 2
/// = a WARM-YELLOW EMISSIVE (the sun-direction marker GLOWS). Emissive is ADDED to the lit color, so
/// the markers read as light sources from every angle (a plain dielectric marker at the point's own
/// center is self-shadowed and reads dim). Non-viewer scenes tag every edit material 0, so slots 1/2
/// are unused and those scenes' pixels stay byte-identical.
fn showcase_material_table() -> [u32; 36] {
    let mut t = [0u32; 36];
    t[0..12].copy_from_slice(&pack_material(GoldenMaterial::default()));
    t[12..24].copy_from_slice(&pack_material(GoldenMaterial::new(
        [0.05, 0.10, 0.18, 1.0],
        0.0,
        0.5,
        0.5,
        [0.35, 0.85, 2.6],
    )));
    t[24..36].copy_from_slice(&pack_material(GoldenMaterial::new(
        [0.18, 0.14, 0.05, 1.0],
        0.0,
        0.5,
        0.5,
        [2.8, 2.0, 0.6],
    )));
    t
}

/// The interactive viewer scene: the [`grand_showcase_config`] room with the directional CSM turned
/// ON and RE-FIT PER FRAME to the live camera (with a texel snap) — camera-correct AND shimmer-free —
/// plus bright SDF light MARKERS so the owner sees where each light sits. The point cube shadow
/// ([`point_cube_fit`]) is camera-independent and stays on.
/// The interactive viewer's WORLD-FIXED directional sun-shadow cascade: ONE cascade whose ortho
/// footprint covers the raster casters (the two boxes) + their sun-shadow landing zone on the floor,
/// computed ONCE — camera-INDEPENDENT.
///
/// A sun shadow is glued to the world; it must NOT move as the camera flies. Re-fitting the cascade
/// to the camera frustum each frame (the `csm_demo_cascades` PSSM path the showcase dump uses) makes
/// the hard shadow SWIM across the scene for a free-fly camera — even with a per-texel snap the
/// footprint rides the eye. For a BOUNDED scene (this room) a single fixed map covering the casters
/// is the correct, stable choice: the shadow stays put in the world, the camera flies freely around
/// it, and any mesh in `mesh_draw` casts a real geometric shadow. `split_far` is huge so every
/// visible pixel SELECTs this one cascade (the resolve's `view_z < split_far` test).
fn viewer_csm_fit() -> CsmDemoFit {
    // A FIXED sphere enclosing the two boxes (box [1.6,0.46,-1.2] + slab [2.0,0.95,-2.6]) AND the
    // short sun-shadow they cast on the floor (the sun is high — elevation ~55deg — so the shadows
    // are compact). 2048 texels over a ~9-unit diameter ≈ 0.0044 u/texel: a 0.9-unit box spans ~200
    // texels, sharp enough.
    // Cover the WHOLE room, not just the casters: a tight box-centered fit left the footprint edge
    // cutting across the far/left walls (the slab silhouette projected onto a wall that is not the
    // true receiver = a hard "weird shadow" band). The room AABB is x∈[-4.5,4.5], y∈[0,5.2],
    // z∈[-5,5]; its bounding sphere is center [0,2.6,0], radius ~7.21, so radius 8 encloses it with
    // margin. 2048 texels over a 16-unit diameter ≈ 0.0078 u/texel (~115 texels across a 0.9u box —
    // still a sharp hard shadow), and every receiver in view is inside the footprint (no edge band).
    const VIEWER_CSM_CENTER: [f32; 3] = [0.0, 2.6, 0.0];
    const VIEWER_CSM_RADIUS: f32 = 8.0;
    let half = VIEWER_CSM_RADIUS;
    // Light-space z far: `csm_cascade_view_proj` pulls the light eye back by `z_far*0.5`, so `half*4`
    // puts the casters (within `half` of the center) safely inside `[near, z_far]`.
    let view_proj =
        csm_cascade_view_proj(SHOWCASE_SUN_DIR, VIEWER_CSM_CENTER, half, CSM_DEMO_NEAR, half * 4.0);
    let texel_size = (2.0 * half) / CSM_SHADOW_DIM as f32;
    // Emit THREE IDENTICAL room-covering cascades (NOT one). The resolve's view-z SELECT runs on the
    // LIVE camera; a SINGLE cascade is fragile — if the SELECT ever walks past it (e.g. a header-vs-
    // UBO `active_count` mismatch), it lands on a zeroed cascade whose `split_far == 0` makes
    // `step(0, view_z) == 1` for every positive view_z, samples a ZERO matrix, and returns "lit" =
    // NO shadow. Three identical valid cascades + ascending `split_far` keep the SELECT on a covering
    // cascade from ANY camera distance (room view-z stays < the last split), on the proven
    // `active_count == 3` depth+resolve path. All three are the SAME world-fixed fit, so whichever is
    // picked (and the cross-fade between them) gives the identical, stable shadow.
    let mut cascades = [CsmDemoCascade { view_proj, split_far: 0.0, texel_size }; CSM_MAX_CASCADES];
    cascades[0].split_far = 6.0;
    cascades[1].split_far = 14.0;
    cascades[2].split_far = 100.0;
    CsmDemoFit { cascades, active_count: 3 }
}

fn viewer_config() -> ShowcaseConfig {
    let mut cfg = grand_showcase_config();

    // CSM is ON with a WORLD-FIXED directional cascade: a sun shadow is glued to the world, so it
    // must NOT move as the camera flies. Grand's seed re-fits the cascades to the camera frustum (it
    // swims for a free-fly camera even with a texel snap); override it with the fixed room fit
    // ([`viewer_csm_fit`]). It still arms the depth pass + activation and is seeded into every UBO
    // ring slot once at build — the fly harnesses never re-fit it, so the shadow stays put.
    cfg.csm = Some(viewer_csm_fit());

    // LIGHT MARKERS (bright SDF spheres so each light's position reads from every side). Appended to
    // the marched edit list ≤ MAX_SDF_EDITS (grand starts at 2 SDF edits; +2 markers = 4 total).
    //   * the POINT light at `GRAND_POINT_POS` — a small sphere right where the cube-shadow light is.
    //   * a "sun" marker UP toward the directional: `ROOM_CAM_TARGET + SHOWCASE_SUN_DIR · 5` (the sun
    //     is directional/infinite, so this is a finite stand-in showing which way the sun comes from).
    let sun_marker = vadd(ROOM_CAM_TARGET, vscale(SHOWCASE_SUN_DIR, 5.0));
    // EMISSIVE markers (material 1 = cool-blue glow at the POINT light, material 2 = warm-yellow glow
    // for the SUN direction) so each light reads as a glowing orb from every angle.
    cfg.sdf
        .push(SdfEdit::sphere(GRAND_POINT_POS, 0.22, sdf_op::UNION, 0.0).with_material(1));
    cfg.sdf
        .push(SdfEdit::sphere(sun_marker, 0.22, sdf_op::UNION, 0.0).with_material(2));
    debug_assert!(
        cfg.sdf.len() <= boyko_sdf_math::MAX_SDF_EDITS,
        "viewer SDF edits ({}) must fit MAX_SDF_EDITS ({})",
        cfg.sdf.len(),
        boyko_sdf_math::MAX_SDF_EDITS
    );

    // Shadow-dolly diagnostic override (`BOYKO_DOLLY_CASCADES=1`): force a SINGLE active
    // cascade so the dolly run can A/B "3 identical cascades + view-z SELECT" against
    // "no SELECT at all" — if a camera-distance shadow flip vanishes here, the cascade
    // LAYERS (or the select) differ in practice despite the identical world-fixed fit.
    if std::env::var_os("BOYKO_DOLLY_CASCADES").is_some_and(|v| v == "1")
        && let Some(fit) = cfg.csm.as_mut()
    {
        fit.active_count = 1;
        fit.cascades[0].split_far = 100.0; // one cascade covers every room view-z
    }

    cfg
}


// === Shadow-motion A/B diagnostic (`BOYKO_SHADOW_AB=1`) =========================================
//
// Deterministically separates the two candidate causes of "shadows render slightly differently
// while the camera is in motion" (the owner-reported viewer artifact):
//
//   1. CROSS-FRAME CONTAMINATION — a single-buffered GPU resource read by frame N while frame
//      N+1 overwrites it (a WAR race): a pose reached IN MOTION then renders differently from
//      the SAME pose reached statically (the sibling in-flight frame's writes tear the reads).
//   2. PURE RESAMPLING SCINTILLATION — every frame is a pure function of the camera pose, and
//      the perceived "dancing" is hard shadow-map / marcher edges requantizing under sub-pixel
//      camera motion: the motion-arrival capture is then BYTE-IDENTICAL to the static one, and
//      the micro-yaw pair quantifies how many pixels flip per milliradian of rotation.
//
// Protocol (every capture lands at the SAME pose P — bitwise-identical floats — so any byte
// difference between captures is motion HISTORY, never pose):
//   A  : 8 static warm frames at P, capture                      (static reference)
//   A2 : capture at P again                                      (repeatability control)
//   B  : 24 frames of ±0.35 rad yaw oscillation, capture at P    (rotation arrival)
//   C  : 24 frames of ±0.8 unit x-strafe oscillation, capture    (translation arrival)
//   D  : 8 static warm frames at P + 3 mrad yaw, capture         (static micro-rotation pair)
// Verdicts print as `[shadow-ab]` lines; BMPs + ×8-amplified diff maps land in `D:\tmp\`.

/// One camera pose the A/B script drives the viewer camera through.
#[derive(Clone, Copy)]
struct AbPose {
    eye: [f32; 3],
    yaw: f32,
    pitch: f32,
}

/// Writes one camera pose exactly the way a live per-frame fly loop does: the 80-byte
/// b5 camera block into THIS frame's ring slot + `scene.mvp` (byte 84 = the instanced arm).
///
/// Returns the minted [`FrameWriteToken`] — the ONE token per presented frame (R0b): the caller
/// threads it (borrowed for any further mid-frame per-slot write) into `ab_present_one`, whose
/// `render_gbuffer_frame` consumes it by value.
fn ab_set_pose(
    ctx: &VulkanContext,
    renderer: &Renderer<'_>,
    scene: &mut GBufferScene<'_>,
    p: AbPose,
) -> FrameWriteToken {
    let world_up = [0.0_f32, 1.0, 0.0];
    let (sy, cy) = p.yaw.sin_cos();
    let (sp, cp) = p.pitch.sin_cos();
    let forward = vnorm_or_zero([sy * cp, sp, -cy * cp]);
    let right = vnorm_or_zero(vcross(forward, world_up));
    let up = vcross(right, forward);
    let cam = CompositePushConstants::perspective(
        p.eye, forward, right, up, ROOM_CAM_FOV_Y, COMPOSITE_W, COMPOSITE_H,
    );
    let mut mvp = perspective_mvp_bytes(
        p.eye, forward, right, up, ROOM_CAM_FOV_Y, COMPOSITE_W as f32 / COMPOSITE_H as f32,
    );
    mvp[84] = 1;
    scene.mvp = mvp;
    let token = renderer
        .wait_frame_in_flight()
        .expect("invariant: slot fence wait precedes the per-slot camera write");
    let s = token.slot();
    if let Some(mapped) = RhiDevice::buffer_mapped_ptr(ctx, &scene.camera_ring[s]) {
        let bytes = cam.as_bytes();
        // SAFETY: identical contract to a live per-frame camera write — the
        // slot fence wait above guarantees slot `s`'s previous occupant (frame N−2) finished all
        // GPU reads of `camera_ring[s]` (the sibling in-flight frame binds + reads `s ^ 1`), the
        // mapped range is host-coherent and ≥ 80 B, and the 80-byte block is written at offset 0.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
        }
    }
    token
}

/// Presents ONE frame at the already-written pose (optionally requesting the swapchain→staging
/// readback), consuming `token` — the frame-write proof `ab_set_pose` minted for this frame
/// (R0b: the by-value consume ends the slot's host-write window). `false` = window closed /
/// swapchain recreated / device error — a recreate invalidates byte-comparison, so the caller
/// aborts the whole A/B run with a SKIP note.
#[allow(clippy::too_many_arguments)]
fn ab_present_one<'ctx>(
    ctx: &VulkanContext,
    surface: &Surface<'_>,
    swapchain: &mut Swapchain<'ctx>,
    renderer: &mut Renderer<'ctx>,
    window: &mut Window,
    scene: &GBufferScene<'_>,
    frame: &mut GBufferFrame,
    token: FrameWriteToken,
    readback: Option<&BoundBuffer>,
) -> bool {
    let present_extent = VkExtent2D { width: COMPOSITE_W, height: COMPOSITE_H };
    let clear = [0.02_f32, 0.02, 0.03, 1.0];
    if !window.pump_events() {
        return false;
    }
    window.refresh_size();
    // SAFETY: identical contract to the interactive-viewer present — one shared device, every
    // scene resource live, the composite extent covered by the dispatch + the camera UBO count.
    let r = unsafe {
        renderer.render_gbuffer_frame(
            token,
            ctx,
            surface,
            swapchain,
            scene,
            frame,
            window.width(),
            window.height(),
            clear,
            present_extent,
            readback,
        )
    };
    matches!(r, Ok(true))
}

/// Renders the readback frame at pose `p`, drains 3 more frames at the SAME pose (the
/// FRAMES_IN_FLIGHT==2 fence discipline of every windowed dump), then copies the staging bytes
/// out as normalized RGBA. `None` = the run is void (window closed / swapchain recreated).
#[allow(clippy::too_many_arguments)]
fn ab_capture<'ctx>(
    ctx: &VulkanContext,
    surface: &Surface<'_>,
    swapchain: &mut Swapchain<'ctx>,
    renderer: &mut Renderer<'ctx>,
    window: &mut Window,
    scene: &mut GBufferScene<'_>,
    frame: &mut GBufferFrame,
    staging: &BoundBuffer,
    is_bgra: bool,
    p: AbPose,
) -> Option<(Vec<u8>, u32, u32)> {
    let token = ab_set_pose(ctx, renderer, scene, p);
    if !ab_present_one(ctx, surface, swapchain, renderer, window, scene, frame, token, Some(staging))
    {
        return None;
    }
    let extent = swapchain.extent();
    for _ in 0..3 {
        let token = ab_set_pose(ctx, renderer, scene, p);
        if !ab_present_one(ctx, surface, swapchain, renderer, window, scene, frame, token, None) {
            return None;
        }
    }
    let (w, h) = (extent.width, extent.height);
    let byte_count = (w * h * 4) as usize;
    let ptr = RhiDevice::buffer_mapped_ptr(ctx, staging)
        .expect("host-visible readback staging buffer is mapped");
    let mut raw = vec![0u8; byte_count];
    // SAFETY: `ptr` maps ≥ `byte_count` host-coherent staging bytes; the readback frame's slot
    // fence was re-waited by the 3 drain frames (3 > FRAMES_IN_FLIGHT == 2), so the copy
    // completed before this read; `raw` is a fresh, non-overlapping allocation.
    unsafe { core::ptr::copy_nonoverlapping(ptr.as_ptr(), raw.as_mut_ptr(), byte_count) };
    Some((readback_to_rgba(&raw, w, h, is_bgra), w, h))
}

/// Counts differing pixels (any RGB channel) + the max channel delta between two RGBA captures.
fn ab_compare(label: &str, a: &[u8], b: &[u8]) -> (usize, u32) {
    let mut n_diff = 0usize;
    let mut max_d = 0u32;
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let d = pa
            .iter()
            .zip(pb)
            .take(3)
            .map(|(x, y)| (i32::from(*x) - i32::from(*y)).unsigned_abs())
            .max()
            .unwrap_or(0);
        if d > 0 {
            n_diff += 1;
            if d > max_d {
                max_d = d;
            }
        }
    }
    println!("[shadow-ab] {label}: {n_diff} differing px, max channel delta {max_d}");
    (n_diff, max_d)
}

/// A ×8-amplified per-channel |a−b| RGBA diff map (alpha forced opaque) for visual inspection.
fn ab_diff_map(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; a.len()];
    for ((pa, pb), po) in a.chunks_exact(4).zip(b.chunks_exact(4)).zip(out.chunks_exact_mut(4)) {
        for c in 0..3 {
            let d = (i32::from(pa[c]) - i32::from(pb[c])).unsigned_abs() * 8;
            po[c] = d.min(255) as u8;
        }
        po[3] = 0xFF;
    }
    out
}

/// === Shadow-dolly diagnostic (`BOYKO_SHADOW_DOLLY=1`) ===========================================
///
/// Reproduces the owner's report: "standing in front of the column its front face is LIT; as I
/// WALK TOWARD it a shadow APPEARS on the face." A whole-face shadow flip driven by camera
/// DISTANCE is not edge scintillation — it is a camera-dependent shadow TERM. The prime suspect
/// is the CSM view-z cascade SELECT (splits at 6.0 / 14.0 in the viewer fit): if the depth
/// layers differ in practice, walking across a split boundary flips the sampled layer.
///
/// Protocol: dolly the camera straight toward the slab's front face from 8.0 to 1.2 units,
/// capturing each pose; sample the CENTER pixel (always on the face — the camera looks straight
/// at it); print `distance | view_z | CPU-selected cascade | center RGB` and flag the largest
/// step-to-step luminance jump. Run once normally and once with `BOYKO_DOLLY_CASCADES=1`
/// (active_count forced to 1): if the flip vanishes in the second run, the cascade select /
/// layer contents are the bug.
#[allow(clippy::too_many_arguments)]
fn run_shadow_dolly<'ctx>(
    ctx: &VulkanContext,
    surface: &Surface<'_>,
    swapchain: &mut Swapchain<'ctx>,
    renderer: &mut Renderer<'ctx>,
    window: &mut Window,
    scene: &mut GBufferScene<'_>,
    frame: &mut GBufferFrame,
    staging: &BoundBuffer,
    is_bgra: bool,
) {
    // The slab (the owner's "column"): instance_affine(-0.3, 1.0, [2.0, 0.95, -2.6]), half-extents
    // (0.30, 0.95, 0.40). The camera orbits its center at eye height and dollies in per bearing —
    // the two single-bearing dollies (straight-on + spawn path) showed NO flip, so this SWEEPS
    // 8 azimuths x distances hunting the owner's dark-face state programmatically. The look-at
    // targets the column center so the center pixel is always on whichever face fronts the camera.
    let target = [2.0_f32, 1.0, -2.6];

    let one_cascade = std::env::var_os("BOYKO_DOLLY_CASCADES").is_some_and(|v| v == "1");
    println!(
        "[shadow-dolly] bearing sweep: {} (viewer fit splits 6.0 / 14.0 / 100.0)",
        if one_cascade { "SINGLE cascade (active_count=1, split 100)" } else { "3 cascades" }
    );

    // 8 azimuths x 8 distances. Rows print as a compact luminance matrix; any cell whose
    // luminance deviates > 40 from its bearing's row median is flagged (the dark-face hunt).
    let bearings = 8u32;
    let dists = [7.0_f32, 5.5, 4.5, 3.5, 2.8, 2.2, 1.7, 1.3];
    let mut flagged: Vec<(f32, f32, f32)> = Vec::new(); // (azimuth_deg, dist, lum)

    for b in 0..bearings {
        let az = (b as f32) * core::f32::consts::TAU / (bearings as f32);
        let mut row: Vec<f32> = Vec::with_capacity(dists.len());
        let mut cells = String::new();
        for &d in &dists {
            let eye = [
                target[0] + az.sin() * d,
                1.55, // eye height ~ the viewer spawn height, constant across the sweep
                target[2] + az.cos() * d,
            ];
            // General look-at (FPS basis: forward = [sin(yaw)cos(p), sin(p), -cos(yaw)cos(p)]).
            let to = vsub(target, eye);
            let len = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
            let f = vscale(to, 1.0 / len);
            let pose = AbPose { eye, yaw: f[0].atan2(-f[2]), pitch: f[1].asin() };

            for _ in 0..2 {
                let token = ab_set_pose(ctx, renderer, scene, pose);
                if !ab_present_one(
                    ctx, surface, swapchain, renderer, window, scene, frame, token, None,
                ) {
                    eprintln!("SKIP shadow-dolly: window closed / swapchain recreated");
                    return;
                }
            }
            let Some((rgba, w, h)) = ab_capture(
                ctx, surface, swapchain, renderer, window, scene, frame, staging, is_bgra, pose,
            ) else {
                eprintln!("SKIP shadow-dolly: capture failed");
                return;
            };
            let ci = (((h / 2) * w + (w / 2)) * 4) as usize;
            let lum = 0.2126 * rgba[ci] as f32
                + 0.7152 * rgba[ci + 1] as f32
                + 0.0722 * rgba[ci + 2] as f32;
            row.push(lum);
            cells.push_str(&format!(" {lum:5.0}"));
        }
        // Median-deviation flagging within the bearing row.
        let mut sorted = row.clone();
        sorted.sort_by(|a, b2| a.partial_cmp(b2).expect("finite luminance"));
        let median = sorted[sorted.len() / 2];
        for (i, &l) in row.iter().enumerate() {
            if (l - median).abs() > 40.0 {
                flagged.push(((az.to_degrees()), dists[i], l));
            }
        }
        println!("[shadow-dolly] az {:5.0}° |{}", az.to_degrees(), cells);
    }
    println!("[shadow-dolly] dists    |  7.0   5.5   4.5   3.5   2.8   2.2   1.7   1.3");
    if flagged.is_empty() {
        println!("[shadow-dolly] NO dark-face flips found across the sweep (median-dev > 40).");
    } else {
        for (az, d, l) in &flagged {
            println!("[shadow-dolly] FLIP CANDIDATE: az {az:.0}° dist {d:.1} lum {l:.0}");
        }
    }

    // Dump full frames around the FIRST flip candidate (the flagged distance ± its sweep
    // neighbors) for visual term identification: which shadow/light term flips.
    if let Some(&(az_deg, d_mid, _)) = flagged.first() {
        let az = az_deg.to_radians();
        let mid_idx = dists.iter().position(|&x| (x - d_mid).abs() < 1e-3).unwrap_or(3);
        let lo = if mid_idx > 0 { dists[mid_idx - 1] } else { d_mid + 1.0 };
        let hi = if mid_idx + 1 < dists.len() { dists[mid_idx + 1] } else { d_mid - 0.5 };
        for (tag, d) in [("before", lo), ("flip", d_mid), ("after", hi)] {
            let eye = [target[0] + az.sin() * d, 1.55, target[2] + az.cos() * d];
            let to = vsub(target, eye);
            let len = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
            let f = vscale(to, 1.0 / len);
            let pose = AbPose { eye, yaw: f[0].atan2(-f[2]), pitch: f[1].asin() };
            for _ in 0..2 {
                let token = ab_set_pose(ctx, renderer, scene, pose);
                if !ab_present_one(
                    ctx, surface, swapchain, renderer, window, scene, frame, token, None,
                ) {
                    return;
                }
            }
            if let Some((rgba, w, h)) = ab_capture(
                ctx, surface, swapchain, renderer, window, scene, frame, staging, is_bgra, pose,
            ) {
                let path = format!(r"D:\tmp\shadow_dolly_{tag}_d{d:.1}.bmp");
                match write_bmp(&path, &rgba, w, h) {
                    Ok(()) => println!("[shadow-dolly] wrote {path}"),
                    Err(e) => eprintln!("[shadow-dolly] write failed {path}: {e:?}"),
                }
            }
        }
    }
}

/// === Shadow-lag diagnostic (`BOYKO_SHADOW_LAG=1`) ===============================================
///
/// The owner's decisive observation: the column-face shadow flip happens ONLY while the camera is
/// IN MOTION and converges as soon as it stops. Every prior capture protocol (A/B, dolly)
/// presented warm-up / drain frames at a FROZEN pose before reading back — which lets any
/// one-frame-stale camera input (or an in-flight mapped-UBO overwrite race) converge before the
/// comparison, masking exactly this bug class. This protocol removes the mask: walk the owner's
/// exact P-key-probed segment with the pose advancing EVERY frame, request the readback mid-walk,
/// and keep the pose ADVANCING through the fence-drain frames (the drains never alter the sampled
/// frame's bytes — its swapchain→staging copy is recorded inside the sampled frame itself); then
/// byte-compare each in-motion capture against a settled capture at the identical pose. Nonzero
/// diff convicts a camera-lag-class defect (stale ring slot / write race / intra-frame camera
/// inconsistency); all-zero pushes the hunt into viewer-loop-only per-frame writes instead.
#[allow(clippy::too_many_arguments)]
fn run_shadow_lag<'ctx>(
    ctx: &VulkanContext,
    surface: &Surface<'_>,
    swapchain: &mut Swapchain<'ctx>,
    renderer: &mut Renderer<'ctx>,
    window: &mut Window,
    scene: &mut GBufferScene<'_>,
    frame: &mut GBufferFrame,
    staging: &BoundBuffer,
    is_bgra: bool,
) {
    // Owner-reported repro segment (P-key pose probe): front face LIT here → shadow appears while
    // walking to there, aim fixed the whole way.
    let a = [0.762_f32, 1.953, 1.456];
    let b = [1.040_f32, 1.835, 0.583];
    let (yaw, pitch) = (0.3080_f32, -0.1285);
    const STEPS: usize = 60; // ~0.92 units over 60 frames ≈ the owner's walk speed at 60 fps

    for (leg, from, to) in [("fwd", a, b), ("rev", b, a)] {
        let pose_at = |k: usize| {
            let t = k as f32 / STEPS as f32;
            AbPose {
                eye: [
                    from[0] + (to[0] - from[0]) * t,
                    from[1] + (to[1] - from[1]) * t,
                    from[2] + (to[2] - from[2]) * t,
                ],
                yaw,
                pitch,
            }
        };
        // Settle the pipeline at the leg start so a sampled frame can differ from its settled
        // twin only through the in-motion history of the frames right before it.
        for _ in 0..3 {
            let token = ab_set_pose(ctx, renderer, scene, pose_at(0));
            if !ab_present_one(
                ctx, surface, swapchain, renderer, window, scene, frame, token, None,
            ) {
                eprintln!("SKIP shadow-lag: window closed / swapchain recreated");
                return;
            }
        }

        // One continuous walk with mid-walk samples.
        let samples = [15usize, 30, 45];
        let mut captures: Vec<(usize, Vec<u8>, u32, u32)> = Vec::with_capacity(samples.len());
        let mut k = 0usize;
        while k <= STEPS {
            let token = ab_set_pose(ctx, renderer, scene, pose_at(k));
            let sampled = samples.contains(&k);
            let rb = if sampled { Some(staging) } else { None };
            if !ab_present_one(ctx, surface, swapchain, renderer, window, scene, frame, token, rb) {
                eprintln!("SKIP shadow-lag: window closed / swapchain recreated");
                return;
            }
            if sampled {
                let extent = swapchain.extent();
                for d in 1..=3usize {
                    let token = ab_set_pose(ctx, renderer, scene, pose_at((k + d).min(STEPS)));
                    if !ab_present_one(
                        ctx, surface, swapchain, renderer, window, scene, frame, token, None,
                    ) {
                        eprintln!("SKIP shadow-lag: window closed / swapchain recreated");
                        return;
                    }
                }
                let (w, h) = (extent.width, extent.height);
                let byte_count = (w * h * 4) as usize;
                let ptr = RhiDevice::buffer_mapped_ptr(ctx, staging)
                    .expect("host-visible readback staging buffer is mapped");
                let mut raw = vec![0u8; byte_count];
                // SAFETY: same fence discipline as `ab_capture` — the 3 drain presents re-waited
                // the sampled frame's slot fence (3 > FRAMES_IN_FLIGHT == 2), so its swapchain→
                // staging copy completed; `raw` is a fresh, non-overlapping allocation.
                unsafe {
                    core::ptr::copy_nonoverlapping(ptr.as_ptr(), raw.as_mut_ptr(), byte_count)
                };
                captures.push((k, readback_to_rgba(&raw, w, h, is_bgra), w, h));
                k += 3; // the drains already advanced the walk
            }
            k += 1;
        }

        // Settled twins at the identical poses + verdicts.
        for (ks, motion_rgba, w, h) in &captures {
            let pose = pose_at(*ks);
            for _ in 0..2 {
                let token = ab_set_pose(ctx, renderer, scene, pose);
                if !ab_present_one(
                    ctx, surface, swapchain, renderer, window, scene, frame, token, None,
                ) {
                    eprintln!("SKIP shadow-lag: window closed / swapchain recreated");
                    return;
                }
            }
            let Some((static_rgba, _, _)) = ab_capture(
                ctx, surface, swapchain, renderer, window, scene, frame, staging, is_bgra, pose,
            ) else {
                eprintln!("SKIP shadow-lag: static capture failed");
                return;
            };
            let (n_diff, max_d) = ab_compare(
                &format!("{leg} k={ks} motion vs settled"),
                motion_rgba,
                &static_rgba,
            );
            if n_diff > 0 {
                let base = format!(r"D:\tmp\shadow_lag_{leg}_k{ks}");
                let _ = write_bmp(&format!("{base}_motion.bmp"), motion_rgba, *w, *h);
                let _ = write_bmp(&format!("{base}_static.bmp"), &static_rgba, *w, *h);
                let _ = write_bmp(
                    &format!("{base}_diff.bmp"),
                    &ab_diff_map(motion_rgba, &static_rgba),
                    *w,
                    *h,
                );
                println!("[shadow-lag] wrote {base}_{{motion,static,diff}}.bmp (max delta {max_d})");
            }
        }
    }
    println!(
        "[shadow-lag] verdict key: nonzero diffs = camera-lag-class defect (stale slot / write \
         race / intra-frame inconsistency); all-zero = the lag lives in viewer-loop-only writes."
    );
}

/// The scripted-camera A/B loop (`BOYKO_SHADOW_AB=1` swaps this in for the interactive loop; the
/// heavy `run_showcase_dump` setup + teardown are shared verbatim). See the block comment above.
#[allow(clippy::too_many_arguments)]
fn run_shadow_motion_ab<'ctx>(
    ctx: &VulkanContext,
    surface: &Surface<'_>,
    swapchain: &mut Swapchain<'ctx>,
    renderer: &mut Renderer<'ctx>,
    window: &mut Window,
    scene: &mut GBufferScene<'_>,
    frame: &mut GBufferFrame,
    staging: &BoundBuffer,
    is_bgra: bool,
) {
    // Pose P — the viewer spawn pose. All five captures land bitwise-exactly here (or at the
    // micro-yaw variant), so capture differences can only come from the frames BEFORE them.
    let pose_p = AbPose { eye: ROOM_CAM_EYE, yaw: 0.0, pitch: VIEWER_INITIAL_PITCH };

    // Runs `n` scripted frames, pose per frame from `f(i)`. `false` = abort (window/swapchain).
    macro_rules! sweep {
        ($n:expr, $f:expr) => {{
            let mut ok = true;
            for i in 0..$n {
                let p: AbPose = $f(i);
                let token = ab_set_pose(ctx, renderer, scene, p);
                if !ab_present_one(
                    ctx, surface, swapchain, renderer, window, scene, frame, token, None,
                ) {
                    ok = false;
                    break;
                }
            }
            ok
        }};
    }
    macro_rules! capture_or_skip {
        ($p:expr, $label:expr) => {
            match ab_capture(ctx, surface, swapchain, renderer, window, scene, frame, staging, is_bgra, $p) {
                Some(c) => c,
                None => {
                    eprintln!("SKIP shadow-ab: window closed / swapchain recreated during {}", $label);
                    return;
                }
            }
        };
    }

    // A + A2: static reference + repeatability control.
    if !sweep!(8, |_i| pose_p) {
        eprintln!("SKIP shadow-ab: window closed during warm-up");
        return;
    }
    let (a, w, h) = capture_or_skip!(pose_p, "capture A");
    let (a2, ..) = capture_or_skip!(pose_p, "capture A2");

    // B: rotation arrival — 24 frames of fast yaw oscillation (~±20°, sign flips), then P.
    if !sweep!(24, |i: u32| AbPose { yaw: 0.35 * (0.8 * i as f32).sin(), ..pose_p }) {
        eprintln!("SKIP shadow-ab: window closed during the yaw sweep");
        return;
    }
    let (b, ..) = capture_or_skip!(pose_p, "capture B");

    // C: translation arrival — 24 frames of fast x-strafe oscillation (±0.8 units), then P.
    if !sweep!(24, |i: u32| AbPose {
        eye: vadd(pose_p.eye, [0.8 * (0.8 * i as f32).sin(), 0.0, 0.0]),
        ..pose_p
    }) {
        eprintln!("SKIP shadow-ab: window closed during the strafe sweep");
        return;
    }
    let (c, ..) = capture_or_skip!(pose_p, "capture C");

    // D: STATIC micro-rotation pair — 3 mrad of yaw (≈0.17°, ~1–2 px of screen shift at this
    // FOV), statically warmed like A. A-vs-D quantifies edge requantization per milliradian.
    let pose_d = AbPose { yaw: 0.003, ..pose_p };
    if !sweep!(8, |_i| pose_d) {
        eprintln!("SKIP shadow-ab: window closed during the micro-yaw warm-up");
        return;
    }
    let (d, ..) = capture_or_skip!(pose_d, "capture D");

    // Verdicts.
    let (rep, _) = ab_compare("A vs A2 (static repeatability)", &a, &a2);
    let (rot, rot_max) = ab_compare("A vs B  (rotation arrival)", &a, &b);
    let (mov, mov_max) = ab_compare("A vs C  (translation arrival)", &a, &c);
    let (micro_n, micro_max) = ab_compare("A vs D  (static 3 mrad yaw pair)", &a, &d);
    let contaminated = rot > 0 || mov > 0;
    println!(
        "[shadow-ab] VERDICT: repeatability {}; motion contamination {} (rot {rot} px max {rot_max}, \
         move {mov} px max {mov_max}); micro-yaw requantization {micro_n} px (max {micro_max})",
        if rep == 0 { "OK" } else { "FAIL — nondeterministic even static!" },
        if contaminated { "PRESENT — a cross-frame race is alive" } else { "NONE — the frame is a pure function of pose" },
    );

    // Artifacts for visual inspection (the diff maps are ×8-amplified).
    let dump = |name: &str, rgba: &[u8]| {
        let path = format!(r"D:\tmp\shadow_ab_{name}.bmp");
        match write_bmp(&path, rgba, w, h) {
            Ok(()) => println!("[shadow-ab] wrote {path}"),
            Err(e) => eprintln!("[shadow-ab] failed to write {path}: {e:?}"),
        }
    };
    dump("a_static", &a);
    dump("b_rot_arrival", &b);
    dump("c_move_arrival", &c);
    dump("d_micro_yaw", &d);
    dump("diff_a_vs_b_x8", &ab_diff_map(&a, &b));
    dump("diff_a_vs_c_x8", &ab_diff_map(&a, &c));
    dump("diff_a_vs_d_x8", &ab_diff_map(&a, &d));
}

/// Pillar B B3 GPU KEYSTONE (`BOYKO_INTERP_SMOKE=1`). Proves the WIRED interp compute pre-pass
/// moves the drawn geometry per the interpolated pose, on real GPU:
///
///   1. Renders the production scene through the interp pass at `alpha = 0.0` with instance 0 given
///      a MOVING pair (`prev` = its base pose, `curr` = base + a visible +X/+Y delta) and every other
///      instance STILL (`prev == curr`). At `alpha = 0` the interpolated pose == `prev` == the base
///      pose (instance 0 unmoved).
///   2. Renders again at `alpha ≈ 0.5` with the SAME pairs — instance 0's interpolated pose is now
///      halfway to `curr` (visibly shifted); every still instance is bitwise-unchanged (the B2
///      keystone: `mix(prev, curr, a) == prev == curr`).
///   3. Asserts the two captures DIFFER (the moving instance shifted → some pixels changed) — the
///      interp pass IS driving the draw — and dumps both + an ×8 diff map for the owner.
///
/// The alpha is scripted (not `overstep_fraction()`) so the proof is deterministic; the production
/// wiring feeds the real overstep in a live fly loop. Runs on the SAME production path
/// (`render_gbuffer_frame` + the framegraph COMPUTE→VERTEX barrier), so a green run validates the
/// whole B3 GPU chain. The interp draw SSBO the compute writes IS the SSBO the raster VS reads
/// (`scene.instance_bind_group = interp.draw_bg[fi]` each frame).
#[allow(clippy::too_many_arguments)]
fn run_interp_smoke<'ctx, 's>(
    ctx: &VulkanContext,
    surface: &Surface<'_>,
    swapchain: &mut Swapchain<'ctx>,
    renderer: &mut Renderer<'ctx>,
    window: &mut Window,
    scene: &mut GBufferScene<'s>,
    frame: &mut GBufferFrame,
    staging: &BoundBuffer,
    is_bgra: bool,
    interp: &'s InterpGpu,
    base_pairs: &[InterpPair],
) {
    // The scripted move on instance 0: `curr = prev + delta` (a visible in-plane shift). The room's
    // first instanced entry is the moving subject; every other instance stays still (prev == curr).
    let delta = [0.9_f32, 0.6, 0.0];
    let mut moved: Vec<InterpPair> = base_pairs.to_vec();
    if let Some(p0) = moved.first_mut() {
        p0.curr[0] += delta[0];
        p0.curr[1] += delta[1];
        p0.curr[2] += delta[2];
    }

    let pose = AbPose { eye: ROOM_CAM_EYE, yaw: 0.0, pitch: VIEWER_INITIAL_PITCH };

    // Binds THIS frame slot's interp set + draw-read instance bind group + the pose, then presents
    // one frame (optionally with readback). A macro (not a closure) so it drives the outer `scene`
    // directly — assigning `scene.instance_bind_group = &interp.draw_bg[fi]` needs the `interp`
    // borrow to share the scene's `'s` lifetime, which a closure cannot express.
    //
    // ONE token per presented frame (R0b): `ab_set_pose` mints it (camera-write order vs the pair
    // write does not matter — both precede recording), the pair write borrows it, and
    // `ab_present_one`'s `render_gbuffer_frame` consumes it by value.
    macro_rules! present_interp {
        ($alpha:expr, $readback:expr) => {{
            let token = ab_set_pose(ctx, renderer, scene, pose);
            let fi = token.slot();
            interp.write_pairs(ctx, &token, &moved);
            scene.instance_bind_group = &interp.draw_bg[fi];
            scene.interp = Some(interp.activation(fi, $alpha));
            ab_present_one(ctx, surface, swapchain, renderer, window, scene, frame, token, $readback)
        }};
    }
    // Captures one readback frame at `alpha`: 1 readback + 3 drain presents (the FRAMES_IN_FLIGHT==2
    // fence discipline), each re-wiring interp for the slot the upcoming present binds. Yields the
    // normalized RGBA + dims, or `None` (window close / swapchain recreate — the run is void).
    macro_rules! capture_at {
        ($alpha:expr) => {{
            if !present_interp!($alpha, Some(staging)) {
                None
            } else {
                let extent = swapchain.extent();
                let mut ok = true;
                for _ in 0..3 {
                    if !present_interp!($alpha, None) {
                        ok = false;
                        break;
                    }
                }
                if !ok {
                    None
                } else {
                    let (w, h) = (extent.width, extent.height);
                    let byte_count = (w * h * 4) as usize;
                    let ptr = RhiDevice::buffer_mapped_ptr(ctx, staging)
                        .expect("host-visible readback staging buffer is mapped");
                    let mut raw = vec![0u8; byte_count];
                    // SAFETY: `ptr` maps ≥ `byte_count` host-coherent staging bytes; the readback
                    // frame's slot fence was re-waited by the 3 drain presents (3 > FRAMES_IN_FLIGHT
                    // == 2), so the copy completed; `raw` is a fresh non-overlapping allocation.
                    unsafe { core::ptr::copy_nonoverlapping(ptr.as_ptr(), raw.as_mut_ptr(), byte_count) };
                    Some((readback_to_rgba(&raw, w, h, is_bgra), w, h))
                }
            }
        }};
    }

    // Warm-up so the swapchain/pipelines settle (the dumps' discipline).
    for _ in 0..4 {
        if !present_interp!(0.0_f32, None) {
            eprintln!("SKIP interp-smoke: window closed / swapchain recreated during warm-up");
            return;
        }
    }

    let Some((a0, w, h)) = capture_at!(0.0_f32) else {
        eprintln!("SKIP interp-smoke: window closed / swapchain recreated during alpha=0 capture");
        return;
    };
    let Some((a5, ..)) = capture_at!(0.5_f32) else {
        eprintln!("SKIP interp-smoke: window closed / swapchain recreated during alpha=0.5 capture");
        return;
    };

    let (n_diff, max_d) = ab_compare("interp alpha=0 vs alpha=0.5 (moving instance)", &a0, &a5);
    // Dump artifacts for the owner's visual confirmation.
    let dump = |name: &str, rgba: &[u8]| {
        let path = format!(r"D:\tmp\interp_smoke_{name}.bmp");
        match write_bmp(&path, rgba, w, h) {
            Ok(()) => println!("[interp-smoke] wrote {path}"),
            Err(e) => eprintln!("[interp-smoke] failed to write {path}: {e:?}"),
        }
    };
    dump("alpha0", &a0);
    dump("alpha5", &a5);
    dump("diff_x8", &ab_diff_map(&a0, &a5));

    // THE KEYSTONE: the interpolated pose at alpha=0.5 moved instance 0, so the two captures DIFFER.
    // (If interp were a no-op, or the barrier were missing and the VS read a stale/empty draw SSBO,
    // the two frames would be identical — a zero-diff FAIL.)
    assert!(
        n_diff > 0,
        "interp GPU keystone: alpha=0 and alpha=0.5 captures are IDENTICAL — the interp pre-pass \
         did NOT move the drawn geometry (the wired compute pass / COMPUTE→VERTEX barrier is dead). \
         Expected the moving instance's pixels to differ (max channel delta was {max_d})."
    );
    println!(
        "[interp-smoke] PASS: interp pre-pass drives the draw — {n_diff} px differ between alpha=0 \
         and alpha=0.5 (max channel delta {max_d})."
    );
}


/// **Shadow-motion A/B diagnostic.** Runs the EXACT interactive-viewer scene/path with a
/// scripted camera instead of live input: captures pose P reached statically vs reached in
/// motion (yaw / strafe oscillation) and byte-compares — separating a cross-frame WAR race
/// (motion arrival ≠ static) from pure pose-resampling scintillation (byte-identical arrivals;
/// the static 3 mrad micro-yaw pair then quantifies edge requantization). Prints `[shadow-ab]`
/// verdict lines and dumps BMPs + ×8 diff maps to `D:\tmp\shadow_ab_*.bmp`.
#[test]
#[ignore = "needs a real RTX windowed device; scripted shadow-motion A/B capture protocol"]
fn shadow_motion_ab_dump() {
    // SAFETY: set before any other thread reads the environment (the test body is the process's
    // first activity under `--test-threads=1`, the only supported way to run windowed dumps).
    unsafe { std::env::set_var("BOYKO_SHADOW_AB", "1") };
    run_showcase_dump(
        "boyko_engine shadow motion A/B",
        GRAND_SHOWCASE_BMP,
        viewer_config(),
        true,
    );
}

/// **Shadow-dolly diagnostic.** Reproduces the owner's "a shadow APPEARS on the column's front
/// face as I walk toward it" report deterministically: dollies the camera straight toward the
/// slab's front face (8.0 → 1.2 units, crossing the viewer fit's 6.0 view-z split), tables the
/// center-pixel shadow state vs the CPU-mirrored cascade SELECT, then repeats with the fit
/// forced to a SINGLE cascade. A luminance flip at the split boundary that vanishes in the
/// single-cascade run convicts the cascade select / layer contents.
#[test]
#[ignore = "needs a real RTX windowed device; scripted camera-dolly shadow diagnostic"]
fn shadow_dolly_dump() {
    // SAFETY: set before any other thread reads the environment (the test body is the process's
    // first activity under `--test-threads=1`, the only supported way to run windowed dumps).
    unsafe {
        std::env::set_var("BOYKO_SHADOW_DOLLY", "1");
        std::env::remove_var("BOYKO_DOLLY_CASCADES");
    }
    run_showcase_dump("boyko_engine shadow dolly (3 cascades)", GRAND_SHOWCASE_BMP, viewer_config(), true);

    // SAFETY: same single-threaded windowed-test contract as above.
    unsafe { std::env::set_var("BOYKO_DOLLY_CASCADES", "1") };
    run_showcase_dump("boyko_engine shadow dolly (1 cascade)", GRAND_SHOWCASE_BMP, viewer_config(), true);
}

/// **Shadow-lag diagnostic.** The owner pinned the flip to MOTION: the column-face shadow appears
/// only while the camera moves and settles back once it stops. Walks his exact P-key-probed
/// segment with the pose advancing every frame, captures mid-walk WITHOUT freezing the pose
/// during the fence drains, and byte-compares each in-motion frame against a settled frame at the
/// identical pose. Nonzero diff = camera-lag-class defect (stale ring slot / mapped-write race /
/// intra-frame camera inconsistency); all-zero = the lag lives in viewer-loop-only writes.
#[test]
#[ignore = "needs a real RTX windowed device; in-motion vs settled same-pose byte comparison"]
fn shadow_lag_dump() {
    // SAFETY: set before any other thread reads the environment (the test body is the process's
    // first activity under `--test-threads=1`, the only supported way to run windowed dumps).
    unsafe { std::env::set_var("BOYKO_SHADOW_LAG", "1") };
    run_showcase_dump(
        "boyko_engine shadow lag (motion vs settled)",
        GRAND_SHOWCASE_BMP,
        viewer_config(),
        true,
    );
}

// === CSM Increment 3 — Rung B: the N-cascade (multi-distance) demo + the select+blend golden. ===

/// CSM Increment 3 (Rung B): the demo's cascade count (3) + the view-z range the inline fit
/// partitions. The camera looks down a LONG raster floor; three PSSM cascades cover near→far so the
/// receding caster boxes land in cascades 0, 1, 2 and the boundaries cross-fade.
const CSM_CASCADES_COUNT: usize = 3;
const CSM_CASCADES_FAR: f32 = 18.0; // the shadow distance (the last cascade's far)

/// CSM Increment 3 (Rung B): builds the demo's 3-cascade fit from the room camera basis + the sun.
/// The cascades PSSM-partition the camera frustum's `[near, CSM_CASCADES_FAR]` view-z range (the
/// inline mirror of `boyko_render::resolve_csm`), so the SELECT picks the tightest cascade per pixel
/// and the boundaries blend.
fn csm_cascades_fit() -> CsmDemoFit {
    csm_demo_cascades(
        SHOWCASE_SUN_DIR,
        ROOM_CAM_EYE,
        ROOM_CAM_FORWARD,
        ROOM_CAM_RIGHT,
        ROOM_CAM_UP,
        ROOM_CAM_FOV_Y,
        COMPOSITE_W as f32 / COMPOSITE_H as f32,
        CSM_DEMO_NEAR,
        CSM_CASCADES_FAR,
        CSM_CASCADES_COUNT,
    )
}

/// **CSM Increment 3 (Rung B) — the N-cascade (multi-distance) showcase config.** Several ASYMMETRIC
/// caster boxes RECEDING into the distance down a LONG raster floor, a perspective room camera, ONE
/// directional sun, `cascade_count == 3`. The near boxes' shadows (cascade 0, dense) + the far boxes'
/// shadows (cascade 1/2) all render, with a SMOOTH transition at the cascade boundaries (the blend
/// band — no hard seam). NO SDF (the floor's `gMaterial.r == 1`, so `min(1, csm_vis) == csm_vis`).
fn csm_cascades_config() -> ShowcaseConfig {
    // ONE directional sun (the cascades are fit to it) + a dim sky for ambient fill.
    let header = GoldenLightHeader::new(2, 0, 1.0).with_ssao_mode(0);
    let lights = vec![
        GoldenLight::directional(SHOWCASE_SUN_DIR, [1.0, 0.97, 0.92], 3.0),
        GoldenLight::sky([0.08, 0.08, 0.10], [0.08, 0.08, 0.10]),
    ];

    // NO SDF: the marcher owns no surface; a single degenerate far edit keeps the edit-list valid.
    let sdf = vec![SdfEdit::sphere([0.0, -1000.0, 0.0], 0.01, sdf_op::UNION, 0.0)];

    // The CASTERS: asymmetric slabs receding down the floor (increasing -Z), so they fall into
    // cascades 0 (near), 1 (mid), 2 (far). A yaw makes each shadow's orientation read.
    let (slab_v, slab_i) = mesh_box_model([0.35, 0.9, 0.55], [0.82, 0.42, 0.30, 1.0]);
    let affines = vec![
        instance_affine(0.4, 1.0, [-1.0, 0.9, -1.5]), // near — cascade 0
        instance_affine(-0.3, 1.0, [0.8, 0.9, -5.0]), // mid  — cascade 1
        instance_affine(0.6, 1.0, [-0.6, 0.9, -10.0]), // far  — cascade 2
    ];

    // The raster FLOOR: a LONG flat slab spanning near→far so every cast shadow lands on it (mask=1).
    let (floor_v, floor_i) = mesh_box_model([5.0, 0.05, 12.0], ROOM_FLOOR_COLOR);
    let floor_affine = instance_affine(0.0, 1.0, [0.0, -0.05, -7.0]);

    ShowcaseConfig {
        sdf,
        camera: room_camera(),
        light_header: header,
        light_elems: lights,
        vertices: showcase_quad_vertices(),
        mvp: instanced_room_mvp_bytes(),
        ssao_quality: None,
        mesh_sdf: None,
        instanced: Some(InstancedMesh {
            meshes: vec![
                // The floor (batch 0, base 0) + the 3 receding casters (batch 1, nonzero base).
                InstancedMeshEntry { vertices: floor_v, indices: floor_i, affines: vec![floor_affine] },
                InstancedMeshEntry { vertices: slab_v, indices: slab_i, affines },
            ],
            non_casters: vec![],
        }),
        // CSM ON (Rung B): 3 cascades PSSM-fit over the floor's near→far range.
        csm: Some(csm_cascades_fit()),
        // This is the CSM (directional) cascades demo — the sparse SPOT path stays OFF here.
        spot_atlas: None,
    }
}

/// **CSM Increment 3 (Rung B) — the N-cascade hardware shadow screenshot.** Drives the cascade
/// DEPTH loop (the 3 receding boxes rendered from the sun POV into cascade layers 0/1/2) + the
/// resolve SELECT + blend, dumping a TRUE 512×512 BMP to [`CSM_CASCADES_BMP`] for the owner's RTX
/// visual sign-off (the deliverable: all three boxes' shadows render with a SMOOTH cascade
/// transition — no hard seam).
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the CSM cascades screenshot"]
fn engine_csm_cascades_512_screenshot_dump() {
    run_showcase_dump("boyko_engine CSM cascades 512", CSM_CASCADES_BMP, csm_cascades_config(), false);
}

/// **CSM Increment 3 (Rung B) — the cascade SELECT + BLEND host golden.** Pins the resolve's
/// `csm_visibility` control flow ([`csm_host_select_blend`], its mirror): a NEAR point picks cascade
/// 0, a MID point picks cascade 1, a FAR point picks cascade 2, the SELECT boundaries match
/// `split_far`, the band picks the blend (`band_t` ramps 0→1), and a point past the last split is
/// fully lit (not covered). The host mirror is the same arithmetic the GPU runs under the `csm_mode`
/// gate, so a drift would break the on-screen cascade transition.
#[test]
fn csm_cascade_select_blend_golden() {
    let fit = csm_cascades_fit();
    assert_eq!(fit.active_count as usize, CSM_CASCADES_COUNT);
    let s0 = fit.cascades[0].split_far;
    let s1 = fit.cascades[1].split_far;
    let s2 = fit.cascades[2].split_far;
    assert!(s0 < s1 && s1 < s2, "PSSM splits must be monotone: {s0} < {s1} < {s2}");

    // A NEAR receiver (view_z < s0, outside the band) selects cascade 0, no blend.
    let near_z = s0 * 0.5;
    let (sel, _next, band_t, covered) = csm_host_select_blend(&fit, near_z);
    assert!(covered, "a near point must be covered");
    assert_eq!(sel, 0, "view_z {near_z} (< split0 {s0}) must select cascade 0");
    assert_eq!(band_t, 0.0, "a near point outside the band must not blend");

    // A MID receiver between s0 and s1 (outside the band) selects cascade 1.
    let mid_z = s0 + (s1 - s0) * 0.5;
    let (sel, _n, band_mid, _c) = csm_host_select_blend(&fit, mid_z);
    assert_eq!(sel, 1, "view_z {mid_z} (split0 {s0}..split1 {s1}) must select cascade 1");
    assert_eq!(band_mid, 0.0, "the mid-cascade center must not blend");

    // A FAR receiver between s1 and s2 selects cascade 2 (the last cascade — never blends out).
    let far_z = s1 + (s2 - s1) * 0.5;
    let (sel, _n, band_far, _c) = csm_host_select_blend(&fit, far_z);
    assert_eq!(sel, 2, "view_z {far_z} (split1 {s1}..split2 {s2}) must select the last cascade 2");
    assert_eq!(band_far, 0.0, "the last cascade has no successor → no fade-out");

    // The SELECT boundary pin: a view_z JUST below split0 selects 0; just above selects 1.
    let (sel_lo, ..) = csm_host_select_blend(&fit, s0 - 1.0e-3);
    let (sel_hi, ..) = csm_host_select_blend(&fit, s0 + 1.0e-3);
    assert_eq!(sel_lo, 0, "just below split0 selects cascade 0");
    assert_eq!(sel_hi, 1, "just above split0 crosses to cascade 1");

    // The BLEND band: a view_z INSIDE cascade 0's trailing overlap band yields band_t in (0,1) and
    // blends toward cascade 1. At the band start band_t == 0; at split0 band_t == 1.
    let range0 = s0; // cascade 0's range is [0, s0]
    let band_start = s0 - CSM_OVERLAP_PROPORTION * range0;
    let (sel_b, next_b, band_mid_b, _c) =
        csm_host_select_blend(&fit, band_start + (s0 - band_start) * 0.5);
    assert_eq!(sel_b, 0, "the band still selects cascade 0");
    assert_eq!(next_b, 1, "the band blends toward cascade 1");
    assert!(
        (0.0..=1.0).contains(&band_mid_b) && band_mid_b > 0.0,
        "mid-band band_t must be in (0,1]: {band_mid_b}"
    );
    let (_s, _n, band_at_start, _c) = csm_host_select_blend(&fit, band_start + 1.0e-4);
    let (_s, _n, band_at_split, _c) = csm_host_select_blend(&fit, s0 - 1.0e-4);
    assert!(band_at_start < 0.05, "band_t ~0 at the band start: {band_at_start}");
    assert!(band_at_split > 0.95, "band_t ~1 at the split: {band_at_split}");

    // Past the last split → NOT covered (the resolve returns fully lit, no shadow data).
    let (_s, _n, _b, covered_far) = csm_host_select_blend(&fit, s2 + 5.0);
    assert!(!covered_far, "a point past the shadow distance must be uncovered (fully lit)");
}

// === Mesh foundation M3 — the multi-mesh batch-loop + nonzero-base + mixed-width demo. ===

/// A MODEL-SPACE box at the origin with per-axis half-extents `half`, each face SUBDIVIDED into
/// `seg × seg` quads — `(seg+1)² × 6` UNIQUE vertices + `seg² × 6 × 6` indices. Used to push the
/// unique-vertex count ABOVE [`U16_INDEX_VERTEX_LIMIT`] so the registry's O3 width pick mints
/// `Uint32` indices (the mixed-width proof): at `seg = 120` the count is `121² × 6 = 87 846 >
/// 65 536`. Visually a box; geometrically a high-poly mesh forcing the wide index path.
fn mesh_box_model_subdivided(half: [f32; 3], color: [f32; 4], seg: u32) -> (Vec<Vertex>, Vec<u32>) {
    assert!(seg >= 1, "a subdivided box face needs at least one segment");
    let [hx, hy, hz] = half;
    // One subdivided face: outward `normal`, the two in-plane axes (`u`, `v`) spanning
    // [-h, +h], and the fixed out-of-plane `center`. CCW from outside matches
    // `mesh_box_model`'s winding.
    type FaceSpec = ([f32; 3], [f32; 3], [f32; 3], [f32; 3]);
    let faces: [FaceSpec; 6] = [
        // +Z: u = +X, v = +Y, plane z = +hz.
        ([0.0, 0.0, 1.0], [hx, 0.0, 0.0], [0.0, hy, 0.0], [0.0, 0.0, hz]),
        // -Z: u = -X, v = +Y, plane z = -hz.
        ([0.0, 0.0, -1.0], [-hx, 0.0, 0.0], [0.0, hy, 0.0], [0.0, 0.0, -hz]),
        // +X: u = -Z, v = +Y, plane x = +hx.
        ([1.0, 0.0, 0.0], [0.0, 0.0, -hz], [0.0, hy, 0.0], [hx, 0.0, 0.0]),
        // -X: u = +Z, v = +Y, plane x = -hx.
        ([-1.0, 0.0, 0.0], [0.0, 0.0, hz], [0.0, hy, 0.0], [-hx, 0.0, 0.0]),
        // +Y: u = +X, v = -Z, plane y = +hy.
        ([0.0, 1.0, 0.0], [hx, 0.0, 0.0], [0.0, 0.0, -hz], [0.0, hy, 0.0]),
        // -Y: u = +X, v = +Z, plane y = -hy.
        ([0.0, -1.0, 0.0], [hx, 0.0, 0.0], [0.0, 0.0, hz], [0.0, -hy, 0.0]),
    ];

    let row = seg + 1;
    let per_face = (row * row) as usize;
    let mut verts: Vec<Vertex> = Vec::with_capacity(per_face * 6);
    let mut indices: Vec<u32> = Vec::with_capacity((seg * seg * 6 * 6) as usize);
    for (n, u, v, c) in faces {
        let face_base = verts.len() as u32;
        for iy in 0..row {
            for ix in 0..row {
                // Barycentric in [-1, +1] along each in-plane axis.
                let su = (ix as f32 / seg as f32) * 2.0 - 1.0;
                let sv = (iy as f32 / seg as f32) * 2.0 - 1.0;
                let p = [
                    c[0] + u[0] * su + v[0] * sv,
                    c[1] + u[1] * su + v[1] * sv,
                    c[2] + u[2] * su + v[2] * sv,
                ];
                verts.push(Vertex { position: p, normal: n, color });
            }
        }
        for iy in 0..seg {
            for ix in 0..seg {
                let a = face_base + iy * row + ix;
                let b = a + 1;
                let cc = a + row;
                let d = cc + 1;
                // Two CCW triangles per quad: (a, b, d) + (a, d, cc).
                indices.extend_from_slice(&[a, b, d, a, d, cc]);
            }
        }
    }
    (verts, indices)
}

/// **Mesh foundation M3 — the multi-mesh instanced config.** TWO distinct registered meshes
/// drawn through the recorder's BATCH LOOP under the [`room_camera`] perspective:
///
///   * **Mesh A** — a small model-space box ([`mesh_box_model`], 24 unique verts ⇒ O3
///     `Uint16` indices), 3 instances at `base_instance == 0`.
///   * **Mesh B** — a SUBDIVIDED box ([`mesh_box_model_subdivided`], `seg = 120` ⇒ 87 846
///     unique verts ⇒ O3 `Uint32` indices — the mixed-width proof), 2 instances at a NONZERO
///     `base_instance` (== mesh A's instance count, 3 — the C1 GPU proof).
///
/// The two meshes are placed at DISTINCT world X/Z/yaw so per-instance + per-mesh placement is
/// visible. If mesh B's instances render at mesh A's positions, the recorder ignored
/// `base_instance` and the C1 mechanism is broken — the screenshot will show it. Co-scened with
/// an SDF sphere (the C2 depth-ownership backdrop).
fn multimesh_persp_config() -> ShowcaseConfig {
    let header = GoldenLightHeader::new(2, 0, 1.0).with_ssao_mode(0);
    let lights = vec![
        GoldenLight::directional(SHOWCASE_SUN_DIR, [1.0, 0.97, 0.92], 3.0),
        GoldenLight::sky([0.05, 0.05, 0.05], [0.05, 0.05, 0.05]),
    ];
    let sdf = vec![SdfEdit::sphere([1.6, 0.6, -0.6], 0.6, sdf_op::UNION, 0.0)];

    // Mesh A: a small u16-indexed box (warm orange). THREE instances on the LEFT.
    let (a_verts, a_indices) = mesh_box_model([0.4, 0.4, 0.4], [0.82, 0.45, 0.30, 1.0]);
    let a_affines = vec![
        instance_affine(0.0, 1.0, [-2.0, 0.42, -0.6]),
        instance_affine(0.5, 0.85, [-1.6, 0.36, -1.8]),
        instance_affine(-0.4, 1.1, [-2.2, 0.50, -3.0]),
    ];

    // Mesh B: a high-poly SUBDIVIDED box (cool teal) — `seg = 120` forces O3 Uint32 indices.
    // TWO instances on the RIGHT at a NONZERO `base_instance` (== mesh A's 3 instances).
    let (b_verts, b_indices) =
        mesh_box_model_subdivided([0.45, 0.45, 0.45], [0.25, 0.62, 0.70, 1.0], 120);
    let b_affines = vec![
        instance_affine(0.3, 1.05, [0.4, 0.45, -1.4]),
        instance_affine(-0.6, 0.9, [0.9, 0.40, -2.8]),
    ];

    ShowcaseConfig {
        sdf,
        camera: room_camera(),
        light_header: header,
        light_elems: lights,
        vertices: showcase_quad_vertices(),
        mvp: instanced_room_mvp_bytes(),
        ssao_quality: None,
        mesh_sdf: None,
        instanced: Some(InstancedMesh {
            meshes: vec![
                InstancedMeshEntry { vertices: a_verts, indices: a_indices, affines: a_affines },
                InstancedMeshEntry { vertices: b_verts, indices: b_indices, affines: b_affines },
            ],
            non_casters: vec![],
        }),
        // CSM Increment 1b: OFF for this showcase (the 0%-gate — no cascade depth pass).
        csm: None,
        // Shadow Phase 5 Inc-1-GPU: no sparse spot shadow (the 0%-gate — no punctual depth pass,
        // `punctual_shadow_mode == 0`).
        spot_atlas: None,
    }
}

/// **Mesh foundation M3 — the multi-mesh batch-loop screenshot.** Drives TWO distinct meshes
/// (a u16-indexed box + a u32-indexed subdivided box) through the recorder's batch loop: mesh A
/// at `base_instance == 0`, mesh B at a NONZERO base — the C1 GPU proof — with mixed index width
/// (O3). Dumps a TRUE 512×512 BMP to [`MULTIMESH_PERSP_BMP`] for the owner's RTX sign-off.
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the multi-mesh screenshot"]
fn engine_multimesh_persp_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine multi-mesh perspective 512",
        MULTIMESH_PERSP_BMP,
        multimesh_persp_config(),
        false,
    );
}

// === Mesh foundation M4 — the non-uniform-scale normal demo (inverse-transpose vs mul(m3)). ===

/// **Mesh foundation M4 — the NON-UNIFORM-scale normal config.** ONE registered model-space box
/// drawn through the `use_model_matrix == 1` instanced arm by affines with deliberately
/// NON-UNIFORM per-axis scale: a box squashed flat (wide + thin), a box stretched tall, and a
/// wide-thin slab — placed under a single directional sun so the lit shading on the
/// stretched/squashed faces is the M4 visual proof. With the M4 inverse-transpose normal the
/// faces shade correctly (the normals stay perpendicular to the deformed surface); with the old
/// `mul(m3, normal)` the same faces are over-bright / wrongly lit because the skewed normals
/// tilt toward/away from the sun. Co-scened with an SDF sphere (the C2 depth backdrop).
///
/// The `vertices` legacy field is the degenerate zero-area mesh (the recorder draws the instanced
/// mesh instead). `lighting_flags` SHADOWS|AO is set by the shared body.
fn nonuniform_normals_config() -> ShowcaseConfig {
    // One directional sun (raking, so the per-face normal genuinely modulates the shading) + a
    // dim sky fill. A grazing sun makes the inverse-transpose vs mul(m3) difference pronounced.
    let header = GoldenLightHeader::new(2, 0, 1.0).with_ssao_mode(0);
    let lights = vec![
        GoldenLight::directional(SHOWCASE_SUN_DIR, [1.0, 0.97, 0.92], 3.0),
        GoldenLight::sky([0.05, 0.05, 0.05], [0.05, 0.05, 0.05]),
    ];

    // The C2 depth backdrop: a hero SDF sphere resting on the floor.
    let sdf = vec![SdfEdit::sphere([1.7, 0.6, -0.6], 0.6, sdf_op::UNION, 0.0)];

    // The model-space unit box (half-extent 0.4) registered ONCE; the non-uniform affines below
    // squash/stretch it per-axis so the inverse-transpose normal correction is exercised.
    let (verts, indices) = mesh_box_model([0.4, 0.4, 0.4], [0.78, 0.50, 0.32, 1.0]);

    // THREE non-uniform placements (the y of `t` lifts each so its base rests near the floor):
    //   * a SQUASHED slab — wide in X, thin in Y, mid in Z (a flattened pancake).
    //   * a STRETCHED column — thin in X/Z, tall in Y (a pillar).
    //   * a wide-thin SHARD — very wide in X, thin in Z, mid in Y, yawed so a deformed face
    //     rakes the sun.
    let affines = vec![
        instance_affine_nonuniform(0.0, [2.4, 0.35, 1.3], [-1.9, 0.14, -0.8]),
        instance_affine_nonuniform(0.4, [0.40, 2.6, 0.40], [-0.5, 1.04, -1.9]),
        instance_affine_nonuniform(0.8, [2.2, 0.45, 0.35], [0.8, 0.18, -3.0]),
    ];

    ShowcaseConfig {
        sdf,
        camera: room_camera(),
        light_header: header,
        light_elems: lights,
        vertices: showcase_quad_vertices(),
        mvp: instanced_room_mvp_bytes(),
        ssao_quality: None,
        mesh_sdf: None,
        instanced: Some(InstancedMesh {
            meshes: vec![InstancedMeshEntry { vertices: verts, indices, affines }],
            non_casters: vec![],
        }),
        // CSM Increment 1b: OFF for this showcase (the 0%-gate — no cascade depth pass).
        csm: None,
        // Shadow Phase 5 Inc-1-GPU: no sparse spot shadow (the 0%-gate — no punctual depth pass,
        // `punctual_shadow_mode == 0`).
        spot_atlas: None,
    }
}

/// **Mesh foundation M4 — the non-uniform-scale normal screenshot.** Drives the instanced arm
/// (`use_model_matrix == 1`) with NON-UNIFORM-scale affines so the M4 inverse-transpose normal
/// path is exercised end-to-end on the GPU: squashed + stretched boxes under a raking directional
/// sun, where the inverse-transpose-vs-`mul(m3)` shading difference is visible on the deformed
/// faces. Dumps a TRUE 512×512 BMP to [`NONUNIFORM_NORMALS_BMP`] for the owner's RTX sign-off.
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the non-uniform-normals screenshot"]
fn engine_nonuniform_normals_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine non-uniform normals 512",
        NONUNIFORM_NORMALS_BMP,
        nonuniform_normals_config(),
        false,
    );
}

// === Render Shadow Phase 1 — the capsule-character proxy demo. ===

/// The capsule cap radius for the character limbs (a coarse humanoid; the torso uses a
/// thicker radius below). Small enough that 6 capsules read as a stick-figure silhouette.
const CHAR_LIMB_RADIUS: f32 = 0.09;
/// The torso cap radius — thicker than the limbs so the body reads as a trunk.
const CHAR_TORSO_RADIUS: f32 = 0.16;
/// The head cap radius.
const CHAR_HEAD_RADIUS: f32 = 0.17;

/// A COARSE 6-capsule humanoid character proxy standing on the floor (feet at y = 0),
/// rooted at world `root` (the feet midpoint), scaled to total `height`, facing the
/// `facing` heading (radians about +Y; 0 faces +Z toward the camera). An ASYMMETRIC pose
/// — one leg forward / one back, one arm out / one down — so the cast SDF shadow reads
/// unmistakably as a humanoid rather than a blob.
///
/// The 6 capsules: torso (hip→shoulder), head (neck→crown), left+right legs (hip→foot),
/// left+right arms (shoulder→hand). HARD unions, smoothness 0.0 (a crisp silhouette).
/// `≤ MAX_SDF_EDITS` (6 edits) with room to spare.
fn character_capsules(root: [f32; 3], height: f32, facing: f32) -> Vec<SdfEdit> {
    // Proportions as fractions of `height` (a coarse 7.5-head canon, simplified).
    let hip_y = 0.50 * height; // pelvis
    let shoulder_y = 0.82 * height;
    let neck_y = 0.84 * height;
    let crown_y = 1.00 * height;
    let foot_y = 0.0; // on the floor
    let hand_y = 0.42 * height;

    // The facing basis in the xz-plane: `fwd` is the heading, `side` is its right-hand
    // perpendicular. The asymmetric pose offsets are expressed in (side, fwd) and rotated
    // into world xz so the whole figure turns with `facing`.
    let (s, c) = facing.sin_cos();
    // fwd = (sin, cos), side = (cos, -sin)  (right-handed about +Y, 0 -> +Z).
    let place = |side: f32, fwd: f32| -> [f32; 2] { [side * c + fwd * s, -side * s + fwd * c] };

    // Lateral half-stance (hips/shoulders), the forward/back leg split, and the arm reach.
    let hip_dx = 0.10 * height;
    let shoulder_dx = 0.17 * height;
    let leg_fwd = 0.14 * height; // right leg forward, left leg back (asymmetric stride)
    let arm_out = 0.26 * height; // right arm raised out to the side; left arm hangs down

    let p = |side: f32, fwd: f32, y: f32| -> [f32; 3] {
        let xz = place(side, fwd);
        [root[0] + xz[0], root[1] + y, root[2] + xz[1]]
    };

    // Hips and shoulders.
    let hip_l = p(-hip_dx, 0.0, hip_y);
    let hip_r = p(hip_dx, 0.0, hip_y);
    let hip_c = p(0.0, 0.0, hip_y);
    let sh_l = p(-shoulder_dx, 0.0, shoulder_y);
    let sh_r = p(shoulder_dx, 0.0, shoulder_y);

    vec![
        // Torso: hip center -> shoulder center (thick).
        SdfEdit::capsule(hip_c, p(0.0, 0.0, shoulder_y), CHAR_TORSO_RADIUS, sdf_op::UNION, 0.0),
        // Head: neck -> crown.
        SdfEdit::capsule(p(0.0, 0.0, neck_y), p(0.0, 0.0, crown_y), CHAR_HEAD_RADIUS, sdf_op::UNION, 0.0),
        // Right leg: hip -> foot, planted FORWARD (the asymmetric stride).
        SdfEdit::capsule(hip_r, p(hip_dx, leg_fwd, foot_y), CHAR_LIMB_RADIUS, sdf_op::UNION, 0.0),
        // Left leg: hip -> foot, planted BACK.
        SdfEdit::capsule(hip_l, p(-hip_dx, -leg_fwd, foot_y), CHAR_LIMB_RADIUS, sdf_op::UNION, 0.0),
        // Right arm: shoulder -> hand, raised OUT to the side (reads as a wave).
        SdfEdit::capsule(sh_r, p(shoulder_dx + arm_out, 0.0, shoulder_y + 0.06 * height), CHAR_LIMB_RADIUS, sdf_op::UNION, 0.0),
        // Left arm: shoulder -> hand, hanging DOWN.
        SdfEdit::capsule(sh_l, p(-shoulder_dx, 0.04 * height, hand_y), CHAR_LIMB_RADIUS, sdf_op::UNION, 0.0),
    ]
}

/// The capsule-character backdrop mesh: just the FLOOR quad (y = 0) + the BACK-WALL quad
/// (z = -4), NO cubes — so the SDF scene is the 6 character capsules ALONE (no shadow
/// proxies needed) and the humanoid shadow falls on a clean floor/wall. Reuses the proven
/// `hybrid_room_mesh` floor/wall geometry (the cubes are intentionally dropped).
fn capsule_character_mesh() -> Vec<Vertex> {
    let mut verts = Vec::new();
    // Floor: y = 0, outward normal +Y (CCW from above), spanning x[-3,3] z[-4,1].
    verts.extend_from_slice(&mesh_quad(
        [[-3.0, 0.0, 1.0], [3.0, 0.0, 1.0], [3.0, 0.0, -4.0], [-3.0, 0.0, -4.0]],
        [0.0, 1.0, 0.0],
        ROOM_FLOOR_COLOR,
    ));
    // Back wall: z = -4, outward normal +Z, spanning x[-3,3] y[0,4].
    verts.extend_from_slice(&mesh_quad(
        [[-3.0, 0.0, -4.0], [3.0, 0.0, -4.0], [3.0, 4.0, -4.0], [-3.0, 4.0, -4.0]],
        [0.0, 0.0, 1.0],
        ROOM_WALL_COLOR,
    ));
    verts
}

/// The capsule-character showcase config (Render Shadow Phase 1): the 6-capsule humanoid
/// proxy standing on the mesh floor under the [`room_camera`] perspective, lit by the
/// showcase sun + a dim sky, casting an analytic SDF shadow onto the floor/wall mesh.
/// Analytic path (`ssao_quality: None`); HARD unions (smoothness 0.0). The 6 capsules are
/// ≤ MAX_SDF_EDITS with room to spare (no cube proxies — the figure stands on a clean floor).
///
/// `contact_shadow` arms Render Shadow Phase 3's Screen-Space Contact Shadows (`with_contact_
/// shadow_mode` — header word 7 bit 1). `false` is the byte-identical 0%-gate (the SSCS march
/// block never runs); `true` tightens the shadow where the feet meet the floor.
fn capsule_character_config(contact_shadow: bool) -> ShowcaseConfig {
    let header = GoldenLightHeader::new(2, 0, 1.0)
        .with_ssao_mode(0)
        .with_contact_shadow_mode(contact_shadow);
    let lights = vec![
        GoldenLight::directional(SHOWCASE_SUN_DIR, [1.0, 0.97, 0.92], 3.0),
        GoldenLight::sky([0.05, 0.05, 0.05], [0.05, 0.05, 0.05]),
    ];
    // The humanoid stands a little behind the room center, facing the camera (+Z) so its
    // front is lit and the shadow rakes back/aside onto the floor and wall.
    let character = character_capsules([0.0, 0.0, -1.0], 1.8, 0.0);
    debug_assert!(
        character.len() <= boyko_sdf_math::MAX_SDF_EDITS,
        "invariant: the character must fit the edit-list budget"
    );
    ShowcaseConfig {
        sdf: character,
        camera: room_camera(),
        light_header: header,
        light_elems: lights,
        vertices: capsule_character_mesh(),
        mvp: perspective_mvp_bytes(
            ROOM_CAM_EYE,
            ROOM_CAM_FORWARD,
            ROOM_CAM_RIGHT,
            ROOM_CAM_UP,
            ROOM_CAM_FOV_Y,
            COMPOSITE_W as f32 / COMPOSITE_H as f32,
        ),
        ssao_quality: None,
        mesh_sdf: None,
        // M2: no instanced mesh — the legacy merged draw runs (byte-identical).
        instanced: None,
        // CSM Increment 1b: OFF (the 0%-gate — no cascade depth pass, `csm_mode == 0`).
        csm: None,
        // Shadow Phase 5 Inc-1-GPU: no sparse spot shadow (the 0%-gate — no punctual depth pass,
        // `punctual_shadow_mode == 0`).
        spot_atlas: None,
    }
}

/// **Render Shadow Phase 1 — the capsule-character screenshot dump (the visual oracle).**
/// Renders the 6-capsule humanoid proxy ([`character_capsules`]) standing on the mesh
/// floor+wall ([`capsule_character_mesh`]) under the [`room_camera`] perspective, lit by the
/// showcase sun + a dim sky, and dumps a TRUE 512×512 24-bit BMP to [`CAPSULE_CHARACTER_BMP`].
/// The deliverable is the cast SDF shadow reading as a humanoid silhouette on the floor.
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1` so the
/// (broken-on-this-box) validation layer does not crash the process; the screenshot is the
/// deliverable, not a golden assertion.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the capsule-character screenshot"]
fn engine_capsule_character_512_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine capsule character 512",
        CAPSULE_CHARACTER_BMP,
        capsule_character_config(false),
        false,
    );
}

/// **Render Shadow Phase 3 — Screen-Space Contact Shadows A/B screenshot dump (the visual
/// oracle).** The SAME capsule character feet-on-floor scene as
/// [`engine_capsule_character_512_screenshot_dump`], rendered TWICE: once with
/// `contact_shadow_mode` OFF (dumped to [`CONTACT_SHADOW_OFF_BMP`] — the A/B reference AND the
/// 0%-gate visual proof, since the SSCS march block is structurally skipped) and once with it ON
/// (dumped to [`CONTACT_SHADOW_ON_BMP`] — the contact-shadow tightening where the feet meet the
/// floor). Two windowed renders (each boots + tears down its own device).
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1` so the
/// (broken-on-this-box) validation layer does not crash the process; the screenshots are the
/// deliverable, not a golden assertion.
/// `#[ignore]`: needs a real RTX windowed device. SPLIT into two ONE-render-per-process tests —
/// a second windowed render in the same process trips the swapchain-recreate path and never dumps.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator dumps the contact-shadow OFF screenshot"]
fn engine_contact_shadow_off_512_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine contact shadow OFF 512",
        CONTACT_SHADOW_OFF_BMP,
        capsule_character_config(false),
        false,
    );
}

#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator dumps the contact-shadow ON screenshot"]
fn engine_contact_shadow_on_512_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine contact shadow ON 512",
        CONTACT_SHADOW_ON_BMP,
        capsule_character_config(true),
        false,
    );
}

/// The MDF Stage-2c showcase config (ORTHO mesh-floor frame): the raster mesh-floor quad + a
/// PROCEDURAL TORUS standing in FRONT of it (between the floor at `MESH_Z == 1.0` and the camera at
/// `SDF_CAMERA_Z == 2.0`), both RASTER-rendered, with the torus baked into the dedicated dense
/// `MeshSdfTexture` and casting its MDF soft shadow onto the floor (the marcher's
/// `sdf_soft_shadow_mesh` over the uploaded grid). NO SDF edits (the field is empty — the floor +
/// torus are raster, and the SHADOW comes purely from the mesh SDF texture). `mesh_sdf_enabled` is
/// armed by `run_showcase_dump` because `mesh_sdf` is `Some`.
fn mdf_shadow_config() -> ShowcaseConfig {
    // The shared raking upper-left sun (`SHOWCASE_SUN_DIR`, low +z) — `marcher_light_dir` feeds the
    // SAME vector to the marcher push, so the cast shadow direction matches the lit direction. A
    // raking (not head-on) sun is ESSENTIAL here: the torus faces the ORTHO camera and the floor is
    // parallel to the image plane, so a +z-dominant (near-camera) light would cast the shadow almost
    // straight BEHIND the torus — hidden by the torus's own silhouette ("flash photography"). The low
    // +z (0.36) casts a LONG shadow offset down-and-right; the torus is parked HIGH-LEFT (below) so
    // that long shadow lands fully on the open lower-right floor, clear of the torus.
    let (light_header, light_elems) = showcase_light_table();

    // The torus: parked HIGH-LEFT, axis +Z (the ring faces the ORTHO camera). Major radius 0.35
    // (the ring), minor 0.14 (the tube — ~3.5 voxels across at the 0.04 bake voxel, well-resolved);
    // 32 rings × 16 sides is smooth. Its raking-sun shadow casts down-right onto the open floor.
    let torus_center = [-0.2, 0.5, 1.45];
    let (torus_verts, torus_pos, torus_idx) =
        torus_mesh(torus_center, 0.35, 0.14, 32, 16, [0.85, 0.55, 0.20, 1.0]);

    // A FULL-FRAME floor wall at MESH_Z. The ORTHO view maps world [-1, 1] → screen [0, 512] on both
    // axes, so a [-1.05, 1.05] quad overscans the frame edges — wherever the down-left cast shadow
    // lands, it falls on LIT floor (the shared `quad_vertices` floor stops at x = 0.2, short of the
    // shadow). White so the cool cast shadow is high-contrast.
    let floor = mesh_quad(
        [
            [-1.05, -1.05, MESH_Z],
            [1.05, -1.05, MESH_Z],
            [1.05, 1.05, MESH_Z],
            [-1.05, 1.05, MESH_Z],
        ],
        [0.0, 0.0, 1.0],
        [1.0, 1.0, 1.0, 1.0],
    );
    // The raster geometry: the full-frame floor (faces +Z at MESH_Z) + the torus, drawn as ONE mesh.
    let mut vertices = floor.to_vec();
    vertices.extend_from_slice(&torus_verts);

    ShowcaseConfig {
        // No SDF edits — the scene is pure mesh (floor + torus raster) + the MDF shadow caster.
        sdf: Vec::new(),
        camera: CompositePushConstants::ortho(COMPOSITE_W, COMPOSITE_H),
        // SSAO OFF (the deliverable is the cast MDF shadow, not ambient occlusion).
        light_header: light_header.with_ssao_mode(0),
        light_elems,
        vertices,
        mvp: ortho_mvp_bytes(),
        ssao_quality: None,
        // The MDF caster geometry the baker turns into the dense `MeshSdfTexture` — the SAME torus
        // the raster draw renders (the visible mesh is its own invisible shadow proxy).
        mesh_sdf: Some((torus_pos, torus_idx)),
        // M2: no instanced mesh — the legacy merged draw runs (byte-identical).
        instanced: None,
        // CSM Increment 1b: OFF (the 0%-gate — no cascade depth pass, `csm_mode == 0`).
        csm: None,
        // Shadow Phase 5 Inc-1-GPU: no sparse spot shadow (the 0%-gate — no punctual depth pass,
        // `punctual_shadow_mode == 0`).
        spot_atlas: None,
    }
}

/// **MDF Stage-2c — the mesh-distance-field shadow screenshot dump (the visual oracle).** Renders
/// the ORTHO mesh floor + a raster TORUS standing in front of it ([`mdf_shadow_config`]), with the
/// torus baked into the dedicated dense `MeshSdfTexture` and casting its baked MDF SOFT SHADOW onto
/// the floor (the marcher's `sdf_soft_shadow_mesh` over the uploaded grid, `mesh_sdf_enabled`). The
/// deliverable is the torus's ring+hole shadow reading on the floor — proof the dense mesh-SDF
/// upload + the marcher's mesh-aware shadow march work end-to-end.
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1` so the
/// (broken-on-this-box) validation layer does not crash the process; the screenshot is the
/// deliverable, not a golden assertion.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the MDF-shadow screenshot"]
fn engine_mdf_shadow_512_screenshot_dump() {
    run_showcase_dump("boyko_engine MDF shadow 512", MDF_SHADOW_BMP, mdf_shadow_config(), false);
}

/// The shared 512×512-native multi-light SDF-shadow + SSAO showcase dump body. `window_title` is
/// the window caption; `bmp_path` is the TRUE 512×512 24-bit BMP destination (no upscale); `cfg`
/// supplies the variable scene (SDF edits, camera, light table, raster mesh + MVP). SSAO is ON (the
/// `cfg` builder arms `ssao_mode == 1`; `scene.ssao = Some(..)` records the pass that writes it).
fn run_showcase_dump(window_title: &str, bmp_path: &str, cfg: ShowcaseConfig, interactive: bool) {
    with_windowed_present(window_title, "engine_showcase_512", |bp| {
        run_showcase_body(bp, bmp_path, cfg, interactive)
    });
}

/// SDFDDGI I4 — the GI-ON variant of [`run_showcase_body`]. It shares the whole GI-OFF golden setup
/// VERBATIM (so the byte-identical golden body is untouched) and layers on the deltas that light up
/// dynamic diffuse GI: the light-header word-7 bit-4 injection gate ([`GoldenLightHeader::with_ddgi_mode`]),
/// a snugly-fit probe grid reseeded into BOTH the resolve grid UBO (`csm.ddgi_ubo`) and a dedicated
/// update UBO, the live probe-update compute pass ([`DdgiUpdateActivation`], its ray table + params
/// UBO + pipeline + layout), and a [`DDGI_CONVERGE_FRAMES`]-frame convergence loop (rewriting
/// `frame_index` each frame for the I4 ray rotation) before the single readback frame. Only ever
/// called with `interactive == false` (the readback dump path); the interactive viewer branch is
/// intentionally absent.
///
/// The resolve injects GI ONLY on `is_sdf_lit` pixels — in [`grand_showcase_config`] that is the two
/// SDF spheres — so the converged sun-driven indirect bounce reads there and nowhere else (the mesh
/// walls/floor/boxes are `mask == 0`). The update shader shades DIRECTIONAL lights only, so the warm
/// sun is the GI driver.
fn run_showcase_body_ddgi(
    bp: BootPresent<'_, '_>,
    bmp_path: &str,
    cfg: ShowcaseConfig,
    interactive: bool,
    gpu_timing: Option<&TimestampCollector>,
) {
    let BootPresent { window, ctx, surface, mut swapchain, mut renderer, is_bgra, swap_color_format } =
        bp;
    debug_assert!(!interactive, "invariant: run_showcase_body_ddgi is a readback-only dump path");

    let device: &VulkanContext = ctx;
    let sdf = &cfg.sdf;

    // SDFDDGI I4 grid fit — snugly encloses the grand_showcase room (X∈[-4.5,4.5], Y∈[0,5.2],
    // Z∈[-5,3.5]) and both SDF spheres. 16×8×16 probes, uniform spacing 0.7 → AABB X∈[-5.25,5.25],
    // Y∈[0.2,5.1], Z∈[-6,4.5].
    const DDGI_ORIGIN: [f32; 3] = [-5.25, 0.20, -6.00];
    const DDGI_SPACING: f32 = 0.70;
    // dims are boyko_rhi_vulkan::ddgi::{DDGI_GRID_DIM_X/Y/Z} = [16,8,16].
    const DDGI_HYSTERESIS: f32 = 0.9; // static-capture α (lower than the 0.95 runtime default → converges faster).
    const DDGI_CONVERGE_FRAMES: u32 = 160; // 0.9^160 ≈ 1e-7; ramps from the boot-zero atlas.
    // The update UBO's `light_count` — captured before `cfg`'s fields are consumed below (the moves
    // of `cfg.vertices`/`cfg.mvp` and the borrows of `cfg.light_elems` would otherwise clash with a
    // late read at the update-UBO seed site).
    let ddgi_light_count = cfg.light_elems.len() as u32;

    // --- The edit-list SSBO (binding 0), host-seeded ONCE. The resolve binds the SAME buffer
    // at binding 10 for the per-caster shadow march. ---
    let edit_list = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, sdf);
        let mapped = RhiDevice::buffer_mapped_ptr(device, &edit_list)
            .expect("host-visible edit-list buffer is mapped");
        write_words(mapped, &header);
    }

    let mesh_sdf_texture: Option<MeshSdfTexture> = cfg.mesh_sdf.as_ref().map(|(pos, idx)| {
        let mesh = BakeMesh::new(pos, idx);
        let field = MeshSdfField::for_mesh(&mesh, 0.04);
        MeshSdfTexture::create(ctx, &mesh, &field).expect("MDF mesh-SDF texture create + upload")
    });
    let mesh_sdf_enabled = mesh_sdf_texture.is_some();

    let ubo_bytes = if mesh_sdf_enabled {
        B5_CAMERA_UBO_BYTES_MESH_SDF
    } else {
        B5_CAMERA_UBO_BYTES_M4
    };
    let camera_ring: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
        RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: ubo_bytes as u64,
                usage: BufferUsage::UNIFORM,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("camera uniform buffer")
    });
    {
        let pc = &cfg.camera;
        assert_eq!(pc.count, PIXELS);
        let bytes = pc.as_bytes();
        debug_assert_eq!(bytes.len(), M2_GRID_PARAMS_OFFSET, "camera block must be 80 B");
        let mesh_sdf_params = mesh_sdf_texture
            .as_ref()
            .map(|tex| MeshSdfParams::from_field(tex.field()));
        for slot in &camera_ring {
            let mapped = RhiDevice::buffer_mapped_ptr(device, slot)
                .expect("host-visible uniform buffer is mapped");
            // SAFETY: `mapped` points to `ubo_bytes` (224 or 256) mapped host-coherent bytes; the
            // 80-byte camera block is written at offset 0. No GPU work is in flight yet, so the host
            // write is unsynchronized-safe. Every ring slot is seeded with the SAME bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
            }
            if let Some(params) = mesh_sdf_params.as_ref() {
                let pbytes = params.as_bytes();
                debug_assert_eq!(MESH_SDF_PARAMS_OFFSET + pbytes.len(), ubo_bytes);
                // SAFETY: the buffer is `ubo_bytes` (256) here; the 32-byte block is written at
                // `MESH_SDF_PARAMS_OFFSET` (224), entirely within the mapped range; unique host writer
                // before any GPU work. Every ring slot receives the SAME tail bytes.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        pbytes.as_ptr(),
                        mapped.as_ptr().add(MESH_SDF_PARAMS_OFFSET),
                        pbytes.len(),
                    );
                }
            }
        }
    }

    let (tw, th) = tile_grid_extent(COMPOSITE_W, COMPOSITE_H);
    let tiles_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (tw as u64) * (th as u64) * (TILE_BOUND_BYTES as u64),
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("P4b coarse-cull tile-bound storage buffer (vocab binding 6)");

    let mat_table = showcase_material_table();
    let material_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (mat_table.len() as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("PBR material table storage buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &material_table)
            .expect("host-visible material table is mapped");
        write_words(mapped, &mat_table);
    }

    let field = {
        use boyko_sdf_math::SdfEditField;
        let mut f = SdfEditField::new();
        for e in sdf {
            assert!(f.push(*e), "showcase scene must fit MAX_SDF_EDITS");
        }
        f.bump_gen();
        f
    };
    let clipmap = BrickClipmap::create(ctx, &field, [0.0, 0.0, 0.0])
        .expect("brick clip-map (showcase scene) — create + bake + upload");

    // --- The light table SSBO. SDFDDGI I4: `.with_ddgi_mode(true)` sets the resolve's GI-injection
    // gate (header word-7 bit 4), the ONLY header change vs the GI-OFF golden. ---
    let light_header = cfg
        .light_header
        .with_csm_mode(cfg.csm.is_some())
        .with_punctual_shadow_mode(cfg.spot_atlas.is_some())
        .with_ddgi_mode(true);
    let light_elems = &cfg.light_elems;
    let light_words = pack_showcase_light_table(&light_header, light_elems);
    let light_table_bytes = (light_words.len() as u64) * 4;
    let light_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: light_table_bytes,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("showcase light table storage buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_table)
            .expect("host-visible light table is mapped");
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
    .expect("showcase light table staging buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_staging)
            .expect("host-visible light staging is mapped");
        write_words(mapped, &light_words);
    }

    // --- The mesh's vertex buffer (the showcase floor / hybrid-room geometry). ---
    let vertices = cfg.vertices;
    let vertex_bytes = core::mem::size_of_val(vertices.as_slice()) as u64;
    let vertex_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible vertex buffer");
    {
        let vb_ptr = RhiDevice::buffer_mapped_ptr(device, &vertex_buffer)
            .expect("host-visible vertex buffer is mapped");
        // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes; `vertices`'s heap
        // buffer is a distinct `vertex_bytes`-byte region (`vertex_bytes == len * stride`); the
        // write completes before any submit.
        unsafe {
            core::ptr::copy_nonoverlapping(
                vertices.as_ptr().cast::<u8>(),
                vb_ptr.as_ptr(),
                vertex_bytes as usize,
            );
        }
    }

    let depth_sampler = RhiDevice::create_sampler(device, &SamplerDesc::default())
        .expect("depth sampler (ignored by .Load)");
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
    .expect("present nearest/clamp sampler");

    // --- The mesh-MRT G-buffer producer graphics pipeline (Render P5-r0). ---
    let vs = RhiDevice::create_shader_module(device, MRT_VS_SPV.as_words())
        .expect("mesh-MRT vertex shader module");
    let fs = RhiDevice::create_shader_module(device, MRT_FS_SPV.as_words())
        .expect("mesh-MRT fragment shader module");
    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
    ];
    let (instance_layout, instance_buffer, instance_bind_group) = create_identity_instance(device);

    let instanced_gpu: Option<InstancedGpu> = cfg.instanced.as_ref().map(|inst| {
        let mut ring: Vec<[f32; 12]> = Vec::new();
        let mut batches: Vec<InstancedGpuBatch> = Vec::with_capacity(inst.meshes.len());
        for (batch_idx, entry) in inst.meshes.iter().enumerate() {
            let base_instance = ring.len() as u32;
            ring.extend_from_slice(&entry.affines);

            let vbytes = core::mem::size_of_val(entry.vertices.as_slice()) as u64;
            let mvb = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: vbytes,
                    usage: BufferUsage::VERTEX,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("M3 instanced mesh vertex buffer");
            {
                let p = RhiDevice::buffer_mapped_ptr(device, &mvb)
                    .expect("host-visible instanced vertex buffer is mapped");
                // SAFETY: `p` points to `vbytes` mapped host-coherent bytes; `entry.vertices` is
                // a distinct `vbytes`-byte slice; the copy completes before any submit.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        entry.vertices.as_ptr().cast::<u8>(),
                        p.as_ptr(),
                        vbytes as usize,
                    );
                }
            }
            let index_type = if entry.vertices.len() <= u16::MAX as usize + 1 {
                IndexType::Uint16
            } else {
                IndexType::Uint32
            };
            let idx_bytes: Vec<u8> = match index_type {
                IndexType::Uint16 => entry
                    .indices
                    .iter()
                    .flat_map(|&i| (i as u16).to_le_bytes())
                    .collect(),
                IndexType::Uint32 => entry.indices.iter().flat_map(|&i| i.to_le_bytes()).collect(),
            };
            let mib = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: idx_bytes.len() as u64,
                    usage: BufferUsage::INDEX,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("M3 instanced mesh index buffer");
            {
                let p = RhiDevice::buffer_mapped_ptr(device, &mib)
                    .expect("host-visible instanced index buffer is mapped");
                // SAFETY: `p` points to `idx_bytes.len()` mapped host-coherent bytes; `idx_bytes`
                // is a distinct equally-sized alloc; the copy completes before any submit.
                unsafe {
                    core::ptr::copy_nonoverlapping(idx_bytes.as_ptr(), p.as_ptr(), idx_bytes.len());
                }
            }
            batches.push(InstancedGpuBatch {
                vertex_buffer: mvb,
                index_buffer: mib,
                index_count: entry.indices.len() as u32,
                index_type: index_type.as_i32(),
                base_instance,
                instance_count: entry.affines.len() as u32,
                casts_shadow: !inst.non_casters.contains(&batch_idx),
            });
        }
        let (ssbo, bg) = create_instance_buffer(device, &instance_layout, &ring);
        InstancedGpu { batches, instance_ssbo: ssbo, instance_bind_group: bg }
    });

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
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            bind_group_layout: Some(&instance_layout),
            blend: None,
            cull_mode: CullMode::None,
            depth_bias: None,
        },
    )
    .expect("mesh-MRT graphics pipeline");

    // --- The P1b marcher: the vocabulary layout + the marcher pipeline. ---
    let cs = RhiDevice::create_shader_module(device, sdf_gbuffer_composite_spirv())
        .expect("P1b G-buffer marcher compute shader module");
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
    .expect("P1b vocabulary bind-group layout");
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
    .expect("P1b G-buffer marcher compute pipeline");

    // --- The deferred RESOLVE pipeline (binds the light table @6 + the SDF edit-list @10). ---
    let resolve_cs = RhiDevice::create_shader_module(device, deferred_pbr_spirv())
        .expect("deferred resolve compute shader module");
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
        BindGroupLayoutEntry { binding: 16, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 17, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 18, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        // Textured-PBR T6a (the critic's C1 fix): the SOFTWARE-ONLY `gPbr` STORAGE image @19.
        // `GBufferTargets::create` now allocates `gPbr` UNCONDITIONALLY (both feature legs) and
        // `DeferredSets::build`'s software resolve-set loop appends it past the shared 19 —
        // the layout MUST declare it too, or `create_bind_group`'s entry-count check trips (P1a).
        BindGroupLayoutEntry { binding: 19, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
    ];
    let resolve_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &resolve_entries },
    )
    .expect("deferred resolve bind-group layout");
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
    .expect("deferred resolve compute pipeline");

    // --- The present-blit pipeline. ---
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
    .expect("present-blit bind-group layout");
    let sample_vs = RhiDevice::create_shader_module(device, SAMPLE_VS_SPV.as_words())
        .expect("fullscreen vertex shader module");
    let sample_fs = RhiDevice::create_shader_module(device, SAMPLE_FS_SPV.as_words())
        .expect("fullscreen fragment shader module");
    let present_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &sample_vs,
            vertex_entry: c"main",
            fragment_module: &sample_fs,
            fragment_entry: c"main",
            color_formats: &[swap_color_format],
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
    .expect("present-blit fullscreen-sample pipeline");

    // --- Render P7: the SSAO compute pass. ---
    let ssao_variant = cfg.ssao_quality.unwrap_or(SSAO_QUALITY_MEDIUM);
    let ssao_cs = RhiDevice::create_shader_module(device, sdf_ssao_spirv_variant(ssao_variant))
        .expect("Render P7 SSAO compute shader module");
    let ssao_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
    ];
    let ssao_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &ssao_entries },
    )
    .expect("Render P7 SSAO bind-group layout");
    let ssao_pipeline = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &ssao_cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&ssao_layout),
            spec_constants: &[],
        },
    )
    .expect("Render P7 SSAO compute pipeline");

    // The shader modules are consumed by pipeline creation; destroy them now.
    // SAFETY: every module was created on `ctx` above + is no longer needed once its pipeline
    // is created; each is destroyed exactly once.
    unsafe {
        RhiDevice::destroy_shader_module(device, sample_fs);
        RhiDevice::destroy_shader_module(device, sample_vs);
        RhiDevice::destroy_shader_module(device, ssao_cs);
        RhiDevice::destroy_shader_module(device, resolve_cs);
        RhiDevice::destroy_shader_module(device, cs);
        RhiDevice::destroy_shader_module(device, fs);
        RhiDevice::destroy_shader_module(device, vs);
    }

    // CSM Increment 1b (Rung A): the cascade trio + depth-only pipeline.
    let csm = CsmSceneResources::create(device, &instance_layout);

    // SDFDDGI I4: reseed the resolve grid UBO (`csm.ddgi_ubo`, resolve binding 18) — boot-created
    // ZERO — with the fitted grid geometry (the `ResolvedDdgi` 48-byte layout: `origin` vec4,
    // `inv_spacing`+dims vec4, `ddgi_mode_word`, pad). The redundant `ddgi_mode_word` mirror is set
    // too, though the resolve gates GI on the LightBuf word-7 bit-4 flag, not on this word.
    {
        let inv_spacing = 1.0_f32 / DDGI_SPACING;
        let grid_words: [u32; 12] = [
            DDGI_ORIGIN[0].to_bits(),
            DDGI_ORIGIN[1].to_bits(),
            DDGI_ORIGIN[2].to_bits(),
            0, // origin.w pad
            inv_spacing.to_bits(),
            DDGI_GRID_DIM_X,
            DDGI_GRID_DIM_Y,
            DDGI_GRID_DIM_Z,
            1, // ddgi_mode_word (redundant mirror)
            0,
            0,
            0, // pad
        ];
        let mapped = RhiDevice::buffer_mapped_ptr(device, &csm.ddgi_ubo)
            .expect("host-visible DDGI grid UBO is mapped");
        write_words(mapped, &grid_words);
    }

    // SDFDDGI I4: the dedicated probe-update resources (the arm test's template) — a boot-static
    // Fibonacci ray table, the b6 params UBO, the 7-binding update layout, the update module +
    // pipeline. The renderer writes the update bind group itself (`ddgi_update = Some(..)` on the
    // scene); the atlas / classification / light table already exist (reused).
    let ddgi_ray_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (DDGI_UPDATE_RAYS * 16) as u64,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("SDFDDGI update ray table");
    {
        let golden = core::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
        let mut words = vec![0u32; DDGI_UPDATE_RAYS * 4];
        for i in 0..DDGI_UPDATE_RAYS {
            let z = 1.0 - 2.0 * (i as f32 + 0.5) / DDGI_UPDATE_RAYS as f32;
            let r = (1.0 - z * z).max(0.0).sqrt();
            let phi = i as f32 * golden;
            for (k, c) in [r * phi.cos(), r * phi.sin(), z, 0.0].into_iter().enumerate() {
                words[i * 4 + k] = c.to_bits();
            }
        }
        let mapped = RhiDevice::buffer_mapped_ptr(device, &ddgi_ray_table)
            .expect("host-visible DDGI ray table is mapped");
        write_words(mapped, &words);
    }

    let ddgi_update_ubo = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: DDGI_UBO_BYTES,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("SDFDDGI update params UBO");
    {
        // The b6 `DdgiUpdateUbo` layout: `float4 origin` (xyz = grid origin, w = SPACING — the b6
        // convention carries spacing, NOT inv_spacing), `uint4 grid_dims` (xyz = raw dims, w =
        // `asfloat(hysteresis α)`), then `frame_index / subset_n / rays_per_probe / light_count`.
        let mut w = [0u32; (DDGI_UBO_BYTES / 4) as usize];
        w[0] = DDGI_ORIGIN[0].to_bits();
        w[1] = DDGI_ORIGIN[1].to_bits();
        w[2] = DDGI_ORIGIN[2].to_bits();
        w[3] = DDGI_SPACING.to_bits();
        w[4] = DDGI_GRID_DIM_X;
        w[5] = DDGI_GRID_DIM_Y;
        w[6] = DDGI_GRID_DIM_Z;
        w[7] = DDGI_HYSTERESIS.to_bits(); // grid_dims.w = asfloat(α) — the I4 update shader reads it.
        w[8] = 0; // frame_index — rewritten each converge frame.
        w[9] = 1; // subset_n = 1 (every probe every frame).
        w[10] = DDGI_UPDATE_RAYS as u32; // rays_per_probe (== ray-table length; ≤ GI_MAX_RAYS).
        w[11] = ddgi_light_count; // light_count.
        let mapped = RhiDevice::buffer_mapped_ptr(device, &ddgi_update_ubo)
            .expect("host-visible DDGI update UBO is mapped");
        write_words(mapped, &w);
    }

    let ddgi_update_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc {
            entries: &[
                BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
            ],
        },
    )
    .expect("SDFDDGI update bind-group layout");
    let ddgi_update_module = RhiDevice::create_shader_module(device, sdf_probe_update_spirv())
        .expect("SDFDDGI probe-update shader module");
    let ddgi_update_pipeline = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &ddgi_update_module,
            entry: c"main",
            // The update shader reads NO push constant (params ride the b6 UBO), but the RHI mandates
            // a non-empty shared range — 4 bytes, never 0.
            push_constant_bytes: 4,
            bind_group_layout: Some(&ddgi_update_layout),
            spec_constants: &[],
        },
    )
    .expect("SDFDDGI probe-update compute pipeline");

    let csm_activation = cfg.csm.map(|fit| {
        for s in 0..FRAMES_IN_FLIGHT {
            csm.upload(device, &fit, s);
        }
        let mut cascade_view_proj = [[0u8; 64]; CSM_MAX_CASCADES];
        for (dst, src) in cascade_view_proj
            .iter_mut()
            .zip(fit.cascades.iter())
            .take(fit.active_count as usize)
        {
            for (i, f) in src.view_proj.iter().enumerate() {
                dst[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
            }
        }
        let mut push = [0u8; GBUFFER_PUSH_BYTES];
        push[84..88].copy_from_slice(&1u32.to_le_bytes());
        (push, cascade_view_proj, fit.active_count)
    });

    let spot_activation = cfg.spot_atlas.map(|fit| {
        csm.upload_atlas(device, &fit);
        let mut face_view_proj = [[0u8; 64]; MAX_TEXTURE_LAYERS];
        let mut face_is_point = [false; MAX_TEXTURE_LAYERS];
        let mut face_light = [[0u8; 16]; MAX_TEXTURE_LAYERS];
        for (slot, src) in fit.faces.iter().enumerate().take(fit.active_layers as usize) {
            for (i, f) in src.view_proj.iter().enumerate() {
                face_view_proj[slot][i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
            }
            face_is_point[slot] = src.is_point;
            face_light[slot][0..4].copy_from_slice(&src.light_pos[0].to_le_bytes());
            face_light[slot][4..8].copy_from_slice(&src.light_pos[1].to_le_bytes());
            face_light[slot][8..12].copy_from_slice(&src.light_pos[2].to_le_bytes());
            face_light[slot][12..16].copy_from_slice(&src.inv_range.to_le_bytes());
        }
        let mut push = [0u8; GBUFFER_PUSH_BYTES];
        push[84..88].copy_from_slice(&1u32.to_le_bytes());
        (push, face_view_proj, face_is_point, face_light, fit.active_layers)
    });

    let mesh_draws: Vec<GBufferMeshDraw> = instanced_gpu
        .as_ref()
        .map(|g| {
            g.batches
                .iter()
                .map(|b| GBufferMeshDraw {
                    vertex_buffer: &b.vertex_buffer,
                    index_buffer: &b.index_buffer,
                    index_count: b.index_count,
                    index_type: b.index_type,
                    base_instance: b.base_instance,
                    instance_count: b.instance_count,
                    casts_shadow: b.casts_shadow,
                })
                .collect()
        })
        .unwrap_or_default();

    let mvp = cfg.mvp;
    let scene = GBufferScene {
        raster_pipeline: &raster_pipeline,
        vertex_buffer: &vertex_buffer,
        vertex_count: vertices.len() as u32,
        mvp,
        instance_bind_group: instanced_gpu
            .as_ref()
            .map_or(&instance_bind_group, |g| &g.instance_bind_group),
        marcher: &marcher,
        vocab_layout: &vocab_layout,
        edit_list: &edit_list,
        camera_ring: &camera_ring,
        tiles_buffer: &tiles_buffer,
        pointer_grid: clipmap.grid_buffer(0),
        atlas: clipmap.atlas(0).texture(),
        atlas_sampler: clipmap.sampler(0),
        level_grids: [clipmap.grid_buffer(1), clipmap.grid_buffer(2)],
        level_atlases: [clipmap.atlas(1).texture(), clipmap.atlas(2).texture()],
        level_atlas_samplers: [clipmap.sampler(1), clipmap.sampler(2)],
        mesh_sdf: mesh_sdf_texture
            .as_ref()
            .map_or_else(|| clipmap.atlas(0).texture(), |t| t.texture()),
        mesh_sdf_sampler: mesh_sdf_texture
            .as_ref()
            .map_or_else(|| clipmap.sampler(0), |t| t.sampler()),
        mesh_sdf_enabled,
        depth_sampler: &depth_sampler,
        material_table: &material_table,
        light_table: &light_table,
        light_staging: &light_staging,
        light_upload_bytes: light_table_bytes,
        light_dirty: false,
        cluster_cull: None,
        cull_layout: None,
        cluster_grid: None,
        light_index: None,
        light_index_alloc: None,
        cluster_cull_push: [0u8; 16],
        cluster_count: 0,
        resolve_pipeline: &resolve_pipeline,
        resolve_layout: &resolve_layout,
        #[cfg(feature = "hwrt")]
        resolve_pipeline_hwrt: None,
        #[cfg(feature = "hwrt")]
        resolve_layout_hwrt: None,
        #[cfg(feature = "hwrt")]
        resolve_tlas_hwrt: None,
        // Rung 1b: the HWRT resolve is OFF in this harness (`resolve_tlas_hwrt: None`), so the
        // shadow-params UBO ring is bound by NO set — a benign valid placeholder (the whole cascade
        // UBO ring, a per-FIF `[BoundBuffer; FRAMES_IN_FLIGHT]`, host-coherent + >= 16 B/slot)
        // satisfies the field type without ever being read.
        #[cfg(feature = "hwrt")]
        ray_shadow_ubo: csm.csm_ring(),
        present_pipeline: &present_pipeline,
        present_layout: &present_layout,
        present_sampler: &present_sampler,
        dispatch_group_count_x: group_count_x(),
        brick: None,
        coarse: None,
        coarse_mode: CoarseMode::EmptySkipOnly,
        lighting_flags: LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO,
        light_dir: marcher_light_dir(&cfg.light_elems),
        ssao: cfg
            .ssao_quality
            .map(|_| SsaoActivation { pipeline: &ssao_pipeline, layout: &ssao_layout }),
        mesh_draw: &mesh_draws,
        csm_cascade_texture: &csm.cascade,
        csm_compare_sampler: &csm.sampler,
        csm_cascade_ring: csm.csm_ring(),
        csm: csm_activation.map(|(push, cascade_view_proj, active_count)| CsmDepthActivation {
            pipeline: &csm.depth_pipeline,
            push,
            cascade_view_proj,
            active_count,
            shadow_dim: CSM_SHADOW_DIM,
        }),
        shadow_atlas_texture: &csm.atlas,
        shadow_atlas_sampler: &csm.atlas_sampler,
        shadow_atlas_ubo: &csm.atlas_ubo,
        // SDFDDGI I4: the resolve grid UBO now carries the fitted grid geometry (reseeded above); the
        // irradiance/depth atlases + samplers are the REAL probe atlas (unchanged from the golden).
        ddgi_irr_texture: csm.ddgi_atlas.irradiance(),
        ddgi_irr_sampler: csm.ddgi_atlas.sampler(),
        ddgi_depth_texture: csm.ddgi_atlas.depth(),
        ddgi_depth_sampler: csm.ddgi_atlas.sampler(),
        ddgi_grid_ubo: &csm.ddgi_ubo,
        // SDFDDGI I4: the probe-update pass is ARMED. The RDG derives the SRO→GENERAL update barriers
        // from `Some(..)`, records the update dispatch AFTER the marcher + L0 light-table copy and
        // BEFORE the resolve, and writes the 7-binding update set itself. The ray table + params UBO
        // are the dedicated buffers created above; the classification buffer is the atlas's own.
        ddgi_update: Some(DdgiUpdateActivation {
            pipeline: &ddgi_update_pipeline,
            layout: &ddgi_update_layout,
            dispatch_group_count_x: DDGI_PROBE_COUNT, // subset_n = 1 → one block per probe.
        }),
        ddgi_classification: csm.ddgi_atlas.classification(),
        ddgi_ray_table: &ddgi_ray_table,
        ddgi_update_ubo: &ddgi_update_ubo,
        atlas_punctual: spot_activation.map(
            |(push, face_view_proj, face_is_point, face_light, active_layers)| {
                PunctualDepthActivation {
                    pipeline: &csm.depth_pipeline,
                    point_pipeline: &csm.point_depth_pipeline,
                    push,
                    face_view_proj,
                    face_is_point,
                    face_light,
                    active_layers,
                    shadow_dim: SPOT_SHADOW_DIM,
                }
            },
        ),
        interp: None,
        // HW-RT rung R0: the caller's GPU timestamp collector. `None` for the byte-identical
        // BMP dump (`engine_grand_showcase_512_ddgi_screenshot_dump`) — ZERO extra commands, so
        // the golden stays byte-identical. `Some(&collector)` for the
        // `engine_grand_showcase_512_gpu_pass_cost` timing test, which brackets the four
        // software-ray passes on this real combined frame.
        gpu_timing,
        // HW-RT rung R2a-3: the per-frame TLAS pack + build OFF (byte-identical command stream).
        #[cfg(feature = "hwrt")]
        tlas: None,
        // HW-RT rung 3a: the spatial (à-trous) RT soft-shadow denoise OFF (byte-identical).
        #[cfg(feature = "hwrt")]
        shadow: None,
        // HW-RT rung 3a: the STABLE denoise-set-build signals — all OFF in this harness (no denoise
        // sets built; byte-identical).
        #[cfg(feature = "hwrt")]
        resolve_layout_denoise_hwrt: None,
        #[cfg(feature = "hwrt")]
        atrous_layout_denoise_hwrt: None,
        #[cfg(feature = "hwrt")]
        shadow_denoise_enabled: false,
        #[cfg(feature = "hwrt")]
        shadow_denoise_final_is_vis2: false,
        // Rung-3b step 5a: the temporal-MV mesh path — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        temporal_enabled: false,
        #[cfg(feature = "hwrt")]
        raster_pipeline_mv: None,
        #[cfg(feature = "hwrt")]
        mv_bind_group: None,
        // F8-mv: the combined MV+PM mesh path — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        raster_pipeline_mvpm: None,
        #[cfg(feature = "hwrt")]
        mvpm_bind_group: None,
        // Rung-3b step 5b: the SDF motion-vector VIS path — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        vis_mv_pipeline: None,
        #[cfg(feature = "hwrt")]
        vis_mv_layout: None,
        #[cfg(feature = "hwrt")]
        motion_cam_ubo_ring: None,
        // Rung-3b step 6: the temporal reproject layout — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        temporal_layout: None,
        // Asset-streaming plan F8: PER_INSTANCE_MATERIAL is OFF in this low-level RHI harness
        // (no ECS gather / material store exists here) — byte-identical to the pre-F8 stream.
        pm_enabled: false,
        raster_pipeline_pm: None,
        pm_bind_group: None,
        // Textured-PBR T6c: TEXTURED is OFF in this low-level RHI harness (no ECS gather /
        // texture asset store exists here) — byte-identical to the pre-T6c stream.
        tex_enabled: false,
        raster_pipeline_tex: None,
        tex_bind_group: None,
        bindless_set: None,
    };

    let present_extent = VkExtent2D { width: COMPOSITE_W, height: COMPOSITE_H };
    let staging_size = (swapchain.extent().width * swapchain.extent().height * 4) as u64;
    let staging = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: staging_size,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible readback staging buffer");
    let alloc_extent = swapchain.extent();
    let mut frame = GBufferFrame::new();

    // SDFDDGI I4 convergence: present `DDGI_CONVERGE_FRAMES` frames so the hysteresis-blended probe
    // atlas ramps from the boot-zero state to a converged indirect field before the readback. Each
    // frame rewrites `frame_index` (update UBO word 8, byte offset 32) so the I4 ray rotation
    // decorrelates the per-frame ray set; only the FINAL frame requests the staging readback.
    const DRAIN_FRAMES: u32 = 3;
    let clear = [0.04_f32, 0.05, 0.07, 1.0];

    let ddgi_ubo_ptr = RhiDevice::buffer_mapped_ptr(device, &ddgi_update_ubo)
        .expect("host-visible DDGI update UBO is mapped");

    // HW-RT rung R0: the GPU-pass-cost TIMING path. When a collector is threaded in, the
    // recorder brackets the four software-ray passes on every frame (`scene.gpu_timing` is
    // `Some`). Run `>= 200` measured frames, reading each frame's pool AFTER a `wait_idle` (the
    // simplest offline discipline — the recorded slot is `renderer.frame_index()` captured
    // BEFORE the submit that then rotates it), accumulate a `[f64; PASS_COUNT]` sample per
    // frame, discard the first 20, and report median + p95 + stddev (ns) per pass + ns/ray.
    // This whole block is skipped on the `None` (BMP dump) path — byte-identical golden.
    if let Some(collector) = gpu_timing {
        run_gpu_pass_cost_timing(
            window,
            ctx,
            surface,
            &mut swapchain,
            &mut renderer,
            &scene,
            &mut frame,
            &clear,
            present_extent,
            alloc_extent,
            ddgi_ubo_ptr,
            collector,
        );
    } else {

    let mut dumped: Option<(Vec<u8>, u32, u32)> = None;
    let mut converge_ok = true;
    for f in 0..DDGI_CONVERGE_FRAMES {
        if !window.pump_events() {
            eprintln!("NOTE engine_showcase_512: window closed during DDGI convergence — skipping");
            converge_ok = false;
            break;
        }
        window.refresh_size();
        let live = swapchain.extent();
        if live.width != alloc_extent.width || live.height != alloc_extent.height {
            eprintln!("NOTE engine_showcase_512: extent changed during DDGI convergence — skipping");
            converge_ok = false;
            break;
        }
        // SAFETY: `ddgi_update_ubo` is `DDGI_UBO_BYTES` (48) host-coherent mapped bytes; offset 32 is
        // the `frame_index` u32 (`DdgiUpdateUbo` layout, word 8). The prior frame's slot fence was
        // re-waited by `wait_frame_in_flight` below before this frame's submit, so no in-flight GPU
        // read of this UBO overlaps the write. The write precedes the submit that consumes it.
        unsafe {
            ddgi_ubo_ptr.as_ptr().add(32).cast::<u32>().write_unaligned(f);
        }
        let is_last = f == DDGI_CONVERGE_FRAMES - 1;
        let token = renderer
            .wait_frame_in_flight()
            .expect("invariant: the frame slot fence wait precedes the submit");
        // SAFETY: `ctx`/`surface`/`swapchain`/`renderer` share one device; every `scene` resource is
        // live; `present_extent` + `scene.dispatch_group_count_x` + the camera UBO `count` cover the
        // composite extent; `staging` is host-visible and ≥ one swapchain image in bytes. The staging
        // readback is requested ONLY on the final converge frame.
        let staging_arg = if is_last { Some(&staging) } else { None };
        let presented = unsafe {
            renderer.render_gbuffer_frame(
                token, ctx, surface, &mut swapchain, &scene, &mut frame,
                window.width(), window.height(), clear, present_extent, staging_arg,
            )
        }
        .unwrap_or_else(|e| panic!("showcase DDGI converge frame failed: {e:?}"));
        if !presented {
            eprintln!("NOTE engine_showcase_512: swapchain recreated during DDGI convergence — skipping");
            converge_ok = false;
            break;
        }
    }

    if converge_ok {
        let extent = swapchain.extent();
        for _ in 0..DRAIN_FRAMES {
            if !window.pump_events() {
                break;
            }
            window.refresh_size();
            let token = renderer
                .wait_frame_in_flight()
                .expect("invariant: the frame slot fence wait precedes the submit");
            // SAFETY: same contract; no readback requested on the drain frames.
            let _ = unsafe {
                renderer.render_gbuffer_frame(
                    token, ctx, surface, &mut swapchain, &scene, &mut frame,
                    window.width(), window.height(), clear, present_extent, None,
                )
            }
            .unwrap_or_else(|e| panic!("showcase DDGI drain frame failed: {e:?}"));
        }

        let w = extent.width;
        let h = extent.height;
        let byte_count = (w * h * 4) as usize;
        let dst_ptr = RhiDevice::buffer_mapped_ptr(device, &staging)
            .expect("host-visible staging buffer is mapped");
        let mut raw = vec![0u8; byte_count];
        // SAFETY: `dst_ptr` points to `staging_size` (≥ `byte_count`) mapped host-coherent bytes; the
        // final converge frame's copy completed before this read (its slot fence was re-waited by the
        // drain frames); `raw` is a distinct, non-overlapping alloc.
        unsafe { core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), raw.as_mut_ptr(), byte_count) };
        dumped = Some((readback_to_rgba(&raw, w, h, is_bgra), w, h));
    }

    if ctx.validation_enabled() {
        let state = ctx
            .debug_state()
            .expect("validation enabled => a debug-messenger state is present");
        assert_eq!(
            state.total(),
            0,
            "validation layer reported {} message(s) during the DDGI showcase present — \
             see the [vk-validation] log",
            state.total()
        );
    }

    match dumped {
        Some((rgba, w, h)) => {
            assert_eq!(
                (w, h),
                (COMPOSITE_W, COMPOSITE_H),
                "the readback must be the native {COMPOSITE_W}x{COMPOSITE_H} composite (no upscale)"
            );
            write_bmp(bmp_path, &rgba, w, h)
                .unwrap_or_else(|e| panic!("failed to write {bmp_path}: {e:?}"));
            let bytes = std::fs::read(bmp_path)
                .unwrap_or_else(|e| panic!("failed to re-read {bmp_path} for header verification: {e:?}"));
            let (bw, bh) = read_bmp_dimensions(&bytes)
                .expect("the dumped showcase must be a valid BM 54-byte-header BMP");
            assert_eq!(
                (bw, bh),
                (COMPOSITE_W as i32, COMPOSITE_H as i32),
                "the dumped BMP header must report {COMPOSITE_W}x{COMPOSITE_H} native dimensions"
            );
            println!("engine DDGI showcase dump -> {bmp_path} ({bw}x{bh} native, GI-ON indirect on the SDF spheres)");
        }
        None => {
            eprintln!(
                "NOTE engine_showcase_512: no DDGI readback frame presented (swapchain kept recreating); \
                 no BMP written"
            );
        }
    }

    } // end of the `None`-gpu_timing (BMP dump) path.

    drop(renderer);
    // SAFETY: the renderer was dropped above (its `Drop` waits the device idle), so no submission
    // references these resources; `ctx` is still alive; each is destroyed exactly once, in reverse
    // dependency order. The SDFDDGI I4 update resources are torn down FIRST (they reference nothing
    // else here; the device is idle).
    unsafe {
        RhiDevice::destroy_compute_pipeline(device, ddgi_update_pipeline);
        RhiDevice::destroy_shader_module(device, ddgi_update_module);
        RhiDevice::destroy_bind_group_layout(device, ddgi_update_layout);
        RhiDevice::destroy_buffer(device, ddgi_update_ubo);
        RhiDevice::destroy_buffer(device, ddgi_ray_table);
        frame.destroy(ctx);
        RhiDevice::destroy_buffer(device, staging);
        csm.destroy(ctx);
        RhiDevice::destroy_graphics_pipeline(device, present_pipeline);
        RhiDevice::destroy_bind_group_layout(device, present_layout);
        RhiDevice::destroy_compute_pipeline(device, ssao_pipeline);
        RhiDevice::destroy_bind_group_layout(device, ssao_layout);
        RhiDevice::destroy_compute_pipeline(device, resolve_pipeline);
        RhiDevice::destroy_bind_group_layout(device, resolve_layout);
        RhiDevice::destroy_compute_pipeline(device, marcher);
        RhiDevice::destroy_bind_group_layout(device, vocab_layout);
        RhiDevice::destroy_graphics_pipeline(device, raster_pipeline);
        drop(mesh_draws);
        if let Some(g) = instanced_gpu {
            RhiDevice::destroy_bind_group(device, g.instance_bind_group);
            RhiDevice::destroy_buffer(device, g.instance_ssbo);
            for b in g.batches {
                RhiDevice::destroy_buffer(device, b.index_buffer);
                RhiDevice::destroy_buffer(device, b.vertex_buffer);
            }
        }
        RhiDevice::destroy_bind_group(device, instance_bind_group);
        RhiDevice::destroy_buffer(device, instance_buffer);
        RhiDevice::destroy_bind_group_layout(device, instance_layout);
        RhiDevice::destroy_sampler(device, present_sampler);
        RhiDevice::destroy_sampler(device, depth_sampler);
        RhiDevice::destroy_buffer(device, vertex_buffer);
        RhiDevice::destroy_buffer(device, tiles_buffer);
        if let Some(t) = mesh_sdf_texture {
            t.destroy(ctx);
        }
        clipmap.destroy(ctx);
        RhiDevice::destroy_buffer(device, light_staging);
        RhiDevice::destroy_buffer(device, light_table);
        RhiDevice::destroy_buffer(device, material_table);
        for slot in camera_ring {
            RhiDevice::destroy_buffer(device, slot);
        }
        RhiDevice::destroy_buffer(device, edit_list);
    }
    drop(swapchain);
    // surface / ctx / window are owned by `with_windowed_present` and dropped in-order at its frame end.
}

// === HW-RT rung R0 — the GPU-pass-cost timing loop + its `#[ignore]` entry point. ===

/// The reported per-pass GPU timing summary (all in nanoseconds, GPU wall-clock).
#[derive(Clone, Copy, Debug, Default)]
struct GpuPassSummary {
    median_ns: f64,
    p95_ns: f64,
    stddev_ns: f64,
}

/// Summarizes a slice of ns samples to a `GpuPassSummary` (median + p95 + stddev). Sorts a copy
/// for the percentiles; `samples` must be non-empty.
fn summarize_gpu_pass(samples_ns: &[f64]) -> GpuPassSummary {
    let mut s = samples_ns.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    let median_ns = s[n / 2];
    let p95_idx = ((n as f64) * 0.95).ceil() as usize;
    let p95_ns = s[p95_idx.min(n - 1)];
    let mean = s.iter().sum::<f64>() / n as f64;
    let var = s.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n as f64;
    GpuPassSummary { median_ns, p95_ns, stddev_ns: var.sqrt() }
}

/// The number of measured frames the GPU-pass-cost timing loop presents (`>= 200`, plan Part C).
const GPU_PASS_COST_FRAMES: u32 = 220;
/// The warm-up frames discarded from the front (shader compile + GPU clock ramp + atlas ramp).
const GPU_PASS_COST_WARMUP: usize = 20;

/// Drives the GPU-pass-cost timing loop (HW-RT rung R0): presents `GPU_PASS_COST_FRAMES` real
/// combined frames with `scene.gpu_timing == Some(collector)` (so the recorder resets the pool
/// at the frame top + brackets the four software-ray passes), reads each frame's pool AFTER a
/// `wait_idle` for `PASS_COUNT` pairs, accumulates a `[f64; PASS_COUNT]` sample per frame,
/// discards the first `GPU_PASS_COST_WARMUP`, and prints the plan §C.3 table (median / p95 /
/// stddev per pass + ns/ray attribution).
///
/// `fi` (the pool slot the recorder used) is `renderer.frame_index()` captured BEFORE the
/// submit — `drive_frame` rotates `frame_index` at its END, so the pre-submit index IS the slot
/// `record_gbuffer`'s internal `let fi = self.frame_index` wrote. A `wait_idle` after each frame
/// (the simplest offline discipline) guarantees that slot's pool is readable before the next
/// frame reuses it.
#[allow(clippy::too_many_arguments)]
fn run_gpu_pass_cost_timing<'ctx>(
    window: &mut Window,
    ctx: &'ctx VulkanContext,
    surface: &Surface<'ctx>,
    swapchain: &mut Swapchain<'ctx>,
    renderer: &mut Renderer<'ctx>,
    scene: &GBufferScene<'_>,
    frame: &mut GBufferFrame,
    clear: &[f32; 4],
    present_extent: VkExtent2D,
    alloc_extent: VkExtent2D,
    ddgi_ubo_ptr: NonNull<u8>,
    collector: &TimestampCollector,
) {
    let device: &VulkanContext = ctx;
    // W1 precondition: the `read_query_pool_ns` below reads ALL `PASS_COUNT` (begin,end) pairs with
    // `VK_QUERY_RESULT_WAIT_BIT`, so EVERY bracketed pass must be recorded (its two queries written)
    // each frame — an unwritten query never becomes available and would HANG the read forever. The
    // grand_showcase GI-ON scene activates all four passes (DDGI update + the always-on deferred
    // resolve + CSM cascade depth + punctual atlas depth); assert it so a future scene config that
    // drops a pass fails LOUDLY here instead of deadlocking on the GPU.
    assert!(
        scene.ddgi_update.is_some() && scene.csm.is_some() && scene.atlas_punctual.is_some(),
        "gpu_pass_cost timing requires the DDGI-update + CSM + punctual passes all ACTIVE (else the \
         WAIT_BIT timestamp readback hangs on an unwritten query)"
    );
    // One `[f64; PASS_COUNT]` sample per measured frame.
    let mut samples: Vec<[f64; PASS_COUNT as usize]> = Vec::with_capacity(GPU_PASS_COST_FRAMES as usize);
    let mut scratch = [0u64; (2 * PASS_COUNT) as usize];
    let mut out_ns = [0.0f64; PASS_COUNT as usize];

    for f in 0..GPU_PASS_COST_FRAMES {
        if !window.pump_events() {
            eprintln!("NOTE gpu_pass_cost: window closed during timing — reporting partial samples");
            break;
        }
        window.refresh_size();
        let live = swapchain.extent();
        if live.width != alloc_extent.width || live.height != alloc_extent.height {
            eprintln!("NOTE gpu_pass_cost: extent changed during timing — reporting partial samples");
            break;
        }
        // Rotate the I4 ray set per frame (as the dump path does) so the DDGI-update cost reflects
        // the shipped per-frame ray rotation, not a degenerate fixed ray set.
        // SAFETY: `ddgi_update_ubo` is `DDGI_UBO_BYTES` (48) host-coherent mapped bytes; offset 32
        // is the `frame_index` u32 (`DdgiUpdateUbo` word 8). The prior frame's slot fence was
        // re-waited by `wait_frame_in_flight` below (and a `wait_idle` runs each iteration), so no
        // in-flight GPU read of this UBO overlaps the write. The write precedes the consuming submit.
        unsafe {
            ddgi_ubo_ptr.as_ptr().add(32).cast::<u32>().write_unaligned(f);
        }

        // The pool slot the recorder will write is the CURRENT frame_index (captured BEFORE the
        // submit that rotates it inside `drive_frame`).
        let fi = renderer.frame_index();
        let token = renderer
            .wait_frame_in_flight()
            .expect("invariant: the frame slot fence wait precedes the submit");
        // SAFETY: `ctx`/`surface`/`swapchain`/`renderer` share one device; every `scene` resource
        // is live; `present_extent` + `scene.dispatch_group_count_x` + the camera UBO `count` cover
        // the composite extent; NO readback buffer (the timing path reads timestamps, not pixels).
        let presented = unsafe {
            renderer.render_gbuffer_frame(
                token, ctx, surface, swapchain, scene, frame,
                window.width(), window.height(), *clear, present_extent, None,
            )
        }
        .unwrap_or_else(|e| panic!("gpu_pass_cost frame failed: {e:?}"));
        if !presented {
            eprintln!("NOTE gpu_pass_cost: swapchain recreated during timing — reporting partial samples");
            break;
        }

        // Offline discipline: wait the device idle so the just-submitted frame's timestamp writes
        // are complete + readable before we read (and before the slot is reused two frames on).
        device.wait_idle().expect("wait_idle");
        // Read the four (begin,end) pairs of THIS frame's pool (`fi`), masked + period-scaled to ns.
        device
            .read_query_pool_ns(collector.pool(fi), PASS_COUNT, &mut scratch, &mut out_ns)
            .expect("read_query_pool_ns");
        samples.push(out_ns);
    }

    if ctx.validation_enabled() {
        let state = ctx
            .debug_state()
            .expect("validation enabled => a debug-messenger state is present");
        assert_eq!(
            state.total(),
            0,
            "validation layer reported {} message(s) during the GPU-pass-cost timing — see the [vk-validation] log",
            state.total()
        );
    }

    if samples.len() <= GPU_PASS_COST_WARMUP {
        eprintln!(
            "NOTE gpu_pass_cost: only {} frame(s) measured (<= {GPU_PASS_COST_WARMUP} warm-up) — no stats reported",
            samples.len()
        );
        return;
    }
    let kept = &samples[GPU_PASS_COST_WARMUP..];

    // Per-pass columns for the summary.
    let pass_names = ["DdgiUpdate", "DeferredResolve", "CsmDepth", "PunctualDepth"];
    let mut per_pass: [Vec<f64>; PASS_COUNT as usize] =
        core::array::from_fn(|_| Vec::with_capacity(kept.len()));
    for sample in kept {
        for (p, &ns) in sample.iter().enumerate() {
            per_pass[p].push(ns);
        }
    }
    let summaries: Vec<GpuPassSummary> = per_pass.iter().map(|c| summarize_gpu_pass(c)).collect();

    // ns/ray attribution (plan Part C):
    //  - DdgiUpdate  = DDGI_PROBE_COUNT * DDGI_UPDATE_RAYS rays.
    //  - DeferredResolve = shaded-pixel count (ns/px; the SDF soft-shadow march is INCLUSIVE).
    //  - CsmDepth / PunctualDepth = n/a (no clean ray count — depth-only passes).
    const DDGI_UPDATE_RAYS: u32 = 64; // the showcase's I4 ray count (subset_n = 1 → one block/probe).
    let ddgi_rays = (DDGI_PROBE_COUNT * DDGI_UPDATE_RAYS) as f64;
    // The resolve dispatches one thread per composite pixel (the marcher's 1:1 grid).
    let shaded_px = (COMPOSITE_W * COMPOSITE_H) as f64;

    println!(
        "engine_grand_showcase_512_gpu_pass_cost on: {} (kept {}/{} frames, GI ON — all four software-ray passes)",
        ctx.device_name(),
        kept.len(),
        samples.len()
    );
    println!(
        "  DDGI update rays = {DDGI_PROBE_COUNT} probes * {DDGI_UPDATE_RAYS} rays = {} rays; resolve shaded px = {}x{} = {}",
        ddgi_rays as u64, COMPOSITE_W, COMPOSITE_H, shaded_px as u64
    );
    println!(
        "  {:<16} {:>14} {:>14} {:>14} {:>16}",
        "pass", "median_ns", "p95_ns", "stddev_ns", "per-ray/px"
    );
    for (p, name) in pass_names.iter().enumerate() {
        let s = summaries[p];
        let attribution = match p {
            0 => format!("{:.3} ns/ray", s.median_ns / ddgi_rays),
            1 => format!("{:.3} ns/px*", s.median_ns / shaded_px),
            _ => "n/a".to_string(),
        };
        println!(
            "  {:<16} {:>14.1} {:>14.1} {:>14.1} {:>16}",
            name, s.median_ns, s.p95_ns, s.stddev_ns, attribution
        );
    }
    println!(
        "  * DeferredResolve ns/px is the WHOLE resolve dispatch, INCLUDING the inline SDF soft-shadow \
         march (R0 brackets passes, not shader sections)."
    );
    println!(
        "  NOTE: TOP/BOTTOM brackets each pass's wall-clock (inclusive of pipeline overlap), not \
         isolated kernel time; the median/p95 are over {} kept frames.",
        kept.len()
    );
}

/// HW-RT rung R0 — the four-pass GPU-pass-cost timing test on the REAL combined showcase frame
/// (`#[ignore]`, plan `docs/RENDER-R0-INSTRUMENT-PLAN.md` Part C).
///
/// Reuses `run_showcase_body_ddgi`'s GI-ON scene setup VERBATIM (so all four software-ray passes
/// run: DDGI probe-update, deferred resolve incl. the inline SDF shadow march, CSM cascade depth,
/// punctual atlas depth), threading a [`TimestampCollector`] so the recorder brackets each pass.
/// Graceful-skip when the device cannot be timed (`!timestamps_usable()`). Reports per-pass
/// GPU wall-clock (median / p95 / stddev, ns) + ns/ray attribution.
///
/// Named `..._gpu_pass_cost` (NOT `..._time_setup`): "cost"/"pass"/"baseline" are safe substrings;
/// "time"/"update"/"setup"/"install"/"patch" trigger Windows os-error-740 (UAC) on the box.
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1` +
/// `--nocapture --test-threads=1` (the orchestrator runs it on the GPU).
#[test]
#[ignore = "GPU-timestamp pass-cost measurement; needs a real RTX windowed device (--nocapture --test-threads=1); the orchestrator runs it"]
fn engine_grand_showcase_512_gpu_pass_cost() {
    with_windowed_present("boyko_engine grand showcase GPU pass cost 512", "engine_showcase_512", |bp| {
        // Graceful-skip BEFORE any resource setup: a device with no valid timestamp bits or an
        // implausible period cannot be measured — print a skip line + return (no panic).
        let caps = bp.ctx.device_caps();
        if !caps.timestamps_usable() {
            println!(
                "SKIP engine_grand_showcase_512_gpu_pass_cost: GPU timestamps unusable \
                 (valid_bits={}, period={} ns/tick)",
                caps.timestamp_valid_bits, caps.timestamp_period
            );
            return;
        }
        println!(
            "engine_grand_showcase_512_gpu_pass_cost: timestamps OK (valid_bits={}, period={} ns/tick, mask=0x{:x})",
            caps.timestamp_valid_bits, caps.timestamp_period, caps.timestamp_mask()
        );

        // Create the R0 collector: one `2 * PASS_COUNT`-query TIMESTAMP pool per in-flight frame.
        let device: &VulkanContext = bp.ctx;
        let pools: [VulkanQueryPool; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            RhiDevice::create_query_pool(device, &QueryPoolDesc { count: 2 * PASS_COUNT })
                .expect("timestamp query pool")
        });
        let collector = TimestampCollector::new(pools);

        // Drive the GI-ON showcase with the collector — `run_showcase_body_ddgi` sets
        // `scene.gpu_timing = Some(&collector)` and takes its timing branch (>= 200 frames,
        // per-pass readback, stats print), leaving the BMP-dump path byte-identical when `None`.
        // Its shared teardown waits the device idle before it returns, so the pools are safe to
        // destroy below.
        run_showcase_body_ddgi(bp, GRAND_SHOWCASE_DDGI_BMP, grand_showcase_config(), false, Some(&collector));

        // SAFETY: `run_showcase_body_ddgi` dropped its `Renderer` (its `Drop` waits the device
        // idle) before returning, so no submission references the pools; each pool was created on
        // `device` and is destroyed exactly once (the by-value move out of the collector).
        unsafe {
            for pool in collector.into_pools() {
                RhiDevice::destroy_query_pool(device, pool);
            }
        }
    });
}

fn run_showcase_body(bp: BootPresent<'_, '_>, bmp_path: &str, cfg: ShowcaseConfig, interactive: bool) {
    let BootPresent { window, ctx, surface, mut swapchain, mut renderer, is_bgra, swap_color_format } =
        bp;

    let device: &VulkanContext = ctx;
    let sdf = &cfg.sdf;
    // M2: the instanced draw's instance count (captured before `cfg.vertices` is moved below).
    // `0` when there is no instanced mesh (the legacy path leaves `scene.mesh_draw == None`).

    // --- The edit-list SSBO (binding 0), host-seeded ONCE. The resolve binds the SAME buffer
    // at binding 10 for the per-caster shadow march. ---
    let edit_list = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, sdf);
        let mapped = RhiDevice::buffer_mapped_ptr(device, &edit_list)
            .expect("host-visible edit-list buffer is mapped");
        write_words(mapped, &header);
    }

    // --- The camera/extent UBO (binding 5), host-seeded ONCE at the COMPOSITE PERSPECTIVE extent
    // ([`showcase_camera`] — a down-looking front camera so the SDF floor + bodies + their cast
    // shadows read as a 3D scene). The M4 tail stays zero (brick is held OFF for the showcase —
    // the analytic marcher is the crisp reference path; bindings 9..=14 still need VALID
    // descriptors below). ---
    // MDF Stage-2c: when the showcase carries a mesh-SDF caster, bake its dense grid + upload the
    // dedicated `MeshSdfTexture` (binding 15) and lay out its grid descriptor (the `MeshSdfField`
    // the b5 `MeshSdfParams` tail mirrors). `None` (every other showcase) keeps the texture absent —
    // the brick atlas placeholder is bound at 15 + the mesh-shadow path is gated OFF.
    let mesh_sdf_texture: Option<MeshSdfTexture> = cfg.mesh_sdf.as_ref().map(|(pos, idx)| {
        let mesh = BakeMesh::new(pos, idx);
        // A voxel fine enough that the P2 lower-bound budget holds (`for_mesh` asserts it). The
        // demo torus tube radius (~0.18) is well-resolved at this scale.
        let field = MeshSdfField::for_mesh(&mesh, 0.04);
        MeshSdfTexture::create(ctx, &mesh, &field).expect("MDF mesh-SDF texture create + upload")
    });
    let mesh_sdf_enabled = mesh_sdf_texture.is_some();

    // The b5 camera UBO is sized to 256 (`B5_CAMERA_UBO_BYTES_MESH_SDF`) when the MDF tail is
    // written, else the 224-byte M4 size (the MDF tail then stays absent — bound-but-unread).
    let ubo_bytes = if mesh_sdf_enabled {
        B5_CAMERA_UBO_BYTES_MESH_SDF
    } else {
        B5_CAMERA_UBO_BYTES_M4
    };
    // The camera/extent UBO RING (binding 5): one host-coherent slot per in-flight frame. Every
    // slot is seeded IDENTICALLY here. For the one-shot readback dump no slot is rewritten (so the
    // output is byte-identical to the pre-ring single buffer); for the interactive viewer the loop
    // writes `camera_ring[frame_index]` per frame — the lock-free write-after-read fix.
    let camera_ring: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
        RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: ubo_bytes as u64,
                usage: BufferUsage::UNIFORM,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("camera uniform buffer")
    });
    {
        let pc = &cfg.camera;
        assert_eq!(pc.count, PIXELS);
        let bytes = pc.as_bytes();
        debug_assert_eq!(bytes.len(), M2_GRID_PARAMS_OFFSET, "camera block must be 80 B");
        let mesh_sdf_params = mesh_sdf_texture
            .as_ref()
            .map(|tex| MeshSdfParams::from_field(tex.field()));
        for slot in &camera_ring {
            let mapped = RhiDevice::buffer_mapped_ptr(device, slot)
                .expect("host-visible uniform buffer is mapped");
            // SAFETY: `mapped` points to `ubo_bytes` (224 or 256) mapped host-coherent bytes; the
            // 80-byte camera block is written at offset 0. No GPU work is in flight yet, so the host
            // write is unsynchronized-safe. Every ring slot is seeded with the SAME bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
            }
            // MDF Stage-2c: write the `MeshSdfParams` grid transform at offset 224 so the marcher's
            // `mesh_sdf_sample` maps a world point into the texture's UVW + decodes to a world distance.
            if let Some(params) = mesh_sdf_params.as_ref() {
                let pbytes = params.as_bytes();
                debug_assert_eq!(MESH_SDF_PARAMS_OFFSET + pbytes.len(), ubo_bytes);
                // SAFETY: the buffer is `ubo_bytes` (256) here; the 32-byte block is written at
                // `MESH_SDF_PARAMS_OFFSET` (224), entirely within the mapped range; unique host writer
                // before any GPU work. Every ring slot receives the SAME tail bytes.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        pbytes.as_ptr(),
                        mapped.as_ptr().add(MESH_SDF_PARAMS_OFFSET),
                        pbytes.len(),
                    );
                }
            }
        }
    }

    // --- The P4b coarse-cull tile StorageBuffer (vocab binding 6), bound-but-unread (the
    // showcase runs the marcher with the coarse cull gated OFF). ---
    let (tw, th) = tile_grid_extent(COMPOSITE_W, COMPOSITE_H);
    let tiles_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (tw as u64) * (th as u64) * (TILE_BOUND_BYTES as u64),
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("P4b coarse-cull tile-bound storage buffer (vocab binding 6)");

    // --- The PBR material table SSBO (vocab binding 7 + resolve binding 4): the default
    // mid-gray dielectric (the showcase edits carry no material id ⇒ every SDF hit picks 0). ---
    // The 3-slot table (slot 0 default + slots 1/2 emissive light markers — see
    // `showcase_material_table`). Non-viewer scenes use only slot 0, so they stay byte-identical.
    let mat_table = showcase_material_table();
    let material_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (mat_table.len() as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("PBR material table storage buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &material_table)
            .expect("host-visible material table is mapped");
        write_words(mapped, &mat_table);
    }

    // --- The brick clip-map: brick is held OFF for the showcase, but the marcher SPIR-V
    // statically references bindings 9..=14 past the runtime gate, so VALID descriptors must be
    // bound. The real clip-map (baked from the SAME authority field) supplies them. ---
    let field = {
        use boyko_sdf_math::SdfEditField;
        let mut f = SdfEditField::new();
        for e in sdf {
            assert!(f.push(*e), "showcase scene must fit MAX_SDF_EDITS");
        }
        f.bump_gen();
        f
    };
    let clipmap = BrickClipmap::create(ctx, &field, [0.0, 0.0, 0.0])
        .expect("brick clip-map (showcase scene) — create + bake + upload");

    // --- The Lighting light table SSBO (resolve binding 6): the SHOWCASE multi-light shadow
    // table (`shadow_mode == 1`, NON-CLUSTERED) + its staging source. Render P7: the `cfg` builder
    // already ARMED `ssao_mode == 1` (header word 11) so the resolve combines the SSAO term
    // (`scene.ssao = Some(..)` records the SSAO pass that writes it). ---
    // CSM Increment 1b (Rung A): arm `csm_mode` (header word 7 bit 2) in lock-step with the depth
    // pass (`cfg.csm.is_some()`). OFF leaves the header byte-identical (the 0%-gate); ON makes the
    // resolve `min`-combine the cascade PCF sample into the primary directional's visibility.
    // Shadow Phase 5 Inc-1-GPU: arm `punctual_shadow_mode` (header word 7 bit 3) in lock-step with
    // the punctual depth pass (`cfg.spot_atlas.is_some()`). OFF leaves the header byte-identical (the
    // 0%-gate); ON makes the resolve MULTIPLY the spot atlas PCF sample into the SPOT's contribution
    // (the spot's `dir_kind.w` carries slot 0, packed in `spot_shadow_config` via `with_atlas_slot`).
    let light_header = cfg
        .light_header
        .with_csm_mode(cfg.csm.is_some())
        .with_punctual_shadow_mode(cfg.spot_atlas.is_some());
    let light_elems = &cfg.light_elems;
    let light_words = pack_showcase_light_table(&light_header, light_elems);
    let light_table_bytes = (light_words.len() as u64) * 4;
    let light_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: light_table_bytes,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("showcase light table storage buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_table)
            .expect("host-visible light table is mapped");
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
    .expect("showcase light table staging buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_staging)
            .expect("host-visible light staging is mapped");
        write_words(mapped, &light_words);
    }

    // --- The mesh's vertex buffer (the showcase floor / hybrid-room geometry). ---
    let vertices = cfg.vertices;
    // `vertices` is a `Vec`, so the byte length is the slice's footprint (NOT `size_of_val`
    // of the `Vec` handle, which is the 24-byte struct, not the heap buffer).
    let vertex_bytes = core::mem::size_of_val(vertices.as_slice()) as u64;
    let vertex_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible vertex buffer");
    {
        let vb_ptr = RhiDevice::buffer_mapped_ptr(device, &vertex_buffer)
            .expect("host-visible vertex buffer is mapped");
        // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes; `vertices`'s heap
        // buffer is a distinct `vertex_bytes`-byte region (`vertex_bytes == len * stride`); the
        // write completes before any submit.
        unsafe {
            core::ptr::copy_nonoverlapping(
                vertices.as_ptr().cast::<u8>(),
                vb_ptr.as_ptr(),
                vertex_bytes as usize,
            );
        }
    }

    let depth_sampler = RhiDevice::create_sampler(device, &SamplerDesc::default())
        .expect("depth sampler (ignored by .Load)");
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
    .expect("present nearest/clamp sampler");

    // --- The mesh-MRT G-buffer producer graphics pipeline (Render P5-r0). ---
    let vs = RhiDevice::create_shader_module(device, MRT_VS_SPV.as_words())
        .expect("mesh-MRT vertex shader module");
    let fs = RhiDevice::create_shader_module(device, MRT_FS_SPV.as_words())
        .expect("mesh-MRT fragment shader module");
    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
    ];
    // M1: the per-instance model SSBO layout + 1-element identity dummy + its bind group
    // (the gbuffer VS statically references `instances` at set 0 binding 0 — the layout MUST
    // declare it + a valid buffer MUST be bound; the legacy draw never reads it).
    let (instance_layout, instance_buffer, instance_bind_group) = create_identity_instance(device);

    // M3: when the config carries instanced meshes, build their GPU resources — each mesh's
    // model-space vertex + index buffers (the SAME buffers
    // `boyko_render::MeshRegistry::register_mesh` mints; `boyko_rhi_vulkan` cannot name
    // `boyko_render` without a dep cycle, so the GPU test builds them inline) + ONE SHARED
    // N-instance model SSBO holding every mesh's affines concatenated in mesh order (mesh 0 at
    // base 0, mesh 1 at base `meshes[0].affines.len()`, …). This MIRRORS the M3 gather's
    // count→prefix-sum→scatter on the host (the algorithm itself is proven by the
    // `boyko_render` step-4 unit test); this GPU demo proves the RECORDER's batch loop + the
    // nonzero `base_instance` (C1) + mixed index width (O3). `None` (legacy scenes) leaves
    // these absent and `scene.mesh_draw` an empty slice.
    let instanced_gpu: Option<InstancedGpu> = cfg.instanced.as_ref().map(|inst| {
        // The shared instance ring: every mesh's affines concatenated in mesh order. Mesh k's
        // `base_instance` is the running offset BEFORE its affines are appended (the prefix-sum
        // of the prior meshes' instance counts — NONZERO for every mesh after the first).
        let mut ring: Vec<[f32; 12]> = Vec::new();
        let mut batches: Vec<InstancedGpuBatch> = Vec::with_capacity(inst.meshes.len());
        for (batch_idx, entry) in inst.meshes.iter().enumerate() {
            let base_instance = ring.len() as u32;
            ring.extend_from_slice(&entry.affines);

            // Vertex buffer (model-space).
            let vbytes = core::mem::size_of_val(entry.vertices.as_slice()) as u64;
            let mvb = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: vbytes,
                    usage: BufferUsage::VERTEX,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("M3 instanced mesh vertex buffer");
            {
                let p = RhiDevice::buffer_mapped_ptr(device, &mvb)
                    .expect("host-visible instanced vertex buffer is mapped");
                // SAFETY: `p` points to `vbytes` mapped host-coherent bytes; `entry.vertices` is
                // a distinct `vbytes`-byte slice; the copy completes before any submit.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        entry.vertices.as_ptr().cast::<u8>(),
                        p.as_ptr(),
                        vbytes as usize,
                    );
                }
            }
            // Index buffer: O3 width pick (Uint16 when the unique vertex count fits a u16).
            let index_type = if entry.vertices.len() <= u16::MAX as usize + 1 {
                IndexType::Uint16
            } else {
                IndexType::Uint32
            };
            let idx_bytes: Vec<u8> = match index_type {
                IndexType::Uint16 => entry
                    .indices
                    .iter()
                    .flat_map(|&i| (i as u16).to_le_bytes())
                    .collect(),
                IndexType::Uint32 => entry.indices.iter().flat_map(|&i| i.to_le_bytes()).collect(),
            };
            let mib = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: idx_bytes.len() as u64,
                    usage: BufferUsage::INDEX,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("M3 instanced mesh index buffer");
            {
                let p = RhiDevice::buffer_mapped_ptr(device, &mib)
                    .expect("host-visible instanced index buffer is mapped");
                // SAFETY: `p` points to `idx_bytes.len()` mapped host-coherent bytes; `idx_bytes`
                // is a distinct equally-sized alloc; the copy completes before any submit.
                unsafe {
                    core::ptr::copy_nonoverlapping(idx_bytes.as_ptr(), p.as_ptr(), idx_bytes.len());
                }
            }
            batches.push(InstancedGpuBatch {
                vertex_buffer: mvb,
                index_buffer: mib,
                index_count: entry.indices.len() as u32,
                index_type: index_type.as_i32(),
                base_instance,
                instance_count: entry.affines.len() as u32,
                casts_shadow: !inst.non_casters.contains(&batch_idx),
            });
        }
        // ONE shared N-instance model SSBO + its bind group on the gbuffer set-0 layout, holding
        // the concatenated ring (the recorder binds this ONCE for the whole batch list).
        let (ssbo, bg) = create_instance_buffer(device, &instance_layout, &ring);
        InstancedGpu { batches, instance_ssbo: ssbo, instance_bind_group: bg }
    });

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
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            bind_group_layout: Some(&instance_layout),
            blend: None,
            cull_mode: CullMode::None,
            depth_bias: None,
        },
    )
    .expect("mesh-MRT graphics pipeline");

    // --- The P1b marcher: the vocabulary layout + the marcher pipeline. ---
    let cs = RhiDevice::create_shader_module(device, sdf_gbuffer_composite_spirv())
        .expect("P1b G-buffer marcher compute shader module");
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
        // MDF Stage-2c: the dedicated dense mesh-SDF shadow-caster texture @15 (the 16th / last vocab
        // entry under the 16-binding cap). The recompiled marcher SPIR-V statically references
        // `MeshSdf`@t15 + `MeshSdfSampler`@s15 inside the runtime-gated `mesh_sdf_enabled` branch, so
        // the layout MUST declare binding 15 — a VALID combined image+sampler must be bound even on
        // the OFF path (`mesh_sdf_enabled == false` → bound-but-unread, byte-identical output).
        BindGroupLayoutEntry { binding: 15, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
    ];
    let vocab_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &vocab_entries },
    )
    .expect("P1b vocabulary bind-group layout");
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
    .expect("P1b G-buffer marcher compute pipeline");

    // --- The deferred RESOLVE pipeline (binds the light table @6 + the SDF edit-list @10 for
    // the per-caster shadow march). ---
    let resolve_cs = RhiDevice::create_shader_module(device, deferred_pbr_spirv())
        .expect("deferred resolve compute shader module");
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
        // P6 R1: the SDF edit-list `Buf` @10 (the `sdf_soft_shadow_ranged` march reads it).
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Render P7: the SSAO term `gSsao` STORAGE image @11 (read under `ssao_mode != 0`; OFF
        // here, bound-but-unread). The production `GBufferTargets` binds the SSAO image at @11,
        // so the resolve layout MUST declare it (the P6 R1 binding-10 discipline).
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        // CSM Increment 1b (Rung A): the cascade combined map+sampler @12 + the cascade UBO @13.
        // The production `GBufferTargets::create` binds the scene's cascade trio at @12/@13, so the
        // resolve layout MUST declare them (the recompiled resolve STATICALLY references `gCsm` +
        // `CsmCascades`). 14 bindings ≤ the 16-binding cap. `csm_mode == 0` on the golden presents
        // → bound-but-unread (the 0%-gate).
        BindGroupLayoutEntry { binding: 12, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 13, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        // Shadow Phase 5 Inc-1-GPU: the sparse spot/point shadow-atlas combined map+sampler @14 + the
        // atlas UBO @15. The production `GBufferTargets::create` binds the scene's atlas trio at
        // @14/@15, so the resolve layout MUST declare them (the recompiled resolve STATICALLY
        // references `gShadowAtlas` + `ShadowAtlas`). 16 bindings == the 16-binding cap (16/16);
        // `punctual_shadow_mode == 0` on the golden presents → bound-but-unread (the 0%-gate).
        BindGroupLayoutEntry { binding: 14, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 15, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        // SDFDDGI I0: the DDGI probe-irradiance combined image @16 + depth combined image @17 + the
        // `ResolvedDdgi` grid UBO @18 (bound-but-unread; the recompiled resolve STATICALLY references
        // `gDdgiIrr`/`gDdgiDepth`/`ResolvedDdgi`, so the layout MUST declare them). Exact-fill 19/19.
        BindGroupLayoutEntry { binding: 16, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 17, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 18, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        // Textured-PBR T6a (the critic's C1 fix): the SOFTWARE-ONLY `gPbr` STORAGE image @19.
        // `GBufferTargets::create` now allocates `gPbr` UNCONDITIONALLY (both feature legs) and
        // `DeferredSets::build`'s software resolve-set loop appends it past the shared 19 —
        // the layout MUST declare it too, or `create_bind_group`'s entry-count check trips (P1a).
        BindGroupLayoutEntry { binding: 19, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
    ];
    let resolve_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &resolve_entries },
    )
    .expect("deferred resolve bind-group layout");
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
    .expect("deferred resolve compute pipeline");

    // --- The present-blit pipeline. ---
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
    .expect("present-blit bind-group layout");
    let sample_vs = RhiDevice::create_shader_module(device, SAMPLE_VS_SPV.as_words())
        .expect("fullscreen vertex shader module");
    let sample_fs = RhiDevice::create_shader_module(device, SAMPLE_FS_SPV.as_words())
        .expect("fullscreen fragment shader module");
    let present_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &sample_vs,
            vertex_entry: c"main",
            fragment_module: &sample_fs,
            fragment_entry: c"main",
            color_formats: &[swap_color_format],
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
    .expect("present-blit fullscreen-sample pipeline");

    // --- Render P7: the SSAO compute pass (dedicated 5-binding set { gNormal @0, gMaterial @1,
    // gViewT @2 (R), the `ssao` out @3 (W), camera UBO @4 }). It gathers a horizon-based AO factor
    // from the G-buffer and stores it into the `ssao` lane the resolve combines under `ssao_mode
    // != 0` (armed via `light_header.with_ssao_mode(1)` above). `GBufferTargets` writes the
    // `ssao_set` against THIS layout, pointing at the per-extent G-buffer + `ssao` images. ---
    // Render P7-Q2: bind the SELECTED quality variant's pre-compiled `.spv` (Mechanism C). When SSAO
    // is OFF (`cfg.ssao_quality == None`) the pipeline is still created (and destroyed) — harmless —
    // but `scene.ssao` below is set to `None`, so the recorder records NO SSAO pass (the 0%-gate).
    let ssao_variant = cfg.ssao_quality.unwrap_or(SSAO_QUALITY_MEDIUM);
    let ssao_cs = RhiDevice::create_shader_module(device, sdf_ssao_spirv_variant(ssao_variant))
        .expect("Render P7 SSAO compute shader module");
    let ssao_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
    ];
    let ssao_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &ssao_entries },
    )
    .expect("Render P7 SSAO bind-group layout");
    let ssao_pipeline = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &ssao_cs,
            entry: c"main",
            // The SSAO shader pushes NO constant (camera is the UBO @4), but the create contract
            // requires a non-empty (multiple-of-4) range; declare the shared range (unused).
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&ssao_layout),
            spec_constants: &[],
        },
    )
    .expect("Render P7 SSAO compute pipeline");

    // The shader modules are consumed by pipeline creation; destroy them now.
    // SAFETY: every module was created on `ctx` above + is no longer needed once its pipeline
    // is created; each is destroyed exactly once.
    unsafe {
        RhiDevice::destroy_shader_module(device, sample_fs);
        RhiDevice::destroy_shader_module(device, sample_vs);
        RhiDevice::destroy_shader_module(device, ssao_cs);
        RhiDevice::destroy_shader_module(device, resolve_cs);
        RhiDevice::destroy_shader_module(device, cs);
        RhiDevice::destroy_shader_module(device, fs);
        RhiDevice::destroy_shader_module(device, vs);
    }

    // CSM Increment 1b (Rung A): the cascade trio + depth-only pipeline (ALWAYS created so the
    // resolve set can bind @12/@13). When `cfg.csm` is `Some`, upload its `view_proj` into the
    // cascade UBO + build the depth-pass push (the O1 single-matrix pin: the UBO + the push carry
    // IDENTICAL `view_proj` bytes); else the trio is bound-but-unread (`csm: None`, the 0%-gate).
    let csm = CsmSceneResources::create(device, &instance_layout);
    // CSM Increment 3 (Rung B): upload the N-cascade fit into the UBO, then build the depth-pass
    // activation — the per-cascade `view_proj` byte blocks (the depth loop stamps `[c]` into the
    // push @0) + the active count. The push TEMPLATE carries `use_model_matrix == 1` (@84); its
    // leading 64 bytes are overwritten per cascade so they are left zero here.
    let csm_activation = cfg.csm.map(|fit| {
        // Seed EVERY ring slot identically (the static dump path never rewrites; the viewer
        // overwrites `csm.ubo[frame_index]` per frame with a re-fit — the lock-free fix).
        for s in 0..FRAMES_IN_FLIGHT {
            csm.upload(device, &fit, s);
        }
        let mut cascade_view_proj = [[0u8; 64]; CSM_MAX_CASCADES];
        for (dst, src) in cascade_view_proj
            .iter_mut()
            .zip(fit.cascades.iter())
            .take(fit.active_count as usize)
        {
            for (i, f) in src.view_proj.iter().enumerate() {
                dst[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
            }
        }
        // The 88-byte depth-pass push TEMPLATE: `use_model_matrix == 1` (@84). The recorder
        // overwrites `view_proj` (@0..64) per cascade + `base_instance` (@80) per caster batch.
        let mut push = [0u8; GBUFFER_PUSH_BYTES];
        push[84..88].copy_from_slice(&1u32.to_le_bytes());
        (push, cascade_view_proj, fit.active_count)
    });

    // Shadow Phase 5 Inc-1-GPU: upload the spot atlas fit into the atlas UBO, then build the punctual
    // depth-pass activation — the per-slot `view_proj` byte blocks (the depth loop stamps `[s]` into
    // the push @0) + the active layer count. The push TEMPLATE carries `use_model_matrix == 1` (@84);
    // its leading 64 bytes are overwritten per slot so they are left zero here. The `csm.atlas*`
    // resources are ALWAYS created (the resolve binds @14/@15); `atlas_punctual` is `Some` only when
    // `cfg.spot_atlas` armed it.
    let spot_activation = cfg.spot_atlas.map(|fit| {
        csm.upload_atlas(device, &fit);
        let mut face_view_proj = [[0u8; 64]; MAX_TEXTURE_LAYERS];
        // Shadow Phase 5 Inc-2: the per-layer TYPE flag + the per-POINT-face `cam_eye@64` lane bytes
        // (`light_pos.xyz` + `inv_range`). A SPOT face leaves `face_is_point == false` (the lane
        // unused); a POINT face sets it `true` + stamps the lane so the FS computes the radial
        // distance. The demo's fits are already type-grouped (a SPOT fit is all-spot, a POINT cube
        // fit is six contiguous point faces), so the recorder binds each pipeline at most once.
        let mut face_is_point = [false; MAX_TEXTURE_LAYERS];
        let mut face_light = [[0u8; 16]; MAX_TEXTURE_LAYERS];
        for (slot, src) in fit.faces.iter().enumerate().take(fit.active_layers as usize) {
            for (i, f) in src.view_proj.iter().enumerate() {
                face_view_proj[slot][i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
            }
            face_is_point[slot] = src.is_point;
            // cam_eye lane: xyz = light_pos, w = inv_range.
            face_light[slot][0..4].copy_from_slice(&src.light_pos[0].to_le_bytes());
            face_light[slot][4..8].copy_from_slice(&src.light_pos[1].to_le_bytes());
            face_light[slot][8..12].copy_from_slice(&src.light_pos[2].to_le_bytes());
            face_light[slot][12..16].copy_from_slice(&src.inv_range.to_le_bytes());
        }
        let mut push = [0u8; GBUFFER_PUSH_BYTES];
        push[84..88].copy_from_slice(&1u32.to_le_bytes());
        (push, face_view_proj, face_is_point, face_light, fit.active_layers)
    });

    // M3: build the per-mesh draw batch LIST (one `GBufferMeshDraw` per registered mesh,
    // carrying its `base_instance` bucket offset + O3 index width), borrowing each mesh's GPU
    // buffers from `instanced_gpu`. Empty (the legacy path) ⇒ `scene.mesh_draw == &[]` ⇒ the
    // recorder keeps the byte-identical `cmd_draw`. The slice is built BEFORE the scene so the
    // scene's `mesh_draw: &[..]` borrow is valid for the frame call.
    let mesh_draws: Vec<GBufferMeshDraw> = instanced_gpu
        .as_ref()
        .map(|g| {
            g.batches
                .iter()
                .map(|b| GBufferMeshDraw {
                    vertex_buffer: &b.vertex_buffer,
                    index_buffer: &b.index_buffer,
                    index_count: b.index_count,
                    index_type: b.index_type,
                    base_instance: b.base_instance,
                    instance_count: b.instance_count,
                    casts_shadow: b.casts_shadow,
                })
                .collect()
        })
        .unwrap_or_default();

    let mvp = cfg.mvp;
    let mut scene = GBufferScene {
        raster_pipeline: &raster_pipeline,
        vertex_buffer: &vertex_buffer,
        vertex_count: vertices.len() as u32,
        mvp,
        // M3: the SHARED instance SSBO bind group — the gather-filled N-instance ring on the
        // instanced path (the recorder binds it ONCE, every batch indexes `base_instance +
        // SV_InstanceID`), or the 1-element identity dummy on the legacy path (bound-but-unread,
        // `use_model_matrix == 0`).
        instance_bind_group: instanced_gpu
            .as_ref()
            .map_or(&instance_bind_group, |g| &g.instance_bind_group),
        marcher: &marcher,
        vocab_layout: &vocab_layout,
        edit_list: &edit_list,
        camera_ring: &camera_ring,
        tiles_buffer: &tiles_buffer,
        pointer_grid: clipmap.grid_buffer(0),
        atlas: clipmap.atlas(0).texture(),
        atlas_sampler: clipmap.sampler(0),
        level_grids: [clipmap.grid_buffer(1), clipmap.grid_buffer(2)],
        level_atlases: [clipmap.atlas(1).texture(), clipmap.atlas(2).texture()],
        level_atlas_samplers: [clipmap.sampler(1), clipmap.sampler(2)],
        // MDF Stage-2c (binding 15): bind the REAL mesh-SDF texture when the showcase carries one
        // (and arm `mesh_sdf_enabled` so the marcher marches `sdf_soft_shadow_mesh`); else bind the
        // brick atlas as a benign placeholder + gate OFF (bound-but-unread — the R2 contract).
        mesh_sdf: mesh_sdf_texture
            .as_ref()
            .map_or_else(|| clipmap.atlas(0).texture(), |t| t.texture()),
        mesh_sdf_sampler: mesh_sdf_texture
            .as_ref()
            .map_or_else(|| clipmap.sampler(0), |t| t.sampler()),
        mesh_sdf_enabled,
        depth_sampler: &depth_sampler,
        material_table: &material_table,
        light_table: &light_table,
        light_staging: &light_staging,
        light_upload_bytes: light_table_bytes,
        light_dirty: false,
        // L1 cluster cull OFF (NON-CLUSTERED): the frozen `cluster_cull.hlsl` drops a
        // shadow-flagged punctual, so the multi-light SDF-shadow path runs on the flat-table
        // (non-clustered) resolve — exactly `p6_r1_multi_light_sdf_shadows_match_oracle`'s path.
        cluster_cull: None,
        cull_layout: None,
        cluster_grid: None,
        light_index: None,
        light_index_alloc: None,
        cluster_cull_push: [0u8; 16],
        cluster_count: 0,
        resolve_pipeline: &resolve_pipeline,
        resolve_layout: &resolve_layout,
        #[cfg(feature = "hwrt")]
        resolve_pipeline_hwrt: None,
        #[cfg(feature = "hwrt")]
        resolve_layout_hwrt: None,
        #[cfg(feature = "hwrt")]
        resolve_tlas_hwrt: None,
        // Rung 1b: the HWRT resolve is OFF in this harness (`resolve_tlas_hwrt: None`), so the
        // shadow-params UBO ring is bound by NO set — a benign valid placeholder (the whole cascade
        // UBO ring, a per-FIF `[BoundBuffer; FRAMES_IN_FLIGHT]`, host-coherent + >= 16 B/slot)
        // satisfies the field type without ever being read.
        #[cfg(feature = "hwrt")]
        ray_shadow_ubo: csm.csm_ring(),
        present_pipeline: &present_pipeline,
        present_layout: &present_layout,
        present_sampler: &present_sampler,
        dispatch_group_count_x: group_count_x(),
        // The analytic marcher (brick OFF) is the crisp reference path for the showcase.
        brick: None,
        coarse: None,
        coarse_mode: CoarseMode::EmptySkipOnly,
        // The real on-screen lit flags: A1 soft shadows + A2 AO. The marcher marches the A1 soft
        // shadow toward `light_dir` (the sun) into `gMaterial.r`, which the resolve's PRIMARY
        // directional consumes — so the sphere/box cast a real shadow ACROSS the SDF floor.
        lighting_flags: LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO,
        // The sun direction (`L`, direction TO the light) — the table's first directional, so the
        // marched cast shadow lands where the resolve lights from (`marcher_light_dir`; byte-identical
        // to the prior hardcoded `SHOWCASE_SUN_DIR` for every showcase that uses it).
        light_dir: marcher_light_dir(&cfg.light_elems),
        // Render P7 / P7-Q2: SSAO is ON only when `cfg.ssao_quality` selected a variant — the
        // recorder then records the SSAO pass (BETWEEN the marcher→resolve barrier and the resolve)
        // that writes the `ssao` lane the resolve combines (`ssao_mode == 1`, armed on `light_header`
        // above), and the contact creases / floor-body junctions darken. `None` = SSAO OFF: NO SSAO
        // pass + `ssao_mode == 0` (the byte-identical 0%-gate `_off` reference for the quality ladder).
        ssao: cfg
            .ssao_quality
            .map(|_| SsaoActivation { pipeline: &ssao_pipeline, layout: &ssao_layout }),
        // M3: when the config carried instanced meshes, pass A runs the batch loop — one
        // INSTANCED INDEXED draw per registered mesh, each at its `base_instance` bucket
        // (the `use_model_matrix == 1` arm — `cfg.mvp` set its byte 84). Every legacy scene
        // leaves `instanced == None` ⇒ an EMPTY slice ⇒ `record_gbuffer` keeps the
        // byte-identical legacy `cmd_draw`.
        mesh_draw: &mesh_draws,
        // CSM Increment 1b (Rung A): the cascade trio bound at resolve @12/@13 (ALWAYS). The depth
        // pass is `Some` only when `cfg.csm` armed it (a real `view_proj` uploaded above) — then the
        // recorder renders the SAME instanced caster batches into cascade layer 0 from the sun POV,
        // and the resolve `min`-combines the exact hard shadow onto the floor. OFF ⇒ bound-but-unread.
        csm_cascade_texture: &csm.cascade,
        csm_compare_sampler: &csm.sampler,
        csm_cascade_ring: csm.csm_ring(),
        csm: csm_activation.map(|(push, cascade_view_proj, active_count)| CsmDepthActivation {
            pipeline: &csm.depth_pipeline,
            push,
            cascade_view_proj,
            active_count,
            shadow_dim: CSM_SHADOW_DIM,
        }),
        // Shadow Phase 5 Inc-1-GPU: the atlas trio bound at resolve @14/@15 (ALWAYS). The punctual
        // depth pass is `Some` only when `cfg.spot_atlas` armed it (a real `view_proj` uploaded
        // above) — then the recorder renders the SAME instanced caster batches into atlas layer 0
        // from the SPOT POV, and the resolve MULTIPLIES the exact hard shadow into the spot's
        // contribution INSIDE the cone. OFF ⇒ bound-but-unread. The depth pass reuses the CSM
        // `depth_pipeline` (SPOT uses NDC-z like a cascade — `csm_depth.vs/fs` verbatim).
        shadow_atlas_texture: &csm.atlas,
        shadow_atlas_sampler: &csm.atlas_sampler,
        shadow_atlas_ubo: &csm.atlas_ubo,
        // SDFDDGI I1: the 3 DDGI resolve bindings (@16/@17/@18) now bind the REAL probe atlas. The GI
        // gate is OFF on every golden present (LightBuf word-7 bit 4 == 0), so the resolve's probe
        // sample never runs and all three are bound-but-unread (the 0%-gate — byte-identical pixels).
        // I1 severs the I0a dummy: the irradiance/depth atlases are the dedicated
        // `B10G11R11_UFLOAT`/`R16G16_SFLOAT` `Texture2DArray`s, each sampled with a dedicated LINEAR
        // (non-comparison) sampler — closing the VUID trap (the old CSM COMPARISON sampler on a
        // non-Dref SampleLevel was UB). The grid UBO is the dedicated zeroed `ddgi_ubo`.
        ddgi_irr_texture: csm.ddgi_atlas.irradiance(),
        ddgi_irr_sampler: csm.ddgi_atlas.sampler(),
        ddgi_depth_texture: csm.ddgi_atlas.depth(),
        ddgi_depth_sampler: csm.ddgi_atlas.sampler(),
        ddgi_grid_ubo: &csm.ddgi_ubo,
        // SDFDDGI I2: the probe-update pass is OFF in these harness scenes (the GI-OFF 0%-gate).
        // `ddgi_update = None` ⇒ no update RDG pass / dispatch / barrier is recorded. The
        // classification / ray-table / update-UBO handles are supplied so the RDG sink can resolve
        // them (unread while the pass is off); the ray-table + update-UBO reuse the bound-but-unread
        // `ddgi_ubo` as a placeholder buffer (never read on the OFF path — this harness does not arm
        // the update pass; a bench/host that arms it supplies real dedicated buffers).
        ddgi_update: None,
        ddgi_classification: csm.ddgi_atlas.classification(),
        ddgi_ray_table: &csm.ddgi_ubo,
        ddgi_update_ubo: &csm.ddgi_ubo,
        atlas_punctual: spot_activation.map(
            |(push, face_view_proj, face_is_point, face_light, active_layers)| {
                PunctualDepthActivation {
                    pipeline: &csm.depth_pipeline,
                    point_pipeline: &csm.point_depth_pipeline,
                    push,
                    face_view_proj,
                    face_is_point,
                    face_light,
                    active_layers,
                    shadow_dim: SPOT_SHADOW_DIM,
                }
            },
        ),
        // Pillar B B3: interp OFF at construction. Every DUMP path leaves it None (byte-
        // identical command stream). The INTERACTIVE branch below rebuilds `scene.interp =
        // Some(..)` per frame with the current-slot draw-SSBO set + this frame's overstep alpha.
        interp: None,
        // HW-RT rung R0: GPU timing OFF (the golden/interactive showcase; byte-identical).
        gpu_timing: None,
        // HW-RT rung R2a-3: the per-frame TLAS pack + build OFF (byte-identical command stream).
        #[cfg(feature = "hwrt")]
        tlas: None,
        // HW-RT rung 3a: the spatial (à-trous) RT soft-shadow denoise OFF (byte-identical).
        #[cfg(feature = "hwrt")]
        shadow: None,
        // HW-RT rung 3a: the STABLE denoise-set-build signals — all OFF in this harness (no denoise
        // sets built; byte-identical).
        #[cfg(feature = "hwrt")]
        resolve_layout_denoise_hwrt: None,
        #[cfg(feature = "hwrt")]
        atrous_layout_denoise_hwrt: None,
        #[cfg(feature = "hwrt")]
        shadow_denoise_enabled: false,
        #[cfg(feature = "hwrt")]
        shadow_denoise_final_is_vis2: false,
        // Rung-3b step 5a: the temporal-MV mesh path — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        temporal_enabled: false,
        #[cfg(feature = "hwrt")]
        raster_pipeline_mv: None,
        #[cfg(feature = "hwrt")]
        mv_bind_group: None,
        // F8-mv: the combined MV+PM mesh path — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        raster_pipeline_mvpm: None,
        #[cfg(feature = "hwrt")]
        mvpm_bind_group: None,
        // Rung-3b step 5b: the SDF motion-vector VIS path — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        vis_mv_pipeline: None,
        #[cfg(feature = "hwrt")]
        vis_mv_layout: None,
        #[cfg(feature = "hwrt")]
        motion_cam_ubo_ring: None,
        // Rung-3b step 6: the temporal reproject layout — OFF in this harness (byte-identical).
        #[cfg(feature = "hwrt")]
        temporal_layout: None,
        // Asset-streaming plan F8: PER_INSTANCE_MATERIAL is OFF in this low-level RHI harness
        // (no ECS gather / material store exists here) — byte-identical to the pre-F8 stream.
        pm_enabled: false,
        raster_pipeline_pm: None,
        pm_bind_group: None,
        // Textured-PBR T6c: TEXTURED is OFF in this low-level RHI harness (no ECS gather /
        // texture asset store exists here) — byte-identical to the pre-T6c stream.
        tex_enabled: false,
        raster_pipeline_tex: None,
        tex_bind_group: None,
        bindless_set: None,
    };

    let present_extent = VkExtent2D { width: COMPOSITE_W, height: COMPOSITE_H };
    let staging_size = (swapchain.extent().width * swapchain.extent().height * 4) as u64;
    let staging = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: staging_size,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible readback staging buffer");
    let alloc_extent = swapchain.extent();
    let mut frame = GBufferFrame::new();

    // INTERACTIVE BRANCH (scripted shadow / interp diagnostics): instead of the one-shot readback dump,
    // around the live scene with WASD + mouse-look. The heavy setup above is SHARED verbatim; this
    // branch only rebuilds the camera (the b5 UBO @0 + `scene.mvp`) per frame and presents to the
    // window in a loop, then falls through to the SAME teardown below. `present_extent`/`staging`
    // are created right after this block, so the interactive loop builds its own present extent.
    if interactive {
        // Pillar B B3: build the interpolation pre-pass resources for the viewer's instanced room.
        // Every rendered instance is routed through the pair path: the room's hand affines are
        // decomposed into `Trs` and seeded `prev == curr` (a STILL instance the B2 keystone renders
        // bitwise-stable), and the interactive loop / smoke moves a chosen instance by advancing its
        // `curr`. `None` when there is no instanced mesh (the legacy identity-dummy path — interp
        // would have nothing to interpolate). The interp draw SSBO the compute writes IS what the
        // raster/shadow VS reads (bound as `scene.instance_bind_group` per frame slot).
        let base_pairs: Vec<InterpPair> = cfg
            .instanced
            .as_ref()
            .map(|inst| {
                inst.meshes
                    .iter()
                    .flat_map(|m| m.affines.iter())
                    .map(|a| {
                        let trs = trs_from_affine(a);
                        InterpPair { prev: trs, curr: trs }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let interp_gpu = (!base_pairs.is_empty())
            .then(|| InterpGpu::create(device, &instance_layout, base_pairs.len() as u32));

        // Shadow-motion A/B diagnostic: `BOYKO_SHADOW_AB=1` swaps the input-driven loop for the
        // scripted-camera capture protocol (static-arrival vs motion-arrival byte comparison at
        // one pose — see `run_shadow_motion_ab`). The heavy setup above + teardown below are
        // shared verbatim, so the A/B frames exercise the EXACT production viewer path.
        if std::env::var_os("BOYKO_SHADOW_LAG").is_some() {
            // Shadow-lag diagnostic: in-motion capture vs settled capture at the identical pose
            // along the owner's exact reported walk (see `run_shadow_lag`).
            run_shadow_lag(
                ctx,
                surface,
                &mut swapchain,
                &mut renderer,
                window,
                &mut scene,
                &mut frame,
                &staging,
                is_bgra,
            );
        } else if std::env::var_os("BOYKO_SHADOW_DOLLY").is_some() {
            // Shadow-dolly diagnostic: walk the camera toward the slab's front face and table
            // the center-pixel shadow state vs view_z / the CSM cascade select (the owner's
            // "a shadow appears on the face as I approach" report). See `run_shadow_dolly`.
            run_shadow_dolly(
                ctx,
                surface,
                &mut swapchain,
                &mut renderer,
                window,
                &mut scene,
                &mut frame,
                &staging,
                is_bgra,
            );
        } else if std::env::var_os("BOYKO_SHADOW_AB").is_some() {
            run_shadow_motion_ab(
                ctx,
                surface,
                &mut swapchain,
                &mut renderer,
                window,
                &mut scene,
                &mut frame,
                &staging,
                is_bgra,
            );
        } else if std::env::var_os("BOYKO_INTERP_SMOKE").is_some() {
            // Pillar B B3 GPU KEYSTONE (`BOYKO_INTERP_SMOKE=1`): the scripted 2-alpha readback proof
            // of the wired interp pass. Renders the SAME production scene through the interp pre-pass
            // at alpha=0.0 then alpha≈0.5, with instance 0 given a MOVING pair (prev != curr) and the
            // rest STILL, and asserts the moving instance's pixels DIFFER between alphas while a still
            // instance's pixels are bitwise-IDENTICAL — the B2 keystone on GPU.
            let interp = interp_gpu
                .as_ref()
                .expect("BOYKO_INTERP_SMOKE needs an instanced scene (the viewer_config supplies one)");
            run_interp_smoke(
                ctx, surface, &mut swapchain, &mut renderer, window, &mut scene, &mut frame,
                &staging, is_bgra, interp, &base_pairs,
            );
        }
        // Skip the dump path entirely; fall through to teardown. `scene` borrows `mesh_draws` + the
        // instanced GPU buffers; its last use is the interactive-diagnostic harness call above (or none), so NLL ends
        // those borrows here and the teardown is free to move/destroy them. (`GBufferScene` is not
        // `Drop`, so an explicit `drop(scene)` would only flag clippy's `drop_non_drop`.)
        drop(renderer);
        // SAFETY: identical contract to the dump path's teardown — the renderer was dropped above
        // (its `Drop` waits the device idle), so no submission references these resources; `ctx` is
        // still alive; each is destroyed exactly once, in reverse dependency order.
        unsafe {
            // Pillar B B3: tear down the interp pipeline + FIF-ringed pair/draw SSBOs + bind groups
            // FIRST (they reference no other resource here; the renderer drop above idled the device).
            if let Some(interp) = interp_gpu {
                interp.destroy(ctx);
            }
            frame.destroy(ctx);
            csm.destroy(ctx);
            RhiDevice::destroy_graphics_pipeline(device, present_pipeline);
            RhiDevice::destroy_bind_group_layout(device, present_layout);
            RhiDevice::destroy_compute_pipeline(device, ssao_pipeline);
            RhiDevice::destroy_bind_group_layout(device, ssao_layout);
            RhiDevice::destroy_compute_pipeline(device, resolve_pipeline);
            RhiDevice::destroy_bind_group_layout(device, resolve_layout);
            RhiDevice::destroy_compute_pipeline(device, marcher);
            RhiDevice::destroy_bind_group_layout(device, vocab_layout);
            RhiDevice::destroy_graphics_pipeline(device, raster_pipeline);
            drop(mesh_draws);
            if let Some(g) = instanced_gpu {
                RhiDevice::destroy_bind_group(device, g.instance_bind_group);
                RhiDevice::destroy_buffer(device, g.instance_ssbo);
                for b in g.batches {
                    RhiDevice::destroy_buffer(device, b.index_buffer);
                    RhiDevice::destroy_buffer(device, b.vertex_buffer);
                }
            }
            RhiDevice::destroy_bind_group(device, instance_bind_group);
            RhiDevice::destroy_buffer(device, instance_buffer);
            RhiDevice::destroy_bind_group_layout(device, instance_layout);
            RhiDevice::destroy_sampler(device, present_sampler);
            RhiDevice::destroy_sampler(device, depth_sampler);
            RhiDevice::destroy_buffer(device, vertex_buffer);
            RhiDevice::destroy_buffer(device, tiles_buffer);
            if let Some(t) = mesh_sdf_texture {
                t.destroy(ctx);
            }
            clipmap.destroy(ctx);
            RhiDevice::destroy_buffer(device, light_staging);
            RhiDevice::destroy_buffer(device, light_table);
            RhiDevice::destroy_buffer(device, material_table);
            for slot in camera_ring {
                RhiDevice::destroy_buffer(device, slot);
            }
            RhiDevice::destroy_buffer(device, edit_list);
        }
        drop(swapchain);
        // surface / ctx / window are owned by `with_windowed_present` and dropped in-order at its frame end.
        return;
    }

    // Render ONE readback frame, then drain so the staging buffer is host-coherent (the same
    // FRAMES_IN_FLIGHT==2 / 3-drain discipline the existing windowed dumps use). The readback is
    // a 4-B/texel BGRA-or-RGBA copy of the FULL swapchain image; `readback_to_rgba` normalizes
    // the swapchain R/B order so the dumped BMP is color-correct.
    const DRAIN_FRAMES: u32 = 3;
    let clear = [0.04_f32, 0.05, 0.07, 1.0];

    let mut dumped: Option<(Vec<u8>, u32, u32)> = None;
    if !window.pump_events() {
        eprintln!("NOTE engine_showcase_512: window closed before the dump frame — skipping");
    } else {
        window.refresh_size();
        let live = swapchain.extent();
        if live.width != alloc_extent.width || live.height != alloc_extent.height {
            eprintln!("NOTE engine_showcase_512: extent changed before the dump frame — skipping");
        } else {
            let token = renderer
                .wait_frame_in_flight()
                .expect("invariant: the frame slot fence wait precedes the submit");
            // SAFETY: `ctx`/`surface`/`swapchain`/`renderer` share one device; every `scene`
            // resource is live; `present_extent` + `scene.dispatch_group_count_x` + the camera UBO
            // `count` cover the composite extent; `staging` is host-visible and ≥ one swapchain
            // image in bytes.
            let presented = unsafe {
                renderer.render_gbuffer_frame(
                    token, ctx, surface, &mut swapchain, &scene, &mut frame,
                    window.width(), window.height(), clear, present_extent, Some(&staging),
                )
            }
            .unwrap_or_else(|e| panic!("showcase readback frame failed: {e:?}"));

            if !presented {
                eprintln!("NOTE engine_showcase_512: swapchain recreated on the readback frame — skipping");
            } else {
                let extent = swapchain.extent();
                for _ in 0..DRAIN_FRAMES {
                    if !window.pump_events() {
                        break;
                    }
                    window.refresh_size();
                    let token = renderer
                        .wait_frame_in_flight()
                        .expect("invariant: the frame slot fence wait precedes the submit");
                    // SAFETY: same contract; no readback requested on the drain frames.
                    let _ = unsafe {
                        renderer.render_gbuffer_frame(
                            token, ctx, surface, &mut swapchain, &scene, &mut frame,
                            window.width(), window.height(), clear, present_extent, None,
                        )
                    }
                    .unwrap_or_else(|e| panic!("showcase drain frame failed: {e:?}"));
                }

                let w = extent.width;
                let h = extent.height;
                let byte_count = (w * h * 4) as usize;
                let dst_ptr = RhiDevice::buffer_mapped_ptr(device, &staging)
                    .expect("host-visible staging buffer is mapped");
                let mut raw = vec![0u8; byte_count];
                // SAFETY: `dst_ptr` points to `staging_size` (≥ `byte_count`) mapped host-coherent
                // bytes; the readback frame's copy completed before this read (its slot fence was
                // re-waited by the drain frames); `raw` is a distinct, non-overlapping alloc.
                unsafe { core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), raw.as_mut_ptr(), byte_count) };
                dumped = Some((readback_to_rgba(&raw, w, h, is_bgra), w, h));
            }
        }
    }

    if ctx.validation_enabled() {
        let state = ctx
            .debug_state()
            .expect("validation enabled => a debug-messenger state is present");
        assert_eq!(
            state.total(),
            0,
            "validation layer reported {} message(s) during the showcase present — \
             see the [vk-validation] log",
            state.total()
        );
    }

    // Write the TRUE 512×512 BMP (no upscale — the composite is already native) + verify the
    // dumped dimensions are exactly 512×512.
    match dumped {
        Some((rgba, w, h)) => {
            assert_eq!(
                (w, h),
                (COMPOSITE_W, COMPOSITE_H),
                "the readback must be the native {COMPOSITE_W}x{COMPOSITE_H} composite (no upscale)"
            );
            write_bmp(bmp_path, &rgba, w, h)
                .unwrap_or_else(|e| panic!("failed to write {bmp_path}: {e:?}"));
            let bytes = std::fs::read(bmp_path)
                .unwrap_or_else(|e| panic!("failed to re-read {bmp_path} for header verification: {e:?}"));
            let (bw, bh) = read_bmp_dimensions(&bytes)
                .expect("the dumped showcase must be a valid BM 54-byte-header BMP");
            assert_eq!(
                (bw, bh),
                (COMPOSITE_W as i32, COMPOSITE_H as i32),
                "the dumped BMP header must report {COMPOSITE_W}x{COMPOSITE_H} native dimensions"
            );
            println!("engine showcase dump -> {bmp_path} ({bw}x{bh} native, multi-light SDF shadows + SSAO)");
        }
        None => {
            eprintln!(
                "NOTE engine_showcase_512: no readback frame presented (swapchain kept recreating); \
                 no BMP written"
            );
        }
    }

    drop(renderer);
    // SAFETY: the renderer was dropped above (its `Drop` waits the device idle), so no submission
    // references these resources; `ctx` is still alive; each is destroyed exactly once, in reverse
    // dependency order.
    unsafe {
        frame.destroy(ctx);
        RhiDevice::destroy_buffer(device, staging);
        // CSM Increment 1b: the cascade trio + depth pipeline.
        csm.destroy(ctx);
        RhiDevice::destroy_graphics_pipeline(device, present_pipeline);
        RhiDevice::destroy_bind_group_layout(device, present_layout);
        RhiDevice::destroy_compute_pipeline(device, ssao_pipeline);
        RhiDevice::destroy_bind_group_layout(device, ssao_layout);
        RhiDevice::destroy_compute_pipeline(device, resolve_pipeline);
        RhiDevice::destroy_bind_group_layout(device, resolve_layout);
        RhiDevice::destroy_compute_pipeline(device, marcher);
        RhiDevice::destroy_bind_group_layout(device, vocab_layout);
        RhiDevice::destroy_graphics_pipeline(device, raster_pipeline);
        // M3 instanced-mesh resources: the per-mesh draw batches (each mesh's index + vertex
        // buffers) + the shared instance SSBO + its bind group, destroyed before the shared
        // `instance_layout` the bind group used. `mesh_draws` borrowed `instanced_gpu`, so it is
        // dropped first (the scene that used it was consumed by the frame call above).
        drop(mesh_draws);
        if let Some(g) = instanced_gpu {
            RhiDevice::destroy_bind_group(device, g.instance_bind_group);
            RhiDevice::destroy_buffer(device, g.instance_ssbo);
            for b in g.batches {
                RhiDevice::destroy_buffer(device, b.index_buffer);
                RhiDevice::destroy_buffer(device, b.vertex_buffer);
            }
        }
        // M1 instance-model resources (bind group → buffer → layout, after the pipeline).
        RhiDevice::destroy_bind_group(device, instance_bind_group);
        RhiDevice::destroy_buffer(device, instance_buffer);
        RhiDevice::destroy_bind_group_layout(device, instance_layout);
        RhiDevice::destroy_sampler(device, present_sampler);
        RhiDevice::destroy_sampler(device, depth_sampler);
        RhiDevice::destroy_buffer(device, vertex_buffer);
        RhiDevice::destroy_buffer(device, tiles_buffer);
        // MDF Stage-2c: destroy the mesh-SDF texture (if the showcase created one). The device is
        // idle (the renderer's Drop waited), so no submission still samples it.
        if let Some(t) = mesh_sdf_texture {
            t.destroy(ctx);
        }
        clipmap.destroy(ctx);
        RhiDevice::destroy_buffer(device, light_staging);
        RhiDevice::destroy_buffer(device, light_table);
        RhiDevice::destroy_buffer(device, material_table);
        for slot in camera_ring {
            RhiDevice::destroy_buffer(device, slot);
        }
        RhiDevice::destroy_buffer(device, edit_list);
    }
    drop(swapchain);
    // surface / ctx / window are owned by `with_windowed_present` and dropped in-order at its frame end.
}
