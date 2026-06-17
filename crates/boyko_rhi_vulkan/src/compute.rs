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

use crate::ffi::VkResult;
use crate::memory::MemoryError;

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

/// SDF image width (pixels) — matches the shader's `IMG_W`.
pub const SDF_IMG_W: u32 = 64;
/// SDF image height (pixels) — matches the shader's `IMG_H`.
pub const SDF_IMG_H: u32 = 64;

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
const SDF_GRAD_H: f32 = 0.0005;

#[inline]
fn v_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn v_len(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

#[inline]
fn v_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn v_normalize(a: [f32; 3]) -> [f32; 3] {
    let len = v_len(a);
    [a[0] / len, a[1] / len, a[2] / len]
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
