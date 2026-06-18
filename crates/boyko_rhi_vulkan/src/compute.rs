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
