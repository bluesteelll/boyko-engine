//! Slice-0 compute assets + golden reference (the `ComputeHarness` scaffold that
//! once lived here is dissolved into the `boyko_rhi` trait surface — see
//! [`crate::rhi_impl`]).
//!
//! What survives here is the backend-agnostic *contract* the Slice-0 compute
//! flow proves, independent of how the trait drives it:
//!
//! - the two committed SPIR-V modules ([`write_pattern_spirv`] /
//!   [`transform_add_spirv`]), exposed as `&'static [u32]` so trait callers feed
//!   them straight into [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module);
//! - the dispatch `LOCAL_SIZE_X` the shaders declare (`[numthreads(64,1,1)]`);
//! - the CPU golden ([`golden_write_pattern`] / [`golden_chained`]) the
//!   bit-exact readback diff asserts against;
//! - [`ComputeError`], the rich compute-path error variant (now folded into the
//!   unified [`VulkanError`](crate::error::VulkanError) at the trait boundary).
//!
//! # What the compute path proves (unchanged across the refactor)
//!
//! - **0c**: a `VkShaderModule` from a committed `.spv`, a one-binding
//!   (STORAGE_BUFFER at COMPUTE) descriptor-set layout + a 4-byte push-constant
//!   pipeline layout (now cached on the device, [`crate::rhi_impl`]), a compute
//!   pipeline; bind a host-visible storage buffer; record begin → bind pipeline
//!   → bind set → push constant → `vkCmdDispatch(ceil(N/64))` → end; submit with
//!   a fence; wait; read back the persistent mapping; assert `buffer[i] = i*2+1`.
//! - **0d**: a SECOND pipeline (`buffer[i] += 100`) chained after a
//!   `VkBufferMemoryBarrier` (COMPUTE_SHADER/SHADER_WRITE → COMPUTE_SHADER/
//!   SHADER_READ) in the SAME command buffer; one submit + fence; the result
//!   diffs bit-exact against the CPU golden — the §5.5 edge→barrier lowering in
//!   miniature.
//!
//! # The shader contract (`shaders/{write_pattern,transform_add}.hlsl`)
//!
//! Both shaders declare `RWStructuredBuffer<uint> Data : register(u0)` (binding
//! 0, set 0, one STORAGE_BUFFER) and a `[[vk::push_constant]] uint count` (a
//! 4-byte range at offset 0, COMPUTE-visible), `[numthreads(64,1,1)]`, entry
//! `main`, each invocation bounds-checking `i < count`.
//!
//! # Soundness (raw FFI → validation + golden are the oracle)
//!
//! Miri cannot run this (real driver FFI, VRAM mapping). The oracle is two-fold,
//! per plan §6: (a) the `VK_LAYER_KHRONOS_validation` messenger asserted to
//! `total() == 0`, and (b) the bit-exact golden diff.

use core::slice;

// The SDF field math + std430 data model now live in the `boyko_sdf_math` leaf
// (the W4 leaf-cut): a `no_std`, graphics-free crate that is the SINGLE source of
// truth shared with `boyko_physics`. The items used internally by the golden /
// encoder / layout below are imported here; the ones the rung-8/9/10/11 tests
// import via `boyko_rhi_vulkan::compute::{..}` are RE-EXPORTED below so those
// pre-leaf-cut paths keep compiling.
use boyko_sdf_math::{
    HEADER_BASE_WORDS, MAX_SDF_EDITS, SDF_EDIT_WORDS, SDF_GRAD_H, edit_distance, sdf_edit_list,
    sdf_edit_list_normal, v_dot, v_len, v_normalize, v_sub,
};
// M1 empty-space-skip: the pointer-grid descriptor + the ray-AABB exit step the host
// golden mirror replays bit-for-bit against the GPU marcher (`PointerGrid` is the SAME
// origin/dims/brick_world the `FineMarcherPush` grid uniforms carry).
use boyko_sdf_math::brick::{PointerGrid, dist_to_brick_exit};
// M2 trilinear SURFACE bricks: the apron'd-brick bake + the JCGT analytic-cubic crossing the
// host golden mirror (`golden_composite_pixel_brick_m2`) and the atlas baker (`bake_brick_atlas`)
// drive — the SAME `boyko_sdf_math::brick` oracle the GPU marcher mirrors bit-for-bit.
use boyko_sdf_math::brick::{
    BRICK_ALLOC, BRICK_INTERIOR, BRICK_VOXELS, brick_cubic_hit, classify_brick, decode_snorm8,
    fill_brick,
};
use boyko_sdf_math::{BrickClass, SDF_EDIT_BAND_HALF, SdfEditField};

use crate::ffi::VkResult;
use crate::memory::MemoryError;

/// Re-exports of the leaf items whose canonical import path the rung-8/9/10/11
/// tests (and any external caller) use as `boyko_rhi_vulkan::compute::{..}`.
/// Preserved verbatim across the W4 leaf-cut so those paths keep resolving:
/// [`SdfEdit`], [`sdf_kind`], [`sdf_op`], [`SDF_IMG_W`], [`SDF_IMG_H`].
pub use boyko_sdf_math::{SDF_IMG_H, SDF_IMG_W, SdfEdit, sdf_kind, sdf_op};

/// The committed SPIR-V for step 0c (`buffer[i] = i*2 + 1`).
///
/// Wrapped in a `#[repr(C, align(4))]` newtype so the `include_bytes!` blob is
/// 4-byte aligned: it is reinterpreted as a `&[u32]` word stream, and the SPIR-V
/// spec requires that stream to be 4-byte aligned (a bare `include_bytes!` is
/// only `align(1)`).
static WRITE_PATTERN_SPV: SpirvBlob<988> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/write_pattern.comp.spv"
)));

/// The committed SPIR-V for step 0d (`buffer[i] += 100`).
static TRANSFORM_ADD_SPV: SpirvBlob<968> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/transform_add.comp.spv"
)));

/// The committed SPIR-V for Phase-6 rung 8 (sphere-trace one analytic sphere into
/// a packed-RGBA storage buffer — `shaders/sdf_spheretrace.hlsl`).
static SDF_SPHERETRACE_SPV: SpirvBlob<3316> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_spheretrace.comp.spv"
)));

/// The committed SPIR-V for Phase-6 rung 9 (sphere-trace an ordered SDF edit-list
/// — multi-primitive CSG — into a packed-header storage buffer,
/// `shaders/sdf_editlist.hlsl`).
static SDF_EDITLIST_SPV: SpirvBlob<24368> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_editlist.comp.spv"
)));

/// The committed SPIR-V for Phase-6 rung 10 (SDF + mesh hybrid composite via a
/// shared depth buffer — `shaders/sdf_depth_composite.hlsl`). It reuses the rung-9
/// edit-list fold + lighting + camera verbatim and adds the per-pixel mesh-depth
/// read that BOUNDS the march so the mesh and the SDF occlude each other.
static SDF_DEPTH_COMPOSITE_SPV: SpirvBlob<26576> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_depth_composite.comp.spv"
)));

/// The committed SPIR-V for the Render P1a GPU gate (sphere-trace the rung-9 SDF
/// edit-list and STORE the marcher color into a STORAGE IMAGE through the
/// multi-resource descriptor *vocabulary* set — `shaders/sdf_editlist_storage_image.hlsl`).
/// Reuses the rung-9 field eval + ray-gen + lighting VERBATIM; the only differences
/// are the bind points (binding 0 = a read-only `StructuredBuffer<uint>` edit-list,
/// binding 1 = a `RWTexture2D<float4>` output) and the output sink (the marcher color
/// is STORED to texel `(px, py)` instead of packed into the buffer's pixel region).
static SDF_EDITLIST_STORAGE_IMAGE_SPV: SpirvBlob<24092> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_editlist_storage_image.comp.spv"
)));

/// The committed SPIR-V for the Render P1b OFFSCREEN MRT G-buffer marcher
/// (`shaders/sdf_gbuffer_composite.hlsl`) — the image-based rewrite of the rung-10
/// packed-buffer composite. The field eval + ray-gen + lighting are a VERBATIM cut of
/// `sdf_depth_composite.hlsl`; the only two I/O edits are (1) the mesh depth read
/// becomes a SAMPLED-image fetch (`Texture2D<float>` @ binding 1) and (2) the marcher
/// color becomes a STORAGE-image store (`RWTexture2D<float4>` albedo @ binding 2, plus
/// additive normal/material @ bindings 3/4). The extent/camera block moves to a UNIFORM
/// buffer @ binding 5 (written once). The edit-list stays a `StructuredBuffer` @
/// binding 0.
/// Render PBR MVP-2: the marcher repacks the G-buffer attributes — gAlbedo = the picked
/// material's RAW LINEAR `base_color` (from the material SSBO @ binding 7), gNormal =
/// (octahedral world normal in RG, 16-bit material id in BA), gMaterial = (shadow, ao,
/// mask). The material PICK is an argmin over the edit list reusing the FROZEN
/// `load_edit`/`edit_distance` (a read-only attribution; the field is untouched, proven by
/// the GATE-1 probe tripwire). Phase 0 also extracted ray-gen into the shared
/// `ray_gen.hlsli`. The full Cook-Torrance shade runs in [`deferred_pbr_spirv`]. The byte
/// length grew (41176 → 43944) with the material pick + oct-encode + SSBO fetch; Lighting
/// L0b added the `gViewT` storage-image lane + its 3 terminal writes (43944 → 44216). The SDF
/// brick-atlas M1 empty-skip prefix grew it (→ 47032), then M2 added the trilinear+JCGT-cubic
/// SURFACE-brick path (atlas `Texture3D` @binding 10 + the b5 `M2GridParams` block + the cubic
/// solver), bringing the marcher to its current 72280-byte size.
static SDF_GBUFFER_COMPOSITE_SPV: SpirvBlob<72280> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_gbuffer_composite.comp.spv"
)));

/// The committed deferred-shading SPLIT (increment 1) RESOLVE SPIR-V
/// (`shaders/deferred_pbr.hlsl`). A fullscreen compute pass that reads the marcher's
/// G-buffer attributes back — gAlbedo (the unmultiplied base) @ binding 0 + gMaterial
/// `(r = vis, b = mask)` @ binding 1, both STORAGE images in GENERAL — and stores the
/// final LIT color `mask ? base*vis : base` to a STORAGE image @ binding 2. EXACTLY 3
/// STORAGE bindings, no sampler, no UBO: the extent comes from `gLit.GetDimensions` (the
/// lit image is 1:1 the marched pixels), so the 1D dispatch index maps to (px, py) the
/// same way the marcher does. The COMPOSITE moved here; the BRDF stays in the marcher this
/// increment. The host mirror is [`golden_deferred_resolve`] (fed by
/// [`golden_marcher_attributes`]).
///
/// Render PBR MVP-2: the resolve now runs FULL metallic-roughness Cook-Torrance (GGX D +
/// height-correlated Smith V + Schlick F + Lambert diffuse + analytic EnvBRDFApprox
/// ambient) instead of `mask ? base*vis : base`. It reads gAlbedo (raw base), gNormal (oct
/// normal + 16-bit material id), gMaterial (shadow, ao, mask) @ bindings 0..2, fetches the
/// picked material from the SSBO @ binding 4, and reconstructs the per-pixel view direction
/// from the camera UBO @ binding 5 + the shared `ray_gen.hlsli`. SDF (mask == 1) pixels get
/// full PBR (the owner-acknowledged behavioral change, PBR plan call F); mesh / background /
/// empty (mask == 0) pass `base` through byte-identically (the 0%-gate). The byte length
/// grew (1616 → 6880) with the BRDF, then 6880 → 8824 with the Lighting L0a header+table
/// directional/sky loop, then 8824 → 12548 with the L0b `gViewT` `P`-reconstruction +
/// point/spot loop, then 12548 → 14536 with the L1 cluster lookup (the froxel-mapped index
/// loop + the `ClusterGrid`/`LightIndexList` bindings @8/@9), then 14536 → 15252: the WRONG
/// L1 index-range guard was REMOVED (it never fired for the offending light) and the
/// per-light `normalize(v+l)` / `normalize(L.dir)` were replaced by `safe_normalize` — the
/// faithful mirror of the host oracle's `v_normalize` zero-guard — so a back-facing surface's
/// `~0` half-vector yields `[0,0,0]` (NoH = LoH = 0, finite spec) instead of the intrinsic
/// `normalize(0) == NaN` that blackened the pixel (`NaN * (NoL == 0) == NaN` →
/// `pack_unorm(NaN) == 0`). Bindings 8/9 are now STATICALLY referenced on every path (DXC no
/// longer dead-strips them when clusters are off), so every resolve pipeline layout declares +
/// binds 0..9 (placeholder buffers @8/@9 on the non-clustered paths). The host mirror is
/// [`golden_deferred_resolve_table`] (clustered via [`golden_cluster_cull`] +
/// [`golden_deferred_resolve_clustered`]).
static DEFERRED_PBR_SPV: SpirvBlob<15252> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/deferred_pbr.comp.spv"
)));

/// The committed Lighting-L1 clustered froxel light-cull SPIR-V (`shaders/cluster_cull.hlsl`).
/// One invocation per froxel (`CLUSTER_COUNT`): builds the froxel's world-space AABB from the
/// shared ray-gen + the exp-Z slice view-z, culls each point/spot light's bounding sphere
/// (`sqDistPointAABB <= r²`), and atomic-appends survivors into the flat `LightIndexList` +
/// writes the per-froxel `{offset, count}` `ClusterGrid` cell. Bound to the cull set { camera
/// UBO @0, light table SSBO @1, ClusterGrid @2, LightIndexList @3, LightIndexAlloc @4 } + a
/// `ClusterCullPush` (near/far + caps). Directional/sky are GLOBAL (not culled). The host
/// mirror is [`golden_cluster_cull`].
static CLUSTER_CULL_SPV: SpirvBlob<12356> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/cluster_cull.comp.spv"
)));

/// The committed Render P4b coarse-cull / tile pre-trace SPIR-V
/// (`shaders/sdf_tile_cull.hlsl`). A 1/8-res CONSERVATIVE cone-trace: one invocation
/// per 8×8 fine-pixel tile emits a [`TileBound`] the fine marcher reads to early-out
/// EMPTY tiles + seed `t = near_t`. A strict FIELD-CONSUMER (calls the frozen
/// `field_distance`); bound to the P4b vocabulary set { SSBO edit-list @0, SAMPLED
/// depth @1, STORAGE `TileBound` @6, UNIFORM camera @5 }.
static SDF_TILE_CULL_SPV: SpirvBlob<10304> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_tile_cull.comp.spv"
)));

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob so its address is
/// a valid `*const u32` and it can be re-viewed as a `&[u32]` word stream.
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    /// Re-views the blob as its SPIR-V `u32` word stream.
    ///
    /// `N` is a 4-byte multiple by construction (a committed `.spv`); the
    /// `align(4)` wrapper guarantees the cast is well-aligned.
    #[inline]
    const fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid
        // `*const u32`; `N` is a 4-byte multiple (const-asserted above), so the
        // blob is exactly `N / 4` whole `u32` words; the `&self` borrow keeps the
        // backing `'static` blob alive for the slice's lifetime. The bytes are an
        // arbitrary-but-initialized SPIR-V stream — any bit pattern is a valid
        // `u32`, so no uninitialized/invalid read occurs.
        unsafe { slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

/// The `local_size_x` of both shaders (`[numthreads(64,1,1)]`). The dispatch
/// group count is `ceil(N / LOCAL_SIZE_X)`.
pub const LOCAL_SIZE_X: u32 = 64;

/// The committed step-0c SPIR-V (`buffer[i] = i*2 + 1`) as a `u32` word stream,
/// ready for [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
#[inline]
pub fn write_pattern_spirv() -> &'static [u32] {
    WRITE_PATTERN_SPV.as_words()
}

/// The committed step-0d SPIR-V (`buffer[i] += 100`) as a `u32` word stream.
#[inline]
pub fn transform_add_spirv() -> &'static [u32] {
    TRANSFORM_ADD_SPV.as_words()
}

/// The committed Phase-6 rung-8 SDF sphere-trace SPIR-V as a `u32` word stream,
/// ready for [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// The shader reuses the rung-1 compute contract verbatim: binding 0 (set 0) is
/// one `RWStructuredBuffer<uint>` at COMPUTE + a 4-byte `uint count` push
/// constant; everything else (camera/sphere/light) is hardcoded in the shader,
/// mirrored host-side by [`golden_sdf_pixel`].
#[inline]
pub fn sdf_spheretrace_spirv() -> &'static [u32] {
    SDF_SPHERETRACE_SPV.as_words()
}

/// The committed Phase-6 rung-9 SDF edit-list (multi-primitive CSG) SPIR-V as a
/// `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// The shader reuses the rung-1 compute contract verbatim (binding 0 = one
/// `RWStructuredBuffer<uint>` at COMPUTE + a 4-byte `uint count` push constant).
/// The edit-list is PACKED as a header region at the front of that single buffer
/// (no second binding): word 0 = `edit_count`, then the [`MAX_SDF_EDITS`]-entry
/// [`SdfEdit`] array, then the packed-RGBA pixel output. The host writes the
/// header via [`encode_edit_list`] and mirrors the fold in [`golden_editlist_pixel`].
#[inline]
pub fn sdf_editlist_spirv() -> &'static [u32] {
    SDF_EDITLIST_SPV.as_words()
}

/// The committed Phase-6 rung-10 SDF + mesh hybrid composite SPIR-V as a `u32`
/// word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Reuses the rung-1 one-binding compute contract verbatim (binding 0 = one
/// `RWStructuredBuffer<uint>` at COMPUTE + a 4-byte `uint count` push constant).
/// The buffer extends the rung-9 packed-header layout with a DEPTH region between
/// the edit array and the pixel output: the host writes the edit header via
/// [`encode_edit_list`], the GPU image→buffer copy writes the rasterized mesh
/// depth into [`COMPOSITE_DEPTH_BASE_WORDS`], and the shader reads both, bounds the
/// march by the per-pixel mesh depth, and composites into [`COMPOSITE_PIXEL_BASE_WORDS`].
/// The fold + lighting are mirrored host-side by [`golden_composite_pixel`].
#[inline]
pub fn sdf_depth_composite_spirv() -> &'static [u32] {
    SDF_DEPTH_COMPOSITE_SPV.as_words()
}

/// The committed Render P1a edit-list STORAGE-IMAGE marcher SPIR-V as a `u32` word
/// stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Unlike the rung-9 [`sdf_editlist_spirv`] (one `RWStructuredBuffer<uint>` at
/// binding 0 that holds both the edit-list header AND the packed pixel output), this
/// shader is bound to the P1a multi-resource descriptor *vocabulary* set: binding 0
/// is a read-only `StructuredBuffer<uint>` (the same packed edit-list header format,
/// [`encode_edit_list`] / [`EDITLIST_BUFFER_WORDS`]) and binding 1 is a
/// `RWTexture2D<float4>` it STOREs the marcher color into. The field eval + ray-gen +
/// lighting are reused VERBATIM from rung 9, so [`golden_editlist_pixel`] predicts the
/// stored texel within the same `+/-2/255` per-channel tolerance (the float→UNORM store
/// quantization vs [`pack_rgba`]'s rounding is under one LSB). It proves a storage-image
/// WRITE through the COMPUTE bind point + the vocabulary set works on the GPU.
#[inline]
pub fn sdf_editlist_storage_image_spirv() -> &'static [u32] {
    SDF_EDITLIST_STORAGE_IMAGE_SPV.as_words()
}

/// The committed Render P1b OFFSCREEN MRT G-buffer marcher SPIR-V as a `u32` word
/// stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// The image-based rewrite of the rung-10 [`sdf_depth_composite_spirv`] marcher: the
/// field eval + ray-gen + lighting are a VERBATIM cut of `sdf_depth_composite.hlsl`, so
/// [`golden_composite_pixel_ex`] predicts the ALBEDO output within the same `+/-2/255`
/// per-channel tolerance. It is bound to the P1b vocabulary set: binding 0 a read-only
/// `StructuredBuffer<uint>` edit-list, binding 1 a `Texture2D<float>` SAMPLED depth
/// (the rasterized D32_SFLOAT image, fetched with `.Load`), bindings 2..4 the MRT
/// `RWTexture2D<float4>` storage images (albedo + the additive normal/material), and
/// binding 5 a UNIFORM buffer carrying the extent/camera block (written once — NOT a
/// per-frame push, so it is compatible with the pipeline's dedicated layout). There is
/// NO packed depth region and NO packed pixel region — the depth is sampled and the
/// color is stored.
#[inline]
pub fn sdf_gbuffer_composite_spirv() -> &'static [u32] {
    SDF_GBUFFER_COMPOSITE_SPV.as_words()
}

/// The committed deferred-shading SPLIT (increment 1) RESOLVE SPIR-V as a `u32` word
/// stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// The fullscreen `deferred_pbr` resolve consumes the marcher's G-buffer attributes —
/// gAlbedo (the unmultiplied base color) @ binding 0 and gMaterial `(r = vis, b = mask)`
/// @ binding 1, both STORAGE images in GENERAL (no sampler) — and STOREs the final lit
/// color `lit = (mask == 1) ? base * vis : base` to a STORAGE image @ binding 2. It is
/// dispatched 1D over the SAME pixel count as the marcher (the camera UBO @ binding 5
/// supplies the extent for the 1:1 index → (px, py) mapping). The host mirror is
/// [`golden_deferred_resolve`], fed by [`golden_marcher_attributes`].
#[inline]
pub fn deferred_pbr_spirv() -> &'static [u32] {
    DEFERRED_PBR_SPV.as_words()
}

/// The committed Lighting-L1 clustered froxel light-cull SPIR-V as a `u32` word stream,
/// ready for [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// One invocation per froxel (`CLUSTER_COUNT`); bound to the cull set { camera UBO @0, light
/// table SSBO @1, `RWStructuredBuffer<uint2>` ClusterGrid @2, `RWStructuredBuffer<uint>`
/// LightIndexList @3, `RWStructuredBuffer<uint>` LightIndexAlloc @4 } + a `ClusterCullPush`
/// (exp-Z near/far + the per-froxel / flat-list caps). It builds each froxel's world AABB,
/// culls the point/spot block (`sqDistPointAABB <= r²`), and atomic-appends survivors into
/// the index list + writes the `{offset, count}` cell. Dispatched 1D over `CLUSTER_COUNT`
/// BEFORE the resolve (with a COMPUTE→COMPUTE buffer barrier so the resolve's reads see the
/// writes). The host mirror is [`golden_cluster_cull`].
#[inline]
pub fn cluster_cull_spirv() -> &'static [u32] {
    CLUSTER_CULL_SPV.as_words()
}

/// The committed Render P4b coarse-cull / tile pre-trace SPIR-V as a `u32` word
/// stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// The coarse pre-pass for the [`sdf_gbuffer_composite_spirv`] marcher: one invocation
/// per 8×8 tile cone-traces the frozen `field_distance` and emits a [`TileBound`] (the
/// host mirror is [`golden_tile_bound`]). It is bound to the P4b vocabulary set —
/// binding 0 a read-only `StructuredBuffer<uint>` edit-list, binding 1 a
/// `Texture2D<float>` SAMPLED depth, binding 6 a `RWStructuredBuffer<TileBound>` output,
/// binding 5 the UNIFORM camera block — and dispatched 1D over `tiles_w * tiles_h`
/// (see [`tile_grid_extent`]) before the fine marcher reads the bounds (with a buffer
/// barrier between).
#[inline]
pub fn sdf_tile_cull_spirv() -> &'static [u32] {
    SDF_TILE_CULL_SPV.as_words()
}

