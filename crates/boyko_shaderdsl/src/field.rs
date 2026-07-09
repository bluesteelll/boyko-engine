//! The SDF edit-list field math, authored ONCE generic over [`FieldScalar`].
//!
//! Every body here mirrors the frozen reference operand-for-operand:
//! `crates/boyko_rhi_vulkan/shaders/sdf_field.hlsli` (the GPU field) AND
//! `boyko_sdf_math` lib.rs:600-689 (the CPU field). Instantiating with `S = f32`
//! ([`crate::scalar`]'s Eval impl) reproduces the hand-written CPU field
//! BYTE-IDENTICALLY; instantiating with `S = Emit` ([`crate::emit`]) records the
//! same op tree for the HLSL printer.
//!
//! # Op-order is load-bearing
//!
//! The determinism contract (`sdf_field.hlsli:20-38`) forbids ANY reordering: a
//! reassociated FMA could push a committed GPU golden past its `±2/255` tolerance.
//! Each function below therefore writes the operands in the SAME order as the
//! frozen reference. This is a REFACTOR — the behavior is unchanged.

use crate::scalar::{self, FieldScalar};

/// The "empty field" sentinel before the first edit. Mirrors the shader's `FAR`
/// (`sdf_field.hlsli:41`) and `boyko_sdf_math`'s `SDF_FAR` (lib.rs:525).
pub const SDF_FAR: f32 = 1.0e9;

/// Fixed capacity of the edit-list. Mirrors the shader's `MAX_SDF_EDITS`
/// (`sdf_field.hlsli:48`) and `boyko_sdf_math`'s `MAX_SDF_EDITS` (lib.rs:101).
pub const MAX_SDF_EDITS: usize = 16;

/// The `dot(ba, ba)` floor in [`sd_capsule`] — guards the degenerate `a == b` segment
/// (a zero-length capsule). With `a == b` the segment direction `ba` is the zero vector
/// and `dot(ba, ba)` is `0`; clamping the denominator to this epsilon makes the
/// projection parameter `h = 0`, so the capsule cleanly collapses to a sphere of radius
/// `r` at `a` (`length(p - a) - r`) instead of producing `0/0 == NaN`. Mirrors the
/// shader's `CAPSULE_DENOM_EPS` (`sdf_field.hlsli`) and `boyko_sdf_math`'s
/// `CAPSULE_DENOM_EPS`.
pub const CAPSULE_DENOM_EPS: f32 = 1.0e-8;

/// SDF boolean-op discriminants. Mirror the shader's `OP_*` (`sdf_field.hlsli:57`)
/// and `boyko_sdf_math::sdf_op` (lib.rs:80).
pub mod op {
    /// Union — `min(acc, d)` (smooth-min when `k > 0`).
    pub const UNION: u32 = 0;
    /// Subtraction — `max(acc, -d)` (smooth-max when `k > 0`).
    pub const SUBTRACT: u32 = 1;
    /// Intersection — `max(acc, d)` (smooth-max when `k > 0`).
    pub const INTERSECT: u32 = 2;
}

/// SDF primitive-kind discriminants. Mirror the shader's `KIND_*`
/// (`sdf_field.hlsli:53`) and `boyko_sdf_math::sdf_kind` (lib.rs:72).
pub mod kind {
    /// A sphere primitive — `params.x` is the radius.
    pub const SPHERE: u32 = 0;
    /// An axis-aligned box primitive — `params.xyz` are the half-extents.
    pub const BOX: u32 = 1;
    /// A capsule primitive — `center.xyz` is endpoint `a`, `params.xyz` is endpoint `b`,
    /// `params.w` (lifted into [`EditView::radius`]) is the cap radius. APPEND-only
    /// (sphere=0, box=1 are frozen).
    pub const CAPSULE: u32 = 2;
}

/// One edit's parameters lifted into the backend scalar `S`.
///
/// The host loop ([`sdf_field_body`]) reads the f32 fields off a `SdfEdit` and
/// lifts them into `S` via [`FieldScalar::lit`] (the [`EditView`] adapter), so the
/// generic body never touches the concrete `SdfEdit` type — keeping `field.rs`
/// `no_std`-clean and free of any `boyko_sdf_math` dependency. `kind`/`op` stay
/// host `u32` (they drive HOST control flow, not traced arithmetic).
#[derive(Clone, Copy)]
pub struct EditView<S: FieldScalar<Vec3 = [S; 3]>> {
    /// The primitive center (xyz) — also the capsule's endpoint `a`.
    pub center: [S; 3],
    /// The radius (sphere) or half-extents (box) — also the capsule's endpoint `b`.
    pub params: [S; 3],
    /// Primitive kind ([`kind`]). A HOST discriminant (selects which distance).
    pub kind: u32,
    /// Boolean op ([`op`]). A HOST discriminant (selects which combine formula).
    pub op: u32,
    /// Smooth-blend radius (0 = hard op).
    pub smoothness: S,
    /// The capsule cap radius (the `params.w` lane). Unused by sphere/box (they pass
    /// `0`); read only by the [`kind::CAPSULE`] arm of [`edit_distance`].
    pub radius: S,
}

