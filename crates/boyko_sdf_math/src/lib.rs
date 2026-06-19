//! `boyko_sdf_math` — the analytic SDF edit-list field math + std430 data model,
//! extracted as a `#![no_std]` leaf so it can be the SINGLE source of truth shared
//! by two consumers that must NOT depend on each other:
//!
//! - `boyko_rhi_vulkan` — the GPU golden mirror of `shaders/sdf_editlist.hlsl`
//!   (rung 8/9/10/11 diff the GPU readback against the field folded here).
//! - `boyko_physics` (W5) — the CPU SDF-collision queries evaluate the SAME
//!   analytic field for narrowphase, with ZERO readback and ZERO graphics deps.
//!
//! Keeping the field math in a graphics-free leaf makes the dependency graph
//! acyclic (`boyko_physics → boyko_sdf_math`, NOT `boyko_physics →
//! boyko_rhi_vulkan`) and guarantees the CPU physics evaluator and the GPU golden
//! fold the bit-identical arithmetic.
//!
//! # The scene model (SDF doc §2)
//!
//! The scene is an ORDERED list of [`SdfEdit`]s: each a primitive (SPHERE or BOX)
//! combined into the accumulated field by a boolean op (union / subtraction /
//! intersection), optionally smoothed (polynomial smooth-min/-max when
//! `smoothness > 0`). [`sdf_edit_list`] folds the list per evaluation point; the
//! gradient ([`sdf_edit_list_normal`]) is a central difference of that fold. This
//! is the analytic base — NO grid cache / brick atlas (deferred).
//!
//! # `no_std`
//!
//! The crate uses ONLY `core` f32 ops (`abs`/`max`/`min`/`clamp`) + fixed
//! `[f32; N]` arrays — no `Vec`, no allocation, ZERO third-party deps. The one
//! exception is `sqrt` ([`v_len`]): IEEE `sqrt` is NOT in stable `core` (it lives
//! in `std`, or behind the nightly `core_intrinsics` feature). To keep the crate
//! compiling on the pinned stable toolchain WITHOUT a `libm`-style dependency,
//! the `sqrt` source is feature-gated (see the private `sqrt` shim in `lib.rs`):
//!
//! - default (stable): links `std` SOLELY for `f32::sqrt` (the crate is otherwise
//!   `core`-only — no `Vec`, no allocation, no graphics).
//! - `nightly` feature: strictly `#![no_std]`, using `core::intrinsics::sqrtf32`.
//!
//! Both lower to the SAME hardware `sqrtss`, so the result is bit-identical in
//! either mode. The rest of the math is a verbatim cut from
//! `boyko_rhi_vulkan::compute`: the float op order is byte-for-byte identical, so
//! the committed GPU goldens are unaffected (a reordered FMA could push a golden
//! past its `±2/255` tolerance — this must NOT be "cleaned up").

// Strictly `#![no_std]` only when the `sqrt` intrinsic is available (the `nightly`
// feature); otherwise `std` is linked solely for `f32::sqrt` (see the `sqrt` shim).
#![cfg_attr(feature = "nightly", no_std)]
#![cfg_attr(feature = "nightly", feature(core_intrinsics))]
#![cfg_attr(feature = "nightly", allow(internal_features))]

/// IEEE-correct `f32` square root, the ONE op the field math needs that stable
/// `core` does not provide. Lowers to the hardware `sqrtss` in both modes, so the
/// result is bit-identical: the `nightly` feature uses `core::intrinsics::sqrtf32`
/// (strict `no_std`); the default build uses `std`'s `f32::sqrt` (links `std`
/// for this op only).
#[inline]
fn sqrt(x: f32) -> f32 {
    // `core::intrinsics::sqrtf32` is a safe intrinsic (a pure, total function over
    // all `f32` bit patterns) and lowers to the same hardware `sqrtss` as `std`'s
    // `f32::sqrt` — no `unsafe`, ZERO-new-unsafe mandate upheld.
    #[cfg(feature = "nightly")]
    {
        core::intrinsics::sqrtf32(x)
    }
    #[cfg(not(feature = "nightly"))]
    {
        x.sqrt()
    }
}

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

/// SDF image width (pixels) — matches the shader's `IMG_W`.
pub const SDF_IMG_W: u32 = 64;
/// SDF image height (pixels) — matches the shader's `IMG_H`.
pub const SDF_IMG_H: u32 = 64;

/// Central-difference step for the SDF gradient (the surface normal). Mirrors the
/// shader's `GRAD_H`; shared so the CPU evaluator and the GPU golden use the same
/// epsilon.
pub const SDF_GRAD_H: f32 = 0.0005;

/// Fixed capacity of the edit-list (the §S2 ceiling, scaled for the basic slice).
/// Matches the shader's `MAX_SDF_EDITS`.
pub const MAX_SDF_EDITS: usize = 16;