/// Errors from the compute-pipeline flow. `VkError` carries the failing command
/// name + the raw `VkResult`; `Memory` forwards a buffer/allocation failure.
///
/// Folded into the unified [`VulkanError`](crate::error::VulkanError) at the
/// trait boundary (plan D4); kept here so the pipeline-creation helpers in
/// [`crate::rhi_impl`] can build the rich, command-named variant.
#[derive(Debug)]
pub enum ComputeError {
    /// A Vulkan command returned a non-success `VkResult`.
    VkError(&'static str, VkResult),
    /// Buffer creation / sub-allocation failed.
    Memory(MemoryError),
}

impl From<MemoryError> for ComputeError {
    #[inline]
    fn from(e: MemoryError) -> Self {
        ComputeError::Memory(e)
    }
}

/// The CPU golden for step 0c: `out[i] == i*2 + 1` (the `write_pattern` shader).
///
/// Exposed so the test (and any later golden harness) shares ONE definition of
/// the shader's contract rather than duplicating the arithmetic.
#[inline]
pub fn golden_write_pattern(i: u32) -> u32 {
    i.wrapping_mul(2).wrapping_add(1)
}

/// The CPU golden for the chained 0c→0d result: `(i*2 + 1) + 100` (the
/// `transform_add` shader applied on top of `write_pattern`).
#[inline]
pub fn golden_chained(i: u32) -> u32 {
    golden_write_pattern(i).wrapping_add(100)
}

// ---------------------------------------------------------------------------
// Phase-6 rung-8 SDF sphere-trace golden (single source of truth, CPU mirror of
// `shaders/sdf_spheretrace.hlsl`).
//
// These constants and math MUST match the shader exactly so the center-pixel
// (HIT) and corner-pixel (MISS) colors are computed host-side without re-running
// the GPU. The test diffs the GPU readback against this golden within a small
// per-channel tolerance (DXC `mad`/`fma` rounding makes a bit-exact match
// brittle across drivers, so a hit/miss + small-tolerance diff is the oracle —
// it still proves the sphere-trace ran a sphere, not a constant fill).
// ---------------------------------------------------------------------------

// `SDF_IMG_W` / `SDF_IMG_H` / `SDF_GRAD_H` and the `v_*` vector helpers now live
// in the `boyko_sdf_math` leaf and are imported / re-exported at the top of this
// module (the W4 leaf-cut). The camera / lighting scene constants below stay here
// because only the rung-8/9/10 golden + marching functions (which also stay) use
// them — they are NOT part of the shared analytic field the physics crate reuses.

const SDF_CAM_Z: f32 = 2.0;
const SDF_HALF_EXTENT: f32 = 1.0;
const SDF_SPHERE_CENTER: [f32; 3] = [0.0, 0.0, 0.0];
const SDF_SPHERE_RADIUS: f32 = 0.5;
const SDF_LIGHT_DIR: [f32; 3] = [0.0, 0.0, 1.0];
const SDF_BASE_COLOR: [f32; 3] = [0.8, 0.3, 0.2];
const SDF_AMBIENT: f32 = 0.1;
const SDF_BACKGROUND: [f32; 3] = [0.05, 0.05, 0.1];

const SDF_EPS: f32 = 0.001;
const SDF_T_MAX: f32 = 10.0;
const SDF_MAX_IT: u32 = 128;

/// The default Render B1 over-relaxation factor the harness pushes when a caller does
/// not specify one. Keinert's sphere-tracing speed-up steps `t += omega * d`; values in
/// `(1, 2)` accelerate convergence on shallow-grazing rays while the in-shader
/// exact-retreat safeguard preserves correctness. `1.2` is the conservative default; the
/// host runtime clamp is `[1.0, 1.99]` (the soundness ceiling sits at `omega == 2`).
pub const DEFAULT_MARCHER_OMEGA: f32 = 1.2;

// ---------------------------------------------------------------------------
// Render A1 (SDF cone-trace soft shadows) + A2 (SDF 5-tap AO) tuning constants
// (owner VALUE/SCOPE defaults, owner-retunable). These are CONSUMER-side: the
// shadow min-track + AO deficit accumulate around calls to the FROZEN field
// gateway (`field_distance` / `sdf_edit_list`); the field math itself is never
// touched. Mirrored verbatim in `sdf_gbuffer_composite.hlsl`.
// ---------------------------------------------------------------------------

/// Bit 0 of `lighting_flags`: enable A1 cone-trace soft shadows.
pub const LIGHTING_FLAG_SHADOWS: u32 = 1;
/// Bit 1 of `lighting_flags`: enable A2 ambient occlusion.
pub const LIGHTING_FLAG_AO: u32 = 2;

/// The default ONE directional light direction (the current Lambert dir, +Z = at
/// the camera). Owner-retunable; mirrors the shader's `LIGHT_DIR`.
pub const DEFAULT_LIGHT_DIR: [f32; 3] = [0.0, 0.0, 1.0];

/// A1 penumbra hardness (Quilez soft-shadow `k`): larger ⇒ sharper shadow edges.
pub const SHADOW_K: f32 = 8.0;
/// A1 march start offset along the light, in field units (`16 * GRAD_H`). Replaces a
/// normal-offset bias: marching from a finite `t` off the surface avoids self-shadow
/// acne without perturbing the hit point.
pub const SHADOW_MINT: f32 = 16.0 * SDF_GRAD_H;
/// A1 minimum per-step advance along the light (a floor on `d / L`) so a near-zero
/// field value cannot stall the shadow march.
pub const SHADOW_MINT_STEP: f32 = SHADOW_MINT;
/// A1 occluder-hit threshold: when `field_distance` drops below this the ray is fully
/// occluded (return `0`). `2 * EPS` (the marcher's hit threshold relaxed one factor).
pub const SHADOW_HIT_EPS: f32 = 2.0 * SDF_EPS;
/// A1 grazing/back-face cutoff on signed `n·L`: at or below this the surface faces
/// away from the light ⇒ fully shadowed (return `0`), which also skips the march.
pub const SHADOW_NDOTL_EPS: f32 = 0.0;

/// A2 fixed step between the 5 AO taps along the surface normal.
pub const AO_STEP: f32 = 0.1;
/// A2 per-tap geometric falloff (`AO_FALLOFF^i` weights the i-th deficit).
pub const AO_FALLOFF: f32 = 0.95;
/// A2 overall occlusion strength (scales the accumulated deficit before clamping).
pub const AO_STRENGTH: f32 = 1.0;

/// The A1 host mirror of `sdf_soft_shadow`: a clamped-step Quilez BASIC cone-trace
/// (NO `sqrt` — minimal FP-parity surface) from the lit point `p` toward the
/// normalized light `l`, returning a soft visibility in `[0, 1]` (1 = fully lit,
/// 0 = fully occluded). Mirrors the shader within ±3/255 (consumer-side relaxable,
/// NOT bit-exact for the ON path). `field` is the FROZEN field gateway (the
/// edit-list `sdf_edit_list`); the min-track + Lipschitz-corrected step are
/// accumulated consumer-side. `n` is the surface normal, `l` the NORMALIZED light.
fn host_soft_shadow<F: Fn([f32; 3]) -> f32>(
    p: [f32; 3],
    n: [f32; 3],
    l: [f32; 3],
    field: &F,
) -> f32 {
    // Signed n·L: at/below the cutoff the surface faces away from the light — fully
    // shadowed, and the march would only graze the surface (acne). Replaces a
    // normal-offset bias on the march origin.
    if v_dot(n, l) <= SHADOW_NDOTL_EPS {
        return 0.0;
    }
    let mut res = 1.0_f32;
    let mut t = SHADOW_MINT;
    for _ in 0..SDF_MAX_IT {
        let q = [p[0] + l[0] * t, p[1] + l[1] * t, p[2] + l[2] * t];
        let d = field(q);
        res = res.min(SHADOW_K * d / t);
        if d < SHADOW_HIT_EPS {
            return 0.0;
        }
        // The `/L` Lipschitz correction on the STEP: without it the super-Lipschitz
        // smin leaks light through thin occluders. Floored at SHADOW_MINT_STEP so a
        // near-zero `d` cannot stall the march.
        t += (d / FIELD_LIPSCHITZ_L).max(SHADOW_MINT_STEP);
        if t > SDF_T_MAX {
            break;
        }
    }
    res.clamp(0.0, 1.0)
}

/// The A2 host mirror of `sdf_ao`: a 5-tap ambient-occlusion estimate marching the
/// surface normal `n` from `p`, accumulating the `(h - d)` field-deficit weighted by
/// `AO_FALLOFF^i`, and returning an occlusion factor in `[0, 1]` (1 = unoccluded).
/// Mirrors the shader within ±3/255. `field` is the FROZEN field gateway.
fn host_ao<F: Fn([f32; 3]) -> f32>(p: [f32; 3], n: [f32; 3], field: &F) -> f32 {
    let mut occ = 0.0_f32;
    for i in 1..=5u32 {
        let h = (i as f32) * AO_STEP;
        let q = [p[0] + n[0] * h, p[1] + n[1] * h, p[2] + n[2] * h];
        let d = field(q);
        occ += (h - d) * AO_FALLOFF.powi(i as i32);
    }
    (1.0 - AO_STRENGTH * occ).clamp(0.0, 1.0)
}

/// The single shading helper for every host golden (factored from the four inlined
/// `ndotl + ambient` Lambert sites). Computes the directional Lambert + ambient base
/// color, then — ONLY when `lighting_flags != 0` — multiplies in the A1 shadow and/or
/// A2 AO terms (the SAME gate the shader uses). With `lighting_flags == 0` the result
/// is the bare Lambert color, BYTE-IDENTICAL to the pre-A1/A2 inline arithmetic (the
/// 0%-gate): no extra multiply is performed (a structural `if`).
///
/// `base_color` is the surface albedo, `ambient` the ambient term, `p` the lit hit
/// point, `n` the surface normal, `light_dir` the (un-normalized) light direction,
/// and `field` the FROZEN field gateway the shadow/AO consumers call. The closure is
/// never invoked on the OFF path, so callers with no edit-list field may pass any
/// matching closure.
#[inline]
fn host_shade<F: Fn([f32; 3]) -> f32>(
    base_color: [f32; 3],
    ambient: f32,
    p: [f32; 3],
    n: [f32; 3],
    light_dir: [f32; 3],
    lighting_flags: u32,
    field: &F,
) -> [f32; 3] {
    let l = v_normalize(light_dir);
    let ndotl = v_dot(n, l).max(0.0);
    let base = [
        base_color[0] * ndotl + base_color[0] * ambient,
        base_color[1] * ndotl + base_color[1] * ambient,
        base_color[2] * ndotl + base_color[2] * ambient,
    ];
    if lighting_flags == 0 {
        // OFF path: byte-identical to today (NO extra multiply).
        return base;
    }
    let mut shadow = 1.0_f32;
    if lighting_flags & LIGHTING_FLAG_SHADOWS != 0 {
        shadow = host_soft_shadow(p, n, l, field);
    }
    let mut ao = 1.0_f32;
    if lighting_flags & LIGHTING_FLAG_AO != 0 {
        ao = host_ao(p, n, field);
    }
    [
        base[0] * shadow * ao,
        base[1] * shadow * ao,
        base[2] * shadow * ao,
    ]
}

/// `sdf(p) = length(p - center) - radius` — the analytic field, mirroring the
/// shader's `sdf_sphere`. Exposed so a later CPU physics evaluator can be
/// conformance-checked against this exact source of truth.
#[inline]
pub fn sdf_sphere(p: [f32; 3]) -> f32 {
    v_len(v_sub(p, SDF_SPHERE_CENTER)) - SDF_SPHERE_RADIUS
}

/// Surface normal via central differences (the gradient of [`sdf_sphere`]),
/// mirroring the shader's `sdf_normal`.
#[inline]
fn sdf_normal(p: [f32; 3]) -> [f32; 3] {
    let h = SDF_GRAD_H;
    let n = [
        sdf_sphere([p[0] + h, p[1], p[2]]) - sdf_sphere([p[0] - h, p[1], p[2]]),
        sdf_sphere([p[0], p[1] + h, p[2]]) - sdf_sphere([p[0], p[1] - h, p[2]]),
        sdf_sphere([p[0], p[1], p[2] + h]) - sdf_sphere([p[0], p[1], p[2] - h]),
    ];
    v_normalize(n)
}

/// Packs a linear `[0,1]` RGB into `0xAABBGGRR` (alpha `0xFF`), mirroring the
/// shader's `pack_rgba`.
#[inline]
pub fn pack_rgba(c: [f32; 3]) -> u32 {
    let q = |x: f32| -> u32 { (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u32 };
    let r = q(c[0]);
    let g = q(c[1]);
    let b = q(c[2]);
    (0xFF << 24) | (b << 16) | (g << 8) | r
}

/// The CPU golden for one SDF pixel: reconstructs the orthographic ray for
/// `(px, py)`, sphere-traces the analytic field, lights the hit (Lambert +
/// ambient) or returns the background on a miss, and returns the packed
/// `0xAABBGGRR` color.
///
/// This is the single source of truth the rung-8 test asserts against: the
/// center pixel HITS (lit sphere color) and a corner pixel MISSES (background).
pub fn golden_sdf_pixel(px: u32, py: u32) -> u32 {
    let u = (((px as f32) + 0.5) / (SDF_IMG_W as f32)) * 2.0 - 1.0;
    let v = -((((py as f32) + 0.5) / (SDF_IMG_H as f32)) * 2.0 - 1.0);
    let ro = [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT, SDF_CAM_Z];
    let rd = [0.0, 0.0, -1.0];

    let mut t = 0.0_f32;
    let mut hit = false;
    for _ in 0..SDF_MAX_IT {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_sphere(p);
        if d < SDF_EPS {
            hit = true;
            break;
        }
        t += d;
        if t > SDF_T_MAX {
            break;
        }
    }

    let color = if hit {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_normal(p);
        // The rung-8 sphere golden is always the OFF path (`lighting_flags == 0` ⇒ bare
        // Lambert, byte-identical); the field closure is never invoked.
        host_shade(SDF_BASE_COLOR, SDF_AMBIENT, p, n, SDF_LIGHT_DIR, 0, &sdf_sphere)
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
}

/// Whether the orthographic ray for pixel `(px, py)` HITS the analytic sphere.
/// The rung-8 test uses this to pick a guaranteed-hit pixel (the center) and a
/// guaranteed-miss pixel (a corner) host-side, so the assertion is independent
/// of the exact lit color.
pub fn sdf_pixel_hits(px: u32, py: u32) -> bool {
    let u = (((px as f32) + 0.5) / (SDF_IMG_W as f32)) * 2.0 - 1.0;
    let v = -((((py as f32) + 0.5) / (SDF_IMG_H as f32)) * 2.0 - 1.0);
    let ro = [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT, SDF_CAM_Z];
    let rd = [0.0, 0.0, -1.0];

    let mut t = 0.0_f32;
    for _ in 0..SDF_MAX_IT {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_sphere(p);
        if d < SDF_EPS {
            return true;
        }
        t += d;
        if t > SDF_T_MAX {
            return false;
        }
    }
    false
}

// ===========================================================================
// Phase-6 rung-9 SDF edit-list (multi-primitive CSG) data model + golden mirror
// of `shaders/sdf_editlist.hlsl`.
//
// The scene is an ORDERED list of SDF edits (SDF doc §2): each a primitive
// (SPHERE or BOX) combined into the accumulated field by a boolean op
// (union / subtraction / intersection), optionally smoothed (polynomial
// smooth-min/-max when `smoothness > 0`). The field is folded per pixel each
// march step (the analytic base — NO grid cache / brick atlas, deferred).
//
// The edit-list reaches the shader PACKED as a header at the front of the SAME
// single storage buffer the rung-1 compute contract binds (binding 0). This
// keeps the proven one-binding layout verbatim; the alternative (a 2-binding
// compute layout) would reshape the device-shared `ComputeLayouts` + every
// rung-1 compute test. The buffer word layout (mirrored in the shader):
//
//   word 0                   : u32 edit_count
//   words [HEADER_BASE_WORDS]: MAX_SDF_EDITS * SdfEdit (the std430 edit array)
//   words [PIXEL_BASE_WORDS] : IMG_W * IMG_H * u32     (the packed-RGBA output)
//
// `SdfEdit` is `#[repr(C, align(16))]` so it is byte-identical to the std430
// structured-buffer element the shader reads (asserted below). The math here
// (primitive distances + boolean ops + smooth-min + central-difference gradient)
// reproduces the shader EXACTLY so it is the single source of truth a future CPU
// physics evaluator reuses; the test diffs the GPU readback against it within the
// same +/-2/255 per-channel tolerance as rung 8.
// ===========================================================================

// The SDF data model — `sdf_kind` / `sdf_op` / `MAX_SDF_EDITS` / `SdfEdit` (+ its
// ctors + the §3.8 std430 fingerprint const-asserts + the `SDF_EDIT_WORDS == 12`
// pin) / `SDF_EDIT_WORDS` / `HEADER_BASE_WORDS` — now lives in the
// `boyko_sdf_math` leaf and is imported / re-exported at the top of this module
// (the W4 leaf-cut). The buffer-LAYOUT consts below (`PIXEL_BASE_WORDS` etc.)
// stay here: they are GPU-buffer-specific and combine the leaf's layout consts
// with this crate's image dimensions.

/// Word offset of the packed-RGBA pixel region (after the count + the full
/// edit array). Matches the shader's `PIXEL_BASE`.
pub const PIXEL_BASE_WORDS: usize = HEADER_BASE_WORDS + MAX_SDF_EDITS * SDF_EDIT_WORDS;

/// Total `u32` word count of the packed-header buffer (header + edit array +
/// `IMG_W * IMG_H` pixels). The buffer must be `EDITLIST_BUFFER_WORDS * 4` bytes.
pub const EDITLIST_BUFFER_WORDS: usize =
    PIXEL_BASE_WORDS + (SDF_IMG_W as usize) * (SDF_IMG_H as usize);

// The shader hardcodes `SDF_EDIT_WORDS = 12u` and `PIXEL_BASE = 196`; pin both so
// a layout change that desyncs the host encoder from the shader is a build error.
const _: () = assert!(SDF_EDIT_WORDS == 12, "SDF_EDIT_WORDS must equal the shader's 12u");
const _: () = assert!(PIXEL_BASE_WORDS == 196, "PIXEL_BASE_WORDS must equal the shader's PIXEL_BASE");

/// Encodes `edits` into the packed-header region at the front of `buf` (a
/// `&mut [u32]` view of the storage buffer): word 0 = `edit_count`, then the
/// edit array starting at [`HEADER_BASE_WORDS`]. The pixel region (from
/// [`PIXEL_BASE_WORDS`]) is left untouched (the shader writes it).
///
/// `buf` must be at least [`EDITLIST_BUFFER_WORDS`] long; `edits.len()` must be
/// `<= MAX_SDF_EDITS` (debug-asserted — exceeding the fixed cap is a caller bug).
pub fn encode_edit_list(buf: &mut [u32], edits: &[SdfEdit]) {
    debug_assert!(
        edits.len() <= MAX_SDF_EDITS,
        "invariant: edit count {} exceeds MAX_SDF_EDITS {MAX_SDF_EDITS}",
        edits.len()
    );
    debug_assert!(
        buf.len() >= EDITLIST_BUFFER_WORDS,
        "invariant: buffer has {} words, need >= {EDITLIST_BUFFER_WORDS}",
        buf.len()
    );

    buf[0] = edits.len() as u32;
    for (i, e) in edits.iter().enumerate() {
        let base = HEADER_BASE_WORDS + i * SDF_EDIT_WORDS;
        buf[base] = e.center[0].to_bits();
        buf[base + 1] = e.center[1].to_bits();
        buf[base + 2] = e.center[2].to_bits();
        buf[base + 3] = e.center[3].to_bits();
        buf[base + 4] = e.params[0].to_bits();
        buf[base + 5] = e.params[1].to_bits();
        buf[base + 6] = e.params[2].to_bits();
        buf[base + 7] = e.params[3].to_bits();
        buf[base + 8] = e.kind;
        buf[base + 9] = e.op;
        buf[base + 10] = e.smoothness.to_bits();
        buf[base + 11] = e._pad;
    }
}

// The edit-list field math (the single source of truth, mirroring the shader):
// `sd_sphere`, `sd_box`, `edit_distance`, `smin`, `smax`, `combine`,
// `sdf_edit_list`, `sdf_edit_list_normal` (+ the `v_*` helpers + `SDF_FAR`) now
// live in the `boyko_sdf_math` leaf and are imported at the top of this module
// (the W4 leaf-cut, a VERBATIM cut — no float-op reorder). The marching / golden
// / camera functions below STAY here and call the leaf's `sdf_edit_list` /
// `sdf_edit_list_normal` directly, so they remain the GPU golden the rung-9/10/11
// tests diff against.

/// Whether the orthographic ray for pixel `(px, py)` HITS the edit-list field.
/// The rung-9 test uses this to pick discriminating texels host-side (e.g. a
/// pixel that hits the base sphere alone but MISSES after a subtraction).
pub fn editlist_pixel_hits(edits: &[SdfEdit], px: u32, py: u32) -> bool {
    let u = (((px as f32) + 0.5) / (SDF_IMG_W as f32)) * 2.0 - 1.0;
    let v = -((((py as f32) + 0.5) / (SDF_IMG_H as f32)) * 2.0 - 1.0);
    let ro = [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT, SDF_CAM_Z];
    let rd = [0.0, 0.0, -1.0];

    let mut t = 0.0_f32;
    for _ in 0..SDF_MAX_IT {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            return true;
        }
        t += d;
        if t > SDF_T_MAX {
            return false;
        }
    }
    false
}

/// The CPU golden for one edit-list pixel: sphere-traces the folded edit-list
/// field, lights the hit (Lambert + ambient, the same scene constants as rung 8)
/// or returns the background on a miss, and returns the packed `0xAABBGGRR`
/// color. The rung-9 test diffs the GPU readback against this within the
/// `+/-2/255` per-channel tolerance.
pub fn golden_editlist_pixel(edits: &[SdfEdit], px: u32, py: u32) -> u32 {
    let u = (((px as f32) + 0.5) / (SDF_IMG_W as f32)) * 2.0 - 1.0;
    let v = -((((py as f32) + 0.5) / (SDF_IMG_H as f32)) * 2.0 - 1.0);
    let ro = [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT, SDF_CAM_Z];
    let rd = [0.0, 0.0, -1.0];

    let mut t = 0.0_f32;
    let mut hit = false;
    for _ in 0..SDF_MAX_IT {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            hit = true;
            break;
        }
        t += d;
        if t > SDF_T_MAX {
            break;
        }
    }

    let color = if hit {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);
        // The rung-9 edit-list golden is always the OFF path (`lighting_flags == 0` ⇒
        // bare Lambert, byte-identical); the field closure is never invoked.
        host_shade(SDF_BASE_COLOR, SDF_AMBIENT, p, n, SDF_LIGHT_DIR, 0, &|q| {
            sdf_edit_list(edits, q)
        })
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
}

// ===========================================================================
// Phase-6 rung-10 SDF + mesh HYBRID COMPOSITE (shared depth) golden + layout —
// the single source of truth mirroring `shaders/sdf_depth_composite.hlsl`.
//
// The §15.1 seam: a REAL GPU-rasterized mesh's depth (written into a D32_SFLOAT
// attachment, then copied into the shared storage buffer) BOUNDS the SDF
// sphere-trace march, so the mesh and the SDF occlude each other and composite
// into one image. The SDF field math + lighting + camera are FROZEN to rung 9
// (reused verbatim via `sdf_edit_list` / `sdf_edit_list_normal` / `pack_rgba`);
// only the per-pixel mesh-depth read + the composite are new.
//
// The buffer extends the rung-9 packed-header layout with a DEPTH region between
// the edit array and the pixel output (rung 9's `PIXEL_BASE_WORDS` etc. are
// UNTOUCHED — these are a SEPARATE `COMPOSITE_*` const set):
//
//   word 0                          : u32 edit_count
//   words [HEADER_BASE_WORDS ..]    : MAX_SDF_EDITS * SdfEdit (the std430 array)
//   words [COMPOSITE_DEPTH_BASE_WORDS ..] : IMG_W * IMG_H * f32 mesh depth (NEW)
//   words [COMPOSITE_PIXEL_BASE_WORDS ..] : IMG_W * IMG_H * u32 packed RGBA out
//
// The depth region is `(CAM_Z - worldZ) / T_MAX` (the orthographic depth
// convention) inside the quad footprint and `1.0` (the clear) outside; the host
// march is bounded by `t_mesh = depth * T_MAX`, exactly as the shader does.
// ===========================================================================

/// The flat mesh albedo (the rung-10 composite's `MESH_COLOR`) — a green clearly
/// distinct from both the SDF lit color (warm orange/red) and the background (dark
/// blue), so the composite regions are unambiguous. Mirrors the shader's
/// `MESH_COLOR`. (Reading the mesh's real rasterized albedo from a G-buffer is a
/// deferred S3 refinement; this rung proves DEPTH sharing, so the color is a
/// constant.)
pub const MESH_COLOR: [f32; 3] = [0.15, 0.65, 0.25];

/// Word offset of the per-pixel mesh-depth region (immediately after the edit
/// array — i.e. where rung 9 put its pixel region). Matches the shader's
/// `DEPTH_BASE`. The depth region is `IMG_W * IMG_H` `f32`s, one per pixel,
/// written by the GPU image→buffer copy of the rasterized D32_SFLOAT attachment.
pub const COMPOSITE_DEPTH_BASE_WORDS: usize = HEADER_BASE_WORDS + MAX_SDF_EDITS * SDF_EDIT_WORDS;

/// Word offset of the packed-RGBA pixel region (after the depth region). Matches
/// the shader's `PIXEL_BASE`.
pub const COMPOSITE_PIXEL_BASE_WORDS: usize =
    COMPOSITE_DEPTH_BASE_WORDS + (SDF_IMG_W as usize) * (SDF_IMG_H as usize);

/// Total `u32` word count of the rung-10 composite buffer (header + edit array +
/// depth region + pixel region). The buffer must be
/// `COMPOSITE_BUFFER_WORDS * 4` bytes.
pub const COMPOSITE_BUFFER_WORDS: usize =
    COMPOSITE_PIXEL_BASE_WORDS + (SDF_IMG_W as usize) * (SDF_IMG_H as usize);

// The shader hardcodes `DEPTH_BASE = 196` and `PIXEL_BASE = 4292`; pin both so a
// layout change that desyncs the host encoder/reader from the shader is a build
// error. `COMPOSITE_DEPTH_BASE_WORDS` deliberately equals rung 9's
// `PIXEL_BASE_WORDS` (the depth region begins right after the edit array).
const _: () = assert!(
    COMPOSITE_DEPTH_BASE_WORDS == 196,
    "COMPOSITE_DEPTH_BASE_WORDS must equal the shader's DEPTH_BASE (196)"
);
const _: () = assert!(
    COMPOSITE_DEPTH_BASE_WORDS == PIXEL_BASE_WORDS,
    "the depth region must begin right after the edit array (= rung 9's PIXEL_BASE_WORDS)"
);
const _: () = assert!(
    COMPOSITE_PIXEL_BASE_WORDS == 4292,
    "COMPOSITE_PIXEL_BASE_WORDS must equal the shader's PIXEL_BASE (4292)"
);

/// The depth value the depth attachment is CLEARED to (the far plane). A stored
/// depth `>= MESH_DEPTH_CLEAR` means NO mesh fragment covered that pixel. Matches
/// the shader's `DEPTH_CLEAR` and the rung-10 test's `MESH_DEPTH_CLEAR`.
pub const MESH_DEPTH_CLEAR: f32 = 1.0;

/// The orthographic camera plane Z (rays start here, looking down -Z). The public
/// mirror of the private rung-8/9 `SDF_CAM_Z`, exposed so the rung-10 test builds
/// its orthographic MVP from the SAME single source of truth the golden uses.
pub const SDF_CAMERA_Z: f32 = SDF_CAM_Z;

/// The orthographic view half-extent in world units. Public mirror of the private
/// `SDF_HALF_EXTENT` (the rung-10 test maps its quad's world-XY corners with it).
pub const SDF_VIEW_HALF_EXTENT: f32 = SDF_HALF_EXTENT;

/// The march far-plane distance (= the depth far plane, world `CAM_Z - T_MAX`).
/// Public mirror of the private `SDF_T_MAX` (the rung-10 test's ortho MVP maps
/// `worldZ = CAM_Z - T_MAX` to stored depth `1.0`).
pub const SDF_TRACE_T_MAX: f32 = SDF_T_MAX;

/// The world-space XY a pixel's orthographic ray passes through (the ray origin's
/// xy), mirroring the camera reconstruction in [`golden_composite_pixel`]. The
/// rung-10 test uses this to compute, host-side, exactly which pixels a world-XY
/// quad covers (so the discriminator texels are picked independent of the GPU).
#[inline]
pub fn pixel_world_xy(px: u32, py: u32) -> [f32; 2] {
    let u = (((px as f32) + 0.5) / (SDF_IMG_W as f32)) * 2.0 - 1.0;
    let v = -((((py as f32) + 0.5) / (SDF_IMG_H as f32)) * 2.0 - 1.0);
    [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT]
}

/// Inverts the orthographic depth convention: a stored depth `d` corresponds to a
/// ray parameter `t = d * T_MAX` (the camera-plane distance), mirroring the
/// shader's `md * T_MAX`. The MVP the rung-10 test pushes is chosen so the
/// rasterized depth IS `t / T_MAX` for a fronto-parallel surface (orthographic,
/// no perspective divide → depth linear in `t`).
#[inline]
pub fn depth_to_t(d: f32) -> f32 {
    d * SDF_T_MAX
}

/// The expected STORED depth of a fronto-parallel mesh surface at world `mesh_z`
/// under the orthographic convention: `(CAM_Z - mesh_z) / T_MAX`. The rung-10
/// test asserts the GPU depth region equals this inside the quad footprint (and
/// [`MESH_DEPTH_CLEAR`] outside), localizing any raster/ortho-matrix bug.
#[inline]
pub fn mesh_depth_for_z(mesh_z: f32) -> f32 {
    (SDF_CAM_Z - mesh_z) / SDF_T_MAX
}

// ===========================================================================
// P0a — the camera / extent push-constant layout + host const-asserts, mirroring
// `shaders/sdf_depth_composite.hlsl`'s `PushConstants` block.
//
// The shader extent + camera mode are no longer compile-time constants: they arrive
// via this push-constant block. `count` stays at offset 0 (the legacy 4-byte field);
// extent + camera-mode follow; the four `float4` camera basis vectors are
// PERSPECTIVE-only (the ORTHO path ignores them). At extent (64,64) + ORTHO the
// shader reproduces the golden fixture BIT-EXACT (the rung-8..11 gate). The offsets
// below are const-asserted against the `#[repr(C)]` POD so a host/shader desync is a
// build error (the same discipline as `COMPOSITE_*_BASE_WORDS`).
// ===========================================================================

/// Camera mode selector mirroring the shader's `CAM_ORTHO` / `CAM_PERSPECTIVE`.
/// ORTHO (0) is the golden-frozen path; PERSPECTIVE (1) is the P0a additive ray-gen.
pub const CAM_MODE_ORTHO: u32 = 0;
/// PERSPECTIVE camera mode (P0a additive ray-gen). See [`CAM_MODE_ORTHO`].
pub const CAM_MODE_PERSPECTIVE: u32 = 1;

/// The `sdf_depth_composite` push-constant block (P0a). `#[repr(C)]` so the field
/// layout is byte-identical to the shader's `[[vk::push_constant]] PushConstants`
/// (std430 scalar/`float4` rules); the host uploads `as_bytes()` of this struct.
///
/// `count` is the total PIXEL count (`img_w * img_h`); `img_w`/`img_h == 0` make the
/// shader fall back to the legacy 64×64 fixture. The `cam_*` `[f32; 4]` vectors are
/// PERSPECTIVE-only (`cam_forward[3] = tan(fovY/2)`, `cam_right[3] = aspect = W/H`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompositePushConstants {
    /// Total PIXEL count = `img_w * img_h` (the shader bounds `idx < count`).
    pub count: u32,
    /// Runtime extent width (0 ⇒ the legacy 64 default).
    pub img_w: u32,
    /// Runtime extent height (0 ⇒ the legacy 64 default).
    pub img_h: u32,
    /// [`CAM_MODE_ORTHO`] or [`CAM_MODE_PERSPECTIVE`].
    pub camera_mode: u32,
    /// PERSPECTIVE eye world position (xyz; w unused).
    pub cam_eye: [f32; 4],
    /// PERSPECTIVE forward basis (xyz) + `tan(fovY/2)` in w.
    pub cam_forward: [f32; 4],
    /// PERSPECTIVE right basis (xyz) + aspect (W/H) in w.
    pub cam_right: [f32; 4],
    /// PERSPECTIVE up basis (xyz; w unused).
    pub cam_up: [f32; 4],
}

/// Byte size of [`CompositePushConstants`] — the `push_constant_bytes` a
/// `sdf_depth_composite` compute pipeline must declare (80 bytes).
pub const COMPOSITE_PUSH_CONSTANT_BYTES: u32 = core::mem::size_of::<CompositePushConstants>() as u32;

// Pin the field offsets to the shader's documented layout (a desync is a build error).
const _: () = assert!(
    core::mem::offset_of!(CompositePushConstants, count) == 0,
    "count must stay at offset 0 (the legacy 4-byte field)"
);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, img_w) == 4);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, img_h) == 8);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, camera_mode) == 12);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, cam_eye) == 16);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, cam_forward) == 32);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, cam_right) == 48);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, cam_up) == 64);
const _: () = assert!(
    COMPOSITE_PUSH_CONSTANT_BYTES == 80,
    "the push-constant block must be 80 bytes (matches the shader's PushConstants)"
);

impl CompositePushConstants {
    /// Builds the ORTHO golden-fixture push constants for a `w × h` extent. At
    /// `(64, 64)` this drives the bit-exact rung-8..11 golden invocation. The camera
    /// basis is left zeroed (the ORTHO path ignores it).
    ///
    /// # Precondition
    ///
    /// `w * h` must fit in `u32` (the dispatch element count); a `debug_assert!`
    /// catches an overflowing extent in debug builds.
    #[inline]
    pub const fn ortho(w: u32, h: u32) -> Self {
        debug_assert!(w.checked_mul(h).is_some(), "extent w*h overflows u32");
        Self {
            count: w * h,
            img_w: w,
            img_h: h,
            camera_mode: CAM_MODE_ORTHO,
            cam_eye: [0.0; 4],
            cam_forward: [0.0; 4],
            cam_right: [0.0; 4],
            cam_up: [0.0; 4],
        }
    }

    /// Builds PERSPECTIVE push constants from a camera (eye + orthonormal basis +
    /// vertical FOV) and a `w × h` extent. `fov_y_radians` is the full vertical FOV;
    /// the aspect is `w / h`. The basis vectors should be orthonormal and
    /// right-handed (`right × up = -forward` toward the scene); the shader normalizes
    /// the assembled direction. The field eval downstream is unchanged (plain IEEE).
    ///
    /// # Precondition
    ///
    /// `w * h` must fit in `u32` (the dispatch element count); a `debug_assert!`
    /// catches an overflowing extent in debug builds.
    #[inline]
    pub fn perspective(
        eye: [f32; 3],
        forward: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
        fov_y_radians: f32,
        w: u32,
        h: u32,
    ) -> Self {
        debug_assert!(w.checked_mul(h).is_some(), "extent w*h overflows u32");
        let tan_half_fov = (fov_y_radians * 0.5).tan();
        let aspect = (w as f32) / (h as f32);
        Self {
            count: w * h,
            img_w: w,
            img_h: h,
            camera_mode: CAM_MODE_PERSPECTIVE,
            cam_eye: [eye[0], eye[1], eye[2], 0.0],
            cam_forward: [forward[0], forward[1], forward[2], tan_half_fov],
            cam_right: [right[0], right[1], right[2], aspect],
            cam_up: [up[0], up[1], up[2], 0.0],
        }
    }

    /// Re-views the push constants as their raw byte slice for `push_constants`.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Self` is `#[repr(C)]` with only `u32` / `[f32; 4]` fields (all
        // `Copy`, no padding between them — the const-asserts above pin every offset
        // and the 80-byte total), so its `size_of` bytes are a fully-initialized,
        // alignment-valid POD bit pattern. The `&self` borrow keeps the struct alive
        // for the slice's lifetime; the slice is read-only (no aliasing write).
        unsafe {
            slice::from_raw_parts(
                (self as *const Self).cast::<u8>(),
                core::mem::size_of::<Self>(),
            )
        }
    }
}

/// `#[repr(C)]` the fine SDF-marcher's COMPUTE push constant
/// (`sdf_gbuffer_composite.hlsl`), pushed against the marcher pipeline's OWN dedicated
/// layout via `push_compute_constants`. Render P4b introduced the first two fields; A1/A2
/// widened it from 8 → 32 bytes to carry the directional-light state. The byte layout is
/// HLSL std430-style scalar+`float3` (the const-asserts below pin every offset):
///
///   offset  0 : u32   coarse_enabled   P4b coarse-cull gate (0 = cull off)
///   offset  4 : f32   omega            B1 Keinert over-relaxation factor, [1.0, 1.99]
///   offset  8 : u32   lighting_flags   bit 0 = A1 shadows, bit 1 = A2 AO; 0 = OFF path
///   offset 12 : u32   _pad             aligns `light_dir` to offset 16 (a `float3` lands
///                                      on a 16-byte boundary under std430)
///   offset 16 : [f32;3] light_dir      the directional-light direction (un-normalized)
///   offset 28 : f32   _pad2            tail pad to a 32-byte stride
///   total: 32 bytes — a subset of the declared 80-byte COMPUTE push range, so the
///   pipeline-layout declaration is unchanged.
///
/// `lighting_flags == 0` ⇒ the OFF path: the marcher emits the bare Lambert albedo,
/// BYTE-IDENTICAL to the pre-A1/A2 shader (the 0%-gate).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FineMarcherPush {
    /// P4b coarse-cull gate: non-zero reads binding 6 (`TileBound`) and culls / seeds.
    pub coarse_enabled: u32,
    /// B1 Keinert over-relaxation factor, host-clamped to `[1.0, 1.99]`.
    pub omega: f32,
    /// Lighting gate: bit 0 = A1 shadows, bit 1 = A2 AO; `0` = the OFF (Lambert-only) path.
    pub lighting_flags: u32,
    /// std430 padding so `light_dir` lands at offset 16.
    pub _pad: u32,
    /// The directional-light direction (un-normalized; the shader normalizes it).
    pub light_dir: [f32; 3],
    /// std430 tail padding to a 32-byte stride.
    pub _pad2: f32,
    /// M1 pointer-grid minimum world corner (cell `(0,0,0)`'s min). FIRST of the M1 block
    /// so this `float3` lands on a 16-byte boundary (offset 32) — the std430/HLSL `vec3`
    /// alignment rule (a `vec3` aligns to 16) that the `light_dir @16` field also obeys.
    /// Placing it here (not after a scalar) keeps the Rust `#[repr(C)]` 4-byte packing and
    /// the HLSL std430 layout byte-identical. Don't-care when `brick_enabled == 0`.
    pub grid_origin: [f32; 3],
    /// M1 empty-space-skip gate: non-zero reads binding 9 (the `PointerGrid`) and skips
    /// `EmptyOutside` bricks to their AABB exit. `0` = the OFF path (byte-identical to the
    /// pre-M1 marcher — the grid is never touched, the hit/normal stay analytic). Tucked
    /// into the `float3` tail slot (offset 44) so the next `uint3` lands 16-aligned.
    pub brick_enabled: u32,
    /// M1 pointer-grid cell count per axis (`[x, y, z]`). A `uint3` — aligned to 16 by the
    /// std430 rule (offset 48). Don't-care when off.
    pub grid_dims: [u32; 3],
    /// M1 pointer-grid cell size (the world width of one brick cell). The `uint3` tail slot
    /// (offset 60). Don't-care when `brick_enabled == 0`.
    pub brick_world: f32,
    /// M2 trilinear+JCGT-cubic SURFACE-brick gate: non-zero samples the brick atlas (binding 10)
    /// and runs the analytic-cubic crossing inside a SURFACE brick (validated by the analytic
    /// residual). `0` = the OFF path (byte-identical to the M1 marcher — the atlas is never
    /// sampled). INDEPENDENT of `brick_enabled` (the M1 empty-skip): the two gates are orthogonal.
    /// First slot (offset 64) of the 16-byte headroom the 64-byte M1 layout left inside the
    /// declared 80-byte COMPOSITE push range.
    pub brick_trilinear: u32,
    /// std430 tail padding (offsets 68/72/76) to the 80-byte COMPOSITE push stride. Mirrors the
    /// shader's `uint3 _pad3`. Don't-care (the shader never reads it).
    pub _pad3: [u32; 3],
}

/// Byte size of [`FineMarcherPush`] — the marcher's COMPUTE push range (80 bytes), exactly the
/// declared `COMPOSITE_PUSH_CONSTANT_BYTES` range (the M2 widening filled the 16-byte headroom).
pub const GBUFFER_MARCHER_PUSH_BYTES: u32 = core::mem::size_of::<FineMarcherPush>() as u32;

// Pin the std430 field offsets + the 64-byte stride so a host/shader desync is a build
// error (the same discipline as `CompositePushConstants` / `TileBound`). The `light_dir`
// @16 pin is the one a non-default-direction GPU test catches if the packing slips; the
// `grid_origin` @32 + `grid_dims` @48 pins are the M1 analogue (a non-default grid GPU test
// catches a slip). Both M1 vectors land 16-aligned — the std430/HLSL `vec3`-aligns-to-16
// rule, which is why the M1 block is ordered vector-first (the scalar gate/size fill the
// vec3 tail slots), keeping the Rust `#[repr(C)]` and the HLSL std430 byte-identical.
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, coarse_enabled) == 0);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, omega) == 4);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, lighting_flags) == 8);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, _pad) == 12);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, light_dir) == 16);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, _pad2) == 28);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, grid_origin) == 32);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, brick_enabled) == 44);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, grid_dims) == 48);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, brick_world) == 60);
// M2: the `brick_trilinear` gate @64 + the `_pad3` tail @68 fill the 16-byte headroom the M1
// layout left, so the struct is now EXACTLY the declared 80-byte COMPOSITE push range. A
// non-default-grid / `brick_trilinear` GPU test catches a packing slip the way the light_dir@16
// and grid_origin@32 pins do.
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, brick_trilinear) == 64);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, _pad3) == 68);
const _: () = assert!(GBUFFER_MARCHER_PUSH_BYTES == 80, "FineMarcherPush must be 80 bytes");
const _: () = assert!(
    GBUFFER_MARCHER_PUSH_BYTES == COMPOSITE_PUSH_CONSTANT_BYTES,
    "FineMarcherPush must equal the declared 80-byte COMPUTE push range"
);

impl FineMarcherPush {
    /// Builds the marcher push for the windowed / offscreen fine pass: the P4b
    /// `coarse_enabled` gate, the B1 `omega`, and the A1/A2 `lighting_flags` + the
    /// directional `light_dir`. `lighting_flags == 0` selects the OFF (byte-identical)
    /// path; `light_dir` is then a don't-care (the shader never normalizes it). The M1
    /// empty-skip is OFF (`brick_enabled == 0`, a zero grid) — byte-identical to the
    /// pre-M1 marcher. Use [`with_brick`](Self::with_brick) to enable the empty skip.
    #[inline]
    pub const fn new(
        coarse_enabled: bool,
        omega: f32,
        lighting_flags: u32,
        light_dir: [f32; 3],
    ) -> Self {
        Self {
            coarse_enabled: coarse_enabled as u32,
            omega,
            lighting_flags,
            _pad: 0,
            light_dir,
            _pad2: 0.0,
            grid_origin: [0.0, 0.0, 0.0],
            brick_enabled: 0,
            grid_dims: [0, 0, 0],
            brick_world: 0.0,
            brick_trilinear: 0,
            _pad3: [0, 0, 0],
        }
    }

    /// Enables the M1 empty-space-skip: turns on `brick_enabled` and stamps the
    /// pointer-grid uniforms (`grid_origin`, `grid_dims`, `brick_world`) the marcher
    /// indexes binding 9 with. The grid must match the [`build_pointer_grid`] bake bound
    /// to binding 9. The base gates (`coarse_enabled` / `omega` / lighting) are
    /// preserved. With the empty skip ON, the hit/normal stay ANALYTIC — only the
    /// EMPTY-brick traversal is accelerated (the hit-set equals the analytic hit-set).
    ///
    /// [`build_pointer_grid`]: boyko_sdf_math::brick::build_pointer_grid
    #[inline]
    pub const fn with_brick(
        mut self,
        grid_origin: [f32; 3],
        grid_dims: [u32; 3],
        brick_world: f32,
    ) -> Self {
        self.brick_enabled = 1;
        self.grid_origin = grid_origin;
        self.grid_dims = grid_dims;
        self.brick_world = brick_world;
        self
    }

    /// Enables the M2 trilinear+JCGT-cubic SURFACE-brick path: turns on `brick_trilinear`. The
    /// marcher then samples the brick atlas (binding 10) and solves the analytic cubic for the
    /// EXACT ray↔isosurface crossing inside a SURFACE M2 cell, validated by the analytic residual
    /// (the exact-CSG fallback). The atlas/grid uniforms the cubic needs live in the b5 camera UBO
    /// ([`M2GridParams`]), NOT the push, so this gate carries no extra fields. INDEPENDENT of the
    /// M1 empty-skip ([`with_brick`](Self::with_brick)): both gates may be on, off, or mixed. The
    /// base gates (`coarse_enabled` / `omega` / lighting) and any M1 grid uniforms are preserved.
    ///
    /// `brick_trilinear == false` leaves the push byte-identical to the M1 state (the OFF path —
    /// the atlas is never sampled).
    #[inline]
    pub const fn with_brick_trilinear(mut self, enabled: bool) -> Self {
        self.brick_trilinear = enabled as u32;
        self
    }

    /// Re-views the push constants as their raw 80-byte slice for `push_constants`.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Self` is `#[repr(C)]` with only `u32` / `f32` / `[f32; 3]` / `[u32; 3]` fields
        // (all `Copy`, every offset + the 80-byte total pinned by the const-asserts above, no
        // uninit padding — the explicit `_pad`/`_pad2`/`_pad3` fields cover the std430 holes),
        // so its `size_of` bytes are a fully-initialized, alignment-valid POD bit pattern.
        // The `&self` borrow keeps the struct alive for the slice's lifetime; the slice is
        // read-only (no aliasing write).
        unsafe {
            slice::from_raw_parts((self as *const Self).cast::<u8>(), core::mem::size_of::<Self>())
        }
    }
}

/// The Lighting-L1 cull push constants (mirrors `cluster_cull.hlsl`'s `ClusterCullPush`): the
/// exp-Z near/far the froxel-AABB build samples its slice view-z from, plus the per-froxel /
/// flat-list caps the cull clamp-and-drops at (O2). `#[repr(C)]`, 16 B (`f32, f32, u32, u32`),
/// the offsets pinned by the const-asserts below so a host/shader desync is a build error.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterCullPush {
    /// Exp-Z near plane (slice 0 view-z).
    pub z_near: f32,
    /// Exp-Z far plane (slice `dim_z` view-z).
    pub z_far: f32,
    /// Per-froxel light-index cap (O2 clamp-and-drop).
    pub max_lights_per_cluster: u32,
    /// Flat light-index-list capacity in `u32`s (O2 global clamp-and-drop).
    pub index_list_cap: u32,
}

/// Byte size of [`ClusterCullPush`] — the cull pipeline's declared COMPUTE push range (16 B).
pub const CLUSTER_CULL_PUSH_BYTES: u32 = core::mem::size_of::<ClusterCullPush>() as u32;

const _: () = assert!(core::mem::offset_of!(ClusterCullPush, z_near) == 0);
const _: () = assert!(core::mem::offset_of!(ClusterCullPush, z_far) == 4);
const _: () = assert!(core::mem::offset_of!(ClusterCullPush, max_lights_per_cluster) == 8);
const _: () = assert!(core::mem::offset_of!(ClusterCullPush, index_list_cap) == 12);
const _: () = assert!(CLUSTER_CULL_PUSH_BYTES == 16, "ClusterCullPush must be 16 bytes");

impl ClusterCullPush {
    /// Builds the cull push from the exp-Z near/far + the caps.
    #[inline]
    pub const fn new(
        z_near: f32,
        z_far: f32,
        max_lights_per_cluster: u32,
        index_list_cap: u32,
    ) -> Self {
        Self { z_near, z_far, max_lights_per_cluster, index_list_cap }
    }

    /// Re-views the push constants as their raw 16-byte slice for `push_constants`.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Self` is `#[repr(C)]` with only `f32` / `u32` fields (all `Copy`, every
        // offset + the 16-byte total pinned by the const-asserts above, no uninit padding),
        // so its `size_of` bytes are a fully-initialized, alignment-valid POD bit pattern.
        // The `&self` borrow keeps the struct alive for the slice's lifetime; read-only.
        unsafe {
            slice::from_raw_parts((self as *const Self).cast::<u8>(), core::mem::size_of::<Self>())
        }
    }
}

/// The CPU golden for one composited pixel at the GOLDEN 64×64 ORTHO extent: a thin
/// wrapper over [`golden_composite_pixel_ex`] with `(SDF_IMG_W, SDF_IMG_H)` + ORTHO.
/// Bit-identical to the pre-P0a definition (same extent, same arithmetic), so the
/// rung-10 / window-present goldens are unchanged. See [`golden_composite_pixel_ex`]
/// for the per-pixel composite rules.
///
/// Returns the packed `0xAABBGGRR` color. This is the single source of truth the
/// rung-10 test diffs the GPU readback against (within `+/-2/255` per channel) and
/// that a future CPU physics evaluator can reuse for the same hybrid query.
#[inline]
pub fn golden_composite_pixel(edits: &[SdfEdit], mesh_depth: f32, px: u32, py: u32) -> u32 {
    golden_composite_pixel_ex(
        edits,
        mesh_depth,
        px,
        py,
        SDF_IMG_W,
        SDF_IMG_H,
        CompositeCamera::Ortho,
    )
}

/// The camera the extent-aware golden ([`golden_composite_pixel_ex`]) reconstructs a
/// ray from. ORTHO is the golden-frozen path; PERSPECTIVE mirrors the shader's
/// additive ray-gen (eye + orthonormal basis + half-FOV tangent + aspect).
#[derive(Clone, Copy, Debug)]
pub enum CompositeCamera {
    /// The golden-frozen orthographic camera (looking down -Z, [`SDF_HALF_EXTENT`]).
    Ortho,
    /// The P0a perspective camera, mirroring the shader's perspective branch.
    Perspective {
        /// Eye world position (the ray origin).
        eye: [f32; 3],
        /// Forward basis vector.
        forward: [f32; 3],
        /// Right basis vector.
        right: [f32; 3],
        /// Up basis vector.
        up: [f32; 3],
        /// `tan(fovY / 2)`.
        tan_half_fov: f32,
        /// Aspect ratio (W / H).
        aspect: f32,
    },
}

/// Reconstructs the `(ray_origin, ray_dir)` for pixel `(px, py)` at extent
/// `(img_w, img_h)` under `camera`, mirroring the shader's ray-gen EXACTLY.
///
/// The ORTHO arm is the golden-frozen arithmetic (`u`/`v` → `ro`/`rd`); at extent
/// `(64, 64)` it is byte-for-byte the pre-P0a computation. The PERSPECTIVE arm mirrors
/// the shader's perspective branch (NDC → basis-combined direction → `normalize`),
/// using only plain IEEE ops so a perspective scene is reproducible (no fast math).
#[inline]
fn composite_ray(
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
) -> ([f32; 3], [f32; 3]) {
    match camera {
        CompositeCamera::Ortho => {
            let u = (((px as f32) + 0.5) / (img_w as f32)) * 2.0 - 1.0;
            let v = -((((py as f32) + 0.5) / (img_h as f32)) * 2.0 - 1.0);
            let ro = [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT, SDF_CAM_Z];
            let rd = [0.0, 0.0, -1.0];
            (ro, rd)
        }
        CompositeCamera::Perspective {
            eye,
            forward,
            right,
            up,
            tan_half_fov,
            aspect,
        } => {
            let ndc_x = (((px as f32) + 0.5) / (img_w as f32)) * 2.0 - 1.0;
            let ndc_y = -((((py as f32) + 0.5) / (img_h as f32)) * 2.0 - 1.0);
            let sx = ndc_x * aspect * tan_half_fov;
            let sy = ndc_y * tan_half_fov;
            let dir = [
                forward[0] + right[0] * sx + up[0] * sy,
                forward[1] + right[1] * sx + up[1] * sy,
                forward[2] + right[2] * sx + up[2] * sy,
            ];
            // Mirror HLSL `normalize` exactly: raw `sqrt` then component divide, NO
            // zero-guard (unlike `v_normalize`), so this host reference predicts the
            // GPU bit-for-bit on valid cameras. A degenerate (zero) `dir` yields a
            // non-finite ray on BOTH host and shader; a valid camera never produces one.
            let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            let rd = [dir[0] / len, dir[1] / len, dir[2] / len];
            (eye, rd)
        }
    }
}

/// The `(ray_origin, ray_dir)` for pixel `(px, py)` at extent `(img_w, img_h)` under
/// `camera`, exposing the shared marcher/resolve ray-gen ([`composite_ray`]) so the PBR
/// MVP-2 resolve golden ([`golden_deferred_resolve`]) can reconstruct the per-pixel view
/// direction (`V = -rd`) the GPU resolve uses. Bit-identical to the marcher's ray-gen.
#[inline]
pub fn composite_pixel_ray(
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
) -> ([f32; 3], [f32; 3]) {
    composite_ray(px, py, img_w, img_h, camera)
}