/// Polynomial smooth-min (IQ `smin`). Mirrors `sdf_field.hlsli:110-113` and
/// `boyko_sdf_math::smin` (lib.rs:627-631).
///
/// ```text
/// hh = clamp(0.5 + 0.5 * (b - a) / k, 0, 1);
/// return lerp(b, a, hh) - k * hh * (1 - hh);
/// ```
#[inline]
pub fn smin<S: FieldScalar<Vec3 = [S; 3]>>(a: S, b: S, k: S) -> S {
    let half = S::lit(0.5);
    let one = S::lit(1.0);
    // hh = clamp(0.5 + 0.5 * (b - a) / k, 0, 1)
    let hh = half.add(half.mul(b.sub(a)).div(k)).clamp01();
    // lerp(b, a, hh) - k * hh * (1 - hh)
    b.lerp(a, hh).sub(k.mul(hh).mul(one.sub(hh)))
}

/// Polynomial smooth-max (the De Morgan dual of [`smin`]). Mirrors
/// `sdf_field.hlsli:116-118` and `boyko_sdf_math::smax` (lib.rs:635-637):
/// `-smin(-a, -b, k)`.
#[inline]
pub fn smax<S: FieldScalar<Vec3 = [S; 3]>>(a: S, b: S, k: S) -> S {
    smin(a.neg(), b.neg(), k).neg()
}

/// Combines the accumulated distance `acc` with one edit's distance `d` under
/// `op` (hard when `k <= 0`, smooth when `k > 0`). Mirrors `sdf_field.hlsli:122-129`
/// and `boyko_sdf_math::combine` (lib.rs:643-669).
///
/// The op-dispatch (UNION / SUBTRACT / INTERSECT) is a HOST branch over the `u32`
/// discriminant — exactly as the frozen `combine`'s `if/else if/else`. Inside each
/// branch the smooth-vs-hard choice is the only TRACED predicate (`k > 0`); it is
/// expressed as a [`FieldScalar::select`] over the two already-computed values so
/// the emitter records a ternary (the frozen `(k > 0.0) ? _ : _`). On the f32 Eval
/// backend both arms are pure, so the SELECTED value is byte-identical to the
/// frozen branch's result.
#[inline]
pub fn combine<S: FieldScalar<Vec3 = [S; 3]>>(acc: S, d: S, op: u32, k: S) -> S {
    let zero = S::lit(0.0);
    let k_pos = k.gt(zero);
    if op == op::SUBTRACT {
        // (k > 0.0) ? smax(acc, -d, k) : max(acc, -d)
        let neg_d = d.neg();
        S::select(k_pos, smax(acc, neg_d, k), acc.max(neg_d))
    } else if op == op::INTERSECT {
        // (k > 0.0) ? smax(acc, d, k) : max(acc, d)
        S::select(k_pos, smax(acc, d, k), acc.max(d))
    } else {
        // UNION (and any unknown discriminant): (k > 0.0) ? smin(acc, d, k) : min(acc, d)
        S::select(k_pos, smin(acc, d, k), acc.min(d))
    }
}

/// `length(p - c) - r` — the analytic sphere distance. Mirrors
/// `sdf_field.hlsli:88-90` and `boyko_sdf_math::sd_sphere` (lib.rs:600-602).
#[inline]
pub fn sd_sphere<S: FieldScalar<Vec3 = [S; 3]>>(p: [S; 3], c: [S; 3], r: S) -> S {
    scalar::v_len(scalar::v_sub(p, c)).sub(r)
}