/// One SDF edit: a primitive + a uniform transform (center) + size (params) + a
/// boolean op + an optional smoothness factor.
///
/// `#[repr(C, align(16))]` so the Rust layout is byte-identical to the std430
/// structured-buffer element `shaders/sdf_editlist.hlsl` reads (the const-asserts
/// below pin offsets/size/align). `center`/`params` are `[f32; 4]` (the std430
/// `float4`) rather than `[f32; 3]` so the following `float4` starts at offset 16
/// without std430 inserting padding the Rust side would have to mirror — the two
/// layouts are then trivially identical.
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

    /// Packs a 16-bit PBR material id into the `center.w` FREE LANE (Render PBR MVP-2,
    /// Decision 4 — NO stride change). The field eval provably SKIPS `center.w` (the
    /// shader's `load_edit` reads only `center.xyz`), so this is determinism-NEUTRAL: the
    /// distance/depth golden + the `cpu_gpu_sdf_agreement` field stay byte-exact. The
    /// marcher reads the id back via `asuint(Buf[base + 3])` in a path the field never
    /// touches, and ATTRIBUTES the nearest-surface edit's material to the hit pixel.
    ///
    /// `material_id` is a 16-bit table index (the `R16`-width G-buffer carrier); the bits
    /// are stored verbatim as `f32::from_bits(id as u32)` (never interpreted as a float
    /// arithmetically — `center.w` is unread by every distance function).
    #[inline]
    pub fn with_material(mut self, material_id: u16) -> Self {
        self.center[3] = f32::from_bits(material_id as u32);
        self
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

// The shader hardcodes `SDF_EDIT_WORDS = 12u`; pin it so a layout change that
// desyncs the host encoder from the shader is a build error.
const _: () = assert!(SDF_EDIT_WORDS == 12, "SDF_EDIT_WORDS must equal the shader's 12u");

// ---- The edit-list field math (single source of truth, mirrors the shader) ----

const SDF_FAR: f32 = 1.0e9;

/// `a - b` — component-wise vector subtraction (mirrors the shader's `-`).
/// Exposed because the rung-8 single-sphere golden helpers in
/// `boyko_rhi_vulkan::compute` reuse it.
#[inline]
pub fn v_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `length(a)` — the Euclidean norm (mirrors the shader's `length`). Exposed
/// because the rung-8 single-sphere golden helpers in
/// `boyko_rhi_vulkan::compute` reuse it.
#[inline]
pub fn v_len(a: [f32; 3]) -> f32 {
    sqrt(a[0] * a[0] + a[1] * a[1] + a[2] * a[2])
}

/// `dot(a, b)` — the 3-component dot product (mirrors the shader's `dot`).
/// Exposed because the golden lighting in `boyko_rhi_vulkan::compute` reuses it.
#[inline]
pub fn v_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `a / length(a)` — the unit vector (mirrors the shader's `normalize`). Exposed
/// because the golden lighting + gradient normalization in
/// `boyko_rhi_vulkan::compute` reuse it.
///
/// # Degenerate (zero-length / non-finite) input
///
/// When `length(a)` is exactly zero — or non-finite — this returns
/// `[0.0, 0.0, 0.0]` instead of the `0.0 / 0.0 == NaN` the raw division would
/// produce, mirroring [`boyko_physics::math::Vec3::normalize`]'s zero-guard. The
/// only such input the field math feeds here is a central-difference gradient
/// ([`sdf_edit_list_normal`]) at a FIELD CRITICAL POINT (e.g. a query point
/// coincident with a primitive center under deep penetration, or a
/// subtract/smooth-blend interior saddle): the difference is `[0, 0, 0]`, so a
/// degenerate gradient now arrives at the physics narrowphase as `Vec3::ZERO`
/// — a usable sentinel its seam-skip test recognizes — rather than as a `NaN`
/// normal that would poison the solver.
///
/// # Golden-neutrality
///
/// The guard intercepts ONLY the exactly-zero / non-finite-length path; for every
/// non-degenerate input the arithmetic is byte-identical to the raw
/// `[a0/len, a1/len, a2/len]`. The committed rung-8/9/10/11 GPU goldens evaluate
/// this normal only at SURFACE hit points where `|grad| ≈ 1` (never at a
/// zero-gradient critical point), and the GPU shader's HLSL `normalize(0)` is
/// undefined there anyway — so the goldens never sample the guarded path and this
/// change is golden-neutral.
#[inline]
pub fn v_normalize(a: [f32; 3]) -> [f32; 3] {
    let len = v_len(a);
    // Degenerate gradient (a field critical point): a zero or non-finite length
    // would make the division `NaN`; return ZERO so the physics seam-skip fires.
    // Non-degenerate inputs take the byte-identical division (golden-neutral).
    if len <= f32::MIN_POSITIVE || !len.is_finite() {
        return [0.0, 0.0, 0.0];
    }
    [a[0] / len, a[1] / len, a[2] / len]
}

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
pub fn sd_sphere(p: [f32; 3], c: [f32; 3], r: f32) -> f32 {
    v_len(v_sub(p, c)) - r
}

/// The exact IQ box distance for an AABB centered at `c` with half-extents `h`
/// (mirrors the shader's `sd_box`).
#[inline]
pub fn sd_box(p: [f32; 3], c: [f32; 3], h: [f32; 3]) -> f32 {
    let q = v_sub(v_abs(v_sub(p, c)), h);
    let outside = v_len(v_max0(q));
    let inside = q[0].max(q[1].max(q[2])).min(0.0);
    outside + inside
}

/// One edit's primitive distance at `p` (mirrors the shader's `edit_distance`).
#[inline]
pub fn edit_distance(e: &SdfEdit, p: [f32; 3]) -> f32 {
    let center = [e.center[0], e.center[1], e.center[2]];
    if e.kind == sdf_kind::BOX {
        sd_box(p, center, [e.params[0], e.params[1], e.params[2]])
    } else {
        sd_sphere(p, center, e.params[0])
    }
}

/// Polynomial smooth-min (IQ `smin`), mirroring the shader's `smin`.
#[inline]
pub fn smin(a: f32, b: f32, k: f32) -> f32 {
    let hh = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    // lerp(b, a, hh) = b + (a - b) * hh
    (b + (a - b) * hh) - k * hh * (1.0 - hh)
}

/// Polynomial smooth-max (the De Morgan dual of [`smin`]), mirroring `smax`.
#[inline]
pub fn smax(a: f32, b: f32, k: f32) -> f32 {
    -smin(-a, -b, k)
}

/// Combines the accumulated distance `acc` with one edit's distance `d` under
/// `op` (hard when `k <= 0`, smooth when `k > 0`), mirroring the shader's
/// `combine`.
#[inline]
pub fn combine(acc: f32, d: f32, op: u32, k: f32) -> f32 {
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
pub fn sdf_edit_list_normal(edits: &[SdfEdit], p: [f32; 3]) -> [f32; 3] {
    let h = SDF_GRAD_H;
    let n = [
        sdf_edit_list(edits, [p[0] + h, p[1], p[2]]) - sdf_edit_list(edits, [p[0] - h, p[1], p[2]]),
        sdf_edit_list(edits, [p[0], p[1] + h, p[2]]) - sdf_edit_list(edits, [p[0], p[1] - h, p[2]]),
        sdf_edit_list(edits, [p[0], p[1], p[2] + h]) - sdf_edit_list(edits, [p[0], p[1], p[2] - h]),
    ];
    v_normalize(n)
}

// The unit tests link `std` for the test harness; they run under the default
// (non-`nightly`) profile, where `std` is already linked for `f32::sqrt`.
#[cfg(test)]
mod tests {
    use super::*;

    /// The load-bearing C1 guard: a zero-length gradient (a field critical point)
    /// must normalize to ZERO, NOT to `[NaN, NaN, NaN]` — otherwise the physics
    /// seam-skip (`length_squared() < eps²`, which is `false` for NaN) never fires.
    #[test]
    fn v_normalize_zero_is_zero_not_nan() {
        let r = v_normalize([0.0, 0.0, 0.0]);
        assert_eq!(r, [0.0, 0.0, 0.0]);
        assert!(r.iter().all(|c| c.is_finite()));
    }

    /// A non-finite-length input (defensive) also collapses to ZERO rather than
    /// propagating NaN/Inf.
    #[test]
    fn v_normalize_non_finite_is_zero() {
        assert_eq!(v_normalize([f32::INFINITY, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        assert_eq!(v_normalize([f32::NAN, 0.0, 0.0]), [0.0, 0.0, 0.0]);
    }

    /// Golden-neutrality: for a NON-degenerate input the guarded `v_normalize`
    /// must return BIT-IDENTICAL bytes to the raw `[a0/len, a1/len, a2/len]` the
    /// committed GPU goldens were produced with (the guard must not perturb the
    /// arithmetic of the surface-hit path).
    #[test]
    fn v_normalize_nonzero_byte_identical_to_raw() {
        for a in [
            [1.0_f32, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [3.0, -4.0, 12.0],
            [0.0005, -0.0005, 0.0005],
            [-1.25, 2.5, -3.75],
        ] {
            let len = v_len(a);
            let raw = [a[0] / len, a[1] / len, a[2] / len];
            let guarded = v_normalize(a);
            // Compare raw bits: the guard must change nothing on this path.
            assert_eq!(guarded[0].to_bits(), raw[0].to_bits());
            assert_eq!(guarded[1].to_bits(), raw[1].to_bits());
            assert_eq!(guarded[2].to_bits(), raw[2].to_bits());
        }
    }

    /// At a field critical point — a query point coincident with a primitive
    /// center under deep penetration — the central-difference gradient is
    /// symmetric and folds to `[0, 0, 0]`, so the normal arrives as ZERO (the
    /// sentinel the physics seam-skip recognizes), never as NaN.
    #[test]
    fn sdf_edit_list_normal_at_sphere_center_is_zero() {
        let edits = [SdfEdit::sphere([0.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0)];
        let n = sdf_edit_list_normal(&edits, [0.0, 0.0, 0.0]);
        assert_eq!(n, [0.0, 0.0, 0.0]);
        assert!(n.iter().all(|c| c.is_finite()));
    }
}