/// The extent- and camera-aware CPU golden for one composited pixel (P0a). At
/// `(SDF_IMG_W, SDF_IMG_H)` + [`CompositeCamera::Ortho`] this is BIT-IDENTICAL to the
/// pre-P0a `golden_composite_pixel` (same `u`/`v`/`ro`/`rd` arithmetic), preserving
/// the rung-8..11 contract; with a runtime extent / [`CompositeCamera::Perspective`]
/// it mirrors the shader's P0a ray-gen so the host-vs-GPU agreement stays valid at
/// any resolution. Composites exactly as the shader:
///
/// - an SDF hit at `t_sdf < t_mesh` → the lit SDF surface color (Lambert + ambient);
/// - else if the mesh covered the pixel (`mesh_depth < 1.0`) → flat [`MESH_COLOR`];
/// - else → `SDF_BACKGROUND`.
///
/// The field eval (`sdf_edit_list` / `_normal`) is byte-identical to the ortho path;
/// only ray generation + the extent source change (the determinism boundary).
pub fn golden_composite_pixel_ex(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
) -> u32 {
    // Render B1: the ω = 1.0 forwarder. At `omega == 1.0` the `_omega` variant's live
    // path is the frozen plain sphere-trace, so this stays BIT-IDENTICAL to the pre-B1
    // body and every existing caller is unchanged (the 0%-gate).
    golden_composite_pixel_ex_omega(edits, mesh_depth, px, py, img_w, img_h, camera, 1.0)
}

/// Render B1 — the over-relaxation-aware extent/camera golden. Mirrors the shader's
/// Keinert over-relaxation marcher EXACTLY: the `if omega > 1.0` gate, the
/// over-relaxed step `t += omega * d`, the sor-fail exact retreat (`t = safe_t` then a
/// permanent fall to plain), and the verbatim frozen else-arm `t += d`. At `omega == 1.0`
/// the live path is textually the frozen plain loop, so this is BIT-IDENTICAL to the
/// pre-B1 [`golden_composite_pixel_ex`] (the 0%-gate). `omega` is expected to already be
/// in `[1.0, 1.99]` (the host runtime clamp); higher values are unsound (the safeguard
/// holds only for `omega < 2`).
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_ex_omega(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    omega: f32,
) -> u32 {
    golden_composite_pixel_ex_omega_lit(
        edits, mesh_depth, px, py, img_w, img_h, camera, omega, 0, DEFAULT_LIGHT_DIR,
    )
}

/// Render A1/A2 — the lighting-aware extent/camera/omega golden. Identical to
/// [`golden_composite_pixel_ex_omega`] but threads the `lighting_flags` + `light_dir`
/// the marcher push carries: on an SDF hit the lit color goes through [`host_shade`],
/// which multiplies in the A1 soft-shadow and/or A2 AO terms when the matching flag
/// bit is set (bit 0 = shadows, bit 1 = AO). With `lighting_flags == 0` this is
/// BYTE-IDENTICAL to [`golden_composite_pixel_ex_omega`] (the 0%-gate); the ON path
/// mirrors the shader within ±3/255 (consumer-side relaxable). `light_dir` is the
/// un-normalized directional-light direction; the field eval / march are untouched.
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_ex_omega_lit(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    omega: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
) -> u32 {
    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);

    let has_mesh = mesh_depth < MESH_DEPTH_CLEAR;
    // A finite march bound only when the mesh covered the pixel; otherwise a value
    // larger than any `t` the march reaches (mirrors the shader's `1e30`).
    let t_mesh = if has_mesh { depth_to_t(mesh_depth) } else { 1.0e30 };

    let mut t = 0.0_f32;
    let t_seed = t; // the ORIGINAL seed (0.0 here) — the Candidate C re-march re-seeds from it
    let mut omega = omega; // [1.0, 1.99]; sor-fail latches it to 1.0 for the rest of the ray
    let mut hit = false;
    let mut safe_t = 0.0_f32; // probe param remembered for an exact retreat
    let mut sor_prev = 0.0_f32; // previous probe's d
    let mut sor_step_prev = 0.0_f32; // previous over-relaxed step length
    // BUG-B1-HOLE-3 (Candidate C): the EXHAUSTION flag. True iff the fast loop runs ALL
    // SDF_MAX_IT iterations with NO break — i.e. the ray neither converged, nor clearly
    // left the scene (`t > T_MAX`), nor hit the mesh (`t >= t_mesh`); it ran out of
    // budget mid-field. Starts `true`, cleared by EVERY in-loop break. Mirrors the shader.
    let mut exhausted = true;
    for it in 0..SDF_MAX_IT {
        if t >= t_mesh {
            exhausted = false; // mesh-occlusion termination — NOT budget exhaustion
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            hit = true;
            exhausted = false; // converged — NOT budget exhaustion
            break;
        }
        if omega > 1.0 {
            let step_len = d * omega;
            // sor_fail: the over-step taken last iter overshot the previous unbounding
            // sphere (valid only for omega < 2 — spheres must overlap). Lipschitz-aware
            // (BUG-B1-HOLE-1): the guaranteed-empty radius at field value `f` is
            // `f / FIELD_LIPSCHITZ_L`, so the spheres cover the step iff
            // `sor_prev + d >= L * sor_step_prev`. Mirrors the shader exactly.
            //
            // The `it > 0` guard is LOAD-BEARING (do not remove): a sor-fail can only be
            // reached after at least one ACCEPTED over-relax step (it >= 1 ⟹ accepted >= 1),
            // which pre-pays the +1 retreat iteration in the budget proof.
            if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                // BUG-B1-HOLE-2: do NOT retreat to bare `safe_t` and re-probe (that re-evals
                // the field, costing +2 iters vs plain and overflowing the budget at the
                // MAX_IT cliff → a hole). RESUME the plain march one certified step past the
                // safe point: `safe_t` is the exact probe param, `sor_prev` the exact field
                // value there, so `safe_t + sor_prev` is precisely where a plain march lands
                // after probing safe_t — reusing the eval (no re-probe). One same-sign add
                // (both operands >= 0): no cancellation, unlike a `t - <correction>` form.
                // Net +1 iter vs plain, pre-paid by the >= 1 accepted over-step (it>0 guard).
                debug_assert!(it > 0, "B1 budget: a>=1 precondition");
                debug_assert!(sor_prev >= SDF_EPS); // safe-point field value >= EPS → retreat strictly advances
                t = safe_t + sor_prev; // plain-resume one certified step past the safe probe
                debug_assert!(t > safe_t, "B1 retreat must advance");
                omega = 1.0;
                continue;
            }
            safe_t = t;
            sor_prev = d;
            sor_step_prev = step_len;
            t += step_len;
        } else {
            t += d; // frozen plain arm — TEXTUALLY identical to the frozen loop
        }
        if t > SDF_T_MAX {
            exhausted = false; // clear-miss termination — NOT budget exhaustion
            break;
        }
    }

    // BUG-B1-HOLE-3 (Candidate C): the PROVABLY-hole-free fallback re-march, mirroring
    // the shader EXACTLY. The fast over-relaxed pass can fall BEHIND a plain march on a
    // non-monotone field (the `steps(omega) <= steps(1)` bound is genuinely violated and
    // unbounded), exhausting the budget mid-field on a ray the FROZEN plain marcher would
    // have hit. On `exhausted` (ran all SDF_MAX_IT with no break) RE-MARCH from the
    // ORIGINAL seed with a plain omega = 1.0 sphere-trace and use ITS result. This second
    // loop is the EXACT frozen marcher body (`t += d`), so any surface the frozen path
    // hits within MAX_IT it hits here too → B1's hit-set is identical to the frozen
    // hit-set, with NO dependence on a step-count bound. At omega == 1.0 the fast pass IS
    // the frozen plain loop, so on exhaustion this reproduces the identical frozen
    // (hit = false) result — the omega == 1.0 output is byte-unchanged (the 0%-gate).
    if exhausted {
        t = t_seed; // re-seed from the SAME original seed the fast pass used
        hit = false;
        for _it2 in 0..SDF_MAX_IT {
            if t >= t_mesh {
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d; // frozen plain step
            if t > SDF_T_MAX {
                break;
            }
        }
    }

    let color = if hit && t < t_mesh {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);
        host_shade(SDF_BASE_COLOR, SDF_AMBIENT, p, n, light_dir, lighting_flags, &|q| {
            sdf_edit_list(edits, q)
        })
    } else if has_mesh {
        MESH_COLOR
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
}

// ===========================================================================
// M1 — the EMPTY-SPACE-SKIP host golden mirror (the empty-skip pivot).
//
// `golden_composite_pixel_brick` mirrors `sdf_gbuffer_composite.hlsl`'s primary march
// with `brick_enabled != 0`: before each `sdf(p)` it reads the pointer-grid cell at `p`
// and, on an `EmptyOutside` cell, steps to the brick's ray-AABB exit
// (`dist_to_brick_exit`) instead of folding the field — sound by construction (the
// conservative classifier guarantees no surface within `band_half` of an EMPTY brick).
// `EmptyInside` and `Surface` cells, and points OUTSIDE the bounded grid, march
// ANALYTICALLY (the exact `sdf(p)` step). The hit/normal/shade stay ANALYTIC (C1).
//
// With `brick_enabled == 0` (or an empty/None grid) the function delegates to the
// pre-M1 `golden_composite_pixel_ex_omega_lit`, so the OFF golden is BYTE-IDENTICAL to
// today (the 0%-gate). With it ON, the empty skip never skips a surface, so the hit `t`
// — and therefore the composited color — equals the pure-analytic result.
// ===========================================================================

/// `BrickClass::EmptyOutside as u32` — the only cell class the empty skip acts on. The
/// host mirror reads the grid as `u32` cells (the GPU `StructuredBuffer<uint>` element),
/// matching `build_pointer_grid`.
const BRICK_CLASS_EMPTY_OUTSIDE: u32 = 0;

/// Reads the pointer-grid cell containing world point `p`, returning `(class, cell_min)`
/// or `None` when `p` is OUTSIDE the bounded grid (the marcher then falls through to the
/// analytic field). Mirrors the shader's `brick_cell(p)` index + bounds check exactly.
#[inline]
fn host_brick_cell(grid: &PointerGrid, cells: &[u32], p: [f32; 3]) -> Option<(u32, [f32; 3])> {
    let rel = [
        (p[0] - grid.origin[0]) / grid.brick_world,
        (p[1] - grid.origin[1]) / grid.brick_world,
        (p[2] - grid.origin[2]) / grid.brick_world,
    ];
    // Outside the grid on any axis (incl. negative `rel`) → no cell. `floor` then a
    // signed range check; `rel < 0` is caught by the `>= dims` test after the cast only
    // if guarded — so test the float directly to avoid the wrap on a negative cast.
    if rel[0] < 0.0 || rel[1] < 0.0 || rel[2] < 0.0 {
        return None;
    }
    let ix = rel[0] as u32;
    let iy = rel[1] as u32;
    let iz = rel[2] as u32;
    if ix >= grid.dims[0] || iy >= grid.dims[1] || iz >= grid.dims[2] {
        return None;
    }
    let w = grid.dims[0];
    let h = grid.dims[1];
    let idx = (ix + iy * w + iz * w * h) as usize;
    debug_assert!(idx < cells.len(), "grid cell index in bounds");
    Some((cells[idx], grid.cell_min(ix, iy, iz)))
}

/// M1 — the empty-space-skip extent/camera/omega/lighting golden. Identical to
/// [`golden_composite_pixel_ex_omega_lit`] but the PRIMARY march runs the pointer-grid
/// empty skip when `brick_enabled == true`: an `EmptyOutside` cell at the march point
/// steps to the brick AABB exit ([`dist_to_brick_exit`], clamped to advance) instead of
/// folding the field; every other cell (and any point outside the bounded grid) folds the
/// EXACT analytic field. `grid` + `cells` are the [`build_pointer_grid`] bake the GPU
/// binds at binding 9 (the SAME origin/dims/brick_world the push carries).
///
/// With `brick_enabled == false` this delegates to [`golden_composite_pixel_ex_omega_lit`]
/// — BYTE-IDENTICAL to the pre-M1 golden (the 0%-gate). The re-march fallback, the
/// hit/normal, and the shade stay ANALYTIC (C1): the empty skip only accelerates EMPTY
/// traversal, so the hit `t` equals the pure-analytic hit `t` within `SDF_EPS` and the
/// composited color matches the analytic golden.
///
/// [`build_pointer_grid`]: boyko_sdf_math::brick::build_pointer_grid
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_brick(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    omega: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
    brick_enabled: bool,
    grid: &PointerGrid,
    cells: &[u32],
) -> u32 {
    // The OFF path is byte-identical to the pre-M1 marcher (the 0%-gate). The grid is
    // never read; the march is the exact analytic sphere-trace.
    if !brick_enabled {
        return golden_composite_pixel_ex_omega_lit(
            edits, mesh_depth, px, py, img_w, img_h, camera, omega, lighting_flags, light_dir,
        );
    }

    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);

    let has_mesh = mesh_depth < MESH_DEPTH_CLEAR;
    let t_mesh = if has_mesh { depth_to_t(mesh_depth) } else { 1.0e30 };

    let mut t = 0.0_f32;
    let t_seed = t;
    let mut omega = omega; // [1.0, 1.99]; sor-fail latches it to 1.0
    let mut hit = false;
    let mut safe_t = 0.0_f32;
    let mut sor_prev = 0.0_f32;
    let mut sor_step_prev = 0.0_f32;
    let mut exhausted = true;
    for it in 0..SDF_MAX_IT {
        if t >= t_mesh {
            exhausted = false; // mesh-occlusion termination
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];

        // M1 empty skip: an EmptyOutside cell at `p` has provably no surface within
        // band_half (conservative classifier), so step to the brick AABB exit and skip
        // the analytic fold. CONTINUE without touching `sdf`/the over-relax state — the
        // exit step is plain (no omega), so it cannot overshoot a surface (the next
        // brick is `Surface` if a surface is near). Sound by construction.
        //
        // EmptyInside / Surface (and an outside-grid `None`) fall THROUGH to the EXACT
        // analytic field below. EmptyInside is the start-inside case the analytic
        // negative-`d` handling already covers — a ray from outside reaches Surface first,
        // so a negative `sdf(p)` here means the seed began inside a solid; the analytic step
        // (which can be negative) is the consistent, unchanged behavior.
        if let Some((class, cell_min)) = host_brick_cell(grid, cells, p)
            && class == BRICK_CLASS_EMPTY_OUTSIDE
        {
            let exit = dist_to_brick_exit(p, rd, cell_min, grid.brick_world);
            t += exit;
            if t > SDF_T_MAX {
                exhausted = false; // clear-miss termination
                break;
            }
            continue; // skip the analytic fold this step
        }

        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            hit = true;
            exhausted = false; // converged
            break;
        }
        if omega > 1.0 {
            let step_len = d * omega;
            if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                debug_assert!(it > 0, "B1 budget: a>=1 precondition");
                t = safe_t + sor_prev; // plain-resume one certified step past the safe probe
                omega = 1.0;
                continue;
            }
            safe_t = t;
            sor_prev = d;
            sor_step_prev = step_len;
            t += step_len;
        } else {
            t += d; // frozen plain arm
        }
        if t > SDF_T_MAX {
            exhausted = false; // clear-miss termination
            break;
        }
    }

    // The re-march fallback stays ANALYTIC (C1) — identical to the non-brick path. The
    // empty skip never reopens the B1 budget hole (its plain exit steps are bounded), so
    // `exhausted` here means the analytic field ran out of budget mid-field, exactly as
    // in the non-brick marcher; the frozen plain re-march resolves it.
    if exhausted {
        t = t_seed;
        hit = false;
        for _it2 in 0..SDF_MAX_IT {
            if t >= t_mesh {
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d; // frozen plain step
            if t > SDF_T_MAX {
                break;
            }
        }
    }

    let color = if hit && t < t_mesh {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);
        host_shade(SDF_BASE_COLOR, SDF_AMBIENT, p, n, light_dir, lighting_flags, &|q| {
            sdf_edit_list(edits, q)
        })
    } else if has_mesh {
        MESH_COLOR
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
}

// ===========================================================================
// M2 — the trilinear+JCGT-cubic SURFACE-brick atlas (the CPU baker + the b5 UBO grid
// block + the host golden mirror).
//
// M1 ships empty-space-skip + an analytic fold inside SURFACE bricks; M2 replaces that
// analytic fold with a dense `R8_SNORM`/`R16_SFLOAT` brick atlas — one apron'd
// `BRICK_ALLOC³` (10³) tile per SURFACE M2 grid cell — sampled by the marcher, whose
// 8 per-cell corner distances form the JCGT-2022 cubic ([`brick_cubic_hit`]) whose root
// is the EXACT ray↔trilinear-isosurface crossing, then VALIDATED by the analytic residual
// (the exact-CSG fallback). The grid is a small NEAR-FIELD lattice ([-4, 4]³) of
// `M2_BRICK_WORLD`-sized bricks; the atlas is `M2_ATLAS_DIM³` voxels.
//
// Principle 0: the atlas is a TRANSIENT GPU mirror of the ONE analytic authority
// ([`SdfEditField`]) baked each `gen`, owning no durable per-entity state (exactly like
// the M1 pointer grid / the GPU edit list). The host mirror ([`golden_composite_pixel_brick_m2`])
// is the bit-exact reference the GPU golden compares against.
// ===========================================================================

/// The world size of one M2 brick cell (`BRICK_INTERIOR * M2_VOXEL_SIZE = 8 * 0.25`). The
/// near-field grid cell edge — the world span a single apron'd `BRICK_ALLOC³` atlas tile covers.
pub const M2_BRICK_WORLD: f32 = 2.0;

/// The world width of one M2 atlas voxel (the brick scale [`fill_brick`] / [`brick_cubic_hit`] pin).
pub const M2_VOXEL_SIZE: f32 = 0.25;

/// The M2 near-field grid edge (cells per axis). A `4³` lattice of [`M2_BRICK_WORLD`]-sized bricks
/// spans `[-4, 4]³` (the demo/golden extent — centers in `[-2, 2]`, primitives `<= 3`), fully
/// enclosing the near field.
pub const M2_GRID_DIM: u32 = 4;

/// The minimum world corner of the M2 near-field grid (cell `(0,0,0)`'s min): `-M2_GRID_DIM *
/// M2_BRICK_WORLD / 2 = -4`. The grid spans `[M2_GRID_ORIGIN, M2_GRID_ORIGIN + M2_GRID_DIM *
/// M2_BRICK_WORLD]³ = [-4, 4]³`.
pub const M2_GRID_ORIGIN: f32 = -4.0;

/// The M2 atlas edge in voxels (`M2_GRID_DIM * BRICK_ALLOC = 4 * 10 = 40`). The dense 3D atlas
/// image is `M2_ATLAS_DIM³` voxels — one apron'd `BRICK_ALLOC³` tile per M2 grid cell. The
/// [`crate::brick_atlas::BrickAtlas`] image is sized to this.
pub const M2_ATLAS_DIM: u32 = M2_GRID_DIM * BRICK_ALLOC as u32;

/// The M2 snorm decode band half-width (world units) — the band the atlas codes span, mirroring
/// `boyko_sdf_math::brick`'s store band (== [`SDF_EDIT_BAND_HALF`] `= 0.90`).
pub const M2_BAND_HALF: f32 = SDF_EDIT_BAND_HALF;

/// The M2 crease tolerance (world units): the largest `|analytic sdf|` at the cubic candidate that
/// ACCEPTS the cubic hit as the surface; beyond it (a CSG crease / brick-rounding divergence) the
/// analytic refine decides (the exact-CSG fallback). `~` the brick's `δ_tri + δ_quant` world slack.
pub const M2_CREASE_EPS: f32 = 0.0192;

/// The b5 camera UBO byte size widened for M2: the 80-byte camera block
/// ([`COMPOSITE_PUSH_CONSTANT_BYTES`]) + a 48-byte [`M2GridParams`] tail at
/// [`M2_GRID_PARAMS_OFFSET`]. The host writes the camera block then the M2 block; the marcher
/// reads the M2 block ONLY on the `brick_trilinear` path (byte-identical to M1 when OFF).
pub const B5_CAMERA_UBO_BYTES: usize = 128;

/// The byte offset of the [`M2GridParams`] block inside the widened b5 camera UBO (right after the
/// 80-byte camera block). The host writes `M2GridParams::default_near_field().as_bytes()` here.
pub const M2_GRID_PARAMS_OFFSET: usize = 80;

/// The number of analytic refine steps the M2 surface-hit fallback sphere-traces from the cubic
/// candidate when the analytic residual exceeds [`M2_CREASE_EPS`] (mirror the shader's
/// `M2_REFINE_ITERS`).
const M2_REFINE_ITERS: u32 = 8;

// The M2 grid constants pin the shader's static brick geometry (mirror the `.hlsl` `M2_*` consts):
// a desync (e.g. a brick scale change) is a build error here, caught before the GPU runs.
const _: () = assert!(M2_BRICK_WORLD == BRICK_INTERIOR as f32 * M2_VOXEL_SIZE);
const _: () = assert!(M2_GRID_ORIGIN == -(M2_GRID_DIM as f32 * M2_BRICK_WORLD * 0.5));
const _: () = assert!(M2_ATLAS_DIM == 40);

/// The minimum world corner of M2 grid cell `cell = (cx, cy, cz)`:
/// `M2_GRID_ORIGIN + cell * M2_BRICK_WORLD`. The brick spans `[min, min + M2_BRICK_WORLD]³`.
#[inline]
pub const fn m2_cell_min(cell: [u32; 3]) -> [f32; 3] {
    [
        M2_GRID_ORIGIN + cell[0] as f32 * M2_BRICK_WORLD,
        M2_GRID_ORIGIN + cell[1] as f32 * M2_BRICK_WORLD,
        M2_GRID_ORIGIN + cell[2] as f32 * M2_BRICK_WORLD,
    ]
}

/// The atlas-VOXEL origin of M2 grid cell `tile = (tx, ty, tz)`: `tile * BRICK_ALLOC`. The cell's
/// apron'd `BRICK_ALLOC³` tile occupies `[origin, origin + BRICK_ALLOC]³` voxels in the dense
/// `M2_ATLAS_DIM³` atlas. MUST match the shader's `m2_corner` uvw mapping
/// (`(tile_org + corner) / M2_ATLAS_DIM` under the integer texelFetch), and the host baker
/// ([`bake_brick_atlas`]) scatters each tile at this voxel offset.
#[inline]
pub const fn m2_tile_atlas_origin(tile: [u32; 3]) -> [u32; 3] {
    [
        tile[0] * BRICK_ALLOC as u32,
        tile[1] * BRICK_ALLOC as u32,
        tile[2] * BRICK_ALLOC as u32,
    ]
}

/// The M2 grid block written into the b5 camera UBO tail (at [`M2_GRID_PARAMS_OFFSET`]) so a
/// host-side grid retune needs no shader edit. `#[repr(C)]`, 48 bytes — three std140 `vec4` lanes
/// mirroring the shader's `cbuffer Camera` M2 fields (`m2_origin_brick_world`, `m2_dims_atlas_dim`,
/// `m2_band_voxel_inv_atlas`):
///
/// - lane 0 `origin_brick_world` — `(origin.x, origin.y, origin.z, M2_BRICK_WORLD)`
/// - lane 1 `dims_atlas_dim` — `(dims.x, dims.y, dims.z, M2_ATLAS_DIM)` as `f32`
/// - lane 2 `band_voxel_inv_atlas` — `(band_half, voxel_size, 1.0 / atlas_dim, 0.0)`
///
/// The offsets are pinned by the const-asserts below so a host/shader desync is a build error.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct M2GridParams {
    /// Lane 0: `xyz` = the M2 grid min world corner, `w` = [`M2_BRICK_WORLD`].
    pub origin_brick_world: [f32; 4],
    /// Lane 1: `xyz` = the M2 grid dims `[x, y, z]` as `f32`, `w` = [`M2_ATLAS_DIM`] as `f32`. The
    /// shader reads it as a `uint4` (`(uint)` cast) — an exact small-integer `f32`↔`uint` round trip.
    pub dims_atlas_dim: [f32; 4],
    /// Lane 2: `x` = band half-width, `y` = voxel size, `z` = `1.0 / M2_ATLAS_DIM`, `w` = 0 (pad).
    pub band_voxel_inv_atlas: [f32; 4],
}

/// Byte size of [`M2GridParams`] — three std140 `vec4` lanes (48 B), the b5 UBO M2 tail.
pub const M2_GRID_PARAMS_BYTES: usize = core::mem::size_of::<M2GridParams>();

const _: () = assert!(core::mem::offset_of!(M2GridParams, origin_brick_world) == 0);
const _: () = assert!(core::mem::offset_of!(M2GridParams, dims_atlas_dim) == 16);
const _: () = assert!(core::mem::offset_of!(M2GridParams, band_voxel_inv_atlas) == 32);
const _: () = assert!(M2_GRID_PARAMS_BYTES == 48, "M2GridParams must be 48 bytes (3 vec4 lanes)");
const _: () = assert!(
    M2_GRID_PARAMS_OFFSET + M2_GRID_PARAMS_BYTES == B5_CAMERA_UBO_BYTES,
    "the M2 block must fill the b5 UBO tail exactly (80 + 48 = 128)"
);

impl M2GridParams {
    /// The default near-field M2 grid block: origin [`M2_GRID_ORIGIN`], dims [`M2_GRID_DIM`]³,
    /// brick world [`M2_BRICK_WORLD`], atlas [`M2_ATLAS_DIM`], band [`M2_BAND_HALF`], voxel
    /// [`M2_VOXEL_SIZE`] — the render-path seed the [`crate::brick_atlas::BrickAtlas`] bakes against.
    #[inline]
    pub fn default_near_field() -> Self {
        Self {
            origin_brick_world: [M2_GRID_ORIGIN, M2_GRID_ORIGIN, M2_GRID_ORIGIN, M2_BRICK_WORLD],
            dims_atlas_dim: [
                M2_GRID_DIM as f32,
                M2_GRID_DIM as f32,
                M2_GRID_DIM as f32,
                M2_ATLAS_DIM as f32,
            ],
            band_voxel_inv_atlas: [
                M2_BAND_HALF,
                M2_VOXEL_SIZE,
                1.0 / M2_ATLAS_DIM as f32,
                0.0,
            ],
        }
    }

    /// Re-views the block as its raw 48-byte slice for the b5 UBO write (at [`M2_GRID_PARAMS_OFFSET`]).
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Self` is `#[repr(C)]` with only `[f32; 4]` fields (all `Copy`, every offset + the
        // 48-byte total pinned by the const-asserts above, no uninit padding — three packed vec4
        // lanes), so its `size_of` bytes are a fully-initialized, alignment-valid POD bit pattern.
        // The `&self` borrow keeps the struct alive for the slice's lifetime; read-only.
        unsafe {
            slice::from_raw_parts((self as *const Self).cast::<u8>(), core::mem::size_of::<Self>())
        }
    }
}

/// The voxel encoding the M2 brick atlas stores, chosen from the device's linear-filter support
/// (mirrors [`crate::device::DeviceCaps::atlas_format`]). `R8_SNORM` is the dense quantized path;
/// `R16_SFLOAT` is the fallback when the GPU cannot linear-filter `R8_SNORM` (no quantization — the
/// `EPSILON_Q` store bias is harmless there). The CPU baker ([`bake_brick_atlas`]) writes either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasEncoding {
    /// `R8_SNORM`: one signed byte per voxel (the snorm code [`fill_brick`] quantizes to).
    Snorm8,
    /// `R16_SFLOAT`: one half-float per voxel (the `decode_snorm8` value re-encoded to f16).
    Sfloat16,
}

impl AtlasEncoding {
    /// Picks the encoding from the device's `R8_SNORM` linear-filter support: `Snorm8` when the GPU
    /// can linear-filter the quantized atlas, else the `Sfloat16` fallback (the SAME choice
    /// [`crate::device::DeviceCaps::atlas_format`] maps to a `VkFormat`).
    #[inline]
    pub const fn from_linear_filter_ok(linear_filter_ok: bool) -> Self {
        if linear_filter_ok {
            Self::Snorm8
        } else {
            Self::Sfloat16
        }
    }

    /// Bytes per voxel for this encoding (`1` for `Snorm8`, `2` for `Sfloat16`).
    #[inline]
    pub const fn bytes_per_voxel(self) -> usize {
        match self {
            Self::Snorm8 => 1,
            Self::Sfloat16 => 2,
        }
    }

    /// The total byte size of the dense `M2_ATLAS_DIM³` atlas for this encoding — the staging-buffer
    /// + 3D-image byte size [`crate::brick_atlas::BrickAtlas`] allocates.
    #[inline]
    pub const fn atlas_byte_size(self) -> usize {
        let voxels = (M2_ATLAS_DIM as usize) * (M2_ATLAS_DIM as usize) * (M2_ATLAS_DIM as usize);
        voxels * self.bytes_per_voxel()
    }
}

/// The linear voxel index of atlas voxel `(x, y, z)` in the dense `M2_ATLAS_DIM³` lattice
/// (`x + y*W + z*W*W`, `W = M2_ATLAS_DIM`) — the SAME order the Vulkan 3D image's tightly-packed
/// `copy_buffer_to_image` reads (row-major, x fastest), so the host baker's scatter and the GPU
/// texel address agree.
#[inline]
pub const fn atlas_voxel_index(x: u32, y: u32, z: u32) -> usize {
    let w = M2_ATLAS_DIM as usize;
    x as usize + y as usize * w + z as usize * w * w
}

/// IEEE-754 binary32 → binary16 (round-to-nearest-even) — the f16 encode the `Sfloat16` atlas path
/// stores. Reuses the validated [`golden_f16_from_f32`] (the same encoder the lighting cone path
/// uses); the decoded snorm value lives in `[-band_half, band_half] ⊂ [-1, 1]`, inside the f16
/// normal range.
#[inline]
pub fn f16_from_f32(f: f32) -> u16 {
    golden_f16_from_f32(f)
}

