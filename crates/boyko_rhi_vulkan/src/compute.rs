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

/// The committed SPIR-V for Phase-6 rung 9 (sphere-trace an ordered SDF edit-list
/// — multi-primitive CSG — into a packed-header storage buffer,
/// `shaders/sdf_editlist.hlsl`).
static SDF_EDITLIST_SPV: SpirvBlob<24368> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_editlist.comp.spv"
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

/// SDF primitive kind discriminant. Matches the shader's `KIND_*` constants.
pub mod sdf_kind {
    /// A sphere primitive — `params.x` is the radius.
    pub const SPHERE: u32 = 0;
    /// An axis-aligned box primitive — `params.xyz` are the half-extents.
    pub const BOX: u32 = 1;
}

/// SDF boolean-op discriminant. Matches the shader's `OP_*` constants.
pub mod sdf_op {
    /// Union — `min(acc, d)` (or smooth-min when `smoothness > 0`).
    pub const UNION: u32 = 0;
    /// Subtraction — `max(acc, -d)` (or smooth-max when `smoothness > 0`).
    pub const SUBTRACT: u32 = 1;
    /// Intersection — `max(acc, d)` (or smooth-max when `smoothness > 0`).
    pub const INTERSECT: u32 = 2;
}

/// Fixed capacity of the edit-list (the §S2 ceiling, scaled for the basic slice).
/// Matches the shader's `MAX_SDF_EDITS`.
pub const MAX_SDF_EDITS: usize = 16;

/// One SDF edit: a primitive + a uniform transform (center) + size (params) + a
/// boolean op + an optional smoothness factor.
///
/// `#[repr(C, align(16))]` so the Rust layout is byte-identical to the std430
/// structured-buffer element `shaders/sdf_editlist.hlsl` reads (an
/// [`abi_guard`](crate::abi_guard)-style const-assert on offsets/size/align pins
/// the contract below). `center`/`params` are `[f32; 4]` (the std430 `float4`)
/// rather than `[f32; 3]` so the following `float4` starts at offset 16 without
/// std430 inserting padding the Rust side would have to mirror — the two layouts
/// are then trivially identical.
///
/// Layout (mirrored in the shader):
/// - offset  0: `center` `[f32; 4]` — xyz = center/position, w unused
/// - offset 16: `params` `[f32; 4]` — xyz = radius / half-extents, w unused
/// - offset 32: `kind` `u32` — [`sdf_kind`]
/// - offset 36: `op` `u32` — [`sdf_op`]
/// - offset 40: `smoothness` `f32` — 0 = hard op; > 0 = smooth-min/-max blend k
/// - offset 44: `_pad` `u32` — keeps the size a 16-byte multiple
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug)]
pub struct SdfEdit {
    /// xyz = primitive center/position; w unused.
    pub center: [f32; 4],
    /// xyz = radius (sphere) / half-extents (box); w unused.
    pub params: [f32; 4],
    /// Primitive kind ([`sdf_kind`]).
    pub kind: u32,
    /// Boolean op ([`sdf_op`]).
    pub op: u32,
    /// Smooth-blend radius (0 = hard op).
    pub smoothness: f32,
    /// Padding to a 16-byte multiple (mirrors the shader's `_pad` word).
    pub _pad: u32,
}

impl SdfEdit {
    /// A sphere edit at `center` with `radius`, combined by `op` with `smoothness`.
    #[inline]
    pub fn sphere(center: [f32; 3], radius: f32, op: u32, smoothness: f32) -> Self {
        Self {
            center: [center[0], center[1], center[2], 0.0],
            params: [radius, 0.0, 0.0, 0.0],
            kind: sdf_kind::SPHERE,
            op,
            smoothness,
            _pad: 0,
        }
    }

    /// A box edit at `center` with `half_extents`, combined by `op` with `smoothness`.
    #[inline]
    pub fn box_shape(center: [f32; 3], half_extents: [f32; 3], op: u32, smoothness: f32) -> Self {
        Self {
            center: [center[0], center[1], center[2], 0.0],
            params: [half_extents[0], half_extents[1], half_extents[2], 0.0],
            kind: sdf_kind::BOX,
            op,
            smoothness,
            _pad: 0,
        }
    }
}

// ---- std430 / repr(C) layout contract (the §3.8 compile-time fingerprint) ----
//
// A mismatch between this Rust struct and the std430 element the shader reads is
// silent GPU corruption that NEITHER the validation layer NOR a golden diff would
// localize (the buffer is the right size; the bytes are read at a shifted offset).
// These const-asserts make any drift a BUILD ERROR. They mirror the shader's
// documented offsets exactly.
const _: () = assert!(
    core::mem::size_of::<SdfEdit>() == 48,
    "SdfEdit must be 48 bytes (std430 element the shader reads)"
);
const _: () = assert!(
    core::mem::align_of::<SdfEdit>() == 16,
    "SdfEdit must be 16-byte aligned (std430 struct alignment)"
);
const _: () = assert!(
    core::mem::offset_of!(SdfEdit, center) == 0,
    "SdfEdit::center must be at offset 0"
);
const _: () = assert!(
    core::mem::offset_of!(SdfEdit, params) == 16,
    "SdfEdit::params must be at offset 16"
);
const _: () = assert!(
    core::mem::offset_of!(SdfEdit, kind) == 32,
    "SdfEdit::kind must be at offset 32"
);
const _: () = assert!(
    core::mem::offset_of!(SdfEdit, op) == 36,
    "SdfEdit::op must be at offset 36"
);
const _: () = assert!(
    core::mem::offset_of!(SdfEdit, smoothness) == 40,
    "SdfEdit::smoothness must be at offset 40"
);