/// The exact IQ box distance for an AABB centered at `c` with half-extents `h`.
/// Mirrors `sdf_field.hlsli:94-97` and `boyko_sdf_math::sd_box` (lib.rs:607-612).
///
/// ```text
/// q = abs(p - c) - h;
/// return length(max(q, 0)) + min(max(q.x, max(q.y, q.z)), 0);
/// ```
#[inline]
pub fn sd_box<S: FieldScalar<Vec3 = [S; 3]>>(p: [S; 3], c: [S; 3], h: [S; 3]) -> S {
    let q = scalar::v_sub(scalar::v_abs(scalar::v_sub(p, c)), h);
    let outside = scalar::v_len(scalar::v_max0(q));
    // q.x.max(q.y.max(q.z)).min(0.0) — the exact fold order of lib.rs:610.
    let inside = q[0].max(q[1].max(q[2])).min(S::lit(0.0));
    outside.add(inside)
}

/// The exact IQ capsule distance for a segment `[a, b]` with cap radius `r`:
///
/// ```text
/// pa = p - a;  ba = b - a;
/// h  = clamp(dot(pa, ba) / max(dot(ba, ba), CAPSULE_DENOM_EPS), 0, 1);
/// return length(pa - ba * h) - r;
/// ```
///
/// Mirrors the hand-written `sd_capsule` in `sdf_field.hlsli` and
/// `boyko_sdf_math::sd_capsule`. Both dot products are spelled via [`scalar::v_dot`]
/// (the EXPLICIT left-associated scalar fold, NOT the HLSL `dot()` intrinsic) so the
/// host f32 and GPU bytes cannot fork. `max(dot(ba,ba), EPS)` guards the `a == b`
/// degenerate (a zero-length capsule collapses to a sphere of radius `r` at `a`).
///
/// # Lower-bound invariant (the sphere-tracing precondition)
///
/// The capsule is the EXACT Euclidean distance to the swept-sphere surface (`length`
/// to the nearest point on the segment, minus `r`), so it is a TIGHT bound — it never
/// over-reports the true distance. It therefore preserves the field's
/// conservative-lower-bound contract (`sdf_field.hlsli` D7), so the marcher never
/// overshoots.
#[inline]
pub fn sd_capsule<S: FieldScalar<Vec3 = [S; 3]>>(p: [S; 3], a: [S; 3], b: [S; 3], r: S) -> S {
    let pa = scalar::v_sub(p, a);
    let ba = scalar::v_sub(b, a);
    // h = clamp(dot(pa, ba) / max(dot(ba, ba), CAPSULE_DENOM_EPS), 0, 1)
    let denom = scalar::v_dot(ba, ba).max(S::lit(CAPSULE_DENOM_EPS));
    let h = scalar::v_dot(pa, ba).div(denom).clamp01();
    // length(pa - ba * h) - r
    scalar::v_len(scalar::v_sub(pa, scalar::v_scale(ba, h))).sub(r)
}

/// One edit's primitive distance at `p`. Mirrors `sdf_field.hlsli:100-105` and
/// `boyko_sdf_math::edit_distance` (lib.rs:616-623).
///
/// The kind test is a HOST branch over the `u32` discriminant (exactly the frozen
/// `if (e.kind == KIND_BOX) ... else if (e.kind == KIND_CAPSULE) ... else sphere`): it
/// selects WHICH distance function to fold, not a traced value, so it is not a
/// `select` (the primitives have different op trees — only one is recorded per edit,
/// matching the GPU's per-edit branch). BOX stays the first branch and SPHERE the
/// final `else` (frozen); CAPSULE is appended between them.
#[inline]
pub fn edit_distance<S: FieldScalar<Vec3 = [S; 3]>>(e: &EditView<S>, p: [S; 3]) -> S {
    if e.kind == kind::BOX {
        sd_box(p, e.center, e.params)
    } else if e.kind == kind::CAPSULE {
        sd_capsule(p, e.center, e.params, e.radius)
    } else {
        sd_sphere(p, e.center, e.params[0])
    }
}

/// Evaluates the ordered edit-list field at `p` (the CSG fold). Mirrors
/// `sdf_field.hlsli:132-146` and `boyko_sdf_math::sdf_edit_list` (lib.rs:677-689).
///
/// The first edit seeds the accumulator hard; each later edit combines under its
/// own op. `edits.len()` is clamped to [`MAX_SDF_EDITS`] (the shader's `min`). The
/// edit loop is a HOST `for 0..n` (not traced): on the f32 Eval backend it is the
/// same fold as the hand-written field; on the `Emit` backend it unrolls into the
/// recorded SSA per edit (the GPU's `[loop]` is the only structural difference,
/// re-introduced in Pass 2 when the bodies are spliced back).
pub fn sdf_field_body<S: FieldScalar<Vec3 = [S; 3]>>(edits: &[EditView<S>], p: [S; 3]) -> S {
    let n = edits.len().min(MAX_SDF_EDITS);
    let mut acc = S::lit(SDF_FAR);
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