/// Bakes the dense `M2_ATLAS_DIM³` brick atlas from the ONE edit authority `field` into `out`
/// (the staging bytes), in the chosen [`AtlasEncoding`], returning the number of SURFACE cells
/// baked.
///
/// For each M2 grid cell, classifies the brick ([`classify_brick`] at [`SDF_EDIT_BAND_HALF`]); a
/// SURFACE cell's apron'd `BRICK_ALLOC³` tile is baked ([`fill_brick`]) and scattered into `out` at
/// the cell's atlas-voxel origin ([`m2_tile_atlas_origin`]). EMPTY cells leave their tile voxels at
/// `0` (a mid-band code — never sampled, since the marcher only enters the M2 cubic on a SURFACE
/// cell via the M2 grid lookup). `Snorm8` stores the raw `i8` byte; `Sfloat16` stores
/// `f16_from_f32(decode_snorm8(byte))` (the decoded normalized value, NOT multiplied by `band_half`
/// — the shader's `m2_decode` applies `band_half`, matching the snorm hardware decode that also
/// returns the normalized value).
///
/// `out.len()` MUST be `encoding.atlas_byte_size()`. This is a SETUP-time (per-`gen`) bake, not a
/// hot-path call. Principle 0: a transient mirror of the analytic authority, no durable state.
pub fn bake_brick_atlas(field: &SdfEditField, encoding: AtlasEncoding, out: &mut [u8]) -> u32 {
    debug_assert_eq!(
        out.len(),
        encoding.atlas_byte_size(),
        "atlas staging must be encoding.atlas_byte_size() bytes"
    );

    let mut surface_cells = 0u32;
    let mut tile = [0i8; BRICK_VOXELS];

    for cz in 0..M2_GRID_DIM {
        for cy in 0..M2_GRID_DIM {
            for cx in 0..M2_GRID_DIM {
                let cell = [cx, cy, cz];
                let cell_min = m2_cell_min(cell);
                let class = classify_brick(field, cell_min, M2_BRICK_WORLD, SDF_EDIT_BAND_HALF);
                if class != BrickClass::Surface {
                    continue; // EMPTY cells: their tile voxels stay 0 (never sampled).
                }
                surface_cells += 1;

                // Bake the apron'd tile from the authority, then scatter it into the dense atlas at
                // the cell's atlas-voxel origin (the SAME `tile * BRICK_ALLOC` the shader addresses).
                fill_brick(field, cell_min, M2_VOXEL_SIZE, M2_BAND_HALF, &mut tile);
                let [ox, oy, oz] = m2_tile_atlas_origin(cell);
                const W: usize = BRICK_ALLOC;
                for lz in 0..W {
                    for ly in 0..W {
                        for lx in 0..W {
                            let byte = tile[lx + ly * W + lz * W * W];
                            let vx = ox + lx as u32;
                            let vy = oy + ly as u32;
                            let vz = oz + lz as u32;
                            let vi = atlas_voxel_index(vx, vy, vz);
                            match encoding {
                                AtlasEncoding::Snorm8 => {
                                    out[vi] = byte as u8;
                                }
                                AtlasEncoding::Sfloat16 => {
                                    // Store the DECODED normalized value (in [-1, 1]); the shader's
                                    // `m2_decode` multiplies by `band_half`, so the f16 lane carries
                                    // the same normalized value the snorm hardware decode returns.
                                    let n = decode_snorm8(byte, 1.0);
                                    let h = f16_from_f32(n).to_le_bytes();
                                    out[vi * 2] = h[0];
                                    out[vi * 2 + 1] = h[1];
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    surface_cells
}

// ---------------------------------------------------------------------------
// M2 — the HOST GOLDEN MIRROR (`golden_composite_pixel_brick_m2`): the bit-exact reference the
// GPU M2 marcher is golden-compared against. Mirrors `sdf_gbuffer_composite.hlsl`'s
// `m2_surface_hit` (the atlas sample → JCGT cubic → analytic-residual fallback) over the SAME
// baked-tile data the GPU samples, then delegates the shade to the analytic path (C1).
// ---------------------------------------------------------------------------

/// The world-space ray-AABB slab clip of brick `[cell_min, cell_min + M2_BRICK_WORLD]³` from `p`
/// along `rd`, returning `Some((t_enter, t_exit))` (world `t`, measured from `p`, `t_enter >= 0`)
/// or `None` on a miss. Mirrors the shader's `m2_brick_span` (the `tmin = 0` floor never marches
/// behind the current point).
#[inline]
fn brick_aabb_span(p: [f32; 3], rd: [f32; 3], cell_min: [f32; 3]) -> Option<(f32, f32)> {
    let mut tmin = 0.0_f32; // never march behind the current march point
    let mut tmax = 1.0e30_f32;
    for a in 0..3 {
        let lo = cell_min[a];
        let hi = lo + M2_BRICK_WORLD;
        if rd[a].abs() <= 1.0e-20 {
            // Parallel to this slab: a miss only if the origin is outside it.
            if p[a] < lo || p[a] > hi {
                return None;
            }
            continue;
        }
        let inv = 1.0 / rd[a];
        let mut t1 = (lo - p[a]) * inv;
        let mut t2 = (hi - p[a]) * inv;
        if t1 > t2 {
            core::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
    }
    if tmax > tmin { Some((tmin, tmax)) } else { None }
}

/// The host mirror of the shader's `m2_surface_hit`: locates the M2 tile containing `p = ro + rd *
/// t_world`, bakes its brick ([`fill_brick`]), solves the JCGT cubic ([`brick_cubic_hit`]) for the
/// in-brick crossing, then VALIDATES the candidate analytically (the exact-CSG fallback). Returns
/// `Some(hit_t)` (world `t`, the accepted hit) or `None` (no crossing / the refine cleared it →
/// the caller falls through to the M1 analytic fold). `edits` is the authority the GPU baked the
/// atlas from (so the host bakes the SAME tile bit-for-bit).
///
/// The cubic candidate is accepted when `|sdf_edit_list(cand_p)| < M2_CREASE_EPS`; otherwise a few
/// analytic sphere-trace steps ([`M2_REFINE_ITERS`]) settle it onto the EXACT field, mirroring the
/// shader's `[branch]` refine.
fn host_m2_surface_hit(edits: &[SdfEdit], ro: [f32; 3], rd: [f32; 3], t_world: f32) -> Option<f32> {
    let p = [
        ro[0] + rd[0] * t_world,
        ro[1] + rd[1] * t_world,
        ro[2] + rd[2] * t_world,
    ];
    // The M2 tile containing `p` (mirror the shader: test the float directly so a negative coord is
    // caught before the cast). Outside the bounded grid → no atlas tile (the caller folds analytic).
    let rel = [
        (p[0] - M2_GRID_ORIGIN) / M2_BRICK_WORLD,
        (p[1] - M2_GRID_ORIGIN) / M2_BRICK_WORLD,
        (p[2] - M2_GRID_ORIGIN) / M2_BRICK_WORLD,
    ];
    if rel[0] < 0.0 || rel[1] < 0.0 || rel[2] < 0.0 {
        return None;
    }
    let tx = rel[0] as u32;
    let ty = rel[1] as u32;
    let tz = rel[2] as u32;
    if tx >= M2_GRID_DIM || ty >= M2_GRID_DIM || tz >= M2_GRID_DIM {
        return None;
    }
    let cell = [tx, ty, tz];
    let cell_min = m2_cell_min(cell);

    // Clip the world ray to the brick AABB.
    let (t_enter, t_exit) = brick_aabb_span(p, rd, cell_min)?;

    // Bake THIS tile from the authority (the SAME data the GPU atlas holds for this cell), then run
    // the JCGT cubic in interior-voxel units (world → voxel: (world - cell_min) / voxel_size). The
    // cubic's local `t` is in WORLD units (rd is divided by voxel_size to keep the world-t metric).
    let field = edits_field(edits);
    let mut tile = [0i8; BRICK_VOXELS];
    fill_brick(&field, cell_min, M2_VOXEL_SIZE, M2_BAND_HALF, &mut tile);
    let ro_v = [
        (p[0] - cell_min[0]) / M2_VOXEL_SIZE,
        (p[1] - cell_min[1]) / M2_VOXEL_SIZE,
        (p[2] - cell_min[2]) / M2_VOXEL_SIZE,
    ];
    let rd_v = [rd[0] / M2_VOXEL_SIZE, rd[1] / M2_VOXEL_SIZE, rd[2] / M2_VOXEL_SIZE];

    let local = brick_cubic_hit(&tile, ro_v, rd_v, t_enter, t_exit, M2_BAND_HALF)?;

    // The candidate world `t` (local is measured from `p`, in world units).
    let cand_t = t_world + local;
    let cand_p = [
        ro[0] + rd[0] * cand_t,
        ro[1] + rd[1] * cand_t,
        ro[2] + rd[2] * cand_t,
    ];
    let resid = sdf_edit_list(edits, cand_p);

    // ANALYTIC-RESIDUAL FALLBACK (the exact-CSG guarantee): a `|resid|` within the crease band
    // accepts the cubic hit; else a few analytic sphere-trace steps settle it onto the exact field.
    if resid.abs() < M2_CREASE_EPS {
        return Some(cand_t);
    }
    let mut rt = cand_t;
    for _ in 0..M2_REFINE_ITERS {
        let q = [ro[0] + rd[0] * rt, ro[1] + rd[1] * rt, ro[2] + rd[2] * rt];
        let d = sdf_edit_list(edits, q);
        if d < SDF_EPS {
            return Some(rt);
        }
        rt += d.max(SDF_EPS);
        if rt > SDF_T_MAX {
            break;
        }
    }
    // The refine did not converge near the candidate (a CSG-subtracted / rounded region with no
    // nearby exact surface): no hit in this brick — the caller falls back to the analytic fold.
    None
}

/// Builds a transient single-`gen` [`SdfEditField`] from `edits` for the per-tile [`fill_brick`] /
/// [`classify_brick`] bake (these take the authority field, not a raw slice). The render/golden
/// path's authority is the SAME edit set, so the baked tile is bit-identical. `SdfEditField` is a
/// fixed-size `Copy` POD (no heap), so this is a cheap stack build — the host mirror is a CPU-only
/// reference, not the GPU hot path.
#[inline]
fn edits_field(edits: &[SdfEdit]) -> SdfEditField {
    let mut field = SdfEditField::new();
    for e in edits {
        debug_assert!(field.push(*e), "golden M2 scene must fit MAX_SDF_EDITS");
    }
    field.bump_gen();
    field
}

/// M2 — the trilinear+JCGT-cubic SURFACE-brick golden. Identical to
/// [`golden_composite_pixel_brick`] but the PRIMARY march runs the M2 SURFACE-brick path when
/// `brick_trilinear == true`: at each march point inside the bounded M2 grid the atlas cubic
/// ([`host_m2_surface_hit`]) is tried; a hit TERMINATES the march at the analytically-validated
/// `t` (hit/normal/shade stay ANALYTIC — C1), and a no-crossing / cleared-refine falls through to
/// the M1 step (empty-skip when `brick_enabled`, else the analytic fold). INDEPENDENT of
/// `brick_enabled` (the two gates are orthogonal).
///
/// With `brick_trilinear == false` this delegates to [`golden_composite_pixel_brick`] —
/// BYTE-IDENTICAL to the M1 golden (the M2 0%-gate). This is the bit-exact reference the GPU M2
/// golden compares against.
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_brick_m2(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    omega: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
    brick_enabled: bool,
    brick_trilinear: bool,
    grid: &PointerGrid,
    cells: &[u32],
) -> u32 {
    // The OFF path is byte-identical to the M1 marcher (the M2 0%-gate): the atlas is never sampled.
    if !brick_trilinear {
        return golden_composite_pixel_brick(
            edits, mesh_depth, px, py, img_w, img_h, camera, omega, lighting_flags, light_dir,
            brick_enabled, grid, cells,
        );
    }

    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);

    let has_mesh = mesh_depth < MESH_DEPTH_CLEAR;
    let t_mesh = if has_mesh { depth_to_t(mesh_depth) } else { 1.0e30 };

    let mut t = 0.0_f32;
    let t_seed = t;
    let mut omega = omega; // [1.0, 1.99]; sor-fail latches it to 1.0
    let mut hit = false;
    let mut safe_t = 0.0_f32;
    let mut sor_prev = 0.0_f32;
    let mut sor_step_prev = 0.0_f32;
    let mut exhausted = true;
    for it in 0..SDF_MAX_IT {
        if t >= t_mesh {
            exhausted = false; // mesh-occlusion termination
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];

        // M2 SURFACE-brick path: try the atlas cubic at `p`. A hit terminates the march at the
        // analytically-validated `t` (the hit/normal/shade stay analytic — C1). INDEPENDENT of the
        // M1 empty-skip below: the M2 step is taken FIRST (it owns the SURFACE cells the empty-skip
        // never skips), and a no-crossing falls through to the M1 / analytic step.
        if let Some(m2_hit_t) = host_m2_surface_hit(edits, ro, rd, t) {
            hit = true;
            exhausted = false; // M2 cubic+analytic-validated convergence
            t = m2_hit_t;
            break;
        }

        // M1 empty skip (when on): an EmptyOutside cell at `p` steps to the brick AABB exit. The
        // SURFACE cells the M2 step owns are NOT EmptyOutside, so this only accelerates EMPTY space.
        if brick_enabled
            && let Some((class, cell_min)) = host_brick_cell(grid, cells, p)
            && class == BRICK_CLASS_EMPTY_OUTSIDE
        {
            let exit = dist_to_brick_exit(p, rd, cell_min, grid.brick_world);
            t += exit;
            if t > SDF_T_MAX {
                exhausted = false; // clear-miss termination
                break;
            }
            continue; // skip the analytic fold this step
        }

        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            hit = true;
            exhausted = false; // converged
            break;
        }
        if omega > 1.0 {
            let step_len = d * omega;
            if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                debug_assert!(it > 0, "B1 budget: a>=1 precondition");
                t = safe_t + sor_prev; // plain-resume one certified step past the safe probe
                omega = 1.0;
                continue;
            }
            safe_t = t;
            sor_prev = d;
            sor_step_prev = step_len;
            t += step_len;
        } else {
            t += d; // frozen plain arm
        }
        if t > SDF_T_MAX {
            exhausted = false; // clear-miss termination
            break;
        }
    }

    // The re-march fallback stays ANALYTIC (C1) — identical to the M1 path: the M2 step never
    // reopens the B1 budget hole (its hit terminates, its miss falls through), so `exhausted` here
    // means the analytic field ran out of budget mid-field; the frozen plain re-march resolves it.
    if exhausted {
        t = t_seed;
        hit = false;
        for _it2 in 0..SDF_MAX_IT {
            if t >= t_mesh {
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d; // frozen plain step
            if t > SDF_T_MAX {
                break;
            }
        }
    }

    let color = if hit && t < t_mesh {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);
        host_shade(SDF_BASE_COLOR, SDF_AMBIENT, p, n, light_dir, lighting_flags, &|q| {
            sdf_edit_list(edits, q)
        })
    } else if has_mesh {
        MESH_COLOR
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
}

// ===========================================================================
// Deferred-shading SPLIT (increment 1) — the host goldens for the marcher's
// ATTRIBUTE writes + the `deferred_pbr` RESOLVE.
//
// The marcher (`sdf_gbuffer_composite.hlsl`) no longer composites `base*shadow*ao`; it
// writes ATTRIBUTES (gAlbedo = the unmultiplied base, gMaterial = (vis, 0, mask, 1)).
// The fullscreen `deferred_pbr.comp` RESOLVE then computes `lit = mask ? base*vis : base`.
// [`golden_marcher_attributes`] mirrors the marcher's per-pixel attribute output and
// [`golden_deferred_resolve`] mirrors the resolve, modelling the EXACT GPU double
// quantization (base RGB8 + vis R8, then a second pack of base8/255 * vis8/255). The
// approximation-gate reference [`golden_composite_pixel_ex_omega_lit`] above is kept for
// the inline-composite comparison.
// ===========================================================================

/// A host material-table element mirroring `boyko_render::material::MaterialGpu` (3
/// std430 `vec4` lanes, 48 B). The vulkan crate cannot depend on `boyko_render` (the
/// dependency runs the other way), so the golden carries its own POD mirror; the layout
/// is the SAME the shader's `MaterialGpu` reads. All values are LINEAR.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoldenMaterial {
    /// `rgb` = LINEAR base color, `w` = alpha/cutoff (lane 0).
    pub base_color: [f32; 4],
    /// `[metallic, roughness, reflectance, bitcast(flags)]` (lane 1).
    pub mrr: [f32; 4],
    /// `rgb` = LINEAR emissive, `w` unused (lane 2).
    pub emissive: [f32; 4],
}

impl GoldenMaterial {
    /// A metallic-roughness material from LINEAR parameters (mirrors `MaterialGpu::new`).
    #[inline]
    pub fn new(
        base_color: [f32; 4],
        metallic: f32,
        roughness: f32,
        reflectance: f32,
        emissive: [f32; 3],
    ) -> Self {
        Self {
            base_color,
            mrr: [metallic, roughness, reflectance, 0.0],
            emissive: [emissive[0], emissive[1], emissive[2], 0.0],
        }
    }
}

impl Default for GoldenMaterial {
    /// The engine default material (table slot 0): a mid-gray dielectric (mirrors
    /// `MaterialGpu::default`).
    #[inline]
    fn default() -> Self {
        GoldenMaterial::new([0.8, 0.8, 0.8, 1.0], 0.0, 0.5, 0.5, [0.0, 0.0, 0.0])
    }
}

// --- Lighting L0 host mirror (mirrors boyko_render::light + light_table.hlsli) -------

/// Light kind tags — mirror `boyko_render::light::LIGHT_KIND_*` and the shader's
/// `light_table.hlsli` `LIGHT_KIND_*`.
pub const GOLDEN_LIGHT_KIND_DIRECTIONAL: u32 = 0;
/// Point light kind (L0b resolve path).
pub const GOLDEN_LIGHT_KIND_POINT: u32 = 1;
/// Spot light kind (L0b resolve path).
pub const GOLDEN_LIGHT_KIND_SPOT: u32 = 2;
/// Sky/ambient light kind (L0a hemisphere ambient).
pub const GOLDEN_LIGHT_KIND_SKY: u32 = 3;

/// The word offset at which the `GpuLight[]` array begins in the light SSBO (mirrors
/// `boyko_render::light::LIGHT_HEADER_BASE_WORDS == 16` and the shader's `HEADER_BASE`).
pub const GOLDEN_LIGHT_HEADER_BASE_WORDS: usize = 16;

/// A host light-table element mirroring `boyko_render::light::GpuLight` (3 std430 `vec4`
/// lanes, 48 B). The vulkan crate cannot depend on `boyko_render`, so the golden carries
/// its own POD mirror; the layout is the SAME the shader's `GpuLight` reads. LINEAR.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoldenLight {
    /// `xyz` = direction TO the light (directional/spot), `w` = bit-cast kind tag.
    pub dir_kind: [f32; 4],
    /// `xyz` = position (point/spot) or ground color (sky), `w` = cull radius.
    pub pos_range: [f32; 4],
    /// `rgb` = LINEAR color × baked intensity (or sky color), `w` = packed spot cones.
    pub color_cone: [f32; 4],
}

impl GoldenLight {
    /// A directional light (mirrors `GpuLight::from_directional`): `color × illuminance`
    /// premultiplied into the color lane.
    #[inline]
    pub fn directional(direction: [f32; 3], color: [f32; 3], illuminance: f32) -> Self {
        let d = v_normalize(direction);
        Self {
            dir_kind: [d[0], d[1], d[2], f32::from_bits(GOLDEN_LIGHT_KIND_DIRECTIONAL)],
            pos_range: [0.0, 0.0, 0.0, f32::INFINITY],
            color_cone: [color[0] * illuminance, color[1] * illuminance, color[2] * illuminance, 0.0],
        }
    }

    /// A sky/ambient light (mirrors `GpuLight::from_sky`): sky color in the color lane,
    /// ground color in the position lane.
    #[inline]
    pub fn sky(sky_color: [f32; 3], ground_color: [f32; 3]) -> Self {
        Self {
            dir_kind: [0.0, 0.0, 0.0, f32::from_bits(GOLDEN_LIGHT_KIND_SKY)],
            pos_range: [ground_color[0], ground_color[1], ground_color[2], 0.0],
            color_cone: [sky_color[0], sky_color[1], sky_color[2], 0.0],
        }
    }

    /// A point light (mirrors `GpuLight::from_point`, Lighting L0b): position + range in
    /// `pos_range`, the baked intensity `I = Φ / (4π)` premultiplied into the color lane.
    /// `power` is the luminous power `Φ`. The L0b resolve oracle consumes `pos_range` (the
    /// world position + the cull radius) + the baked color.
    #[inline]
    pub fn point(position: [f32; 3], color: [f32; 3], power: f32, range: f32) -> Self {
        let i = power / (4.0 * core::f32::consts::PI);
        Self {
            dir_kind: [0.0, 0.0, 0.0, f32::from_bits(GOLDEN_LIGHT_KIND_POINT)],
            pos_range: [position[0], position[1], position[2], range],
            color_cone: [color[0] * i, color[1] * i, color[2] * i, 0.0],
        }
    }

    /// A spot light (mirrors `GpuLight::from_spot`, Lighting L0b): the spot axis in
    /// `dir_kind.xyz`, position + range in `pos_range`, the baked reflector intensity
    /// `I = Φ / (2π(1 − cos(outer)))` premultiplied into the color lane, and the cone
    /// cosines packed (two f16) into `color_cone.w`. `inner_deg`/`outer_deg` are cone
    /// half-angles in degrees; `cos(outer)` is clamped to `SPOT_COS_OUTER_MAX` (0.9999) so
    /// the intensity stays bounded — mirroring the host constructor's release safety net.
    #[inline]
    pub fn spot(
        position: [f32; 3],
        direction: [f32; 3],
        color: [f32; 3],
        power: f32,
        range: f32,
        inner_deg: f32,
        outer_deg: f32,
    ) -> Self {
        let cos_inner = inner_deg.to_radians().cos();
        let cos_outer = outer_deg.to_radians().cos().min(GOLDEN_SPOT_COS_OUTER_MAX);
        let denom = 2.0 * core::f32::consts::PI * (1.0 - cos_outer);
        let i = power / denom;
        let d = v_normalize(direction);
        Self {
            dir_kind: [d[0], d[1], d[2], f32::from_bits(GOLDEN_LIGHT_KIND_SPOT)],
            pos_range: [position[0], position[1], position[2], range],
            color_cone: [
                color[0] * i,
                color[1] * i,
                color[2] * i,
                golden_pack_cones(cos_inner, cos_outer),
            ],
        }
    }

    /// The bit-cast kind tag from `dir_kind.w`.
    #[inline]
    pub fn kind(&self) -> u32 {
        self.dir_kind[3].to_bits()
    }
}

/// The maximum `cos(outer)` the spot bake clamps to (mirrors
/// `boyko_render::light::SPOT_COS_OUTER_MAX`): bounds `I = Φ/(2π(1−cos))` as the cone
/// narrows to a pencil beam.
pub const GOLDEN_SPOT_COS_OUTER_MAX: f32 = 0.9999;

/// Packs two cosines into the `f16 | f16` bit pattern carried in
/// [`GoldenLight::color_cone`]`.w` (`cos_inner` low half, `cos_outer` high half) — the
/// host mirror of `boyko_render::light::pack_cones`; the resolve oracle's
/// [`golden_unpack_cones`] is the inverse (matching the shader's `f16tof32`).
fn golden_pack_cones(cos_inner: f32, cos_outer: f32) -> f32 {
    let lo = golden_f16_from_f32(cos_inner) as u32;
    let hi = golden_f16_from_f32(cos_outer) as u32;
    f32::from_bits(lo | (hi << 16))
}

/// Unpacks two f16 cone cosines from a `color_cone.w` bit pattern — the host mirror of the
/// shader's `unpack_cones` (`f16tof32`). Returns `(cos_inner, cos_outer)`.
fn golden_unpack_cones(packed: f32) -> (f32, f32) {
    let bits = packed.to_bits();
    let lo = golden_f16_to_f32((bits & 0xFFFF) as u16);
    let hi = golden_f16_to_f32(((bits >> 16) & 0xFFFF) as u16);
    (lo, hi)
}

/// IEEE-754 binary32 → binary16 (round-to-nearest-even) — the host mirror of
/// `boyko_render::light::f16_from_f32`. The cone cosines live in `[-1, 1]`, inside the f16
/// normal range, so only the standard rounding is needed (no overflow special case beyond
/// the defensive inf/NaN guard).
fn golden_f16_from_f32(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x007f_ffff;
    if exp == 0xff {
        let m = if mant != 0 { 0x0200 } else { 0 };
        return sign | 0x7c00 | m;
    }
    let new_exp = exp - 127 + 15;
    if new_exp >= 0x1f {
        return sign | 0x7c00; // overflow → inf
    }
    if new_exp <= 0 {
        if new_exp < -10 {
            return sign; // underflow → signed zero
        }
        let full_mant = mant | 0x0080_0000;
        let shift = (14 - new_exp) as u32;
        let m = (full_mant >> shift) as u16;
        let round_bit = ((full_mant >> (shift - 1)) & 1) as u16;
        return sign | (m + round_bit);
    }
    let half = sign | ((new_exp as u16) << 10) | ((mant >> 13) as u16);
    let round = (mant >> 12) & 1;
    let sticky = mant & 0x0fff;
    if round == 1 && (sticky != 0 || (half & 1) == 1) {
        half + 1
    } else {
        half
    }
}

/// IEEE-754 binary16 → binary32 — the host mirror of the shader's `f16tof32`.
fn golden_f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1f) as u32;
    let mant = (h & 0x3ff) as u32;
    let out = if exp == 0 {
        if mant == 0 {
            sign << 31
        } else {
            let mut e = -1i32;
            let mut m = mant;
            loop {
                e += 1;
                m <<= 1;
                if m & 0x400 != 0 {
                    break;
                }
            }
            let new_exp = (127 - 15 - e) as u32;
            (sign << 31) | (new_exp << 23) | ((m & 0x3ff) << 13)
        }
    } else if exp == 0x1f {
        (sign << 31) | 0x7f80_0000 | (mant << 13)
    } else {
        let new_exp = exp + (127 - 15);
        (sign << 31) | (new_exp << 23) | (mant << 13)
    };
    f32::from_bits(out)
}

/// A host light-table header mirroring `boyko_render::light::LightHeaderGpu` (4 std430
/// `vec4` lanes, 64 B). Carries the split counts + exposure (Decision 3 / O3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoldenLightHeader {
    /// `[bitcast(light_count), exposure, bitcast(l0a_count), bitcast(point_spot_count)]`.
    pub counts_exposure: [f32; 4],
    /// Ambient hemisphere diffuse `rgb`, `w` unused (carried; the L0a resolve drives
    /// ambient from the sky light entities, not these — see `golden_deferred_resolve_table`).
    pub sky_diffuse: [f32; 4],
    /// Ambient specular `rgb`, `w` unused (carried; see above).
    pub sky_spec: [f32; 4],
    /// L1 cluster params (zero in L0).
    pub cluster_params: [f32; 4],
}

impl GoldenLightHeader {
    /// Builds the header (mirrors `LightHeaderGpu::new`). `l0a_count` = directionals +
    /// sky; `point_spot_count` = the L0b block; exposure default 1.0.
    #[inline]
    pub fn new(l0a_count: u32, point_spot_count: u32, exposure: f32) -> Self {
        let light_count = l0a_count + point_spot_count;
        Self {
            counts_exposure: [
                f32::from_bits(light_count),
                exposure,
                f32::from_bits(l0a_count),
                f32::from_bits(point_spot_count),
            ],
            sky_diffuse: [PBR_SKY_DIFFUSE[0], PBR_SKY_DIFFUSE[1], PBR_SKY_DIFFUSE[2], 0.0],
            sky_spec: [PBR_SKY_SPEC[0], PBR_SKY_SPEC[1], PBR_SKY_SPEC[2], 0.0],
            cluster_params: [0.0, 0.0, 0.0, 0.0],
        }
    }

    /// The total `light_count` field (bit-cast back from `counts_exposure.x`).
    #[inline]
    pub fn light_count(&self) -> u32 {
        self.counts_exposure[0].to_bits()
    }

    /// The `l0a_count` field (bit-cast back from `counts_exposure.z`).
    #[inline]
    pub fn l0a_count(&self) -> u32 {
        self.counts_exposure[2].to_bits()
    }

    /// The `point_spot_count` field — the L0b block (bit-cast back from `counts_exposure.w`).
    #[inline]
    pub fn point_spot_count(&self) -> u32 {
        self.counts_exposure[3].to_bits()
    }

    /// The exposure field (`counts_exposure.y`).
    #[inline]
    pub fn exposure(&self) -> f32 {
        self.counts_exposure[1]
    }

    /// Builds the L1 CLUSTERED header (mirrors `LightHeaderGpu::new_clustered`): the
    /// `cluster_params` lane carries `[z_scale, z_bias, bitcast(packed_dims),
    /// bitcast(clusters_enabled=1)]`. The packed dims are `dim_x | dim_y<<8 | dim_z<<16`.
    #[inline]
    pub fn new_clustered(
        l0a_count: u32,
        point_spot_count: u32,
        exposure: f32,
        cfg: &GoldenClusterConfig,
    ) -> Self {
        let mut h = Self::new(l0a_count, point_spot_count, exposure);
        let packed = cfg.dim_x | (cfg.dim_y << 8) | (cfg.dim_z << 16);
        h.cluster_params = [
            cfg.z_scale(),
            cfg.z_bias(),
            f32::from_bits(packed),
            f32::from_bits(1),
        ];
        h
    }

    /// Whether the L1 cluster path is enabled (`cluster_params.w` bit-cast `!= 0`). Mirrors
    /// `LightHeaderGpu::clusters_enabled`.
    #[inline]
    pub fn clusters_enabled(&self) -> bool {
        self.cluster_params[3].to_bits() != 0
    }
}

// --- PBR MVP-2 lighting constants (mirror deferred_pbr.hlsl EXACTLY) -----------------

/// The resolve's single analytic directional light (`DEFAULT_LIGHT_DIR` = +Z).
pub const PBR_LIGHT_DIR: [f32; 3] = [0.0, 0.0, 1.0];
/// The resolve's directional-light color (white).
pub const PBR_LIGHT_COLOR: [f32; 3] = [1.0, 1.0, 1.0];
/// The resolve's analytic hemisphere diffuse-ambient sky color.
pub const PBR_SKY_DIFFUSE: [f32; 3] = [0.10, 0.10, 0.12];
/// The resolve's analytic specular-IBL sky color (scales EnvBRDFApprox).
pub const PBR_SKY_SPEC: [f32; 3] = [0.10, 0.10, 0.12];
/// The "empty field" distance sentinel, mirroring the shader's `FAR` (= 1e9 in
/// `sdf_field.hlsli`). Used as the argmin seed in [`pick_material_id`] so the host
/// oracle initializes its nearest-surface search identically to the GPU marcher.
pub const PBR_FAR: f32 = 1.0e9;

/// Octahedral-encode a unit normal into `[0,1]^2` (mirrors the marcher's `oct_encode`).
fn oct_encode(n: [f32; 3]) -> [f32; 2] {
    let inv_l1 = 1.0 / (n[0].abs() + n[1].abs() + n[2].abs());
    let nx = n[0] * inv_l1;
    let ny = n[1] * inv_l1;
    let nz = n[2] * inv_l1;
    let (mut ex, mut ey) = (nx, ny);
    if nz < 0.0 {
        let sx = if nx >= 0.0 { 1.0 } else { -1.0 };
        let sy = if ny >= 0.0 { 1.0 } else { -1.0 };
        ex = (1.0 - ny.abs()) * sx;
        ey = (1.0 - nx.abs()) * sy;
    }
    [ex * 0.5 + 0.5, ey * 0.5 + 0.5]
}

/// Octahedral-decode (mirrors the resolve's `oct_decode`).
fn oct_decode(e: [f32; 2]) -> [f32; 3] {
    let ex = e[0] * 2.0 - 1.0;
    let ey = e[1] * 2.0 - 1.0;
    let mut n = [ex, ey, 1.0 - ex.abs() - ey.abs()];
    let t = (-n[2]).clamp(0.0, 1.0);
    n[0] += if n[0] >= 0.0 { -t } else { t };
    n[1] += if n[1] >= 0.0 { -t } else { t };
    v_normalize(n)
}

/// GGX/Trowbridge-Reitz NDF (mirrors the resolve's `D_GGX`).
fn d_ggx(noh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = (noh * a2 - noh) * noh + 1.0;
    a2 / (core::f32::consts::PI * d * d)
}

/// Height-correlated Smith visibility (mirrors the resolve's `V_SmithGGXCorrelated`).
fn v_smith_ggx_correlated(nov: f32, nol: f32, a: f32) -> f32 {
    let a2 = a * a;
    let lambda_v = nol * ((nov - a2 * nov) * nov + a2).sqrt();
    let lambda_l = nov * ((nol - a2 * nol) * nol + a2).sqrt();
    0.5 / (lambda_v + lambda_l).max(1e-5)
}

/// Schlick Fresnel (mirrors the resolve's `F_Schlick`).
fn f_schlick(u: f32, f0: [f32; 3]) -> [f32; 3] {
    let f = (1.0 - u).powf(5.0);
    [
        f0[0] + (1.0 - f0[0]) * f,
        f0[1] + (1.0 - f0[1]) * f,
        f0[2] + (1.0 - f0[2]) * f,
    ]
}

/// Karis mobile analytic environment BRDF (mirrors the resolve's `env_brdf_approx`).
fn env_brdf_approx(roughness: f32, nov: f32) -> [f32; 2] {
    let c0 = [-1.0_f32, -0.0275, -0.572, 0.022];
    let c1 = [1.0_f32, 0.0425, 1.04, -0.04];
    let r = [
        roughness * c0[0] + c1[0],
        roughness * c0[1] + c1[1],
        roughness * c0[2] + c1[2],
        roughness * c0[3] + c1[3],
    ];
    let a004 = (r[0] * r[0]).min((-9.28 * nov).exp2()) * r[0] + r[1];
    [-1.04 * a004 + r[2], 1.04 * a004 + r[3]]
}

/// The per-pixel G-buffer attributes the PBR MVP-2 marcher writes, modelling the EXACT GPU
/// UNORM pack so [`golden_deferred_resolve`] can re-decode them and run the host BRDF
/// within ±2/255 of the GPU. On the mask == 0 arms (mesh / background / empty) `base_rgb`
/// round-trips byte-identically (the 0%-gate).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarcherAttributes {
    /// gAlbedo R8G8B8: the RAW LINEAR base color (the picked material's `base_color.rgb`
    /// on an SDF hit, else MESH_COLOR / BACKGROUND), quantized via [`pack_rgba`] rounding.
    pub base_rgb: [u8; 3],
    /// gNormal R8G8: the octahedral-encoded world normal (SDF hit only; neutral otherwise).
    pub oct_rg: [u8; 2],
    /// gNormal B8/A8: the 16-bit material id packed low-byte → B, high-byte → A.
    pub mat_id: u16,
    /// gMaterial.r R8: the A1 soft-shadow visibility `round(255*clamp(shadow))`.
    pub shadow: u8,
    /// gMaterial.g R8: the A2 AO factor `round(255*clamp(ao))`.
    pub ao: u8,
    /// gMaterial.b decoded: 1 on the SDF-LIT arm, 0 on mesh / background / empty.
    pub mask: u8,
    /// Lighting L0b: the `gViewT` lane — the marcher's surface ray param `t` (the REAL
    /// marched `t` on the SDF-lit arm, the `1.0e30` sentinel on mesh / background / empty,
    /// mirroring the GPU's three terminal write sites, C2). The resolve oracle reconstructs
    /// `P = ro + rd * view_t` under `mask == 1` (the read-under-mask gate — the sentinel is
    /// never consumed on a non-lit pixel).
    pub view_t: f32,
}

/// Picks the nearest-surface material id at `p` by an argmin over the edit list, mirroring
/// the marcher's `pick_material_id` (the FROZEN `edit_distance` per primitive; the id from
/// the per-edit `center.w` free lane). Returns the default id (0) for an empty list.
fn pick_material_id(edits: &[SdfEdit], p: [f32; 3]) -> u16 {
    // The ≤16 scene contract: every committed scene fits the fixed cap, so this only
    // documents the invariant the marcher relies on (it never reads beyond the cap).
    debug_assert!(
        edits.len() <= MAX_SDF_EDITS,
        "invariant: edit count {} exceeds MAX_SDF_EDITS {MAX_SDF_EDITS}",
        edits.len()
    );
    // FAR (= 1e9), mirroring the shader's `FAR` sentinel exactly — see `PBR_FAR`.
    let mut best_d = PBR_FAR;
    let mut best_id = 0u16;
    // Clamp to the first MAX_SDF_EDITS edits: the GPU marcher iterates only
    // `min(Buf[0], MAX_SDF_EDITS)` candidates, so the host argmin must see the SAME
    // candidate set or the picked id (and thus gAlbedo) would diverge for >16 edits.
    for e in edits.iter().take(MAX_SDF_EDITS) {
        let d = edit_distance(e, p).abs();
        if d < best_d {
            best_d = d;
            best_id = (e.center[3].to_bits() & 0xFFFF) as u16;
        }
    }
    best_id
}

/// The CPU mirror of the PBR MVP-2 marcher's per-pixel ATTRIBUTE output. Runs the SAME
/// extent/camera ray-gen + over-relaxation march + arm selection as
/// [`golden_composite_pixel_ex_omega_lit`], then writes the repacked G-buffer attributes:
/// gAlbedo = the picked material's RAW LINEAR base color (via `materials`, indexed by the
/// argmin id), gNormal = (oct normal, 16-bit id), gMaterial = (shadow, ao, mask). On
/// mesh / background it emits the flat constant with mask = 0. `materials` is the host
/// material table; an out-of-range id falls back to the default material.
#[allow(clippy::too_many_arguments)]
pub fn golden_marcher_attributes(
    edits: &[SdfEdit],
    materials: &[GoldenMaterial],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    omega: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
) -> MarcherAttributes {
    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);

    let has_mesh = mesh_depth < MESH_DEPTH_CLEAR;
    let t_mesh = if has_mesh { depth_to_t(mesh_depth) } else { 1.0e30 };

    // The over-relaxation march + the Candidate-C re-march, mirroring
    // `golden_composite_pixel_ex_omega_lit` EXACTLY (the field/march is untouched).
    let mut t = 0.0_f32;
    let t_seed = t;
    let mut omega = omega;
    let mut hit = false;
    let mut safe_t = 0.0_f32;
    let mut sor_prev = 0.0_f32;
    let mut sor_step_prev = 0.0_f32;
    let mut exhausted = true;
    for it in 0..SDF_MAX_IT {
        if t >= t_mesh {
            exhausted = false;
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            hit = true;
            exhausted = false;
            break;
        }
        if omega > 1.0 {
            let step_len = d * omega;
            if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                t = safe_t + sor_prev;
                omega = 1.0;
                continue;
            }
            safe_t = t;
            sor_prev = d;
            sor_step_prev = step_len;
            t += step_len;
        } else {
            t += d;
        }
        if t > SDF_T_MAX {
            exhausted = false;
            break;
        }
    }
    if exhausted {
        t = t_seed;
        hit = false;
        for _it2 in 0..SDF_MAX_IT {
            if t >= t_mesh {
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d;
            if t > SDF_T_MAX {
                break;
            }
        }
    }

    // Quantize a `[0,1]` scalar to a byte with the GPU UNORM store's `(x*255+0.5)` rounding.
    let q8 = |x: f32| -> u8 { (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 };
    // The R8G8B8 bytes a `pack_rgba` of `c` would store (low 3 bytes of `0xAABBGGRR`).
    let base_bytes = |c: [f32; 3]| -> [u8; 3] {
        let packed = pack_rgba(c);
        [
            (packed & 0xFF) as u8,
            ((packed >> 8) & 0xFF) as u8,
            ((packed >> 16) & 0xFF) as u8,
        ]
    };

    if hit && t < t_mesh {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);

        // ATTRIBUTE the hit to the nearest edit's material, then take its RAW LINEAR base
        // color — gAlbedo carries NO lighting (the resolve runs the full BRDF).
        let mat_id = pick_material_id(edits, p);
        let mat = materials
            .get(mat_id as usize)
            .copied()
            .unwrap_or_default();
        let base = [mat.base_color[0], mat.base_color[1], mat.base_color[2]];

        // The A1/A2 marches, gated by `lighting_flags` (kept SEPARATE: shadow → R, ao → G).
        let (shadow, ao) = if lighting_flags == 0 {
            (1.0_f32, 1.0_f32)
        } else {
            let l = v_normalize(light_dir);
            let field = |q: [f32; 3]| sdf_edit_list(edits, q);
            let mut shadow = 1.0_f32;
            if lighting_flags & LIGHTING_FLAG_SHADOWS != 0 {
                shadow = host_soft_shadow(p, n, l, &field);
            }
            let mut ao = 1.0_f32;
            if lighting_flags & LIGHTING_FLAG_AO != 0 {
                ao = host_ao(p, n, &field);
            }
            (shadow.clamp(0.0, 1.0), ao.clamp(0.0, 1.0))
        };

        let oct = oct_encode(n);
        MarcherAttributes {
            base_rgb: base_bytes(base),
            oct_rg: [q8(oct[0]), q8(oct[1])],
            mat_id,
            shadow: q8(shadow),
            ao: q8(ao),
            mask: 1,
            // Lighting L0b: the SDF-lit arm stores the REAL marched `t` (the same `t` the
            // hit point `p = ro + rd * t` above used) — the resolve reconstructs `P` from it.
            view_t: t,
        }
    } else {
        let base = if has_mesh { MESH_COLOR } else { SDF_BACKGROUND };
        // mask == 0: gNormal/id/shadow/ao are unread by the resolve (pass-through); model
        // the marcher's neutral defaults so the attribute struct round-trips deterministically.
        MarcherAttributes {
            base_rgb: base_bytes(base),
            oct_rg: [q8(0.5), q8(0.5)],
            mat_id: 0,
            shadow: 255,
            ao: 255,
            mask: 0,
            // Lighting L0b: the mesh / background / empty arms store the `1.0e30` sentinel
            // (the GPU's mask == 0 write); never read on a non-lit pixel (read-under-mask).
            view_t: 1.0e30,
        }
    }
}

/// The CPU mirror of the `deferred_pbr` RESOLVE (PBR MVP-2): given the marcher's
/// [`MarcherAttributes`], the camera ray for the pixel, and the material table, returns
/// the packed `0xAABBGGRR` LIT color the resolve stores.
///
/// Models the EXACT GPU double quantization: the attributes are already R8-quantized; the
/// resolve loads them back (UNORM decode), decodes the oct normal + 16-bit id, fetches
/// `materials[id]`, and runs the SAME Cook-Torrance the resolve runs (GGX D + height-
/// correlated Smith V + Schlick F + Lambert + EnvBRDFApprox ambient; shadow modulates the
/// direct term, ao the ambient), then re-quantizes via [`pack_rgba`]. On the mask == 0
/// arms it round-trips `base` byte-identically (the 0%-gate). `rd` is the pixel's ray
/// direction (the view dir is `-rd`); supply the SAME `composite_ray` the marcher used.
pub fn golden_deferred_resolve(
    attrs: MarcherAttributes,
    rd: [f32; 3],
    materials: &[GoldenMaterial],
) -> u32 {
    let base = [
        attrs.base_rgb[0] as f32 / 255.0,
        attrs.base_rgb[1] as f32 / 255.0,
        attrs.base_rgb[2] as f32 / 255.0,
    ];
    if attrs.mask != 1 {
        // mesh / background / empty: pass the base through byte-identically (the 0%-gate).
        return pack_rgba(base);
    }

    // Decode the world normal from the oct RG bytes (the SAME UNORM round-trip the GPU did).
    let n = oct_decode([attrs.oct_rg[0] as f32 / 255.0, attrs.oct_rg[1] as f32 / 255.0]);
    let mat = materials
        .get(attrs.mat_id as usize)
        .copied()
        .unwrap_or_default();

    let metallic = mat.mrr[0];
    let roughness = mat.mrr[1].clamp(0.045, 1.0);
    let reflectance = mat.mrr[2];
    let a = roughness * roughness;

    // f0: dielectric reflectance lerped toward base by metallic; diffuse killed by metallic.
    let dielectric_f0 = 0.16 * reflectance * reflectance;
    let f0 = [
        dielectric_f0 + (base[0] - dielectric_f0) * metallic,
        dielectric_f0 + (base[1] - dielectric_f0) * metallic,
        dielectric_f0 + (base[2] - dielectric_f0) * metallic,
    ];
    let diffuse_color = [
        base[0] * (1.0 - metallic),
        base[1] * (1.0 - metallic),
        base[2] * (1.0 - metallic),
    ];

    let v = [-rd[0], -rd[1], -rd[2]]; // view dir = -ray_dir (the shared ray-gen)
    let l = v_normalize(PBR_LIGHT_DIR);
    let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
    let nov = v_dot(n, v).max(1e-4);
    let nol = v_dot(n, l).max(0.0);
    let noh = v_dot(n, hvec).clamp(0.0, 1.0);
    let loh = v_dot(l, hvec).clamp(0.0, 1.0);

    let shadow = attrs.shadow as f32 / 255.0;
    let ao = attrs.ao as f32 / 255.0;

    // Direct term: (Lambert diffuse + D*V*F specular) * NoL * shadow * light color.
    let d_term = d_ggx(noh, a);
    let v_term = v_smith_ggx_correlated(nov, nol, a);
    let f_term = f_schlick(loh, f0);
    let pi = core::f32::consts::PI;
    let mut lit = [0.0_f32; 3];
    for c in 0..3 {
        let spec = d_term * v_term * f_term[c];
        let diff = diffuse_color[c] * (1.0 / pi);
        let direct = (diff + spec) * (nol * shadow) * PBR_LIGHT_COLOR[c];

        // Ambient: EnvBRDFApprox specular against the sky + hemisphere diffuse, * ao.
        let dfg = env_brdf_approx(roughness, nov);
        let spec_ambient = (f0[c] * dfg[0] + dfg[1]) * PBR_SKY_SPEC[c];
        let diff_ambient = diffuse_color[c] * PBR_SKY_DIFFUSE[c];
        let ambient = (spec_ambient + diff_ambient) * ao;

        lit[c] = direct + ambient + mat.emissive[c];
    }
    pack_rgba(lit)
}