/// `size_of::<SdfEdit>() / 4` — the number of `u32` words one packed edit
/// occupies. Matches the shader's `SDF_EDIT_WORDS`.
pub const SDF_EDIT_WORDS: usize = core::mem::size_of::<SdfEdit>() / 4;

/// Word offset of the edit array (word 0 is `edit_count`, padded to 16 bytes so
/// the array starts 16-byte aligned). Matches the shader's `HEADER_BASE`.
pub const HEADER_BASE_WORDS: usize = 4;

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

// ---- The edit-list field math (single source of truth, mirrors the shader) ----

const SDF_FAR: f32 = 1.0e9;

#[inline]
fn v_abs(a: [f32; 3]) -> [f32; 3] {
    [a[0].abs(), a[1].abs(), a[2].abs()]
}

#[inline]
fn v_max0(a: [f32; 3]) -> [f32; 3] {
    [a[0].max(0.0), a[1].max(0.0), a[2].max(0.0)]
}

/// `length(p - c) - r` — the analytic sphere distance (mirrors `sd_sphere`).
#[inline]
fn sd_sphere(p: [f32; 3], c: [f32; 3], r: f32) -> f32 {
    v_len(v_sub(p, c)) - r
}

/// The exact IQ box distance for an AABB centered at `c` with half-extents `h`
/// (mirrors the shader's `sd_box`).
#[inline]
fn sd_box(p: [f32; 3], c: [f32; 3], h: [f32; 3]) -> f32 {
    let q = v_sub(v_abs(v_sub(p, c)), h);
    let outside = v_len(v_max0(q));
    let inside = q[0].max(q[1].max(q[2])).min(0.0);
    outside + inside
}

/// One edit's primitive distance at `p` (mirrors the shader's `edit_distance`).
#[inline]
fn edit_distance(e: &SdfEdit, p: [f32; 3]) -> f32 {
    let center = [e.center[0], e.center[1], e.center[2]];
    if e.kind == sdf_kind::BOX {
        sd_box(p, center, [e.params[0], e.params[1], e.params[2]])
    } else {
        sd_sphere(p, center, e.params[0])
    }
}

/// Polynomial smooth-min (IQ `smin`), mirroring the shader's `smin`.
#[inline]
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let hh = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    // lerp(b, a, hh) = b + (a - b) * hh
    (b + (a - b) * hh) - k * hh * (1.0 - hh)
}

/// Polynomial smooth-max (the De Morgan dual of [`smin`]), mirroring `smax`.
#[inline]
fn smax(a: f32, b: f32, k: f32) -> f32 {
    -smin(-a, -b, k)
}

/// Combines the accumulated distance `acc` with one edit's distance `d` under
/// `op` (hard when `k <= 0`, smooth when `k > 0`), mirroring the shader's
/// `combine`.
#[inline]
fn combine(acc: f32, d: f32, op: u32, k: f32) -> f32 {
    match op {
        x if x == sdf_op::SUBTRACT => {
            if k > 0.0 {
                smax(acc, -d, k)
            } else {
                acc.max(-d)
            }
        }
        x if x == sdf_op::INTERSECT => {
            if k > 0.0 {
                smax(acc, d, k)
            } else {
                acc.max(d)
            }
        }
        // UNION (and any unknown discriminant falls back to union, matching the
        // shader's `else` branch).
        _ => {
            if k > 0.0 {
                smin(acc, d, k)
            } else {
                acc.min(d)
            }
        }
    }
}

/// Evaluates the ordered edit-list field at `p` (the CSG result), folding the
/// edits in order exactly as the shader's `sdf` does. The first edit seeds the
/// accumulator hard; each later edit combines under its own op.
///
/// This is the single source of truth a future CPU physics evaluator reuses;
/// `edits.len()` is clamped to [`MAX_SDF_EDITS`] to match the shader's `min`.
pub fn sdf_edit_list(edits: &[SdfEdit], p: [f32; 3]) -> f32 {
    let n = edits.len().min(MAX_SDF_EDITS);
    let mut acc = SDF_FAR;
    for (i, e) in edits.iter().take(n).enumerate() {
        let d = edit_distance(e, p);
        if i == 0 {
            acc = d;
        } else {
            acc = combine(acc, d, e.op, e.smoothness);
        }
    }
    acc
}

/// Surface normal via central differences of [`sdf_edit_list`] (the gradient of
/// the WHOLE edit-list field), mirroring the shader's `sdf_normal`.
#[inline]
fn sdf_edit_list_normal(edits: &[SdfEdit], p: [f32; 3]) -> [f32; 3] {
    let h = SDF_GRAD_H;
    let n = [
        sdf_edit_list(edits, [p[0] + h, p[1], p[2]]) - sdf_edit_list(edits, [p[0] - h, p[1], p[2]]),
        sdf_edit_list(edits, [p[0], p[1] + h, p[2]]) - sdf_edit_list(edits, [p[0], p[1] - h, p[2]]),
        sdf_edit_list(edits, [p[0], p[1], p[2] + h]) - sdf_edit_list(edits, [p[0], p[1], p[2] - h]),
    ];
    v_normalize(n)
}

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
