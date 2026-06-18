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
    HEADER_BASE_WORDS, MAX_SDF_EDITS, SDF_EDIT_WORDS, SDF_GRAD_H, sdf_edit_list,
    sdf_edit_list_normal, v_dot, v_len, v_normalize, v_sub,
};

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
static SDF_GBUFFER_COMPOSITE_SPV: SpirvBlob<28544> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_gbuffer_composite.comp.spv"
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
        let l = v_normalize(SDF_LIGHT_DIR);
        let ndotl = v_dot(n, l).max(0.0);
        [
            SDF_BASE_COLOR[0] * ndotl + SDF_BASE_COLOR[0] * SDF_AMBIENT,
            SDF_BASE_COLOR[1] * ndotl + SDF_BASE_COLOR[1] * SDF_AMBIENT,
            SDF_BASE_COLOR[2] * ndotl + SDF_BASE_COLOR[2] * SDF_AMBIENT,
        ]
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
        let l = v_normalize(SDF_LIGHT_DIR);
        let ndotl = v_dot(n, l).max(0.0);
        [
            SDF_BASE_COLOR[0] * ndotl + SDF_BASE_COLOR[0] * SDF_AMBIENT,
            SDF_BASE_COLOR[1] * ndotl + SDF_BASE_COLOR[1] * SDF_AMBIENT,
            SDF_BASE_COLOR[2] * ndotl + SDF_BASE_COLOR[2] * SDF_AMBIENT,
        ]
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
    let (ro, rd) = composite_ray(px, py, img_w, img_h, camera);

    let has_mesh = mesh_depth < MESH_DEPTH_CLEAR;
    // A finite march bound only when the mesh covered the pixel; otherwise a value
    // larger than any `t` the march reaches (mirrors the shader's `1e30`).
    let t_mesh = if has_mesh { depth_to_t(mesh_depth) } else { 1.0e30 };

    let mut t = 0.0_f32;
    let mut hit = false;
    for _ in 0..SDF_MAX_IT {
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

    let color = if hit && t < t_mesh {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);
        let l = v_normalize(SDF_LIGHT_DIR);
        let ndotl = v_dot(n, l).max(0.0);
        [
            SDF_BASE_COLOR[0] * ndotl + SDF_BASE_COLOR[0] * SDF_AMBIENT,
            SDF_BASE_COLOR[1] * ndotl + SDF_BASE_COLOR[1] * SDF_AMBIENT,
            SDF_BASE_COLOR[2] * ndotl + SDF_BASE_COLOR[2] * SDF_AMBIENT,
        ]
    } else if has_mesh {
        MESH_COLOR
    } else {
        SDF_BACKGROUND
    };
    pack_rgba(color)
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
    // The OFF path is byte-identical to the un-culled marcher (the 0%-gate).
    if !coarse_enabled {
        return golden_composite_pixel_ex(edits, mesh_depth, px, py, img_w, img_h, camera);
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
    let mut hit = false;
    for _ in 0..SDF_MAX_IT {
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

    let color = if hit && t < t_mesh {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let n = sdf_edit_list_normal(edits, p);
        let l = v_normalize(SDF_LIGHT_DIR);
        let ndotl = v_dot(n, l).max(0.0);
        [
            SDF_BASE_COLOR[0] * ndotl + SDF_BASE_COLOR[0] * SDF_AMBIENT,
            SDF_BASE_COLOR[1] * ndotl + SDF_BASE_COLOR[1] * SDF_AMBIENT,
            SDF_BASE_COLOR[2] * ndotl + SDF_BASE_COLOR[2] * SDF_AMBIENT,
        ]
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