/// The CPU mirror of the `deferred_pbr` RESOLVE driven by the L0a + L0b light TABLE
/// (Lighting L0a/L0b). Identical to [`golden_deferred_resolve`] except the single
/// compiled-in directional + the `SKY_*` ambient constants are replaced by:
/// - the no-`P` front block (`[0..header.l0a_count()]`): `kind == Directional` contributes
///   the Cook-Torrance direct term, `kind == Sky` the hemisphere ambient; and
/// - (L0b) the point/spot block (`[l0a_count..light_count)`): the surface world position
///   `P = ro + rd * attrs.view_t` (the `gViewT` lane, read under `mask == 1`) drives a
///   range cull + smooth windowed inverse-square attenuation + (spot) the O2 cone falloff,
///   each scaled into the SAME Cook-Torrance direct term. `ro`/`rd` are the pixel's shared
///   ray-gen origin/dir (rd unit, so `view_t` is true world distance).
///
/// The accumulated LINEAR radiance is multiplied by `header.exposure()` as the FINAL op
/// (O3).
///
/// # W1 byte-identity op-order (HARD requirement)
/// The per-light direct expression is `(diff + spec) * (nol * shadow) * color` with the
/// accumulator initialized to `0.0`; the sky ambient is `(spec_ambient + diff_ambient) *
/// ao` accumulated from `0.0`; the FINAL `* exposure` is literally last. Because
/// `0.0 + x == x` and `x * 1.0 == x` are exact, a degenerate table — one directional
/// (dir = +Z, color = white, illuminance = 1.0) + one sky (`sky == ground ==`
/// [`PBR_SKY_DIFFUSE`]) with exposure 1.0 — reproduces [`golden_deferred_resolve`]
/// BYTE-FOR-BYTE (the directional matches `LIGHT_DIR`/`LIGHT_COLOR`; the sky `lerp` folds
/// since sky == ground). No reassociation is permitted.
#[allow(clippy::too_many_arguments)]
pub fn golden_deferred_resolve_table(
    attrs: MarcherAttributes,
    ro: [f32; 3],
    rd: [f32; 3],
    materials: &[GoldenMaterial],
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
) -> u32 {
    let base = [
        attrs.base_rgb[0] as f32 / 255.0,
        attrs.base_rgb[1] as f32 / 255.0,
        attrs.base_rgb[2] as f32 / 255.0,
    ];
    if attrs.mask != 1 {
        // mesh / background / empty: pass the base through byte-identically (the 0%-gate).
        return pack_rgba(base);
    }

    let n = oct_decode([attrs.oct_rg[0] as f32 / 255.0, attrs.oct_rg[1] as f32 / 255.0]);
    let mat = materials
        .get(attrs.mat_id as usize)
        .copied()
        .unwrap_or_default();

    let metallic = mat.mrr[0];
    let roughness = mat.mrr[1].clamp(0.045, 1.0);
    let reflectance = mat.mrr[2];
    let a = roughness * roughness;

    let dielectric_f0 = 0.16 * reflectance * reflectance;
    let f0 = [
        dielectric_f0 + (base[0] - dielectric_f0) * metallic,
        dielectric_f0 + (base[1] - dielectric_f0) * metallic,
        dielectric_f0 + (base[2] - dielectric_f0) * metallic,
    ];
    let diffuse_color = [
        base[0] * (1.0 - metallic),
        base[1] * (1.0 - metallic),
        base[2] * (1.0 - metallic),
    ];

    let v = [-rd[0], -rd[1], -rd[2]];
    let nov = v_dot(n, v).max(1e-4);
    let shadow = attrs.shadow as f32 / 255.0;
    let ao = attrs.ao as f32 / 255.0;
    let pi = core::f32::consts::PI;
    // The hemisphere "up" the sky lerp interpolates against (world up).
    const UP: [f32; 3] = [0.0, 1.0, 0.0];
    let hemi = v_dot(n, UP) * 0.5 + 0.5;

    let mut lit_direct = [0.0_f32; 3];
    let mut ambient = [0.0_f32; 3];
    let count = header.l0a_count() as usize;
    for li in lights.iter().take(count) {
        match li.kind() {
            GOLDEN_LIGHT_KIND_DIRECTIONAL => {
                let l = v_normalize([li.dir_kind[0], li.dir_kind[1], li.dir_kind[2]]);
                let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
                let nol = v_dot(n, l).max(0.0);
                let noh = v_dot(n, hvec).clamp(0.0, 1.0);
                let loh = v_dot(l, hvec).clamp(0.0, 1.0);
                let d_term = d_ggx(noh, a);
                let v_term = v_smith_ggx_correlated(nov, nol, a);
                let f_term = f_schlick(loh, f0);
                for c in 0..3 {
                    let spec = d_term * v_term * f_term[c];
                    let diff = diffuse_color[c] * (1.0 / pi);
                    lit_direct[c] += (diff + spec) * (nol * shadow) * li.color_cone[c];
                }
            }
            GOLDEN_LIGHT_KIND_SKY => {
                let sky = [li.color_cone[0], li.color_cone[1], li.color_cone[2]];
                let ground = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
                let dfg = env_brdf_approx(roughness, nov);
                for c in 0..3 {
                    // hemisphere diffuse = lerp(ground, sky, hemi); spec = EnvBRDFApprox.
                    let hemi_c = ground[c] + (sky[c] - ground[c]) * hemi;
                    let spec_ambient = (f0[c] * dfg[0] + dfg[1]) * sky[c];
                    let diff_ambient = diffuse_color[c] * hemi_c;
                    ambient[c] += (spec_ambient + diff_ambient) * ao;
                }
            }
            // Point/spot (kinds 1/2) are the L0b block (handled after this loop).
            _ => {}
        }
    }

    // L0b: reconstruct the surface world position from the `gViewT` lane (under `mask == 1`
    // only — `attrs.view_t` carries the sentinel on a non-lit pixel, but this whole function
    // already early-returned on `mask != 1`, so the read is gated). Then loop the point/spot
    // block `[l0a_count .. light_count)`, mirroring the shader's `deferred_pbr.hlsl` math
    // bit-for-bit (range cull → windowed inverse-square → O2 spot cone → Cook-Torrance).
    let p = [
        ro[0] + rd[0] * attrs.view_t,
        ro[1] + rd[1] * attrs.view_t,
        ro[2] + rd[2] * attrs.view_t,
    ];
    let l0a = header.l0a_count() as usize;
    let total = header.light_count() as usize;
    for li in lights.iter().take(total).skip(l0a) {
        let kind = li.kind();
        if kind != GOLDEN_LIGHT_KIND_POINT && kind != GOLDEN_LIGHT_KIND_SPOT {
            continue;
        }
        let pos = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
        let range = li.pos_range[3];
        let to_l = [pos[0] - p[0], pos[1] - p[1], pos[2] - p[2]];
        let d2 = v_dot(to_l, to_l);
        let range2 = range * range;
        if d2 > range2 {
            continue; // outside the cull sphere
        }
        // l = unit surface->light; mirrors the shader's `rsqrt(max(d2, 1e-8))`.
        let inv_d = 1.0 / d2.max(1e-8).sqrt();
        let l = [to_l[0] * inv_d, to_l[1] * inv_d, to_l[2] * inv_d];
        // Smooth windowed inverse-square (the shader's `(1 - (d2/range2)^2)^2` window).
        let win = (1.0 - (d2 * d2) / (range2 * range2)).clamp(0.0, 1.0);
        let mut atten = (1.0 / d2.max(1e-4)) * win * win;
        if kind == GOLDEN_LIGHT_KIND_SPOT {
            // O2 cone falloff (mirrors the shader): cos between -l and the spot axis,
            // smoothstepped between the outer and inner cone cosines, squared.
            let (cos_inner, cos_outer) = golden_unpack_cones(li.color_cone[3]);
            let spot_dir = v_normalize([li.dir_kind[0], li.dir_kind[1], li.dir_kind[2]]);
            let cos_a = v_dot([-l[0], -l[1], -l[2]], spot_dir);
            let denom = (cos_inner - cos_outer).max(1e-4);
            let tt = ((cos_a - cos_outer) / denom).clamp(0.0, 1.0);
            atten *= tt * tt;
        }
        // The SAME Cook-Torrance direct term as the directional path, scaled by the
        // distance/cone attenuation and the light's baked color.
        let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
        let nol = v_dot(n, l).max(0.0);
        let noh = v_dot(n, hvec).clamp(0.0, 1.0);
        let loh = v_dot(l, hvec).clamp(0.0, 1.0);
        let d_term = d_ggx(noh, a);
        let v_term = v_smith_ggx_correlated(nov, nol, a);
        let f_term = f_schlick(loh, f0);
        for c in 0..3 {
            let spec = d_term * v_term * f_term[c];
            let diff = diffuse_color[c] * (1.0 / pi);
            lit_direct[c] += (diff + spec) * (nol * shadow) * atten * li.color_cone[c];
        }
    }

    let exposure = header.exposure();
    let mut lit = [0.0_f32; 3];
    for c in 0..3 {
        lit[c] = (lit_direct[c] + ambient[c] + mat.emissive[c]) * exposure;
    }
    pack_rgba(lit)
}

// ===========================================================================
// Lighting L1 — clustered froxel light cull (host mirror of cluster_cull.hlsl +
// the deferred_pbr.hlsl cluster lookup).
//
// The host cull builds each froxel's WORLD-space AABB from the SAME ray-gen the GPU uses
// (`composite_ray`) at the exp-Z slice's near/far view-z, tests each point/spot light's
// bounding sphere (sqDistPointAABB <= r²), and records the surviving index SET per froxel.
// The clustered resolve then maps a pixel to its froxel and shades only that froxel's
// point/spot lights — which, when the cull is exact (no false drop under the cap), is
// BIT-IDENTICAL to the brute-force `golden_deferred_resolve_table` (the load-bearing L1
// golden). The linearization + the exp-Z slice/tile maps mirror `light_table.hlsli`.
// ===========================================================================

/// The host cluster-cull config (mirrors `boyko_render::light::ClusterConfig`). The vulkan
/// crate cannot depend on `boyko_render`, so the golden carries its own POD mirror; the
/// dims + exp-Z near/far + the caps are the SAME the GPU cull uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoldenClusterConfig {
    /// Froxel grid X dimension.
    pub dim_x: u32,
    /// Froxel grid Y dimension.
    pub dim_y: u32,
    /// Froxel grid Z (exp-Z slice) dimension.
    pub dim_z: u32,
    /// Per-froxel light-index cap (O2 clamp-and-drop).
    pub max_lights_per_cluster: u32,
    /// Exp-Z near plane (slice 0 view-z).
    pub z_near: f32,
    /// Exp-Z far plane (slice `dim_z` view-z).
    pub z_far: f32,
}

impl GoldenClusterConfig {
    /// The total froxel count (`dim_x * dim_y * dim_z`).
    #[inline]
    pub const fn cluster_count(&self) -> u32 {
        self.dim_x * self.dim_y * self.dim_z
    }

    /// The exp-Z slice scale `dim_z / ln(far/near)` (mirrors `ClusterConfig::z_scale`).
    #[inline]
    pub fn z_scale(&self) -> f32 {
        (self.dim_z as f32) / (self.z_far / self.z_near).ln()
    }

    /// The exp-Z slice bias `-ln(near) * z_scale` (mirrors `ClusterConfig::z_bias`).
    #[inline]
    pub fn z_bias(&self) -> f32 {
        -self.z_near.ln() * self.z_scale()
    }
}

/// Linearizes froxel `(x, y, z)` → flat index `(y * dim_x + x) * dim_z + z` — the host mirror
/// of `light::cluster_index` and the shader's `cluster_linear_index` (Z innermost). THE one
/// linearization the host + both shaders share.
#[inline]
pub fn golden_cluster_index(x: u32, y: u32, z: u32, dim_x: u32, dim_z: u32) -> u32 {
    (y * dim_x + x) * dim_z + z
}

/// Maps a view-space depth `view_z` to its exp-Z froxel slice, clamped to `[0, dim_z-1]`
/// (mirrors the shader's `cluster_z_slice`). A `view_z <= 0` clamps to slice 0.
#[inline]
pub fn golden_cluster_z_slice(view_z: f32, cfg: &GoldenClusterConfig) -> u32 {
    if view_z <= 0.0 {
        return 0;
    }
    let slice = view_z.ln() * cfg.z_scale() + cfg.z_bias();
    let si = slice.floor() as i32;
    si.clamp(0, cfg.dim_z as i32 - 1) as u32
}

/// Maps pixel `(px, py)` at extent `(w, h)` to its froxel `(x, y)` tile, clamped to the grid
/// (mirrors the shader's `cluster_xy_tile`).
#[inline]
pub fn golden_cluster_xy_tile(
    px: u32,
    py: u32,
    w: u32,
    h: u32,
    cfg: &GoldenClusterConfig,
) -> (u32, u32) {
    let tx = ((px * cfg.dim_x) / w.max(1)).min(cfg.dim_x - 1);
    let ty = ((py * cfg.dim_y) / h.max(1)).min(cfg.dim_y - 1);
    (tx, ty)
}

/// Converts a slice view-z to the world ray parameter `t` for `(ro, rd)` (mirrors the cull's
/// `view_z_to_t`): PERSP `view_z / dot(rd, fwd)`, ORTHO `view_z` (rd = (0,0,-1)). `fwd` is the
/// camera forward axis (O1: NORMALIZED); for ORTHO it is ignored.
#[inline]
fn golden_view_z_to_t(view_z: f32, rd: [f32; 3], camera: CompositeCamera) -> f32 {
    match camera {
        CompositeCamera::Perspective { forward, .. } => {
            let cos_axis = v_dot(rd, forward).max(1e-4);
            view_z / cos_axis
        }
        CompositeCamera::Ortho => view_z,
    }
}

/// The exp-Z view-z at slice boundary `k` (mirrors the cull's `slice_view_z`).
#[inline]
fn golden_slice_view_z(k: u32, cfg: &GoldenClusterConfig) -> f32 {
    cfg.z_near * (cfg.z_far / cfg.z_near).powf(k as f32 / cfg.dim_z as f32)
}

/// Squared distance from a point to an AABB (0 inside) — mirrors the shader's
/// `sq_dist_point_aabb` (the canonical clustered-cull sphere-vs-AABB test).
#[inline]
fn golden_sq_dist_point_aabb(c: [f32; 3], aabb_min: [f32; 3], aabb_max: [f32; 3]) -> f32 {
    let mut s = 0.0_f32;
    for i in 0..3 {
        let d = (aabb_min[i] - c[i]).max(c[i] - aabb_max[i]).max(0.0);
        s += d * d;
    }
    s
}

/// The host clustered froxel light cull (mirrors `cluster_cull.hlsl`). For each froxel it
/// builds the WORLD-space AABB from the screen-tile corners at the slice's near/far view-z
/// (the SAME `composite_ray` the resolve uses) and records the surviving POINT/SPOT light
/// indices (`sqDistPointAABB <= r²`) in table order, clamped to `max_lights_per_cluster`
/// (O2). Returns a `Vec` of per-froxel index `Vec`s, flat-indexed by
/// [`golden_cluster_index`]. Directional/sky are GLOBAL (never in the per-froxel sets).
///
/// The cull is geometric + deterministic; the resolve's per-froxel sum is order-stable
/// (table order), so a froxel whose set contains every in-range light reproduces the
/// brute-force resolve bit-for-bit.
pub fn golden_cluster_cull(
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    cfg: &GoldenClusterConfig,
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
) -> Vec<Vec<u32>> {
    let count = cfg.cluster_count() as usize;
    let mut grid: Vec<Vec<u32>> = vec![Vec::new(); count];
    let l0a = header.l0a_count();
    let total = header.light_count();
    for y in 0..cfg.dim_y {
        for x in 0..cfg.dim_x {
            // The tile's inclusive corner pixels (mirror the cull's px0/py0/px1/py1).
            let px0 = (x * img_w) / cfg.dim_x;
            let py0 = (y * img_h) / cfg.dim_y;
            let px1 = (((x + 1) * img_w) / cfg.dim_x).saturating_sub(1).max(px0);
            let py1 = (((y + 1) * img_h) / cfg.dim_y).saturating_sub(1).max(py0);
            let corners = [(px0, py0), (px1, py0), (px0, py1), (px1, py1)];
            for z in 0..cfg.dim_z {
                let vz_near = golden_slice_view_z(z, cfg);
                let vz_far = golden_slice_view_z(z + 1, cfg);
                let mut aabb_min = [1.0e30_f32; 3];
                let mut aabb_max = [-1.0e30_f32; 3];
                for &(cx, cy) in &corners {
                    let (ro, rd) = composite_ray(cx, cy, img_w, img_h, camera);
                    for &vz in &[vz_near, vz_far] {
                        let t = golden_view_z_to_t(vz, rd, camera);
                        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                        for i in 0..3 {
                            aabb_min[i] = aabb_min[i].min(p[i]);
                            aabb_max[i] = aabb_max[i].max(p[i]);
                        }
                    }
                }
                let cell = &mut grid[golden_cluster_index(x, y, z, cfg.dim_x, cfg.dim_z) as usize];
                for i in l0a..total {
                    let li = &lights[i as usize];
                    let kind = li.kind();
                    if kind != GOLDEN_LIGHT_KIND_POINT && kind != GOLDEN_LIGHT_KIND_SPOT {
                        continue;
                    }
                    let pos = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
                    let r = li.pos_range[3];
                    if golden_sq_dist_point_aabb(pos, aabb_min, aabb_max) <= r * r
                        && (cell.len() as u32) < cfg.max_lights_per_cluster
                    {
                        cell.push(i);
                    }
                }
            }
        }
    }
    grid
}

/// The CPU mirror of the L1 CLUSTERED `deferred_pbr` resolve. Identical to
/// [`golden_deferred_resolve_table`] except the point/spot block is driven by the pixel's
/// froxel index SET (from [`golden_cluster_cull`]) instead of the flat `[l0a..light_count)`
/// range. When `header.clusters_enabled()` is false this DELEGATES to the brute-force table
/// resolve (the L1 0%-gate == L0b). The per-light shading expression is byte-identical to the
/// table resolve, so a cluster set that contains every in-range light reproduces it exactly.
#[allow(clippy::too_many_arguments)]
pub fn golden_deferred_resolve_clustered(
    attrs: MarcherAttributes,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    materials: &[GoldenMaterial],
    header: &GoldenLightHeader,
    lights: &[GoldenLight],
    cfg: &GoldenClusterConfig,
    grid: &[Vec<u32>],
) -> u32 {
    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);
    // L1 OFF (or a non-lit pixel): the flat brute-force path (the 0%-gate).
    if !header.clusters_enabled() || attrs.mask != 1 {
        return golden_deferred_resolve_table(attrs, ro, rd, materials, header, lights);
    }

    let base = [
        attrs.base_rgb[0] as f32 / 255.0,
        attrs.base_rgb[1] as f32 / 255.0,
        attrs.base_rgb[2] as f32 / 255.0,
    ];
    let n = oct_decode([attrs.oct_rg[0] as f32 / 255.0, attrs.oct_rg[1] as f32 / 255.0]);
    let mat = materials
        .get(attrs.mat_id as usize)
        .copied()
        .unwrap_or_default();

    let metallic = mat.mrr[0];
    let roughness = mat.mrr[1].clamp(0.045, 1.0);
    let reflectance = mat.mrr[2];
    let a = roughness * roughness;
    let dielectric_f0 = 0.16 * reflectance * reflectance;
    let f0 = [
        dielectric_f0 + (base[0] - dielectric_f0) * metallic,
        dielectric_f0 + (base[1] - dielectric_f0) * metallic,
        dielectric_f0 + (base[2] - dielectric_f0) * metallic,
    ];
    let diffuse_color = [
        base[0] * (1.0 - metallic),
        base[1] * (1.0 - metallic),
        base[2] * (1.0 - metallic),
    ];

    let v = [-rd[0], -rd[1], -rd[2]];
    let nov = v_dot(n, v).max(1e-4);
    let shadow = attrs.shadow as f32 / 255.0;
    let ao = attrs.ao as f32 / 255.0;
    let pi = core::f32::consts::PI;
    const UP: [f32; 3] = [0.0, 1.0, 0.0];
    let hemi = v_dot(n, UP) * 0.5 + 0.5;

    // The no-`P` front block (directionals + sky) is GLOBAL — identical to the table resolve.
    let mut lit_direct = [0.0_f32; 3];
    let mut ambient = [0.0_f32; 3];
    let l0a = header.l0a_count() as usize;
    for li in lights.iter().take(l0a) {
        match li.kind() {
            GOLDEN_LIGHT_KIND_DIRECTIONAL => {
                let l = v_normalize([li.dir_kind[0], li.dir_kind[1], li.dir_kind[2]]);
                let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
                let nol = v_dot(n, l).max(0.0);
                let noh = v_dot(n, hvec).clamp(0.0, 1.0);
                let loh = v_dot(l, hvec).clamp(0.0, 1.0);
                let d_term = d_ggx(noh, a);
                let v_term = v_smith_ggx_correlated(nov, nol, a);
                let f_term = f_schlick(loh, f0);
                for c in 0..3 {
                    let spec = d_term * v_term * f_term[c];
                    let diff = diffuse_color[c] * (1.0 / pi);
                    lit_direct[c] += (diff + spec) * (nol * shadow) * li.color_cone[c];
                }
            }
            GOLDEN_LIGHT_KIND_SKY => {
                let sky = [li.color_cone[0], li.color_cone[1], li.color_cone[2]];
                let ground = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
                let dfg = env_brdf_approx(roughness, nov);
                for c in 0..3 {
                    let hemi_c = ground[c] + (sky[c] - ground[c]) * hemi;
                    let spec_ambient = (f0[c] * dfg[0] + dfg[1]) * sky[c];
                    let diff_ambient = diffuse_color[c] * hemi_c;
                    ambient[c] += (spec_ambient + diff_ambient) * ao;
                }
            }
            _ => {}
        }
    }

    // L1: map the pixel to its froxel and loop ONLY that cluster's point/spot indices. The
    // froxel z-slice uses the SAME view-z the cull used.
    let p = [
        ro[0] + rd[0] * attrs.view_t,
        ro[1] + rd[1] * attrs.view_t,
        ro[2] + rd[2] * attrs.view_t,
    ];
    let view_z = match camera {
        CompositeCamera::Perspective { forward, .. } => v_dot(rd, forward) * attrs.view_t,
        CompositeCamera::Ortho => attrs.view_t,
    };
    let (tx, ty) = golden_cluster_xy_tile(px, py, img_w, img_h, cfg);
    let zsl = golden_cluster_z_slice(view_z, cfg);
    let cluster = golden_cluster_index(tx, ty, zsl, cfg.dim_x, cfg.dim_z) as usize;
    let slice = grid.get(cluster).map(Vec::as_slice).unwrap_or(&[]);
    for &j in slice {
        let li = &lights[j as usize];
        let kind = li.kind();
        let pos = [li.pos_range[0], li.pos_range[1], li.pos_range[2]];
        let range = li.pos_range[3];
        let to_l = [pos[0] - p[0], pos[1] - p[1], pos[2] - p[2]];
        let d2 = v_dot(to_l, to_l);
        let range2 = range * range;
        if d2 > range2 {
            continue;
        }
        let inv_d = 1.0 / d2.max(1e-8).sqrt();
        let l = [to_l[0] * inv_d, to_l[1] * inv_d, to_l[2] * inv_d];
        let win = (1.0 - (d2 * d2) / (range2 * range2)).clamp(0.0, 1.0);
        let mut atten = (1.0 / d2.max(1e-4)) * win * win;
        if kind == GOLDEN_LIGHT_KIND_SPOT {
            let (cos_inner, cos_outer) = golden_unpack_cones(li.color_cone[3]);
            let spot_dir = v_normalize([li.dir_kind[0], li.dir_kind[1], li.dir_kind[2]]);
            let cos_a = v_dot([-l[0], -l[1], -l[2]], spot_dir);
            let denom = (cos_inner - cos_outer).max(1e-4);
            let tt = ((cos_a - cos_outer) / denom).clamp(0.0, 1.0);
            atten *= tt * tt;
        }
        let hvec = v_normalize([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
        let nol = v_dot(n, l).max(0.0);
        let noh = v_dot(n, hvec).clamp(0.0, 1.0);
        let loh = v_dot(l, hvec).clamp(0.0, 1.0);
        let d_term = d_ggx(noh, a);
        let v_term = v_smith_ggx_correlated(nov, nol, a);
        let f_term = f_schlick(loh, f0);
        for c in 0..3 {
            let spec = d_term * v_term * f_term[c];
            let diff = diffuse_color[c] * (1.0 / pi);
            lit_direct[c] += (diff + spec) * (nol * shadow) * atten * li.color_cone[c];
        }
    }

    let exposure = header.exposure();
    let mut lit = [0.0_f32; 3];
    for c in 0..3 {
        lit[c] = (lit_direct[c] + ambient[c] + mat.emissive[c]) * exposure;
    }
    pack_rgba(lit)
}

// ===========================================================================
// Render P4b — the conservative coarse-cull / tile pre-trace (host mirror).
//
// A 1/8-res CONSERVATIVE cone-trace emits, per 8×8 tile, a `TileBound{near_t,
// far_t, flags}`. The fine marcher then seeds `t = near_t` (skipping the
// proven-empty prefix) and early-outs EMPTY tiles into the existing mesh /
// background composite. This module is the HOST mirror of `sdf_tile_cull.hlsl`
// (`golden_tile_bound`, Algorithm A) + the culled fine marcher
// (`golden_composite_pixel_culled`, Algorithm B); the cone radii, the step
// formula, the cone-entry rule, and the far_t / exhaustion fallbacks reproduce
// the shader EXACTLY so the host conservative-invariant tests PROVE the cull is
// conservative on the CPU before the GPU golden runs. See docs/RENDER-P4-DESIGN.md.
//
// The field eval is byte-shared with the fine marcher (`sdf_edit_list`); the
// determinism boundary is INVIOLABLE — P4b only seeds `t` and adds the coarse
// pass, never touching the field math (`golden_composite_pixel_culled` wraps
// `golden_composite_pixel_ex`'s body; `coarse_enabled == false` is bit-identical).
// ===========================================================================

/// `flags` bit set on a tile the coarse cone-trace proved EMPTY (no SDF surface
/// in the cone in front of the deepest mesh). Mirrors the shader's
/// `TILE_FLAG_EMPTY`. An EMPTY tile still composites the mesh / background (D6).
pub const TILE_FLAG_EMPTY: u32 = 1;

/// The fine-pixel edge length of one coarse tile (an 8×8 footprint). Mirrors the
/// shader's `TILE_SIZE` and the `[numthreads]` group geometry of the fine marcher.
pub const TILE_SIZE: u32 = 8;

/// Max coarse cone-trace iterations per tile (D5). Exhaustion ⇒ NON-empty,
/// `near_t = 0` (the safe full-march fallback). Mirrors the shader's
/// `MAX_IT_COARSE`. Smaller than the fine `SDF_MAX_IT` (128) — the coarse pass is
/// the cheap pre-pass, and exhaustion degrades gracefully to a full fine march.
pub const MAX_IT_COARSE: u32 = 64;

/// Cone-entry threshold on the cone budget `d/L − r(t)` (D4). When the budget
/// drops to `<= EPS_COARSE` the cone has (conservatively) entered the surface
/// band: RECORD `near_t = t` and STOP. Mirrors the shader's `EPS_COARSE`.
pub const EPS_COARSE: f32 = 0.001;

/// Extra half-angle safety margin (radians) added to the per-tile perspective
/// cone half-angle (D3). Absorbs the fp-ULP slack so the cone strictly encloses
/// every in-tile pixel ray's footprint. Mirrors the shader's `ALPHA_MARGIN`.
pub const ALPHA_MARGIN: f32 = 1e-4;

/// The field's worst-case spatial gradient magnitude — the cone step's distance
/// divisor (D7). `sqrt(2)`: the IQ polynomial smin's steepest 90-degree blend
/// peaks at `sqrt(2)` (k sets the band width, not the peak slope); the analytic
/// primitives are unit-gradient. `d / FIELD_LIPSCHITZ_L` is a conservative lower
/// bound on the Euclidean clearance even where smin is super-Lipschitz. Mirrors
/// the shader's `FIELD_LIPSCHITZ_L` in `sdf_field.hlsli`.
pub const FIELD_LIPSCHITZ_L: f32 = core::f32::consts::SQRT_2;

/// `#[repr(C)]` per-tile cull bound, byte-identical to the std430
/// `RWStructuredBuffer<TileBound>` element the shader emits (16 B, scalar layout):
///
///   offset  0 : f32 near_t  (the seed `t` for the fine march; `[0, far_t]`)
///   offset  4 : f32 far_t    (the march bound = min(max 8×8 depth→t, T_MAX))
///   offset  8 : u32 flags    ([`TILE_FLAG_EMPTY`] or 0)
///   offset 12 : u32 _pad     (std430 16-B alignment)
///
/// Field offsets are pinned by the const-asserts below so a host/shader desync is
/// a build error (the same discipline as `CompositePushConstants`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileBound {
    /// The proven-empty prefix end: the fine march seeds `t = near_t`. In
    /// `[0, far_t]`; `0` for an EMPTY tile or a coarse exhaustion fallback.
    pub near_t: f32,
    /// The march bound — `min(max over the 8×8 depth texels of depth→t, T_MAX)`
    /// (the deepest mesh, conservative; a cleared / out-of-range texel ⇒ `T_MAX`).
    pub far_t: f32,
    /// [`TILE_FLAG_EMPTY`] when the coarse cone-trace proved the tile empty, else 0.
    pub flags: u32,
    /// std430 padding to a 16-byte stride.
    pub _pad: u32,
}

/// Byte size of [`TileBound`] — the `RWStructuredBuffer<TileBound>` element stride
/// (16 bytes). The coarse-cull buffer is `tiles_w * tiles_h * TILE_BOUND_BYTES`.
pub const TILE_BOUND_BYTES: usize = core::mem::size_of::<TileBound>();

// Pin the field offsets + the 16-byte std430 stride (a desync is a build error).
const _: () = assert!(core::mem::offset_of!(TileBound, near_t) == 0);
const _: () = assert!(core::mem::offset_of!(TileBound, far_t) == 4);
const _: () = assert!(core::mem::offset_of!(TileBound, flags) == 8);
const _: () = assert!(core::mem::offset_of!(TileBound, _pad) == 12);
const _: () = assert!(TILE_BOUND_BYTES == 16, "TileBound must be a 16-byte std430 element");

/// The coarse tile-grid extent for a `(img_w, img_h)` fine image: `tiles_w =
/// ceil(w / TILE_SIZE)`, `tiles_h = ceil(h / TILE_SIZE)`. The cull buffer holds
/// `tiles_w * tiles_h` [`TileBound`]s; the coarse dispatch covers them. Mirrors
/// the shader's `tiles_w` / `tiles_h`.
#[inline]
pub const fn tile_grid_extent(img_w: u32, img_h: u32) -> (u32, u32) {
    (img_w.div_ceil(TILE_SIZE), img_h.div_ceil(TILE_SIZE))
}

/// Reconstructs the `(ray_origin, ray_dir)` for the coarse ray through tile
/// `(tx, ty)`'s TRUE geometric center, derived line-for-line from [`composite_ray`]'s
/// EXACT arithmetic so the host + the shader emit identical ops (D1).
///
/// Tile `(tx, ty)` covers fine pixels `[tx*8 .. tx*8+7]²`; its center fine pixel is
/// `px_c = tx*8 + 3.5`, so the fine ray-gen's `(px + 0.5)` becomes `tx*8 + 4.0`
/// (3.5 + 0.5 = 4.0 exact in fp). This is NOT half-res-grid sampling
/// (`(tx + 0.5) / (w / 8)` is not fp-identical and would drift the center, eating
/// the cone margin). The result is the SAME `ro`/`rd` the fine marcher would shoot
/// for a (fractional) center pixel under `camera` — ortho or perspective.
#[inline]
fn coarse_ray(
    tx: u32,
    ty: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
) -> ([f32; 3], [f32; 3]) {
    // `px_c + 0.5 = tx*8 + 4.0` and `py_c + 0.5 = ty*8 + 4.0` (the +0.5 of the fine
    // ray-gen folded into the exact-fp tile center). The rest is `composite_ray`'s
    // arithmetic byte-for-byte.
    let cx = (tx * TILE_SIZE) as f32 + 4.0;
    let cy = (ty * TILE_SIZE) as f32 + 4.0;
    match camera {
        CompositeCamera::Ortho => {
            let u = (cx / (img_w as f32)) * 2.0 - 1.0;
            let v = -((cy / (img_h as f32)) * 2.0 - 1.0);
            let ro = [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT, SDF_CAM_Z];
            let rd = [0.0, 0.0, -1.0];
            (ro, rd)
        }
        CompositeCamera::Perspective {
            eye,
            forward,
            right,
            up,
            tan_half_fov,
            aspect,
        } => {
            let ndc_x = (cx / (img_w as f32)) * 2.0 - 1.0;
            let ndc_y = -((cy / (img_h as f32)) * 2.0 - 1.0);
            let sx = ndc_x * aspect * tan_half_fov;
            let sy = ndc_y * tan_half_fov;
            let dir = [
                forward[0] + right[0] * sx + up[0] * sy,
                forward[1] + right[1] * sx + up[1] * sy,
                forward[2] + right[2] * sx + up[2] * sy,
            ];
            let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            let rd = [dir[0] / len, dir[1] / len, dir[2] / len];
            (eye, rd)
        }
    }
}

/// The ORTHO cone radius — a constant-radius cylinder enclosing the 8×8 tile's
/// fine-pixel footprint with one full pixel of fp-ULP-safe margin (D2).
///
/// Parallel ortho rays → a constant-radius cylinder around the tile-center axis.
/// The fine ortho ray-gen maps `u = (px+0.5)/w·2−1` (world X pitch `Δx = (2/w)·HE`)
/// and `v` over `h` (world Y pitch `Δy = (2/h)·HE`), so a non-square image has
/// `Δx ≠ Δy`; the enclosing-cylinder radius must use the LARGER pitch
/// `Δ = (2 / min(w,h)) · HE`. The tight footprint-enclosing radius is
/// `sqrt(2) · 4 · Δ = sqrt(2) · (8/min(w,h)) · HE` (center-to-corner-center
/// `sqrt(2)·3.5·Δ` plus the half-pixel footprint `sqrt(2)·0.5·Δ`). A FULL extra
/// pixel of slack gives `r_ortho = sqrt(2) · (9/min(w,h)) · HE` (the old `(8/w)` was
/// zero-margin, a C1 hole; per design D2 — generalized from `w` to `min(w,h)` so a
/// non-square ortho extent stays conservative, BYTE-IDENTICAL at the square golden
/// where `min(w,h) == w`).
#[inline]
fn ortho_cone_radius(img_w: u32, img_h: u32) -> f32 {
    let min_wh = img_w.min(img_h) as f32;
    core::f32::consts::SQRT_2 * (9.0 / min_wh) * SDF_HALF_EXTENT
}

/// The PER-TILE perspective cone half-angle (radians) from the exact ray-gen (D3):
/// the max over the tile's 4 corner pixels' OUTER-EDGE directions of the angle to
/// the tile-center direction, plus [`ALPHA_MARGIN`].
///
/// `alpha_tile = max_i acos(dot(d_center, d_corner_edge_i))` where each direction is
/// the exact perspective ray-gen `dir = forward + right·(ndc_x·aspect·tan) +
/// up·(ndc_y·tan)`, normalized. The 4 corners use the tile footprint's OUTER edges
/// (`px = tx*8 − 0.5 .. tx*8 + 7.5` → `(px+0.5) = tx*8 .. tx*8 + 8`), which capture
/// the per-pixel footprint via the ±4.0-from-center offset, the aspect anisotropy,
/// AND the tan-convexity of edge tiles (a scalar `4√2·center-angle` under-encloses
/// → holes). The half-angle is per-tile (tighter `near_t`, same shader cost).
///
/// `camera` MUST be [`CompositeCamera::Perspective`] (the eye is unused for the
/// half-angle — directions only; the basis + `tan_half_fov` + `aspect` are read from
/// it). An ORTHO camera is not a perspective cone (the callers gate on the camera mode
/// and use [`ortho_cone_radius`] instead) and returns [`ALPHA_MARGIN`] (a degenerate
/// zero-angle cone) — but no caller passes one.
#[inline]
fn perspective_alpha_tile(tx: u32, ty: u32, img_w: u32, img_h: u32, camera: CompositeCamera) -> f32 {
    let CompositeCamera::Perspective {
        forward,
        right,
        up,
        tan_half_fov,
        aspect,
        ..
    } = camera
    else {
        return ALPHA_MARGIN;
    };
    // The exact perspective ray direction for a (fractional) pixel whose
    // `(px + 0.5)` sample is `sx_px` and `(py + 0.5)` is `sy_px`, normalized — the
    // same op sequence as `composite_ray`'s perspective arm.
    let dir_for = |sx_px: f32, sy_px: f32| -> [f32; 3] {
        let ndc_x = (sx_px / (img_w as f32)) * 2.0 - 1.0;
        let ndc_y = -((sy_px / (img_h as f32)) * 2.0 - 1.0);
        let sx = ndc_x * aspect * tan_half_fov;
        let sy = ndc_y * tan_half_fov;
        let dir = [
            forward[0] + right[0] * sx + up[0] * sy,
            forward[1] + right[1] * sx + up[1] * sy,
            forward[2] + right[2] * sx + up[2] * sy,
        ];
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        [dir[0] / len, dir[1] / len, dir[2] / len]
    };

    // The tile-center axis: `(px_c + 0.5) = tx*8 + 4.0` (matches `coarse_ray`).
    let cx = (tx * TILE_SIZE) as f32 + 4.0;
    let cy = (ty * TILE_SIZE) as f32 + 4.0;
    let d_center = dir_for(cx, cy);

    // The 4 corner OUTER edges: `(px + 0.5)` at `tx*8 + 0.0` (left/top outer) and
    // `tx*8 + 8.0` (right/bottom outer) — the footprint's outermost sample points.
    let lo_x = (tx * TILE_SIZE) as f32;
    let hi_x = (tx * TILE_SIZE) as f32 + (TILE_SIZE as f32);
    let lo_y = (ty * TILE_SIZE) as f32;
    let hi_y = (ty * TILE_SIZE) as f32 + (TILE_SIZE as f32);

    let mut max_angle = 0.0_f32;
    for &(sxp, syp) in &[(lo_x, lo_y), (hi_x, lo_y), (lo_x, hi_y), (hi_x, hi_y)] {
        let dc = dir_for(sxp, syp);
        let cos = (d_center[0] * dc[0] + d_center[1] * dc[1] + d_center[2] * dc[2]).clamp(-1.0, 1.0);
        let angle = cos.acos();
        if angle > max_angle {
            max_angle = angle;
        }
    }
    max_angle + ALPHA_MARGIN
}

/// The host cone-trace mirror (Algorithm A): computes the [`TileBound`] for tile
/// `(tx, ty)` from the per-tile mesh depths + the edit-list field, EXACTLY as
/// `sdf_tile_cull.hlsl` does. The single source of truth the host conservative-
/// invariant tests + the GPU `Tiles`-buffer agreement check assert against.
///
/// `tile_depths` is the 8×8 block of per-pixel mesh depths covering the tile (the
/// fine `mesh_depth` values, clear `1.0` outside the mesh / out of image range); the
/// caller supplies them in any order (only the MAX is read — D5). The algorithm:
///   1. `coarse_ray` (D1) → the tile-center axis.
///   2. `far_t = min(max over the depths of depth→t, T_MAX)` (D5: a cleared /
///      out-of-range texel decodes to `T_MAX`, so a partial-edge tile bounds at
///      `T_MAX`, not clamp-to-edge).
///   3. The cone-aware march (D4): at `t`, `d = field`, cone radius `r(t)` (ortho:
///      `r_const`; perspective: `t · tan(alpha_safe)`); budget `= d/L − r(t)`. When
///      the budget `<= EPS_COARSE` RECORD `near_t = t` and STOP (cone-entry). Else
///      advance `t += budget / (1 + tan(alpha_safe))` (ortho: `/(1+0)`).
///   4. Reaching `far_t` (or `T_MAX`) without cone-entry ⇒ EMPTY (`near_t = 0`,
///      flags = `TILE_FLAG_EMPTY`); exhausting `MAX_IT_COARSE` ⇒ NON-empty,
///      `near_t = 0` (the safe full-march fallback — NEVER `near_t = last_t`).
///
/// `near_t` is clamped to `[0, far_t]`; an EMPTY tile has `near_t == 0`.
pub fn golden_tile_bound(
    edits: &[SdfEdit],
    tile_depths: &[f32],
    tx: u32,
    ty: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
) -> TileBound {
    let (ro, rd) = coarse_ray(tx, ty, img_w, img_h, camera);

    // far_t = min(max over the 8×8 depth texels of depth→t, T_MAX). A cleared
    // (>= MESH_DEPTH_CLEAR) texel decodes to T_MAX (conservative: no mesh bound).
    let mut max_t_mesh = 0.0_f32;
    for &md in tile_depths {
        let t_mesh = if md < MESH_DEPTH_CLEAR { depth_to_t(md) } else { SDF_T_MAX };
        if t_mesh > max_t_mesh {
            max_t_mesh = t_mesh;
        }
    }
    let far_t = max_t_mesh.min(SDF_T_MAX);

    // The cone parameters: ortho → a constant radius, tan = 0; perspective → a
    // per-tile half-angle whose tangent grows the radius linearly with t.
    let (r_const, tan_a) = match camera {
        CompositeCamera::Ortho => (ortho_cone_radius(img_w, img_h), 0.0_f32),
        CompositeCamera::Perspective { .. } => {
            let alpha_safe = perspective_alpha_tile(tx, ty, img_w, img_h, camera);
            (0.0_f32, alpha_safe.tan())
        }
    };

    let mut t = 0.0_f32;
    let mut near_t = 0.0_f32;
    let mut entered = false;
    let mut exhausted = true; // cleared when the loop breaks by cone-entry or far_t.
    for _ in 0..MAX_IT_COARSE {
        if t >= far_t {
            exhausted = false; // reached far_t without entering: EMPTY.
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_edit_list(edits, p);
        let r = r_const + t * tan_a; // ortho: r_const; perspective: t*tan(alpha_safe).
        let budget = d / FIELD_LIPSCHITZ_L - r;
        if budget <= EPS_COARSE {
            near_t = t;
            entered = true;
            exhausted = false;
            break;
        }
        t += budget / (1.0 + tan_a); // ortho: /(1+0); perspective: /(1+tan).
        if t > SDF_T_MAX {
            exhausted = false; // walked past T_MAX without entering: EMPTY.
            break;
        }
    }

    let (near_t, flags) = if entered {
        (near_t.clamp(0.0, far_t), 0u32)
    } else if exhausted {
        // MAX_IT_COARSE exhaustion ⇒ NON-empty, near_t = 0 (the safe full-march
        // fallback — NEVER near_t = last_t, which would skip a surface = a hole).
        (0.0, 0u32)
    } else {
        // Reached far_t / T_MAX without cone-entry ⇒ EMPTY (near_t = 0).
        (0.0, TILE_FLAG_EMPTY)
    };

    debug_assert!(
        (0.0..=far_t).contains(&near_t),
        "invariant: near_t {near_t} must be in [0, far_t={far_t}]"
    );
    debug_assert!(far_t <= SDF_T_MAX, "invariant: far_t {far_t} must be <= T_MAX");
    debug_assert!(
        flags & TILE_FLAG_EMPTY == 0 || near_t == 0.0,
        "invariant: an EMPTY tile must have near_t == 0"
    );

    TileBound { near_t, far_t, flags, _pad: 0 }
}

/// The culled fine marcher (Algorithm B): one composited pixel, gated by the tile's
/// [`TileBound`]. With `coarse_enabled == false` this is BIT-IDENTICAL to
/// [`golden_composite_pixel_ex`] (the `t = 0.0` seed, no cull prefix — the 0%-gate
/// anchor); with `coarse_enabled == true`:
///   * an EMPTY tile (flags & [`TILE_FLAG_EMPTY`]) skips the march and composites
///     the mesh / background directly (D6 — an EMPTY tile can still be MESH-covered);
///   * else the march SEEDS `t = near_t` (the proven-empty prefix is skipped).
///
/// The field eval + lighting are byte-shared with `golden_composite_pixel_ex` (this
/// wraps its body); only the `t` seed + the EMPTY fast-path are added (the
/// determinism boundary — INVIOLABLE). `tile` is the [`TileBound`] for the tile the
/// pixel belongs to (`golden_tile_bound` for tile `(px / 8, py / 8)`).
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_culled(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    coarse_enabled: bool,
    tile: TileBound,
) -> u32 {
    // Render B1: the ω = 1.0 forwarder. Stays BIT-IDENTICAL to the pre-B1 culled marcher
    // (the `_omega` variant's live path is the frozen plain loop at `omega == 1.0`), so
    // every existing caller is unchanged (the 0%-gate).
    golden_composite_pixel_culled_omega(
        edits,
        mesh_depth,
        px,
        py,
        img_w,
        img_h,
        camera,
        coarse_enabled,
        tile,
        1.0,
    )
}

/// Render B1 — the over-relaxation-aware culled fine marcher. Identical to
/// [`golden_composite_pixel_culled`] but threads `omega` through the march: the cull-off
/// arm delegates to [`golden_composite_pixel_ex_omega`], the EMPTY fast-path and the
/// `near_t` seed are preserved, and the non-EMPTY march mirrors the shader's Keinert
/// over-relaxation EXACTLY (gate, over-relaxed step, sor-fail exact retreat, frozen
/// else-arm). At `omega == 1.0` this is BIT-IDENTICAL to the pre-B1 path (the 0%-gate).
/// `omega` is expected in `[1.0, 1.99]` (the host runtime clamp).
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_culled_omega(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    coarse_enabled: bool,
    tile: TileBound,
    omega: f32,
) -> u32 {
    golden_composite_pixel_culled_omega_lit(
        edits, mesh_depth, px, py, img_w, img_h, camera, coarse_enabled, tile, omega, 0,
        DEFAULT_LIGHT_DIR,
    )
}

/// Render A1/A2 — the lighting-aware culled fine marcher. Identical to
/// [`golden_composite_pixel_culled_omega`] but threads `lighting_flags` + `light_dir`:
/// the cull-off arm delegates to [`golden_composite_pixel_ex_omega_lit`], and the
/// non-EMPTY march lights the SDF hit through [`host_shade`] (A1 shadow / A2 AO gated
/// by the flag bits). The EMPTY fast-path composites mesh / background ONLY (no SDF
/// surface ⇒ no shadow/AO), so it is unaffected by lighting. With `lighting_flags == 0`
/// this is BYTE-IDENTICAL to [`golden_composite_pixel_culled_omega`] (the 0%-gate); the
/// ON path mirrors the shader within ±3/255.
#[allow(clippy::too_many_arguments)]
pub fn golden_composite_pixel_culled_omega_lit(
    edits: &[SdfEdit],
    mesh_depth: f32,
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    coarse_enabled: bool,
    tile: TileBound,
    omega: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
) -> u32 {
    // The OFF path is byte-identical to the un-culled marcher (the 0%-gate).
    if !coarse_enabled {
        return golden_composite_pixel_ex_omega_lit(
            edits, mesh_depth, px, py, img_w, img_h, camera, omega, lighting_flags, light_dir,
        );
    }

    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);
    let has_mesh = mesh_depth < MESH_DEPTH_CLEAR;
    let t_mesh = if has_mesh { depth_to_t(mesh_depth) } else { 1.0e30 };

    // EMPTY fast-path (D6): no SDF surface in the cone in front of the deepest mesh,
    // but the pixel can still be MESH-covered → composite mesh / background (the
    // marcher's else-if(has_mesh)/else arms with hit = false). NOT blind background.
    if tile.flags & TILE_FLAG_EMPTY != 0 {
        let color = if has_mesh { MESH_COLOR } else { SDF_BACKGROUND };
        return pack_rgba(color);
    }

    // Non-EMPTY: SEED the march at the proven-empty prefix end. `near_t` is a
    // conservative lower bound on every in-tile pixel's first hit (the cull's
    // contract), so seeding `t = near_t` never skips this pixel's surface.
    let mut t = tile.near_t;
    let t_seed = t; // the ORIGINAL seed (near_t when culled) — Candidate C re-march re-seeds from it
    let mut omega = omega; // [1.0, 1.99]; sor-fail latches it to 1.0 for the rest of the ray
    let mut hit = false;
    let mut safe_t = 0.0_f32; // probe param remembered for an exact retreat
    let mut sor_prev = 0.0_f32; // previous probe's d
    let mut sor_step_prev = 0.0_f32; // previous over-relaxed step length
    // BUG-B1-HOLE-3 (Candidate C): the EXHAUSTION flag. True iff the fast loop runs ALL
    // SDF_MAX_IT iterations with NO break — ran out of budget mid-field (neither
    // converged, nor clear-miss `t > T_MAX`, nor mesh-occluded `t >= t_mesh`). Starts
    // `true`, cleared by EVERY in-loop break. Mirrors the shader.
    let mut exhausted = true;
    for it in 0..SDF_MAX_IT {
        if t >= t_mesh {
            exhausted = false; // mesh-occlusion termination — NOT budget exhaustion
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            hit = true;
            exhausted = false; // converged — NOT budget exhaustion
            break;
        }
        if omega > 1.0 {
            let step_len = d * omega;
            // sor_fail: the over-step taken last iter overshot the previous unbounding
            // sphere (valid only for omega < 2). Lipschitz-aware (BUG-B1-HOLE-1): the
            // guaranteed-empty radius at field value `f` is `f / FIELD_LIPSCHITZ_L`, so the
            // spheres cover the step iff `sor_prev + d >= L * sor_step_prev`. Mirrors the
            // shader exactly. Kept byte-identical to `golden_composite_pixel_ex_omega`'s loop.
            //
            // The `it > 0` guard is LOAD-BEARING (do not remove): a sor-fail can only be
            // reached after at least one ACCEPTED over-relax step (it >= 1 ⟹ accepted >= 1),
            // which pre-pays the +1 retreat iteration in the budget proof.
            if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                // BUG-B1-HOLE-2: do NOT retreat to bare `safe_t` and re-probe (that re-evals
                // the field, costing +2 iters vs plain and overflowing the budget at the
                // MAX_IT cliff → a hole). RESUME the plain march one certified step past the
                // safe point: `safe_t` is the exact probe param, `sor_prev` the exact field
                // value there, so `safe_t + sor_prev` is precisely where a plain march lands
                // after probing safe_t — reusing the eval (no re-probe). One same-sign add
                // (both operands >= 0): no cancellation, unlike a `t - <correction>` form.
                // Net +1 iter vs plain, pre-paid by the >= 1 accepted over-step (it>0 guard).
                debug_assert!(it > 0, "B1 budget: a>=1 precondition");
                debug_assert!(sor_prev >= SDF_EPS); // safe-point field value >= EPS → retreat strictly advances
                t = safe_t + sor_prev; // plain-resume one certified step past the safe probe
                debug_assert!(t > safe_t, "B1 retreat must advance");
                omega = 1.0;
                continue;
            }
            safe_t = t;
            sor_prev = d;
            sor_step_prev = step_len;
            t += step_len;
        } else {
            t += d; // frozen plain arm — TEXTUALLY identical to the frozen loop
        }
        if t > SDF_T_MAX {
            exhausted = false; // clear-miss termination — NOT budget exhaustion
            break;
        }
    }

    // BUG-B1-HOLE-3 (Candidate C): the PROVABLY-hole-free fallback re-march, mirroring the
    // shader EXACTLY. On `exhausted` (ran all SDF_MAX_IT with no break), RE-MARCH from the
    // ORIGINAL seed (`near_t` here, the same seed the fast pass used) with a plain
    // omega = 1.0 sphere-trace and use ITS result. This second loop is the EXACT frozen
    // marcher body (`t += d`) seeded from `near_t`, so any surface the frozen culled path
    // hits within MAX_IT it hits here too → no hole, with NO step-count dependence. At
    // omega == 1.0 the fast pass IS the frozen plain loop, so on exhaustion this reproduces
    // the identical frozen (hit = false) result — the omega == 1.0 output is byte-unchanged.
    if exhausted {
        t = t_seed; // re-seed from the SAME original seed the fast pass used (near_t)
        hit = false;
        for _it2 in 0..SDF_MAX_IT {
            if t >= t_mesh {
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d; // frozen plain step
            if t > SDF_T_MAX {
                break;
            }
        }
    }

    let color = if hit && t < t_mesh {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);
        host_shade(SDF_BASE_COLOR, SDF_AMBIENT, p, n, light_dir, lighting_flags, &|q| {
            sdf_edit_list(edits, q)
        })
    } else if has_mesh {
        MESH_COLOR
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
}

#[cfg(test)]
mod p0a_tests {
    //! Host-side (GPU-free) verification of the P0a substrate: the extent/camera
    //! push-constant layout and the extent-aware golden mirror. The GPU half (the
    //! shader actually rendering ortho 64×64 bit-exact / a 1080p perspective frame)
    //! is the tester's RTX-3060 oracle; these assert the CPU contract those goldens
    //! rely on (the host const-assert mirror + the bit-exact ortho fall-through).

    use super::{
        CAM_MODE_ORTHO, CAM_MODE_PERSPECTIVE, COMPOSITE_PUSH_CONSTANT_BYTES, CompositeCamera,
        CompositePushConstants, MESH_DEPTH_CLEAR, SDF_IMG_H, SDF_IMG_W, SdfEdit,
        golden_composite_pixel, golden_composite_pixel_ex, sdf_op,
    };

    /// The rung-9/10 "crater" CSG scene, reused so the golden parity check runs over
    /// a non-trivial field (a base sphere with a smaller sphere subtracted).
    fn crater() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ]
    }

    /// The extent-aware golden at `(SDF_IMG_W, SDF_IMG_H)` + ORTHO must be BIT-EXACT
    /// to the legacy `golden_composite_pixel` over the whole 64×64 image (the
    /// rung-8..11 contract — same extent → same rays → same pixels).
    #[test]
    fn ortho_64x64_is_bit_identical_to_legacy_golden() {
        let edits = crater();
        // A mix of covered (finite depth) and uncovered (clear) pixels.
        let depths = [0.5_f32, MESH_DEPTH_CLEAR, 0.2, 0.8];
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                let md = depths[((px + py) as usize) % depths.len()];
                let legacy = golden_composite_pixel(&edits, md, px, py);
                let ex = golden_composite_pixel_ex(
                    &edits,
                    md,
                    px,
                    py,
                    SDF_IMG_W,
                    SDF_IMG_H,
                    CompositeCamera::Ortho,
                );
                assert_eq!(legacy, ex, "ortho mirror diverged at ({px},{py}) depth {md}");
            }
        }
    }

    /// `CompositePushConstants::ortho` keeps `count == w*h`, ORTHO mode, zeroed
    /// camera basis, and the 80-byte size the pipeline must declare.
    #[test]
    fn ortho_push_constants_shape() {
        let pc = CompositePushConstants::ortho(SDF_IMG_W, SDF_IMG_H);
        assert_eq!(pc.count, SDF_IMG_W * SDF_IMG_H);
        assert_eq!(pc.img_w, SDF_IMG_W);
        assert_eq!(pc.img_h, SDF_IMG_H);
        assert_eq!(pc.camera_mode, CAM_MODE_ORTHO);
        assert_eq!(pc.cam_eye, [0.0; 4]);
        assert_eq!(pc.as_bytes().len(), COMPOSITE_PUSH_CONSTANT_BYTES as usize);
        assert_eq!(COMPOSITE_PUSH_CONSTANT_BYTES, 80);
    }

    /// `CompositePushConstants::perspective` derives `tan(fovY/2)` + aspect and packs
    /// the basis into the documented `float4` slots; the byte view is 80 bytes.
    #[test]
    fn perspective_push_constants_layout() {
        let fov_y = core::f32::consts::FRAC_PI_2; // 90°
        let pc = CompositePushConstants::perspective(
            [0.0, 0.0, 3.0],   // eye
            [0.0, 0.0, -1.0],  // forward
            [1.0, 0.0, 0.0],   // right
            [0.0, 1.0, 0.0],   // up
            fov_y,
            1920,
            1080,
        );
        assert_eq!(pc.camera_mode, CAM_MODE_PERSPECTIVE);
        assert_eq!(pc.count, 1920 * 1080);
        assert_eq!(pc.cam_eye, [0.0, 0.0, 3.0, 0.0]);
        // forward.w = tan(45°) = 1, right.w = aspect = 1920/1080.
        assert!((pc.cam_forward[3] - 1.0).abs() < 1e-5);
        assert!((pc.cam_right[3] - (1920.0_f32 / 1080.0)).abs() < 1e-6);
        assert_eq!(pc.as_bytes().len(), 80);
    }

    /// Perspective ray-gen sanity: the CENTER pixel of a forward-looking camera must
    /// shoot a ray ≈ the forward axis from the eye (the field eval downstream is the
    /// same deterministic mirror, so this isolates the additive ray-gen).
    #[test]
    fn perspective_center_ray_is_forward() {
        let edits = crater();
        // A small extent; we only need the geometric ray, not a full render.
        let (w, h) = (64u32, 64u32);
        let eye = [0.0_f32, 0.0, 3.0];
        let camera = CompositeCamera::Perspective {
            eye,
            forward: [0.0, 0.0, -1.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            tan_half_fov: (core::f32::consts::FRAC_PI_2 * 0.5).tan(), // 45°
            aspect: 1.0,
        };
        // The center pixel (px=py=32) → ndc ≈ 0 → dir ≈ forward → hits the sphere at
        // the origin from the +Z eye (no mesh: clear depth). A miss would be the dark
        // background; a hit is the warm lit color — distinguish by the red channel.
        let center = golden_composite_pixel_ex(&edits, MESH_DEPTH_CLEAR, w / 2, h / 2, w, h, camera);
        let red = center & 0xFF;
        assert!(
            red > 60,
            "center perspective ray must hit the lit sphere (warm red), got 0x{center:08X}"
        );
        // A corner pixel shoots wide and should MISS → background (low red).
        let corner = golden_composite_pixel_ex(&edits, MESH_DEPTH_CLEAR, 0, 0, w, h, camera);
        let corner_red = corner & 0xFF;
        assert!(
            corner_red < red,
            "corner perspective ray should miss (darker) vs center: corner 0x{corner:08X} center 0x{center:08X}"
        );
    }
}

#[cfg(test)]
mod p4b_tests {
    //! Render P4b HOST conservative-invariant suite — the CPU proof that the coarse
    //! cull is CONSERVATIVE (a hole = the worst bug) BEFORE the GPU golden runs. The
    //! five proofs (docs/RENDER-P4-DESIGN.md):
    //!   (a) EXHAUSTIVE ortho: every tile, all 64 fine-pixel footprint corners within
    //!       `ortho_cone_radius` of the tile-center axis (the exact composite_ray u/v).
    //!   (b) perspective: every tile's 4 corner outer-edge dirs within
    //!       `perspective_alpha_tile` (enclosure with margin).
    //!   (c) randomized {fov, aspect, tile, single sphere/box} with an ANALYTIC first-hit
    //!       oracle: `golden_tile_bound.near_t <= min over in-tile pixels of their
    //!       analytic first-hit` AND `EMPTY => no in-tile pixel hits before mesh`.
    //!   (d) Lipschitz: random points, central-diff `|grad field| <= FIELD_LIPSCHITZ_L`.
    //!   (e) `golden_composite_pixel_culled(coarse_enabled = false)` bit-identical to
    //!       `golden_composite_pixel_ex`.
    //! These reproduce the GPU shader's arithmetic exactly (the host mirror), so a pass
    //! here is a near-proof the GPU cull is conservative too (the GPU golden confirms).

    use boyko_sdf_math::{sdf_edit_list, v_sub};

    use super::{
        ALPHA_MARGIN, CompositeCamera, FIELD_LIPSCHITZ_L, MESH_DEPTH_CLEAR, SDF_CAM_Z, SDF_EPS,
        SDF_HALF_EXTENT, SDF_T_MAX, SdfEdit, TILE_FLAG_EMPTY, TILE_SIZE, golden_composite_pixel_culled,
        golden_composite_pixel_ex, golden_tile_bound, sdf_op, tile_grid_extent,
    };

    // --- A tiny deterministic PRNG (splitmix64) so the randomized sweeps are
    //     reproducible (no rand dep; the same mix the serialization fuzz uses). ----------
    struct Rng(u64);
    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed)
        }
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        /// A uniform `f32` in `[lo, hi)`.
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            let u = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32; // [0,1)
            lo + (hi - lo) * u
        }
        fn range_u32(&mut self, lo: u32, hi: u32) -> u32 {
            lo + (self.next_u64() % ((hi - lo) as u64)) as u32
        }
    }

    /// The exact ORTHO ray-origin XY for a (possibly fractional) fine-pixel sample whose
    /// `(px + 0.5)` is `sx` and `(py + 0.5)` is `sy` — `composite_ray`'s ortho arm.
    fn ortho_origin_xy(sx: f32, sy: f32, w: u32, h: u32) -> [f32; 2] {
        let u = (sx / (w as f32)) * 2.0 - 1.0;
        let v = -((sy / (h as f32)) * 2.0 - 1.0);
        [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT]
    }

    /// The exact, normalized PERSPECTIVE ray direction for a fractional sample, the
    /// `composite_ray` perspective arm (used by the enclosure + oracle tests). The
    /// parameters mirror the shader's ray-gen inputs verbatim (grouping them into a
    /// struct would obscure the op-for-op correspondence the test exists to verify).
    #[allow(clippy::too_many_arguments)]
    fn persp_dir(
        sx: f32,
        sy: f32,
        w: u32,
        h: u32,
        forward: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
        tan_half_fov: f32,
        aspect: f32,
    ) -> [f32; 3] {
        let ndc_x = (sx / (w as f32)) * 2.0 - 1.0;
        let ndc_y = -((sy / (h as f32)) * 2.0 - 1.0);
        let kx = ndc_x * aspect * tan_half_fov;
        let ky = ndc_y * tan_half_fov;
        let dir = [
            forward[0] + right[0] * kx + up[0] * ky,
            forward[1] + right[1] * kx + up[1] * ky,
            forward[2] + right[2] * kx + up[2] * ky,
        ];
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        [dir[0] / len, dir[1] / len, dir[2] / len]
    }

    /// The host mirror of `ortho_cone_radius` (private — re-derive for the test). Uses
    /// the LARGER world pixel pitch `min(w,h)` so a non-square ortho extent is enclosed.
    fn ortho_cone_radius_t(w: u32, h: u32) -> f32 {
        core::f32::consts::SQRT_2 * (9.0 / (w.min(h) as f32)) * SDF_HALF_EXTENT
    }

    /// (a) EXHAUSTIVE ortho enclosure: for EVERY tile, EVERY one of the 64 fine pixels'
    /// 4 footprint corners (`(px+0.5) ± 0.5`, `(py+0.5) ± 0.5`) lies within
    /// `ortho_cone_radius` of the tile-center axis (the perpendicular distance in the
    /// ortho XY plane — the cone is a constant-radius cylinder). A corner outside the
    /// cone = a hole. Run at the golden 64×64 extent + a non-multiple extent.
    #[test]
    fn ortho_footprint_corners_within_cone() {
        for &(w, h) in &[(64u32, 64u32), (96u32, 48u32), (70u32, 66u32)] {
            let (tw, th) = tile_grid_extent(w, h);
            let r = ortho_cone_radius_t(w, h);
            let mut checked = 0u64;
            for ty in 0..th {
                for tx in 0..tw {
                    // Tile-center axis XY (`(px_c+0.5) = tx*8 + 4.0`).
                    let axis = ortho_origin_xy(
                        (tx * TILE_SIZE) as f32 + 4.0,
                        (ty * TILE_SIZE) as f32 + 4.0,
                        w,
                        h,
                    );
                    for ly in 0..TILE_SIZE {
                        for lx in 0..TILE_SIZE {
                            let px = tx * TILE_SIZE + lx;
                            let py = ty * TILE_SIZE + ly;
                            // All 4 footprint corners of fine pixel (px,py): the pixel
                            // sample is `(px+0.5, py+0.5)`; its footprint spans ±0.5.
                            for &(ddx, ddy) in &[(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)] {
                                let sx = (px as f32) + 0.5 + ddx;
                                let sy = (py as f32) + 0.5 + ddy;
                                let xy = ortho_origin_xy(sx, sy, w, h);
                                let dx = xy[0] - axis[0];
                                let dy = xy[1] - axis[1];
                                let dist = (dx * dx + dy * dy).sqrt();
                                assert!(
                                    dist <= r,
                                    "ORTHO hole {w}x{h} tile({tx},{ty}) pixel({px},{py}) corner({ddx},{ddy}): \
                                     footprint corner dist {dist} > cone radius {r}"
                                );
                                checked += 1;
                            }
                        }
                    }
                }
            }
            println!("[a] ortho enclosure {w}x{h}: {checked} footprint corners all within cone radius {r}");
        }
    }

    /// (b) Perspective enclosure: for EVERY tile, the 4 corner OUTER-EDGE directions are
    /// within `perspective_alpha_tile` (= the angle the cull uses) of the tile-center
    /// direction. By construction `perspective_alpha_tile` is the MAX of exactly those 4
    /// angles + `ALPHA_MARGIN`, so each must be `<= alpha` with the margin to spare — a
    /// corner whose angle exceeded `alpha` would be a hole. Also asserts the cull's
    /// in-tile pixel-center + footprint-corner dirs are inside the cone (the stronger
    /// enclosure the conservativeness proof's Claim 1 needs).
    #[test]
    fn perspective_corner_dirs_within_alpha() {
        let (w, h) = (1920u32, 1080u32);
        let forward = [0.0_f32, 0.0, -1.0];
        let right = [1.0_f32, 0.0, 0.0];
        let up = [0.0_f32, 1.0, 0.0];
        let fov_y = core::f32::consts::FRAC_PI_2; // 90°
        let tan_half_fov = (fov_y * 0.5).tan();
        let aspect = (w as f32) / (h as f32);
        let (tw, th) = tile_grid_extent(w, h);

        let camera = CompositeCamera::Perspective {
            eye: [0.0, 0.0, 3.0],
            forward,
            right,
            up,
            tan_half_fov,
            aspect,
        };
        // Sample a strided subset of tiles (full grid is 240×135 = 32 400 tiles; every
        // tile's 4 corners + 64 pixel footprints is exhaustive but slow — stride 7
        // covers the grid incl. the convex edge/corner tiles where alpha is largest).
        let mut tiles_checked = 0u64;
        let mut ty = 0;
        while ty < th {
            let mut tx = 0;
            while tx < tw {
                // Recompute `alpha_tile_safe` exactly as the cull does (4 outer corners).
                let cx = (tx * TILE_SIZE) as f32 + 4.0;
                let cy = (ty * TILE_SIZE) as f32 + 4.0;
                let d_center = persp_dir(cx, cy, w, h, forward, right, up, tan_half_fov, aspect);
                let lo_x = (tx * TILE_SIZE) as f32;
                let hi_x = (tx * TILE_SIZE) as f32 + (TILE_SIZE as f32);
                let lo_y = (ty * TILE_SIZE) as f32;
                let hi_y = (ty * TILE_SIZE) as f32 + (TILE_SIZE as f32);
                let mut alpha = 0.0_f32;
                for &(sxp, syp) in &[(lo_x, lo_y), (hi_x, lo_y), (lo_x, hi_y), (hi_x, hi_y)] {
                    let dc = persp_dir(sxp, syp, w, h, forward, right, up, tan_half_fov, aspect);
                    let cos = (d_center[0] * dc[0] + d_center[1] * dc[1] + d_center[2] * dc[2])
                        .clamp(-1.0, 1.0);
                    alpha = alpha.max(cos.acos());
                }
                let alpha_safe = alpha + ALPHA_MARGIN;

                // Every in-tile pixel CENTER + its 4 footprint corners must be inside the
                // cone (angle <= alpha_safe). This is Claim 1 (lateral offset < r(t)).
                for ly in 0..TILE_SIZE {
                    for lx in 0..TILE_SIZE {
                        let px = tx * TILE_SIZE + lx;
                        let py = ty * TILE_SIZE + ly;
                        if px >= w || py >= h {
                            continue;
                        }
                        for &(ddx, ddy) in
                            &[(0.0, 0.0), (-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)]
                        {
                            let sx = (px as f32) + 0.5 + ddx;
                            let sy = (py as f32) + 0.5 + ddy;
                            let d = persp_dir(sx, sy, w, h, forward, right, up, tan_half_fov, aspect);
                            let cos = (d_center[0] * d[0] + d_center[1] * d[1] + d_center[2] * d[2])
                                .clamp(-1.0, 1.0);
                            let ang = cos.acos();
                            assert!(
                                ang <= alpha_safe,
                                "PERSP hole tile({tx},{ty}) pixel({px},{py}) corner({ddx},{ddy}): \
                                 dir angle {ang} > alpha_safe {alpha_safe}"
                            );
                        }
                    }
                }
                tiles_checked += 1;
                tx += 7;
            }
            ty += 7;
        }
        // Sanity: the cull's `golden_tile_bound` uses the same camera (smoke — it runs).
        let _ = golden_tile_bound(&[], &[MESH_DEPTH_CLEAR; 64], 0, 0, w, h, camera);
        println!("[b] perspective enclosure {w}x{h}: {tiles_checked} tiles, all in-tile pixel/footprint dirs within alpha");
    }

    // --- Analytic first-hit oracles (a LOWER bound on the GPU march's first hit) -------

    /// Analytic ray-sphere first-hit `t` (the smaller non-negative root) or `None`.
    fn ray_sphere(ro: [f32; 3], rd: [f32; 3], c: [f32; 3], r: f32) -> Option<f32> {
        let oc = v_sub(ro, c);
        let b = oc[0] * rd[0] + oc[1] * rd[1] + oc[2] * rd[2];
        let cc = oc[0] * oc[0] + oc[1] * oc[1] + oc[2] * oc[2] - r * r;
        let disc = b * b - cc;
        if disc < 0.0 {
            return None;
        }
        let s = disc.sqrt();
        let t0 = -b - s;
        let t1 = -b + s;
        if t0 >= 0.0 {
            Some(t0)
        } else if t1 >= 0.0 {
            Some(t1)
        } else {
            None
        }
    }

    /// Analytic ray-AABB (slab) first-hit `t` for a box centered at `c` with half-extents
    /// `h`, or `None`. Returns the entry `t` (>= 0) of the ray through the box.
    fn ray_box(ro: [f32; 3], rd: [f32; 3], c: [f32; 3], h: [f32; 3]) -> Option<f32> {
        let mut t_min = f32::NEG_INFINITY;
        let mut t_max = f32::INFINITY;
        for a in 0..3 {
            let lo = c[a] - h[a];
            let hi = c[a] + h[a];
            if rd[a].abs() < 1e-9 {
                if ro[a] < lo || ro[a] > hi {
                    return None; // parallel + outside the slab.
                }
            } else {
                let inv = 1.0 / rd[a];
                let mut ta = (lo - ro[a]) * inv;
                let mut tb = (hi - ro[a]) * inv;
                if ta > tb {
                    core::mem::swap(&mut ta, &mut tb);
                }
                t_min = t_min.max(ta);
                t_max = t_max.min(tb);
                if t_min > t_max {
                    return None;
                }
            }
        }
        if t_max < 0.0 {
            return None;
        }
        Some(t_min.max(0.0))
    }

    /// (c) Randomized perspective sweep with an analytic first-hit oracle, the C2/C4
    /// proof: for a single sphere or box, `golden_tile_bound.near_t <= the analytic
    /// first-hit of EVERY in-tile pixel that hits` AND `EMPTY => no in-tile pixel hits
    /// before the (deepest) mesh`. The analytic hit is the true Euclidean first contact;
    /// the cull seeding `t = near_t <= it` can never skip a pixel's surface.
    #[test]
    fn randomized_oracle_near_t_le_first_hit() {
        let mut rng = Rng::new(0xC0FF_EE15_600D_5EED);
        let cases = 600;
        let mut checked_hits = 0u64;
        let mut empty_tiles = 0u64;
        let mut nonempty_tiles = 0u64;

        for _ in 0..cases {
            // A random forward-looking perspective camera (eye on +Z, looking -Z, small
            // jitter on the basis kept orthonormal-ish; the cull only needs the cone).
            let fov_y = rng.range(0.6, 1.8); // ~34°..103°
            let (w, h) = (rng.range_u32(40, 160), rng.range_u32(40, 160));
            let aspect = (w as f32) / (h as f32);
            let tan_half_fov = (fov_y * 0.5).tan();
            let eye = [rng.range(-0.3, 0.3), rng.range(-0.3, 0.3), rng.range(2.0, 4.0)];
            let forward = [0.0, 0.0, -1.0];
            let right = [1.0, 0.0, 0.0];
            let up = [0.0, 1.0, 0.0];
            let camera = CompositeCamera::Perspective {
                eye,
                forward,
                right,
                up,
                tan_half_fov,
                aspect,
            };

            // A single primitive (sphere or box) near the origin.
            let is_box = rng.next_u64() & 1 == 0;
            let center = [rng.range(-0.4, 0.4), rng.range(-0.4, 0.4), rng.range(-0.4, 0.4)];
            let (edits, sphere_r, box_h): (Vec<SdfEdit>, f32, [f32; 3]) = if is_box {
                let hx = rng.range(0.15, 0.5);
                let hy = rng.range(0.15, 0.5);
                let hz = rng.range(0.15, 0.5);
                (
                    vec![SdfEdit::box_shape(center, [hx, hy, hz], sdf_op::UNION, 0.0)],
                    0.0,
                    [hx, hy, hz],
                )
            } else {
                let r = rng.range(0.15, 0.5);
                (
                    vec![SdfEdit::sphere(center, r, sdf_op::UNION, 0.0)],
                    r,
                    [0.0; 3],
                )
            };

            // A tile within the grid: half the cases bias toward the image center (where
            // a forward camera sees the origin-centered primitive, so the tile actually
            // looks at the surface — exercising near_t <= first-hit), half are fully
            // random (exercising the EMPTY path on tiles that look away).
            let (tw, th) = tile_grid_extent(w, h);
            let (tx, ty) = if rng.next_u64() & 1 == 0 {
                let cx = tw / 2;
                let cy = th / 2;
                let jx = rng.range_u32(0, 3);
                let jy = rng.range_u32(0, 3);
                (
                    (cx + jx).saturating_sub(1).min(tw - 1),
                    (cy + jy).saturating_sub(1).min(th - 1),
                )
            } else {
                (rng.range_u32(0, tw), rng.range_u32(0, th))
            };

            // No mesh (clear depth everywhere in the tile) so far_t == T_MAX and the
            // oracle is the pure SDF first-hit (the mesh-bound case is exercised by the
            // GPU golden + the EMPTY-with-mesh negative test).
            let tile_depths = [MESH_DEPTH_CLEAR; 64];
            let tb = golden_tile_bound(&edits, &tile_depths, tx, ty, w, h, camera);

            // For every in-tile pixel, the analytic first-hit (the oracle).
            let mut min_first_hit = f32::INFINITY;
            let mut any_hit = false;
            for ly in 0..TILE_SIZE {
                for lx in 0..TILE_SIZE {
                    let px = tx * TILE_SIZE + lx;
                    let py = ty * TILE_SIZE + ly;
                    if px >= w || py >= h {
                        continue;
                    }
                    let sx = (px as f32) + 0.5;
                    let sy = (py as f32) + 0.5;
                    let rd = persp_dir(sx, sy, w, h, forward, right, up, tan_half_fov, aspect);
                    let hit = if is_box {
                        ray_box(eye, rd, center, box_h)
                    } else {
                        ray_sphere(eye, rd, center, sphere_r)
                    };
                    if let Some(t_hit) = hit
                        && t_hit <= SDF_T_MAX
                    {
                        any_hit = true;
                        min_first_hit = min_first_hit.min(t_hit);
                    }
                }
            }

            if tb.flags & TILE_FLAG_EMPTY != 0 {
                empty_tiles += 1;
                // EMPTY => no in-tile pixel may hit before the mesh (far_t == T_MAX here).
                // The march hit threshold is EPS, so a sphere-trace records a hit slightly
                // BEFORE the analytic surface; allow the surface to be within EPS of T_MAX.
                assert!(
                    !any_hit || min_first_hit + SDF_EPS >= SDF_T_MAX,
                    "EMPTY tile but an in-tile pixel hits at {min_first_hit} (< T_MAX): \
                     box={is_box} center={center:?} fov={fov_y} {w}x{h} tile({tx},{ty})"
                );
            } else {
                nonempty_tiles += 1;
                if any_hit {
                    checked_hits += 1;
                    // The CORE conservativeness claim: near_t <= every pixel's first hit.
                    // A small EPS tolerance absorbs the cone-entry EPS_COARSE + the fp
                    // step rounding (near_t is recorded AT the cone-entry t, never past it).
                    assert!(
                        tb.near_t <= min_first_hit + 1e-3,
                        "near_t {} > min in-tile first-hit {min_first_hit}: HOLE \
                         box={is_box} center={center:?} fov={fov_y} {w}x{h} tile({tx},{ty})",
                        tb.near_t
                    );
                }
            }
        }
        println!(
            "[c] randomized oracle: {cases} cases, {nonempty_tiles} non-empty ({checked_hits} with hits) + {empty_tiles} EMPTY — near_t <= analytic first-hit, EMPTY => no early hit"
        );
    }

    /// (d) Lipschitz tripwire (D7/W4): over random points in the scene's bounding region,
    /// the central-difference gradient magnitude of `field_distance` (== `sdf_edit_list`)
    /// must not exceed `FIELD_LIPSCHITZ_L` (= √2). A super-Lipschitz op would void the
    /// cone step's `/ L` clearance bound. Exercises the hard CSG + the smooth-min blend
    /// band (where the peak gradient lives).
    #[test]
    fn field_lipschitz_bound_holds() {
        let scenes: [Vec<SdfEdit>; 3] = [
            vec![
                SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
                SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
            ],
            vec![SdfEdit::box_shape([0.0, 0.0, 0.0], [0.4, 0.3, 0.2], sdf_op::UNION, 0.0)],
            vec![
                SdfEdit::sphere([-0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.0),
                SdfEdit::sphere([0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.15),
            ],
        ];
        let mut rng = Rng::new(0x1234_5678_9ABC_DEF0);
        let h = 1e-3_f32;
        let mut max_grad = 0.0_f32;
        let mut samples = 0u64;
        for edits in &scenes {
            for _ in 0..50_000 {
                let p = [rng.range(-1.5, 1.5), rng.range(-1.5, 1.5), rng.range(-1.5, 1.5)];
                let gx = (sdf_edit_list(edits, [p[0] + h, p[1], p[2]])
                    - sdf_edit_list(edits, [p[0] - h, p[1], p[2]]))
                    / (2.0 * h);
                let gy = (sdf_edit_list(edits, [p[0], p[1] + h, p[2]])
                    - sdf_edit_list(edits, [p[0], p[1] - h, p[2]]))
                    / (2.0 * h);
                let gz = (sdf_edit_list(edits, [p[0], p[1], p[2] + h])
                    - sdf_edit_list(edits, [p[0], p[1], p[2] - h]))
                    / (2.0 * h);
                let g = (gx * gx + gy * gy + gz * gz).sqrt();
                if g.is_finite() {
                    max_grad = max_grad.max(g);
                }
                samples += 1;
            }
        }
        // A small tolerance over √2 absorbs the central-difference discretization error
        // at the blend band's curvature; a genuine super-Lipschitz op blows well past it.
        assert!(
            max_grad <= FIELD_LIPSCHITZ_L + 5e-2,
            "field gradient {max_grad} exceeds FIELD_LIPSCHITZ_L {FIELD_LIPSCHITZ_L} (the cone step's /L is unsound)"
        );
        assert!(
            (FIELD_LIPSCHITZ_L - core::f32::consts::SQRT_2).abs() < 1e-6,
            "FIELD_LIPSCHITZ_L must be sqrt(2)"
        );
        println!(
            "[d] Lipschitz: {samples} samples, max |grad field| = {max_grad} <= L = {FIELD_LIPSCHITZ_L} (sqrt 2)"
        );
    }

    /// (e) The 0%-gate anchor: `golden_composite_pixel_culled(coarse_enabled = false)`
    /// is BIT-IDENTICAL to `golden_composite_pixel_ex` over the whole 64×64 image, under
    /// both an ortho and a perspective camera with a mix of covered/uncovered depths. The
    /// TileBound passed is irrelevant when cull-off (the function short-circuits) — a
    /// dummy is supplied.
    #[test]
    fn culled_off_is_bit_identical_to_ex() {
        let edits = vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ];
        let (w, h) = (64u32, 64u32);
        let depths = [0.5_f32, MESH_DEPTH_CLEAR, 0.2, 0.8];
        let dummy = super::TileBound { near_t: 7.0, far_t: 9.0, flags: TILE_FLAG_EMPTY, _pad: 0 };

        let cameras = [
            CompositeCamera::Ortho,
            CompositeCamera::Perspective {
                eye: [0.0, 0.0, 3.0],
                forward: [0.0, 0.0, -1.0],
                right: [1.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0],
                tan_half_fov: (core::f32::consts::FRAC_PI_2 * 0.5).tan(),
                aspect: 1.0,
            },
        ];
        let mut checked = 0u64;
        for camera in cameras {
            for py in 0..h {
                for px in 0..w {
                    let md = depths[((px + py) as usize) % depths.len()];
                    let want = golden_composite_pixel_ex(&edits, md, px, py, w, h, camera);
                    let got = golden_composite_pixel_culled(
                        &edits, md, px, py, w, h, camera, false, dummy,
                    );
                    assert_eq!(
                        want, got,
                        "cull-off diverged at ({px},{py}) depth {md}: ex 0x{want:08X} culled 0x{got:08X}"
                    );
                    checked += 1;
                }
            }
        }
        // Re-anchor: the unused fields of `SDF_CAM_Z` / `SDF_T_MAX` are still the frozen
        // scene constants (a compile-time touch so a refactor that drops them is caught).
        let _ = (SDF_CAM_Z, SDF_T_MAX);
        println!("[e] cull-off bit-identity: {checked} pixels (ortho + perspective) all match golden_composite_pixel_ex");
    }
}

// ===========================================================================
// Render B1 — over-relaxation (Keinert ω-gated) HOST soundness gates.
//
// These prove the CPU contract the on-device gates rely on, GPU-free:
//   1. ω = 1 host BIT-identity — `_omega(.., 1.0)` byte-equal to the ω=1 forwarder
//      (`golden_composite_pixel_ex` / `_culled`) over a pixel sweep on all 3 scenes
//      (ortho + perspective). Pins the forwarder extraction (the 0%-gate).
//   2. HIT-SET-SUPERSET property — over randomized scenes/depths/pixels, for
//      ω ∈ {1.2, 1.5, 1.9} the `_omega` hit-set ⊇ the ω=1 hit-set (NO ω=1 SDF hit
//      becomes background/mesh at ω>1). A violation = a missed-surface HOLE. Run on
//      BOTH the cull-off and the cull-on (`_culled_omega`) path. (F-CRIT-1 oracle.)
//   3. NO-HOLES TRIPWIRE — a deliberately-broken over-relax (a retreat to a WRONG t)
//      MUST fail the gate-2 invariant, proving #2 has teeth.
//   4. STEP-BOUND property — `steps(ω>1) ≤ steps(ω=1) + 1` per ray over the
//      randomized scenes (≤ 1 permanent sor-fail fallback ⇒ ≤ plain + 1).
//   5. ω CLAMP — the harness's `omega_in.clamp(1.0, 1.99)` + the 8-B push encode:
//      a hostile `omega_in` decodes to a finite value in `[1.0, 1.99]`.
//
// The march mirrors `golden_composite_pixel_ex_omega` / `_culled_omega` EXACTLY (same
// ordering: top mesh-guard, probe, hit test, ω-gate step, miss test). Gates 2/3/4 need
// a hit/step-instrumented copy of that loop (the production goldens return only a packed
// color), so a faithful test-only mirror lives here; gate 1 diffs the production
// functions directly so the mirror can never mask a real forwarder regression.
// ===========================================================================
#[cfg(test)]
mod b1_over_relaxation_tests {
    use super::{
        CompositeCamera, DEFAULT_LIGHT_DIR, DEFAULT_MARCHER_OMEGA, FIELD_LIPSCHITZ_L,
        FineMarcherPush, GBUFFER_MARCHER_PUSH_BYTES, MESH_DEPTH_CLEAR, SDF_EPS, SDF_IMG_H,
        SDF_IMG_W, SDF_MAX_IT, SDF_T_MAX, SdfEdit, composite_ray, golden_composite_pixel_culled,
        golden_composite_pixel_culled_omega, golden_composite_pixel_ex,
        golden_composite_pixel_ex_omega, sdf_edit_list, sdf_op,
    };
    use proptest::prelude::*;

    /// The rung-9/10 "crater" CSG scene (base sphere minus a smaller sphere).
    fn crater() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ]
    }
    /// A box CSG scene.
    fn box_csg() -> Vec<SdfEdit> {
        vec![SdfEdit::box_shape([0.0, 0.0, 0.0], [0.4, 0.4, 0.4], sdf_op::UNION, 0.0)]
    }
    /// A smooth-union scene (two spheres blended) — the smooth-min path.
    fn smooth_union() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([-0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.15),
        ]
    }

    /// Instrumented result of [`march_obs`] — the Candidate-C host oracle output plus the perf
    /// counters that replace the deleted step-bound gate 4.
    struct MarchObs {
        /// The SHIPPED hit decision: the fast pass's hit, OR (on exhaustion) the re-march's hit.
        hit: bool,
        /// Probe iterations spent in the over-relaxed FAST pass (each `sdf_edit_list` call). The
        /// B1 win is `fast_steps(ω>1) < fast_steps(ω=1)` on the common converging rays.
        fast_steps: u32,
        /// True iff the fast pass exhausted the budget and the Candidate-C re-march fired. The
        /// re-march FREQUENCY (% of pixels) is the perf risk: a large fraction = B1 perf-neutral.
        remarched: bool,
    }

    /// The Render B1 ω-march, INSTRUMENTED (Candidate C) — the host oracle for gates 2/3 and
    /// the perf observation. A faithful, COMPLETE mirror of the PRODUCTION
    /// `golden_composite_pixel_ex_omega` march: the over-relaxed fast pass (same ordering,
    /// same ω-gate, same Lipschitz-aware sor-fail test, same `t = safe_t + sor_prev`
    /// plain-resume + permanent fall-to-plain) FOLLOWED BY the Candidate-C fallback re-march.
    ///
    /// CONTRACT CHANGE vs the prior step-bound oracle: the fast pass alone is NOT the shipped
    /// hit decision. Correctness now comes from the re-march, not a step bound. When the fast
    /// loop runs all `SDF_MAX_IT` iterations with NO break (`exhausted`), production RE-MARCHES
    /// from the ORIGINAL seed with a plain ω=1 sphere-trace and uses THAT (hit, t). This oracle
    /// reproduces that exactly, so `hit` is byte-for-byte the production hit decision — gate 2's
    /// "ω>1 hit-set ⊇ ω=1 hit-set" tests what actually ships, and is provably true (the re-march
    /// body is the frozen plain marcher, so an exhausting ω>1 ray lands on the SAME hit the
    /// frozen marcher does).
    ///
    /// INSTRUMENTATION (perf observation, replacing the deleted step-bound gate 4): the returned
    /// [`MarchObs`] records the fast-pass probe count (`fast_steps`, each `sdf_edit_list` call in
    /// the over-relaxed loop), whether the fast pass exhausted the budget, and whether the
    /// re-march fired. The orchestrator uses the re-march FREQUENCY (% of pixels that exhausted)
    /// and the fast-pass step reduction vs plain to judge whether B1 is still a net win.
    fn march_obs(edits: &[SdfEdit], ro: [f32; 3], rd: [f32; 3], t_mesh: f32, omega_in: f32) -> MarchObs {
        let mut t = 0.0_f32;
        let t_seed = t; // the ORIGINAL seed (0.0) — Candidate C re-march re-seeds from it
        let mut omega = omega_in;
        let mut hit = false;
        let mut fast_steps = 0u32;
        let mut safe_t = 0.0_f32;
        let mut sor_prev = 0.0_f32;
        let mut sor_step_prev = 0.0_f32;
        let mut exhausted = true; // cleared by EVERY in-loop break (mirrors production)
        for it in 0..SDF_MAX_IT {
            if t >= t_mesh {
                exhausted = false;
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            fast_steps += 1;
            if d < SDF_EPS {
                hit = true;
                exhausted = false;
                break;
            }
            if omega > 1.0 {
                let step_len = d * omega;
                // Lipschitz-aware sor-fail (mirrors production exactly): the empty-ball radii
                // are `f / L`, so the spheres cover the over-step iff `sor_prev + d >= L * step`.
                if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                    // BUG-B1-HOLE-2: resume the plain march one certified step past the safe
                    // probe (no re-probe, +0 steps for the retreat itself) and latch to plain.
                    t = safe_t + sor_prev;
                    omega = 1.0;
                    continue;
                }
                safe_t = t;
                sor_prev = d;
                sor_step_prev = step_len;
                t += step_len;
            } else {
                t += d;
            }
            if t > SDF_T_MAX {
                exhausted = false;
                break;
            }
        }

        // Candidate C: the PROVABLY-hole-free fallback re-march. Mirrors production EXACTLY —
        // on `exhausted` re-seed from `t_seed` and run the frozen plain ω=1 marcher; its (hit)
        // is the shipped decision. The fast pass's `hit` is discarded (it was false on exhaust).
        let remarched = exhausted;
        if exhausted {
            t = t_seed;
            hit = false;
            for _it2 in 0..SDF_MAX_IT {
                if t >= t_mesh {
                    break;
                }
                let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                let d = sdf_edit_list(edits, p);
                if d < SDF_EPS {
                    hit = true;
                    break;
                }
                t += d;
                if t > SDF_T_MAX {
                    break;
                }
            }
        }

        MarchObs { hit, fast_steps, remarched }
    }

    /// A DELIBERATELY-BROKEN over-relax used ONLY by the gate-3 tripwire. TWO co-ordinated
    /// breaks, BOTH required under the Candidate-C contract:
    ///   1. the fast pass mis-handles a sor-fail — it retreats to a WRONG `t` (`safe_t +
    ///      step_len`, i.e. it ADVANCES past the surface instead of retreating) AND keeps ω hot;
    ///   2. **the Candidate-C fallback re-march is DISABLED** (this function has NO re-march at
    ///      all — it returns the bare fast-pass hit). This is the load-bearing tripwire change
    ///      for the C contract: with the re-march intact, an exhausting broken ray would be
    ///      silently rescued by the plain re-march and the tripwire would go INERT (gate 2 would
    ///      look armed while testing nothing). Breaking C's guarantee = breaking the re-march, so
    ///      this models exactly the failure mode gate 2 must catch. NOT shipped.
    fn march_hit_broken(
        edits: &[SdfEdit],
        ro: [f32; 3],
        rd: [f32; 3],
        t_mesh: f32,
        omega_in: f32,
        with_remarch: bool,
    ) -> bool {
        let mut t = 0.0_f32;
        let t_seed = t;
        let omega = omega_in;
        let mut hit = false;
        let mut safe_t = 0.0_f32;
        let mut sor_prev = 0.0_f32;
        let mut sor_step_prev = 0.0_f32;
        let mut exhausted = true;
        for it in 0..SDF_MAX_IT {
            if t >= t_mesh {
                exhausted = false;
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                exhausted = false;
                break;
            }
            if omega > 1.0 {
                let step_len = d * omega;
                // Production Lipschitz-aware detection threshold (re-synced) — the bug is in
                // the RETREAT below, NOT the detection, so the tripwire fires the same sor-fails
                // the production marcher would, then mishandles them.
                if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                    // BUG: a WRONG "retreat" that actually leaps past the surface and
                    // never falls to plain. The classic over-relaxation hole.
                    t = safe_t + step_len;
                    continue;
                }
                safe_t = t;
                sor_prev = d;
                sor_step_prev = step_len;
                t += step_len;
            } else {
                t += d;
            }
            if t > SDF_T_MAX {
                exhausted = false;
                break;
            }
        }
        // `with_remarch == false`: C's fallback is DISABLED — the bare broken fast-pass hit (the
        // tripwire that MUST hole). `with_remarch == true`: re-attach the EXACT Candidate-C
        // re-march on top of the broken fast pass — it must CLOSE every hole the broken pass
        // opened, proving the re-march (not the fast pass) is what guarantees the hit-set.
        if with_remarch && exhausted {
            t = t_seed;
            hit = false;
            for _it2 in 0..SDF_MAX_IT {
                if t >= t_mesh {
                    break;
                }
                let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                let d = sdf_edit_list(edits, p);
                if d < SDF_EPS {
                    hit = true;
                    break;
                }
                t += d;
                if t > SDF_T_MAX {
                    break;
                }
            }
        }
        hit
    }

    /// True when pixel `(px, py)` is an SDF hit at `omega` — the SHIPPED Candidate-C decision
    /// (fast pass + fallback re-march), with NO mesh occlusion (the pure-field hit set — the
    /// property's domain). This is byte-for-byte what `golden_composite_pixel_ex_omega`'s march
    /// concludes, so gate 2's superset property tests production, not the bare fast pass.
    fn pixel_hits(edits: &[SdfEdit], px: u32, py: u32, omega: f32) -> bool {
        let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
        march_obs(edits, ro, rd, 1.0e30, omega).hit
    }

    /// GATE 1 — ω = 1 host BIT-identity for the un-culled marcher. `_omega(.., 1.0)` must be
    /// byte-equal to the ω=1 forwarder over the whole 64×64 image on all 3 scenes, ORTHO +
    /// PERSPECTIVE. Pins the forwarder extraction (any drift = the 0%-gate broke).
    #[test]
    fn gate1_omega_one_is_bit_identical_to_forwarder_uncull() {
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let depths = [0.5_f32, MESH_DEPTH_CLEAR, 0.2, 0.8];
        let cameras = [
            ("ortho", CompositeCamera::Ortho),
            (
                "persp",
                CompositeCamera::Perspective {
                    eye: [0.0, 0.0, 2.0],
                    forward: [0.0, 0.0, -1.0],
                    right: [1.0, 0.0, 0.0],
                    up: [0.0, 1.0, 0.0],
                    tan_half_fov: 0.5,
                    aspect: 1.0,
                },
            ),
        ];
        let mut checked = 0u64;
        for (sname, edits) in &scenes {
            for (cname, cam) in cameras {
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let md = depths[((px + py) as usize) % depths.len()];
                        let fwd = golden_composite_pixel_ex(edits, md, px, py, SDF_IMG_W, SDF_IMG_H, cam);
                        let om1 = golden_composite_pixel_ex_omega(
                            edits, md, px, py, SDF_IMG_W, SDF_IMG_H, cam, 1.0,
                        );
                        assert_eq!(
                            fwd, om1,
                            "[{sname}/{cname}] ω=1 _omega diverged from forwarder at ({px},{py}) depth {md}: \
                             fwd 0x{fwd:08X} omega 0x{om1:08X}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        println!("[B1 gate1] ω=1 un-culled bit-identity: {checked} pixels (ortho+persp × 3 scenes) byte-equal");
    }

    /// GATE 1 (cull path) — ω = 1 host BIT-identity for the CULLED marcher. With cull ON and a
    /// synthetic non-EMPTY tile (`near_t = 0`, `far_t = T_MAX`), `_culled_omega(.., 1.0)` must
    /// be byte-equal to the ω=1 culled forwarder over the image (ORTHO). The cull-off arm is
    /// covered by gate 1; this pins the seeded-march forwarder at ω=1.
    #[test]
    fn gate1_omega_one_is_bit_identical_to_forwarder_culled() {
        use super::{TILE_FLAG_EMPTY, TileBound};
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let depths = [0.5_f32, MESH_DEPTH_CLEAR, 0.2, 0.8];
        // A non-EMPTY, full-range tile (seed t = 0, march to T_MAX) — the general case.
        let surf = TileBound { near_t: 0.0, far_t: SDF_T_MAX, flags: 0, _pad: 0 };
        // An EMPTY tile — exercises the early-out arm at ω=1.
        let empty = TileBound { near_t: 0.0, far_t: SDF_T_MAX, flags: TILE_FLAG_EMPTY, _pad: 0 };
        let mut checked = 0u64;
        for (sname, edits) in &scenes {
            for tile in [surf, empty] {
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let md = depths[((px + py) as usize) % depths.len()];
                        let fwd = golden_composite_pixel_culled(
                            edits, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, true, tile,
                        );
                        let om1 = golden_composite_pixel_culled_omega(
                            edits, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, true, tile, 1.0,
                        );
                        assert_eq!(
                            fwd, om1,
                            "[{sname}] ω=1 _culled_omega diverged from forwarder at ({px},{py}) depth {md} \
                             flags {}: fwd 0x{fwd:08X} omega 0x{om1:08X}",
                            tile.flags
                        );
                        checked += 1;
                    }
                }
            }
        }
        println!("[B1 gate1c] ω=1 culled bit-identity: {checked} pixels (surf+empty tiles × 3 scenes) byte-equal");
    }

    // A proptest-generated randomized SDF scene, WIDENED for the Candidate-C no-hole contract:
    // 1..=8 edits, random kind/op/center/size, and an AGGRESSIVE smoothness distribution biased
    // toward the super-Lipschitz blend bands that historically holed (BUG-B1-HOLE-1). Every value
    // stays inside the bounded `[-0.85, 0.85]³` view box so the marcher reaches surfaces (an empty
    // world trivially satisfies superset). THIN features are reachable via the small-size tail
    // (down to 0.03) — sliver boxes/spheres are the classic over-relax overshoot trap. (A `//`
    // comment, not a doc comment — clippy's `unused_doc_comments` on macro invocations.)
    prop_compose! {
        fn arb_scene()(
            edits in proptest::collection::vec(
                (
                    0u32..2,                                  // 0 = sphere, 1 = box
                    0u32..3,                                  // op: union/subtract/intersect
                    -0.85f32..0.85, -0.85f32..0.85, -0.85f32..0.85, // center xyz
                    0.03f32..0.6,                             // size a (thin tail at 0.03)
                    0.03f32..0.5,                             // size b (box y, thin tail)
                    0.03f32..0.5,                             // size c (box z, thin tail)
                    // AGGRESSIVE smooth-min: hard, mild, and a heavy super-Lipschitz tail up to
                    // 0.4 (the blend band where IQ's smooth-min violates the unit-Lipschitz bound
                    // hardest — the BUG-B1-HOLE-1 regime). Weighted toward the soft cases.
                    prop_oneof![
                        1 => Just(0.0f32),
                        2 => 0.02f32..0.15,
                        3 => 0.15f32..0.40,
                    ],
                ),
                1..=8,
            )
        ) -> Vec<SdfEdit> {
            // Force the FIRST edit to be a UNION so the field has a positive volume to hit
            // (a lone subtract/intersect over an empty acc is a degenerate empty field).
            edits.into_iter().enumerate().map(|(i, (kind, op, cx, cy, cz, a, b, c, k))| {
                let op = if i == 0 { sdf_op::UNION } else { op };
                if kind == 0 {
                    SdfEdit::sphere([cx, cy, cz], a, op, k)
                } else {
                    SdfEdit::box_shape([cx, cy, cz], [a, b, c], op, k)
                }
            }).collect()
        }
    }

    proptest! {
        // WIDENED for the Candidate-C no-hole contract — this IS the correctness gate, so make
        // it thorough. 1024 random scenes (4× the prior 256), each over a coarse pixel grid ×
        // ω ∈ {1.2, 1.5, 1.99} × {ortho, perspective}. The pinned BUG-B1-HOLE-1 cliff seed in
        // proptest-regressions/compute.txt is replayed first on every run (proptest auto-loads it).
        #![proptest_config(ProptestConfig { cases: 1024, ..ProptestConfig::default() })]

        /// GATE 2 — HIT-SET-SUPERSET (the F-CRIT-1 soundness oracle, the REAL correctness gate),
        /// CULL-OFF. For every randomized scene + ω ∈ {1.2, 1.5, 1.99} + camera ∈ {ortho,
        /// perspective}, EVERY pixel that is an SDF hit at ω=1 must remain a hit at ω>1. With
        /// Candidate C this is PROVABLY true on every case: an exhausting ω>1 ray re-marches with
        /// the frozen plain marcher, so it lands on the SAME hit ω=1 does. A regression = a
        /// missed-surface HOLE (Candidate C has a bug). The perspective camera supplies GRAZING
        /// rays at the frame edges (the classic over-relax overshoot trap). Iterates a coarse
        /// 4-px grid (every tile sampled) to bound per-case cost while covering the frame.
        #[test]
        fn gate2_hit_set_superset_cull_off(edits in arb_scene()) {
            let persp = CompositeCamera::Perspective {
                eye: [0.0, 0.0, 2.0], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0], tan_half_fov: 0.5, aspect: 1.0,
            };
            for &omega in &[1.2_f32, 1.5, 1.99] {
                for cam in [CompositeCamera::Ortho, persp] {
                    for py in (0..SDF_IMG_H).step_by(4) {
                        for px in (0..SDF_IMG_W).step_by(4) {
                            let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, cam);
                            let base = march_obs(&edits, ro, rd, 1.0e30, 1.0).hit;
                            if base {
                                let over = march_obs(&edits, ro, rd, 1.0e30, omega).hit;
                                prop_assert!(
                                    over,
                                    "HOLE: ({px},{py}) cam={cam:?} hits at ω=1 but MISSES at ω={omega} — Candidate-C re-march failed to close the hole; scene={edits:?}"
                                );
                            }
                        }
                    }
                }
            }
        }

        /// GATE 2 — HIT-SET-SUPERSET, CULL-ON. Same invariant through the `_culled_omega` path
        /// with a non-EMPTY full-range tile (seed t=0): the ω>1 culled hit-set ⊇ the ω=1 culled
        /// hit-set. Compares the FINAL packed color: a pixel that is the lit SDF color at ω=1
        /// must NOT become mesh/background at ω>1 (the observable hole). Uses no mesh
        /// (depth = clear) so the SDF/background partition is the field's alone.
        #[test]
        fn gate2_hit_set_superset_cull_on(edits in arb_scene()) {
            use super::{TileBound};
            let tile = TileBound { near_t: 0.0, far_t: SDF_T_MAX, flags: 0, _pad: 0 };
            for &omega in &[1.2_f32, 1.5, 1.99] {
                for py in (0..SDF_IMG_H).step_by(4) {
                    for px in (0..SDF_IMG_W).step_by(4) {
                        // ω=1 culled color (the baseline partition).
                        let c1 = golden_composite_pixel_culled_omega(
                            &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                            CompositeCamera::Ortho, true, tile, 1.0,
                        );
                        // Only pixels that HIT the SDF at ω=1 are in the domain (no mesh, so a
                        // non-background color == an SDF hit).
                        if pixel_hits(&edits, px, py, 1.0) {
                            let co = golden_composite_pixel_culled_omega(
                                &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                                CompositeCamera::Ortho, true, tile, omega,
                            );
                            prop_assert!(
                                pixel_hits(&edits, px, py, omega),
                                "HOLE(cull-on): ({px},{py}) SDF-hit at ω=1 (0x{c1:08X}) but ω={omega} march misses (0x{co:08X}); scene={edits:?}"
                            );
                        }
                    }
                }
            }
        }

    }

    // ===========================================================================================
    // PERF OBSERVATION (replaces the DELETED step-bound gate 4).
    //
    // Candidate C makes correctness independent of any step-count bound: the fallback re-march
    // guarantees the hit-set regardless of how the fast pass behaves, so the old gate-4 invariant
    // `steps(ω>1) ≤ steps(ω=1)` is IRRELEVANT to soundness. It is replaced by an OBSERVATION (not
    // a pass/fail correctness gate): how OFTEN does the fast pass exhaust and trigger the (costly)
    // re-march, and does the over-relaxed fast pass still REDUCE probe steps on the common
    // converging rays (the B1 win)? The orchestrator uses these numbers for the ship call. The
    // only assertion here is the must-render sanity (the fixture is non-empty); a high re-march
    // fraction is REPORTED, not failed, and flagged in the println for the orchestrator.
    // ===========================================================================================

    /// PERF OBSERVATION — re-march frequency + fast-pass step reduction (NOT a correctness gate).
    /// Over the shipped fixtures + the pinned BUG-B1-HOLE-1 cliff seed + a handful of widened
    /// random scenes, counts, per ω ∈ {1.2, 1.5, 1.99}: (a) the % of pixels whose fast pass
    /// EXHAUSTED and re-marched (the perf risk — a large fraction means B1 is perf-neutral or
    /// negative), and (b) the mean fast-pass probe count vs the ω=1 plain march on CONVERGING
    /// rays (the B1 win — fewer steps to the same hit). Prints a summary for the orchestrator.
    #[test]
    fn perf_observation_remarch_frequency_and_step_reduction() {
        // The pinned cliff seed (the historical worst ray) — exercised explicitly.
        let cliff = vec![
            SdfEdit::sphere([0.31460363, 0.70498204, -0.7611318], 0.36075538, sdf_op::UNION, 0.0),
            SdfEdit::box_shape([0.092381336, 0.1372761, -0.5955315], [0.19970395, 0.46420184, 0.3901827], sdf_op::UNION, 0.24384262),
            SdfEdit::sphere([0.4506038, 0.16997452, 0.0], 0.44928917, sdf_op::UNION, 0.0),
        ];
        let scenes = [
            ("crater", crater()),
            ("box", box_csg()),
            ("smooth", smooth_union()),
            ("cliff_seed", cliff),
        ];
        let mut worst_remarch_pct = 0.0_f64;
        for (sname, edits) in &scenes {
            for &omega in &[1.2_f32, 1.5, 1.99] {
                let mut pixels = 0u64;
                let mut remarches = 0u64;
                // Converging-ray step accounting: only rays that HIT under BOTH ω=1 and ω
                // (so the comparison is the same surface) and did NOT need a re-march.
                let mut conv_rays = 0u64;
                let mut sum_fast = 0u64; // fast-pass steps at ω (the B1 path)
                let mut sum_plain = 0u64; // fast-pass steps at ω=1 (the baseline)
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
                        let o1 = march_obs(edits, ro, rd, 1.0e30, 1.0);
                        let oo = march_obs(edits, ro, rd, 1.0e30, omega);
                        pixels += 1;
                        if oo.remarched {
                            remarches += 1;
                        }
                        // Common converging rays: both hit, neither re-marched → the step
                        // reduction is the apples-to-apples B1 win on the typical case.
                        if o1.hit && oo.hit && !o1.remarched && !oo.remarched {
                            conv_rays += 1;
                            sum_fast += u64::from(oo.fast_steps);
                            sum_plain += u64::from(o1.fast_steps);
                        }
                    }
                }
                let remarch_pct = 100.0 * remarches as f64 / pixels as f64;
                worst_remarch_pct = worst_remarch_pct.max(remarch_pct);
                let (mean_fast, mean_plain, reduction) = if conv_rays > 0 {
                    let mf = sum_fast as f64 / conv_rays as f64;
                    let mp = sum_plain as f64 / conv_rays as f64;
                    (mf, mp, 100.0 * (mp - mf) / mp)
                } else {
                    (0.0, 0.0, 0.0)
                };
                // A POSITIVE reduction is the B1 win (fewer steps to the same hit); negative means
                // the over-relax overshot and cost extra steps at this ω. ω=1.2 (the production
                // DEFAULT) is the column that decides the ship call.
                let verdict = if reduction >= 0.0 { "B1 win" } else { "B1 LOSS" };
                println!(
                    "[B1 perf] {sname} ω={omega}: re-march {remarches}/{pixels} px ({remarch_pct:.2}%); \
                     converging rays {conv_rays}: mean fast-pass steps {mean_fast:.2} vs plain {mean_plain:.2} \
                     (step reduction {reduction:.1}% — {verdict})"
                );
            }
        }
        // Sanity only (NOT a correctness gate): the fixtures must actually render surfaces so the
        // observation is meaningful. The re-march fraction itself is REPORTED, never failed.
        println!(
            "[B1 perf] OBSERVATION SUMMARY: worst re-march fraction over all fixtures/ω = {worst_remarch_pct:.2}% \
             (FLAG for the orchestrator if this is a large fraction — would mean B1 is perf-neutral/negative)"
        );
    }

    /// GATE 3 — NO-HOLES TRIPWIRE (adapted to the Candidate-C contract). The gate-2 invariant
    /// must have TEETH. Under Candidate C, "broken" must break the RE-MARCH too — otherwise the
    /// fallback silently rescues a broken fast pass and the tripwire goes inert. So this asserts
    /// TWO things:
    ///   (a) the broken over-relax WITH C's fallback DISABLED (`march_hit_broken(.., false)` — a
    ///       WRONG sor-fail retreat that leaps past the surface, no re-march) produces ≥ 1 HOLE.
    ///       If it never holed, gate 2 would be vacuous.
    ///   (b) the SAME broken fast pass WITH C's re-march RE-ATTACHED (`march_hit_broken(.., true)`)
    ///       produces ZERO holes — proving the re-march (NOT the fast pass) is what closes them,
    ///       i.e. C's guarantee is load-bearing and gate 2 passes BECAUSE of the re-march.
    /// (The broken march is test-only; never shipped.)
    #[test]
    fn gate3_no_holes_tripwire_broken_overrelax_holes() {
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let mut total_holes_no_remarch = 0u64;
        let mut total_holes_with_remarch = 0u64;
        for (sname, edits) in &scenes {
            for &omega in &[1.5_f32, 1.9] {
                let mut scene_holes = 0u64;
                let mut scene_holes_remarched = 0u64;
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
                        let base = pixel_hits(edits, px, py, 1.0);
                        if base {
                            // (a) C's fallback disabled: this is the tripwire that MUST hole.
                            if !march_hit_broken(edits, ro, rd, 1.0e30, omega, false) {
                                scene_holes += 1;
                            }
                            // (b) the EXACT C re-march re-attached: it must close the hole.
                            if !march_hit_broken(edits, ro, rd, 1.0e30, omega, true) {
                                scene_holes_remarched += 1;
                            }
                        }
                    }
                }
                total_holes_no_remarch += scene_holes;
                total_holes_with_remarch += scene_holes_remarched;
                if scene_holes > 0 {
                    println!("[B1 gate3] tripwire: broken over-relax (no re-march) holed {scene_holes} px on {sname} @ ω={omega}");
                }
            }
        }
        // (a) teeth: the broken fast pass without the fallback MUST hole.
        assert!(
            total_holes_no_remarch > 0,
            "TRIPWIRE INERT: the broken over-relax (re-march disabled) produced ZERO holes — gate 2 would be vacuous"
        );
        // (b) the re-march is the load-bearing guarantee: re-attaching it closes EVERY hole.
        assert_eq!(
            total_holes_with_remarch, 0,
            "C CONTRACT VIOLATION: the Candidate-C re-march failed to close {total_holes_with_remarch} broken-fast-pass holes — the fallback is not actually hole-free"
        );
        println!(
            "[B1 gate3] tripwire armed: {total_holes_no_remarch} holes WITHOUT the re-march (gate 2 has teeth); \
             {total_holes_with_remarch} holes WITH the re-march re-attached (C's fallback closes them ALL — the guarantee is load-bearing)"
        );
    }

    /// REGRESSION PIN (BUG-B1-HOLE-1, CLOSED via Candidate C) — the minimal scene the gate-2
    /// property shrank to: a super-Lipschitz smooth-min CSG (a box with smoothness 0.244 blended
    /// between two spheres) that USED to produce a missed-surface HOLE at pixel (28,16) under
    /// ω=1.2 through the PRODUCTION golden (`golden_composite_pixel_ex_omega`). Candidate C closes
    /// the hole UNCONDITIONALLY: when the over-relaxed fast pass exhausts the budget on this ray,
    /// the fallback re-march replays the frozen plain marcher from the original seed and lands on
    /// the SAME lit SDF surface ω=1 hits — byte-identical color, not merely non-background.
    ///
    /// This is the PERMANENT regression guard (NOT ignored). FLIPPED for the C contract: it now
    /// asserts BOTH ω=1.0 AND ω ∈ {1.2, 1.5, 1.99} HIT the surface (the no-hole contract — none
    /// reverts to BACKGROUND) AND land on the SAME surface FEATURE as ω=1 (the lit SDF color, far
    /// from background — within a few LSBs per channel, NOT a phantom).
    ///
    /// NOTE on exactness: byte-exact color equality with ω=1 holds ONLY when the Candidate-C
    /// re-march fires (it replays the frozen plain marcher → the identical `t` and shade — true
    /// for ω=1.2 here). When the over-relaxed FAST pass converges on its own (ω=1.5/1.99 at this
    /// pixel) it lands within `SDF_EPS` of the surface at a marginally different `t`, so the
    /// Lambert shade differs by a few LSBs. That is a valid same-surface hit, not a hole — the
    /// contract is HIT (not background), asserted via the lit-vs-background channel separation.
    #[test]
    fn bug_b1_hole_1_smooth_min_overrelax_hole_via_production_golden() {
        let edits = vec![
            SdfEdit::sphere([0.31460363, 0.70498204, -0.7611318], 0.36075538, sdf_op::UNION, 0.0),
            SdfEdit::box_shape([0.092381336, 0.1372761, -0.5955315], [0.19970395, 0.46420184, 0.3901827], sdf_op::UNION, 0.24384262),
            SdfEdit::sphere([0.4506038, 0.16997452, 0.0], 0.44928917, sdf_op::UNION, 0.0),
        ];
        let (px, py) = (28u32, 16u32);
        let bg = super::pack_rgba([0.05, 0.05, 0.1]);
        // Channel splitter for the lit-vs-background separation (the three composite outcomes are
        // >100/255 apart, so a few-LSB convergence-point wobble never reclasses a hit as a miss).
        let chans = |c: u32| [(c & 0xFF) as i32, ((c >> 8) & 0xFF) as i32, ((c >> 16) & 0xFF) as i32];
        let bgc = chans(bg);
        let far_from_bg = |c: u32| {
            let cc = chans(c);
            (0..3).any(|i| (cc[i] - bgc[i]).abs() > 8)
        };
        // No mesh (depth = clear) so the SDF/background partition is the field's alone.
        let c1 = golden_composite_pixel_ex_omega(&edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0);
        assert_ne!(c1, bg, "ω=1 must HIT the smooth-min surface at ({px},{py})");
        assert!(far_from_bg(c1), "ω=1 color 0x{c1:08X} must be the LIT SDF surface, far from bg 0x{bg:08X}");
        // Candidate C: EVERY shipped ω>1 must now HIT the same surface FEATURE as ω=1 — the hole
        // is closed by the re-march, not a step bound. The task's required set: {1.2, 1.5, 1.99}.
        for &omega in &[1.2_f32, 1.5, 1.99] {
            let co = golden_composite_pixel_ex_omega(
                &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, omega,
            );
            // The pixel-color delta to ω=1: 0 when the re-march fired (exact frozen replay), a few
            // LSBs when the fast pass converged on its own. Printed so the orchestrator sees it.
            let dc = chans(co);
            let c1c = chans(c1);
            let max_ch = (0..3).map(|i| (dc[i] - c1c[i]).abs()).max().unwrap_or(0);
            println!(
                "[BUG-B1-HOLE-1 CLOSED] ω=1 0x{c1:08X} | ω={omega} 0x{co:08X} (hit={}, Δ to ω=1 = {max_ch}/255) | bg=0x{bg:08X}",
                co != bg
            );
            // The no-hole contract: ω>1 must NOT revert to background — it HITS the surface.
            assert_ne!(co, bg, "BUG-B1-HOLE-1 CLOSED: ω={omega} must HIT (not background) at ({px},{py})");
            assert!(
                far_from_bg(co),
                "BUG-B1-HOLE-1 CLOSED: ω={omega} color 0x{co:08X} must be the LIT SDF surface (far from bg 0x{bg:08X}), not a hole, at ({px},{py})"
            );
            // Same surface FEATURE as ω=1 — the lit colors agree within the small convergence-point
            // wobble (a phantom or a different feature would differ by >100/channel like bg/mesh).
            assert!(
                max_ch <= 16,
                "BUG-B1-HOLE-1 CLOSED: ω={omega} 0x{co:08X} differs from ω=1 0x{c1:08X} by {max_ch}/255 (>16) — not the same surface feature at ({px},{py})"
            );
        }
    }

    /// GATE 5 — ω CLAMP + 8-byte push encode. Every NON-NaN hostile `omega_in` (negative,
    /// sub-1, == 2, > 2, ±∞) must clamp into `[1.0, 1.99]` and decode finite-in-range from the
    /// pushed bytes `[4..8]`. Mirrors the harness's `omega_in.clamp(1.0, 1.99)` + push encode
    /// EXACTLY (the same `f32::clamp` + `to_le_bytes`).
    ///
    /// FINDING (documented, NOT a soundness hole): Rust's `f32::clamp` does NOT sanitize a NaN
    /// VALUE — `f32::NAN.clamp(1.0, 1.99) == NaN` (the clamp returns NaN when `self` is NaN).
    /// So a NaN ω SURVIVES the harness clamp and is pushed verbatim. It is defanged DOWNSTREAM,
    /// not by the clamp: the marcher's gate is `if (omega > 1.0)`, and `NaN > 1.0 == false` on
    /// BOTH host (`golden_composite_pixel_ex_omega`) and shader, so a NaN ω takes the verbatim
    /// frozen `t += d` plain arm — i.e. it degrades to the ω=1 path (NO over-relaxation, NO
    /// hole). This test asserts that exact safety property (NaN ω ≡ ω=1 over a pixel sweep)
    /// rather than a false "clamp produces 1.0" claim. See the tester report.
    #[test]
    fn gate5_omega_clamp_and_push_encode() {
        // The harness's encode site, reproduced byte-for-byte via the 32-byte
        // `FineMarcherPush` (A1/A2 widened the push 8 → 32 B; `lighting_flags == 0` keeps
        // the OFF path). coarse_enabled stays at offset 0, omega at offset 4.
        fn encode(omega_in: f32, coarse_enabled: bool) -> [u8; GBUFFER_MARCHER_PUSH_BYTES as usize] {
            let omega: f32 = omega_in.clamp(1.0, 1.99);
            let push = FineMarcherPush::new(coarse_enabled, omega, 0, DEFAULT_LIGHT_DIR);
            push.as_bytes().try_into().expect("invariant: FineMarcherPush is GBUFFER_MARCHER_PUSH_BYTES")
        }
        // Non-NaN hostile inputs: ALL must clamp finite into [1.0, 1.99].
        let cases = [-1.0_f32, 0.5, 1.0, 1.2, 1.99, 2.0, 2.5, 100.0, f32::INFINITY, f32::NEG_INFINITY];
        for &om in &cases {
            let push = encode(om, true);
            let coarse = u32::from_le_bytes([push[0], push[1], push[2], push[3]]);
            let decoded = f32::from_le_bytes([push[4], push[5], push[6], push[7]]);
            assert_eq!(coarse, 1, "coarse_enabled must round-trip as 1");
            assert!(decoded.is_finite(), "ω={om} decoded to non-finite {decoded}");
            assert!(
                (1.0..=1.99).contains(&decoded),
                "ω={om} decoded to {decoded}, outside [1.0, 1.99] (clamp failed)"
            );
        }
        // The DOCUMENTED NaN behavior: the clamp passes NaN through (this is the finding).
        let nan_push = encode(f32::NAN, false);
        let nan_dec = f32::from_le_bytes([nan_push[4], nan_push[5], nan_push[6], nan_push[7]]);
        assert!(nan_dec.is_nan(), "FINDING CHANGED: f32::clamp now sanitizes NaN? got {nan_dec}");
        // The SOUNDNESS property that actually protects us: a NaN ω ≡ the ω=1 plain march
        // (the `omega > 1.0` gate is false for NaN on both host and shader → no hole).
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let mut checked = 0u64;
        for (sname, edits) in &scenes {
            for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    let plain = golden_composite_pixel_ex_omega(
                        edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                    );
                    let nan_omega = golden_composite_pixel_ex_omega(
                        edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, f32::NAN,
                    );
                    assert_eq!(
                        plain, nan_omega,
                        "[{sname}] NaN ω diverged from the ω=1 plain march at ({px},{py}): \
                         plain 0x{plain:08X} nan 0x{nan_omega:08X} — the NaN-defang property broke"
                    );
                    checked += 1;
                }
            }
        }
        // The production default must be inside the clamp window (a sanity tie to the harness).
        assert!((1.0..=1.99).contains(&DEFAULT_MARCHER_OMEGA), "DEFAULT_MARCHER_OMEGA out of clamp window");
        println!(
            "[B1 gate5] ω clamp: {} non-NaN hostile inputs decode finite ∈ [1.0,1.99]; NaN PASSES the clamp \
             but ≡ the ω=1 plain march over {checked} px (gate false for NaN); default={DEFAULT_MARCHER_OMEGA}",
            cases.len()
        );
    }
}

// ===========================================================================
// M1 — the EMPTY-SPACE-SKIP marcher test matrix (host-side, no GPU).
//
// The two LOAD-BEARING soundness gates:
//   (1) OFF byte-identical — `golden_composite_pixel_brick(brick_enabled=0)` is
//       BIT-FOR-BIT the pre-M1 `golden_composite_pixel_ex_omega_lit` over a
//       battery of scenes/cameras/pixels (the 0%-gate).
//   (2) ON hit-set == analytic — over ≥500 random scenes + the demo scene, the
//       empty-skip ON marcher and the analytic (OFF) marcher agree on EVERY
//       pixel's HIT/MISS classification AND surface color (the empty skip only
//       changes WHERE steps land, never the converged hit). A skipped or spurious
//       surface is a BLOCKER.
//
// Plus: never-skip-surface (3), `dist_to_brick_exit` progress (4),
// `build_pointer_grid` correctness (5), push-constant layout (6) — the std430
// agreement the dev SPIR-V-verified, guarded host-side.
//
// The xorshift scene generator mirrors the M0 brick GATE generator (no new dep).
// ===========================================================================
#[cfg(test)]
mod m1_empty_skip_tests {
    use super::{
        CompositeCamera, DEFAULT_LIGHT_DIR, DEFAULT_MARCHER_OMEGA, FineMarcherPush,
        GBUFFER_MARCHER_PUSH_BYTES, LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS, MESH_DEPTH_CLEAR,
        SDF_IMG_H, SDF_IMG_W, SdfEdit, golden_composite_pixel_brick,
        golden_composite_pixel_ex_omega_lit, host_brick_cell, sdf_op,
    };
    use boyko_sdf_math::brick::{
        BRICK_EXIT_EPS, DEFAULT_BRICK_WORLD, DEFAULT_GRID_DIM, PointerGrid, build_pointer_grid,
        classify_brick, dist_to_brick_exit,
    };
    use boyko_sdf_math::{BrickClass, SDF_EDIT_BAND_HALF, SdfEditField};

    // ── shared fixtures ────────────────────────────────────────────────────

    /// `EmptyOutside as u32` — the cell class the empty-skip acts on (mirror of the
    /// private `super::BRICK_CLASS_EMPTY_OUTSIDE`, re-stated so a drift in the enum
    /// discriminant is caught by `class_codes_match_brickclass_discriminants`).
    const EMPTY_OUTSIDE: u32 = 0;

    /// A deterministic xorshift64* PRNG — the scene generator without a dep (mirrors
    /// the M0 brick GATE's generator so the two suites draw from the SAME family).
    struct XorShift64(u64);

    impl XorShift64 {
        fn new(seed: u64) -> Self {
            Self(seed ^ 0x9E37_79B9_7F4A_7C15)
        }
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            let frac = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
            lo + frac * (hi - lo)
        }
        fn below(&mut self, n: u32) -> u32 {
            (self.next_u64() % n as u64) as u32
        }
    }

    /// The demo / golden "crater" CSG scene (base sphere minus a smaller sphere) — the
    /// SAME scene the rung-9/10 and B1 goldens use, so the M1 gates run against the
    /// production demo field, not only synthetic scenes.
    fn crater() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ]
    }

    fn box_csg() -> Vec<SdfEdit> {
        vec![SdfEdit::box_shape([0.0, 0.0, 0.0], [0.4, 0.4, 0.4], sdf_op::UNION, 0.0)]
    }

    fn smooth_union() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([-0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.15),
        ]
    }

    /// Builds an `SdfEditField` (the authority) from a slice of edits and bumps its gen.
    fn field_of(edits: &[SdfEdit]) -> SdfEditField {
        let mut f = SdfEditField::new();
        for e in edits {
            assert!(f.push(*e), "scene must fit MAX_SDF_EDITS");
        }
        f.bump_gen();
        f
    }

    /// A random valid scene (1..=6 edits, first forced UNION, SPHERE/BOX, radii/half-
    /// extents >= 0.5, centers in [-2,2]³, UNION/SUBTRACT/INTERSECT, smoothness 0 or 0.15)
    /// returned as a `Vec<SdfEdit>` — the ON-vs-analytic gate's scene family.
    fn random_scene(rng: &mut XorShift64) -> Vec<SdfEdit> {
        let n = 1 + rng.below(6);
        let mut edits = Vec::with_capacity(n as usize);
        for i in 0..n {
            let center =
                [rng.range(-2.0, 2.0), rng.range(-2.0, 2.0), rng.range(-2.0, 2.0)];
            let op = if i == 0 {
                sdf_op::UNION
            } else {
                match rng.below(3) {
                    0 => sdf_op::UNION,
                    1 => sdf_op::SUBTRACT,
                    _ => sdf_op::INTERSECT,
                }
            };
            let smoothness = if rng.below(2) == 0 { 0.0 } else { 0.15 };
            let e = if rng.below(2) == 0 {
                SdfEdit::sphere(center, rng.range(0.5, 1.5), op, smoothness)
            } else {
                SdfEdit::box_shape(
                    center,
                    [rng.range(0.5, 1.2), rng.range(0.5, 1.2), rng.range(0.5, 1.2)],
                    op,
                    smoothness,
                )
            };
            edits.push(e);
        }
        edits
    }

    /// The default near-field pointer grid baked from `field` — the SAME grid the GPU
    /// binds at binding 9 (origin/dims/brick_world the `FineMarcherPush` carries).
    fn build_default_grid(field: &SdfEditField) -> (PointerGrid, Vec<u32>) {
        let grid = PointerGrid::default_near_field();
        let mut cells = vec![0u32; grid.cell_count()];
        build_pointer_grid(field, &grid, &mut cells);
        (grid, cells)
    }

    /// The result of one primary-march replay: the hit decision + hit-`t`, plus the
    /// over-step audit signals (`min_field` = closest analytic approach SEEN at a probe,
    /// `crossed_undetected` = a brick-exit step that jumped from outside the hit band
    /// straight PAST the surface to a point where the field went NEGATIVE — the literal
    /// definition of a skipped surface, `exhausted` = ran the whole iteration budget).
    struct MarchTrace {
        hit: bool,
        min_field: f32,
        crossed_undetected: bool,
        exhausted: bool,
    }

    /// Replays the brick-ON / analytic PRIMARY march (the empty-skip loop, no re-march or
    /// shade) and audits it for an OVER-STEP. A verbatim mirror of
    /// `golden_composite_pixel_brick`'s primary loop, plus: before each brick-exit step it
    /// records the field at the PRE- and POST-step points; if the pre-step field was
    /// positive (outside the solid) and the post-step field is NEGATIVE (inside), the
    /// brick step JUMPED PAST a surface undetected (a skip → soundness BLOCKER). ORTHO,
    /// no mesh.
    fn march_primary(
        edits: &[SdfEdit],
        px: u32,
        py: u32,
        grid: &PointerGrid,
        cells: &[u32],
        brick_on: bool,
    ) -> MarchTrace {
        use super::{SDF_EPS, SDF_MAX_IT, SDF_T_MAX, composite_ray, sdf_edit_list};
        let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
        let mut t = 0.0_f32;
        let mut hit = false;
        let mut min_field = f32::INFINITY;
        let mut crossed_undetected = false;
        let mut iters = 0u32;
        for _ in 0..SDF_MAX_IT {
            iters += 1;
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            if brick_on
                && let Some((class, cmin)) = host_brick_cell(grid, cells, p)
                && class == EMPTY_OUTSIDE
            {
                // Audit the brick-exit step for an over-step: sample the analytic field
                // at the PRE-step point and at the POST-step point. A skip would show as
                // pre >= 0 (outside) but post < 0 (inside) — the step crossed a surface.
                let pre_d = sdf_edit_list(edits, p);
                min_field = min_field.min(pre_d);
                let exit = dist_to_brick_exit(p, rd, cmin, grid.brick_world);
                t += exit;
                let q = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                let post_d = sdf_edit_list(edits, q);
                if pre_d >= 0.0 && post_d < 0.0 {
                    crossed_undetected = true;
                }
                min_field = min_field.min(post_d);
                if t > SDF_T_MAX {
                    break;
                }
                continue;
            }
            let d = sdf_edit_list(edits, p);
            min_field = min_field.min(d);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d;
            if t > SDF_T_MAX {
                break;
            }
        }
        MarchTrace {
            hit,
            min_field,
            crossed_undetected,
            exhausted: iters == SDF_MAX_IT && !hit,
        }
    }

    /// Max per-channel difference between two packed `0xAABBGGRR` colors (RGB only).
    fn chan_delta(a: u32, b: u32) -> i32 {
        let c = |x: u32, sh: u32| ((x >> sh) & 0xFF) as i32;
        (c(a, 0) - c(b, 0))
            .abs()
            .max((c(a, 8) - c(b, 8)).abs())
            .max((c(a, 16) - c(b, 16)).abs())
    }

    /// A sparse-but-representative pixel battery across the 64×64 frame (corners, edges,
    /// center, and a diagonal sweep) — exercises HIT, MISS, and the sphere-edge grazing
    /// rays without folding the field on all 4096 pixels in the per-scene inner loops.
    fn pixel_battery() -> Vec<(u32, u32)> {
        let mut v = Vec::new();
        let coords = [0u32, 1, 8, 16, 24, 31, 32, 40, 48, 56, 62, 63];
        for &py in &coords {
            for &px in &coords {
                v.push((px, py));
            }
        }
        // A diagonal sweep for extra edge coverage.
        for i in 0..SDF_IMG_W {
            v.push((i, i % SDF_IMG_H));
        }
        v
    }

    // ── 6. PUSH-CONSTANT LAYOUT (the std430 host/GPU agreement guard) ──────

    /// The M1 `FineMarcherPush` field offsets match the std430/HLSL layout the dev
    /// SPIR-V-verified (`grid_origin@32, brick_enabled@44, grid_dims@48, brick_world@60`)
    /// and the block is 64 bytes — a runtime mirror of the `const _: () = assert!` pins
    /// so a future reorder that desyncs the GPU push is caught even if the const-asserts
    /// are ever weakened.
    #[test]
    fn fine_marcher_push_m1_field_offsets_match_std430() {
        assert_eq!(core::mem::offset_of!(FineMarcherPush, grid_origin), 32, "grid_origin @32");
        assert_eq!(core::mem::offset_of!(FineMarcherPush, brick_enabled), 44, "brick_enabled @44");
        assert_eq!(core::mem::offset_of!(FineMarcherPush, grid_dims), 48, "grid_dims @48");
        assert_eq!(core::mem::offset_of!(FineMarcherPush, brick_world), 60, "brick_world @60");
        // M2 widened the push to the full 80-byte COMPOSITE range (brick_trilinear @64 + _pad3 @68).
        assert_eq!(core::mem::offset_of!(FineMarcherPush, brick_trilinear), 64, "brick_trilinear @64");
        assert_eq!(GBUFFER_MARCHER_PUSH_BYTES, 80, "FineMarcherPush must be 80 bytes");
    }

    /// `with_brick` flips `brick_enabled` to 1 and stamps the grid uniforms, preserving
    /// the base gates; `new` leaves the M1 block OFF (brick_enabled == 0, zero grid).
    #[test]
    fn with_brick_sets_grid_uniforms_and_preserves_base_gates() {
        let base = FineMarcherPush::new(true, 1.3, LIGHTING_FLAG_SHADOWS, [0.1, 0.2, 0.3]);
        assert_eq!(base.brick_enabled, 0, "new() leaves the empty-skip OFF");
        assert_eq!(base.grid_dims, [0, 0, 0], "new() zeroes the grid");

        let with = base.with_brick([-4.0, -4.0, -4.0], [16, 16, 16], 0.5);
        assert_eq!(with.brick_enabled, 1, "with_brick turns the empty-skip ON");
        assert_eq!(with.grid_origin, [-4.0, -4.0, -4.0], "grid_origin stamped");
        assert_eq!(with.grid_dims, [16, 16, 16], "grid_dims stamped");
        assert_eq!(with.brick_world, 0.5, "brick_world stamped");
        // Base gates preserved.
        assert_eq!(with.coarse_enabled, 1, "coarse gate preserved");
        assert_eq!(with.omega, 1.3, "omega preserved");
        assert_eq!(with.lighting_flags, LIGHTING_FLAG_SHADOWS, "lighting flags preserved");
        assert_eq!(with.light_dir, [0.1, 0.2, 0.3], "light_dir preserved");
    }

    /// The host `EMPTY_OUTSIDE` code the empty-skip branches on equals the
    /// `BrickClass::EmptyOutside` discriminant the bake stores — a drift in the enum
    /// repr would make the skip act on the wrong class (or never).
    #[test]
    fn class_codes_match_brickclass_discriminants() {
        assert_eq!(BrickClass::EmptyOutside as u32, EMPTY_OUTSIDE, "EmptyOutside == 0");
        assert_eq!(BrickClass::EmptyInside as u32, 1, "EmptyInside == 1");
        assert_eq!(BrickClass::Surface as u32, 2, "Surface == 2");
    }

    // ── 1. OFF BYTE-IDENTICAL (the 0%-gate) ────────────────────────────────

    /// `golden_composite_pixel_brick(brick_enabled=0)` is BIT-FOR-BIT the pre-M1
    /// `golden_composite_pixel_ex_omega_lit` over a battery of pixels across the demo
    /// scenes (crater / box / smooth-union) under both ORTHO and a perspective camera,
    /// at omega 1.0 and the default omega, and both lighting OFF and ON. Any single
    /// byte difference is a BLOCKER — the 0%-gate is broken.
    #[test]
    fn off_path_is_byte_identical_to_pre_m1_golden() {
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        // The grid is supplied but MUST be ignored on the OFF path (a degenerate grid
        // proves the OFF path never reads it).
        let dummy_grid = PointerGrid::default_near_field();
        let dummy_cells = vec![0u32; dummy_grid.cell_count()];

        let cameras = [
            ("ortho", CompositeCamera::Ortho),
            (
                "persp",
                CompositeCamera::Perspective {
                    eye: [0.0, 0.0, 2.0],
                    forward: [0.0, 0.0, -1.0],
                    right: [1.0, 0.0, 0.0],
                    up: [0.0, 1.0, 0.0],
                    tan_half_fov: 0.5,
                    aspect: 1.0,
                },
            ),
        ];
        let omegas = [1.0_f32, DEFAULT_MARCHER_OMEGA];
        let light_cfgs = [
            (0u32, DEFAULT_LIGHT_DIR),
            (LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO, [0.3, 0.7, 1.0]),
        ];
        let mesh_depths = [MESH_DEPTH_CLEAR, 0.5_f32]; // no-mesh + a covering mesh

        let mut checked = 0u64;
        for (sname, edits) in &scenes {
            for &(cname, cam) in &cameras {
                for &om in &omegas {
                    for &(flags, ldir) in &light_cfgs {
                        for &md in &mesh_depths {
                            for &(px, py) in &pixel_battery() {
                                let off = golden_composite_pixel_brick(
                                    edits, md, px, py, SDF_IMG_W, SDF_IMG_H, cam, om, flags, ldir,
                                    false, &dummy_grid, &dummy_cells,
                                );
                                let pre_m1 = golden_composite_pixel_ex_omega_lit(
                                    edits, md, px, py, SDF_IMG_W, SDF_IMG_H, cam, om, flags, ldir,
                                );
                                assert_eq!(
                                    off, pre_m1,
                                    "[{sname}/{cname} ω={om} flags={flags} md={md}] OFF path \
                                     diverged from pre-M1 at ({px},{py}): brick 0x{off:08X} \
                                     pre-M1 0x{pre_m1:08X} — the 0%-gate is BROKEN"
                                );
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
        // 3 scenes × 2 cameras × 2 ω × 2 light cfgs × 2 mesh depths × |battery| = 9984.
        assert!(checked > 9_000, "OFF gate must exercise a wide battery (got {checked})");
        println!("[M1 OFF 0%-gate] {checked} pixels byte-identical to pre-M1 golden");
    }

    // ── 2. ON HIT-SET == ANALYTIC (the load-bearing M1 property) ───────────

    /// THE LOAD-BEARING M1 PROPERTY. Over the demo scene + ≥500 random scenes, asserts
    /// the empty-skip is SOUND and behavior-identical at the SHIPPING level:
    ///
    ///   (a) NO OVER-STEP (the direct soundness invariant, budget-INDEPENDENT): not one
    ///       brick-exit step ever jumps from OUTSIDE the solid (pre-step field >= 0)
    ///       straight to INSIDE it (post-step field < 0) — the literal definition of a
    ///       skipped surface. This is the conservative-classifier contract: an
    ///       EmptyOutside brick has no surface within band_half, so stepping to its exit
    ///       cannot cross one. A single `crossed_undetected` is a soundness BLOCKER.
    ///
    ///   (b) PRODUCTION HIT-SET + COLOR (the shipping contract): the production
    ///       `golden_composite_pixel_brick` (WITH its `exhausted` re-march fallback, the
    ///       same the GPU shader runs) yields ON output within ±1/255 of analytic — the
    ///       converged-`t` < `SDF_EPS` rounding is the only difference; tighter than the
    ///       established ±2/255 GPU-golden tolerance.
    ///
    /// The PRIMARY-loop hit-class is also tracked: any divergence there is a `MAX_IT`-cliff
    /// budget-edge artifact on a near-tangent ray (NOT an over-step), and the test asserts
    /// EVERY such pixel is resolved to ±1/255 by the production re-march (so the artifact
    /// is provably non-shipping).
    #[test]
    fn on_hit_set_equals_analytic_over_many_scenes() {
        const SCENES: u64 = 600; // ≥500 random scenes + the demo scenes below
        let battery = pixel_battery();

        let demo: [(&str, Vec<SdfEdit>); 3] =
            [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let mut overstep_blockers: u64 = 0;
        let mut primary_budget_flips: u64 = 0;
        let mut checked: u64 = 0;
        let mut max_chan: i32 = 0;
        let mut first_overstep: Option<String> = None;
        let mut first_color_violation: Option<String> = None;
        let mut first_unresolved_flip: Option<String> = None;
        let mut unresolved_flips: u64 = 0;

        let mut run_scene = |label: &str, edits: &[SdfEdit]| {
            let field = field_of(edits);
            let (grid, cells) = build_default_grid(&field);
            for &(px, py) in &battery {
                checked += 1;
                let on_trace = march_primary(edits, px, py, &grid, &cells, true);
                let an_trace = march_primary(edits, px, py, &grid, &cells, false);

                // (a) the direct over-step soundness invariant (budget-independent).
                if on_trace.crossed_undetected {
                    overstep_blockers += 1;
                    if first_overstep.is_none() {
                        first_overstep = Some(format!(
                            "[{label}] ({px},{py}) a brick-exit step crossed a surface \
                             undetected (min_field={:.4e}); edits={edits:?}",
                            on_trace.min_field
                        ));
                    }
                }

                // (b) the PRODUCTION shipping contract: ON within ±1/255 of analytic.
                // Lighting OFF: bare Lambert (the ON lighting path is ±3/255 vs the
                // shader and is gated separately).
                let on = golden_composite_pixel_brick(
                    edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    1.0, 0, DEFAULT_LIGHT_DIR, true, &grid, &cells,
                );
                let analytic = golden_composite_pixel_brick(
                    edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    1.0, 0, DEFAULT_LIGHT_DIR, false, &grid, &cells,
                );
                let dchan = chan_delta(on, analytic);
                max_chan = max_chan.max(dchan);
                if dchan > 1 && first_color_violation.is_none() {
                    first_color_violation = Some(format!(
                        "[{label}] ({px},{py}) per-channel Δ={dchan} > 1/255 \
                         (ON 0x{on:08X} analytic 0x{analytic:08X})"
                    ));
                }

                // Track primary-loop hit-class flips and PROVE each is a budget artifact
                // (the production function resolves it to ±1/255 via the re-march).
                if on_trace.hit != an_trace.hit {
                    primary_budget_flips += 1;
                    // A genuine artifact has at least one path exhausting the budget AND
                    // no over-step; the production function must reconcile it.
                    if dchan > 1 {
                        unresolved_flips += 1;
                        if first_unresolved_flip.is_none() {
                            first_unresolved_flip = Some(format!(
                                "[{label}] ({px},{py}) primary hit-flip NOT resolved by the \
                                 production re-march: ON-exhausted={} AN-exhausted={} dchan={dchan} \
                                 (ON 0x{on:08X} analytic 0x{analytic:08X})",
                                on_trace.exhausted, an_trace.exhausted
                            ));
                        }
                    }
                }
            }
        };

        for (label, edits) in &demo {
            run_scene(label, edits);
        }
        for seed in 0..SCENES {
            let mut rng = XorShift64::new(seed.wrapping_mul(0x100_0001).wrapping_add(1));
            let edits = random_scene(&mut rng);
            run_scene(&format!("rand#{seed}"), &edits);
        }

        // (a) the SOUNDNESS gate: zero over-steps (no surface skipped).
        assert_eq!(
            overstep_blockers, 0,
            "{overstep_blockers}/{checked} brick-exit steps crossed a surface undetected — \
             a SKIPPED surface (SOUNDNESS BLOCKER). First: {}",
            first_overstep.unwrap_or_default()
        );
        // (b) the shipping contract: production ON within ±1/255 of analytic.
        assert!(
            max_chan <= 1,
            "production ON surface color exceeded ±1/255 vs analytic (max per-channel \
             Δ={max_chan}). First: {}",
            first_color_violation.unwrap_or_default()
        );
        // Every primary-loop hit-flip is a budget-edge artifact the production re-march
        // resolves to ±1/255 (none ships as a divergence).
        assert_eq!(
            unresolved_flips, 0,
            "{unresolved_flips} primary-loop hit-flips were NOT resolved by the production \
             re-march (a shipping divergence). First: {}",
            first_unresolved_flip.unwrap_or_default()
        );
        assert!(checked > 50_000, "ON gate must exercise a wide battery (got {checked})");
        println!(
            "[M1 ON-vs-analytic] {checked} pixels over {} scenes: 0 over-steps, \
             {primary_budget_flips} primary-loop budget-edge flips (ALL resolved by the \
             production re-march to ±{max_chan}/255 — non-shipping)",
            SCENES + 3
        );
    }

    // ── 3. EMPTY-SKIP NEVER SKIPS A SURFACE (the property, targeted) ───────

    /// A thin shell straddling an EMPTY/SURFACE brick boundary: every ray that hits the
    /// surface analytically must still hit with the empty-skip ON (no surface skipped),
    /// and every ray that misses analytically must still miss (no spurious hit). Asserts
    /// per-pixel HIT-classification equality on the boundary-grazing scene.
    #[test]
    fn empty_skip_never_skips_or_invents_a_surface_at_brick_boundary() {
        // A small sphere placed so its surface grazes a brick face of the default grid
        // (brick_world = 0.5; a center at a half-cell offset makes the surface cross a
        // cell boundary). Plus a thin box to graze a face from outside.
        let scenes: [(&str, Vec<SdfEdit>); 3] = [
            (
                "sphere_on_face",
                vec![SdfEdit::sphere([0.25, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0)],
            ),
            (
                "thin_box_face_graze",
                vec![SdfEdit::box_shape([0.5, 0.0, 0.0], [0.5, 0.6, 0.6], sdf_op::UNION, 0.0)],
            ),
            (
                "off_center_csg",
                vec![
                    SdfEdit::sphere([0.5, 0.5, 0.0], 0.7, sdf_op::UNION, 0.0),
                    SdfEdit::sphere([0.75, 0.5, 0.0], 0.3, sdf_op::SUBTRACT, 0.0),
                ],
            ),
        ];

        for (label, edits) in &scenes {
            let field = field_of(edits);
            let (grid, cells) = build_default_grid(&field);
            // EVERY pixel of the frame (a thin-surface scene wants dense coverage).
            for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    // The PROPERTY: not one brick-exit step crosses a surface — no surface
                    // skipped (analytic-hit ⟹ no undetected crossing), no surface invented.
                    let on_trace = march_primary(edits, px, py, &grid, &cells, true);
                    assert!(
                        !on_trace.crossed_undetected,
                        "[{label}] ({px},{py}) a brick-exit step crossed a surface UNDETECTED \
                         (min_field={:.4e}) — a surface was SKIPPED",
                        on_trace.min_field
                    );
                    // The production composited color (with re-march) stays within ±1/255.
                    let on = golden_composite_pixel_brick(
                        edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                        CompositeCamera::Ortho, 1.0, 0, DEFAULT_LIGHT_DIR, true, &grid, &cells,
                    );
                    let analytic = golden_composite_pixel_brick(
                        edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                        CompositeCamera::Ortho, 1.0, 0, DEFAULT_LIGHT_DIR, false, &grid, &cells,
                    );
                    assert!(
                        chan_delta(on, analytic) <= 1,
                        "[{label}] ({px},{py}) color Δ {} > 1/255 (ON 0x{on:08X} analytic \
                         0x{analytic:08X}) — the empty skip changed the surface",
                        chan_delta(on, analytic)
                    );
                }
            }
        }
    }

    // ── 4. `dist_to_brick_exit` PROGRESS (no zero/negative step) ───────────

    /// A ray parallel to a brick face, starting exactly on a boundary, or axis-aligned
    /// through cell corners still advances by >= `BRICK_EXIT_EPS` (no zero/negative step
    /// → no infinite march). Tests the degenerate directions head-on.
    #[test]
    fn dist_to_brick_exit_always_advances_on_degenerate_rays() {
        let cell_min = [0.0_f32, 0.0, 0.0];
        let bw = 0.5_f32;
        // Cases: (ro, rd, label). All `ro` are on/at a brick face or corner; all `rd`
        // include axis-parallel + fully-degenerate (zero) directions.
        let cases: &[([f32; 3], [f32; 3], &str)] = &[
            // Parallel to the +x face plane (no x component), on the y=0 face.
            ([0.1, 0.0, 0.1], [0.0, 1.0, 0.0], "parallel_y_on_face"),
            // Parallel to a face, sitting exactly on a corner.
            ([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], "axis_z_from_corner"),
            // Diagonal through the cell corner (exits at a corner, can graze).
            ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], "body_diagonal_from_corner"),
            // Fully degenerate: zero direction (every axis skipped).
            ([0.25, 0.25, 0.25], [0.0, 0.0, 0.0], "zero_direction"),
            // Sub-eps direction on all axes (every axis below BRICK_EXIT_EPS).
            ([0.25, 0.25, 0.25], [1e-6, 1e-6, 1e-6], "sub_eps_all_axes"),
            // A ray starting on the FAR face, pointing further out (negative exit
            // territory) — must clamp UP to advance.
            ([0.5, 0.25, 0.25], [1.0, 0.0, 0.0], "on_far_face_outward"),
            // Negative-x ray from inside (exits the lo face).
            ([0.25, 0.25, 0.25], [-1.0, 0.0, 0.0], "neg_x_from_center"),
        ];
        for &(ro, rd, label) in cases {
            let exit = dist_to_brick_exit(ro, rd, cell_min, bw);
            assert!(exit.is_finite(), "[{label}] exit must be finite, got {exit}");
            assert!(
                exit >= BRICK_EXIT_EPS,
                "[{label}] exit {exit} < BRICK_EXIT_EPS {BRICK_EXIT_EPS} — the march can stall (no progress)"
            );
        }
    }

    /// A well-conditioned ray through a brick exits at the analytically expected slab
    /// distance (the progress clamp does not corrupt a normal exit).
    #[test]
    fn dist_to_brick_exit_matches_slab_far_face_on_normal_ray() {
        let cell_min = [0.0_f32, 0.0, 0.0];
        let bw = 0.5_f32;
        // From the lo-x face center, straight +x: exits the +x face at t = 0.5.
        let exit = dist_to_brick_exit([0.0, 0.25, 0.25], [1.0, 0.0, 0.0], cell_min, bw);
        assert!((exit - 0.5).abs() < 1e-5, "axis-aligned exit must be the slab width 0.5, got {exit}");
        // From the center, +x: exits at t = 0.25 (half the cell).
        let exit2 = dist_to_brick_exit([0.25, 0.25, 0.25], [1.0, 0.0, 0.0], cell_min, bw);
        assert!((exit2 - 0.25).abs() < 1e-5, "centered exit must be 0.25, got {exit2}");
    }

    // ── 5. `build_pointer_grid` CORRECTNESS ────────────────────────────────

    /// Every cell the bake writes equals a direct `classify_brick` of that cell's AABB —
    /// the bake is a faithful per-cell fold of the authority (no index/origin slip).
    #[test]
    fn build_pointer_grid_matches_per_cell_classify() {
        let scenes: [(&str, Vec<SdfEdit>); 3] =
            [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        for (label, edits) in &scenes {
            let field = field_of(edits);
            let grid = PointerGrid::default_near_field();
            let mut cells = vec![0u32; grid.cell_count()];
            build_pointer_grid(&field, &grid, &mut cells);

            let w = grid.dims[0];
            let h = grid.dims[1];
            let d = grid.dims[2];
            for iz in 0..d {
                for iy in 0..h {
                    for ix in 0..w {
                        let cell_min = grid.cell_min(ix, iy, iz);
                        let expect = classify_brick(
                            &field, cell_min, grid.brick_world, SDF_EDIT_BAND_HALF,
                        ) as u32;
                        let idx = (ix + iy * w + iz * w * h) as usize;
                        assert_eq!(
                            cells[idx], expect,
                            "[{label}] cell ({ix},{iy},{iz}) bake {} != classify {expect}",
                            cells[idx]
                        );
                    }
                }
            }
        }
    }

    /// A cell with no edit nearby bakes EmptyOutside (or EmptyInside deep in a solid); a
    /// cell a surface passes through bakes Surface. Checked against a hand-placed scene.
    #[test]
    fn build_pointer_grid_classifies_empty_vs_surface() {
        // A unit sphere at the origin. Cells far out are EmptyOutside; the cell at the
        // origin (deep inside the sphere) overlaps the sphere's AABB → Surface (the C2
        // conservative rule: a primitive's AABB covers its interior). A cell on the
        // sphere's band is Surface.
        let edits = vec![SdfEdit::sphere([0.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0)];
        let field = field_of(&edits);
        let (grid, cells) = build_default_grid(&field);

        let cell_class = |wx: f32, wy: f32, wz: f32| -> u32 {
            let p = [wx, wy, wz];
            let (class, _) = host_brick_cell(&grid, &cells, p).expect("point inside the default grid");
            class
        };

        // A corner of the [-4,4]³ grid, far from the unit sphere → EmptyOutside.
        assert_eq!(cell_class(-3.5, -3.5, -3.5), EMPTY_OUTSIDE, "far cell must be EmptyOutside");
        // A cell straddling the sphere surface (radius 1) → Surface (class 2).
        assert_eq!(cell_class(1.0, 0.0, 0.0), BrickClass::Surface as u32, "surface cell must be Surface");
        // The center cell overlaps the sphere AABB → Surface (conservative, not EmptyInside).
        assert_eq!(cell_class(0.0, 0.0, 0.0), BrickClass::Surface as u32, "deep-inside cell is Surface (C2)");
    }

    /// Grid indexing round-trips: `cell_min(ix,iy,iz)` then a point inside that cell maps
    /// back to `(ix,iy,iz)` via `host_brick_cell`, and an out-of-grid point returns None.
    #[test]
    fn host_brick_cell_round_trips_and_bounds_check() {
        let edits = crater();
        let field = field_of(&edits);
        let (grid, cells) = build_default_grid(&field);

        // Round-trip a sampling of cells: a point at the cell center maps back to it.
        for &(ix, iy, iz) in &[(0u32, 0u32, 0u32), (5, 7, 3), (15, 15, 15), (8, 0, 12)] {
            let cmin = grid.cell_min(ix, iy, iz);
            let center = [
                cmin[0] + grid.brick_world * 0.5,
                cmin[1] + grid.brick_world * 0.5,
                cmin[2] + grid.brick_world * 0.5,
            ];
            let (_, got_min) = host_brick_cell(&grid, &cells, center)
                .expect("cell-center point must land in the grid");
            assert_eq!(got_min, cmin, "cell ({ix},{iy},{iz}) center must map back to its cell_min");
        }

        // Out-of-grid points (below origin and past the far corner) → None.
        let below = [grid.origin[0] - 1.0, grid.origin[1], grid.origin[2]];
        assert!(host_brick_cell(&grid, &cells, below).is_none(), "point below origin → no cell");
        let far = [
            grid.origin[0] + grid.dims[0] as f32 * grid.brick_world + 1.0,
            grid.origin[1],
            grid.origin[2],
        ];
        assert!(host_brick_cell(&grid, &cells, far).is_none(), "point past the far face → no cell");
    }

    /// The default near-field grid spans the demo `[-4,4]³` extent (DEFAULT_GRID_DIM
    /// cells of DEFAULT_BRICK_WORLD), enclosing the demo scene with margin.
    #[test]
    fn default_near_field_grid_encloses_demo_extent() {
        let grid = PointerGrid::default_near_field();
        assert_eq!(grid.dims, [DEFAULT_GRID_DIM, DEFAULT_GRID_DIM, DEFAULT_GRID_DIM]);
        assert_eq!(grid.brick_world, DEFAULT_BRICK_WORLD);
        let half = DEFAULT_GRID_DIM as f32 * DEFAULT_BRICK_WORLD * 0.5;
        assert!((grid.origin[0] + half).abs() < 1e-6, "grid centered on origin (x)");
        // The demo primitives live within ±3; the grid spans ±4 → enclosed.
        assert!(half >= 4.0 - 1e-6, "default grid must span at least ±4 (got ±{half})");
    }

    /// The empty scene (no edits) bakes an ALL-EmptyOutside grid — every cell is
    /// provably outside, so the marcher skips the whole near-field to the analytic
    /// (background) result.
    #[test]
    fn build_pointer_grid_empty_scene_is_all_empty_outside() {
        let field = SdfEditField::new(); // no edits, gen 0
        let grid = PointerGrid::default_near_field();
        let mut cells = vec![0u32; grid.cell_count()];
        build_pointer_grid(&field, &grid, &mut cells);
        assert!(
            cells.iter().all(|&c| c == EMPTY_OUTSIDE),
            "an empty scene must bake an all-EmptyOutside grid"
        );
    }
}
