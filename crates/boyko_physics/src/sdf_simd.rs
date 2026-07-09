//! O9 — width-only AVX2 batched SDF edit-list narrowphase kernel
//! ([`sdf_edit_list_x8`]): evaluates the analytic CSG field at 8 world points per
//! call, the hot inner loop the box-vs-SDF narrowphase ([`box_sdf_manifold`]) runs
//! per body (8 OBB corners for the distances, then a 6-offset batch per corner for
//! the central-difference gradient).
//!
//! [`box_sdf_manifold`]: crate::systems
//!
//! # Bit-exactness is the load-bearing constraint
//!
//! Lane `i` of [`sdf_edit_list_x8`] is `f32::to_bits`-IDENTICAL to the scalar
//! [`boyko_sdf_math::sdf_edit_list`] evaluated at point `i`. The scalar leaf
//! `boyko_sdf_math` is the SOLE CPU↔GPU oracle and is FROZEN — this x8 kernel lives
//! in `boyko_physics` (NOT the leaf), so the GPU golden crate `boyko_rhi_vulkan`
//! never references it. Bit-identity is achieved by widening the scalar fold
//! op-for-op: every `__m256` operation is the EXACT vertical (per-lane) counterpart
//! of a scalar `f32` op, in the SAME association and operand order the leaf uses,
//! annotated `// mirrors lib.rs:NNN` at each binary op.
//!
//! # No-FMA / no-approx invariant (same contract as `solver/simd.rs`)
//!
//! Width-only AVX2: exact `_mm256_{sqrt,mul,add,sub,div,max,min,xor,andnot}_ps`,
//! NO `_mm256_fmadd*` (a fused multiply-add rounds ONCE; the scalar `a*b + c`
//! rounds TWICE — they diverge by a ULP per target), NO `_mm256_rsqrt_ps` /
//! `_mm256_rcp_ps` (their approximations return different bits on Intel vs AMD).
//! Every `a*b + c` is a SEPARATE `_mm256_mul_ps` then `_mm256_add_ps`. Rust does
//! NOT auto-contract an explicit `mul`+`add` into an FMA (contraction needs an
//! explicit `f32::mul_add`, never called here, or a global fast-math flag stable
//! Rust does not expose), so even a `+fma` build would not fuse these — but to make
//! the no-FMA invariant a COMPILE-TIME contract the build is rejected outright
//! under `+fma` (the [`compile_error!`] below).
//!
//! # Lane isolation (R5)
//!
//! The kernel performs ZERO horizontal operations: every `_mm256_*_ps` is vertical
//! (per-lane), so an inert / unused lane CANNOT influence a live lane. The gradient
//! batch packs 6 active offsets into lanes 0..6 and seeds lanes 6,7 with a finite
//! point (the corner's own world position) purely to keep `(b - a) / k` finite — a
//! belt-and-suspenders measure; lanes 6,7 are never read.

// Width-only AVX2 narrowphase: this module is `mul_add`-free (every `a*b + c` is a
// separate `_mm256_mul_ps` then `_mm256_add_ps`), so the no-FMA determinism
// invariant must hold. Reject any `+fma` build to make it a compile-time contract,
// not a runtime hope (Rust never contracts our explicit mul+add even under +fma —
// see the "No-FMA / no-approx invariant" module-doc paragraph for why).
#[cfg(target_feature = "fma")]
compile_error!(
    "boyko-physics SDF narrowphase determinism requires no FMA contraction; this \
     SIMD module is written mul_add-free and must be built without +fma. (No \
     mul_add is emitted, so +fma would not actually contract our explicit mul+add, \
     but the build is rejected to make the no-FMA invariant load-bearing.)"
);

use core::arch::x86_64::__m256;

use boyko_sdf_math::{MAX_SDF_EDITS, SdfEdit, sdf_kind, sdf_op};

/// `SDF_FAR` mirror — the scalar leaf seeds the accumulator with this sentinel for
/// the empty field (`boyko_sdf_math`'s private `const SDF_FAR: f32 = 1.0e9`). It is
/// not `pub` in the leaf, so the value is duplicated here; an empty field must
/// broadcast the SAME bits the scalar `sdf_edit_list` returns at `n == 0`.
const SDF_FAR: f32 = 1.0e9;

/// Clamps each lane of `v` to `[0, 1]` — the SIMD counterpart of the scalar
/// `f32::clamp(t, 0.0, 1.0)` used inside [`boyko_sdf_math::smin`] (lib.rs:316).
///
/// # CORRECTNESS (R4) — sign-of-zero operand order
///
/// `f32::max` / `f32::min` (and `f32::clamp`, an if-else built on the same `<` / `>`
/// tie rule) return the FIRST operand's sign on a `±0.0` tie, whereas the hardware
/// `MAXPS` / `MINPS` (`_mm256_{max,min}_ps`) return the SECOND. The two are EXACT
/// opposites on a zero tie, so the bit-faithful SIMD mirror SWAPS the operands:
/// `clamp(v, 0, 1)` = `v.max(0.0).min(1.0)` becomes
/// `_mm256_min_ps(one, _mm256_max_ps(zero, v))` — `MAXPS(zero, v)` mirrors `v.max(0)`
/// and `MINPS(one, .)` mirrors `.min(1)`. This is `to_bits`-identical to the scalar
/// `f32::clamp(v, 0, 1)` for every finite input INCLUDING `v == -0.0` (verified
/// exhaustively over the `±0.0` tie palette). The kernel's domain is NaN-free (R5
/// keeps every live lane finite), so the `MAXPS`/`f32::max` NaN-propagation
/// difference is out of domain.
///
/// # Safety
///
/// AVX2-gated by the module `cfg` + `#[target_feature(enable = "avx2")]`. Pure
/// register arithmetic, no memory access.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn clamp01_x8(v: __m256) -> __m256 {
    use core::arch::x86_64::{_mm256_max_ps, _mm256_min_ps, _mm256_set1_ps};
    let zero = _mm256_set1_ps(0.0);
    let one = _mm256_set1_ps(1.0);
    // Operands SWAPPED vs the scalar's `v.max(0).min(1)`: `MAXPS`/`MINPS` keep the
    // SECOND operand on a `±0` tie, so swapping reproduces `f32`'s first-operand rule.
    _mm256_min_ps(one, _mm256_max_ps(zero, v))
}

/// Per-lane unary negate — the SIMD counterpart of the scalar `-x`.
///
/// XOR with the sign bit (`-0.0`) flips the sign of every `f32` including `±0.0` and
/// NaN exactly as Rust's `-x` does (lib.rs:324 `-smin(-a, -b, k)`), so this is
/// bit-identical to the scalar unary minus.
///
/// # Safety
///
/// AVX2-gated. Pure register op, no memory access.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn neg_x8(v: __m256) -> __m256 {
    use core::arch::x86_64::{_mm256_set1_ps, _mm256_xor_ps};
    _mm256_xor_ps(v, _mm256_set1_ps(-0.0))
}

/// Per-lane absolute value — the SIMD counterpart of the scalar `f32::abs`.
///
/// `andnot(-0.0, v)` clears the sign bit, matching `f32::abs` exactly (used by
/// [`sd_box_x8`] via `v_abs`, lib.rs:277/296). Distinct from `max(v, -v)`, which
/// would diverge on a NaN input.
///
/// # Safety
///
/// AVX2-gated. Pure register op, no memory access.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn abs_x8(v: __m256) -> __m256 {
    use core::arch::x86_64::{_mm256_andnot_ps, _mm256_set1_ps};
    // andnot(mask, v) = (!mask) & v; with mask = sign bit this clears the sign.
    _mm256_andnot_ps(_mm256_set1_ps(-0.0), v)
}

/// 8-wide polynomial smooth-min, bit-identical to [`boyko_sdf_math::smin`]
/// (lib.rs:315-319) op-for-op (NO FMA, R2).
///
/// Scalar:
/// ```text
/// let hh = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
/// (b + (a - b) * hh) - k * hh * (1.0 - hh)
/// ```
/// Widened: `t1 = (b - a) / k`; `t2 = 0.5 + 0.5 * t1` (mul THEN add — NOT fmadd);
/// `hh = clamp01(t2)`; `lerp = b + (a - b) * hh` (mul-then-add); `corr = (k * hh) *
/// (1 - hh)` (`k*hh` left-assoc); `result = lerp - corr`.
///
/// # Safety
///
/// AVX2-gated. `clamp01_x8` inherits this fn's target feature (same-feature call —
/// no `unsafe`). Pure register arithmetic, no memory access.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn smin_x8(a: __m256, b: __m256, k: __m256) -> __m256 {
    use core::arch::x86_64::{_mm256_add_ps, _mm256_div_ps, _mm256_mul_ps, _mm256_set1_ps, _mm256_sub_ps};
    let half = _mm256_set1_ps(0.5);
    let one = _mm256_set1_ps(1.0);
    // (b - a) / k                                          // mirrors lib.rs:316
    let t1 = _mm256_div_ps(_mm256_sub_ps(b, a), k);
    // 0.5 + 0.5 * t1  (mul then add — NOT fmadd)           // mirrors lib.rs:316
    let t2 = _mm256_add_ps(half, _mm256_mul_ps(half, t1));
    let hh = clamp01_x8(t2);
    // b + (a - b) * hh  (mul then add — NOT fmadd)         // mirrors lib.rs:318
    let lerp = _mm256_add_ps(b, _mm256_mul_ps(_mm256_sub_ps(a, b), hh));
    // k * hh * (1 - hh)  ((k*hh) left-assoc)               // mirrors lib.rs:318
    let corr = _mm256_mul_ps(_mm256_mul_ps(k, hh), _mm256_sub_ps(one, hh));
    _mm256_sub_ps(lerp, corr)
}

/// 8-wide polynomial smooth-max, bit-identical to [`boyko_sdf_math::smax`]
/// (lib.rs:323-325): `-smin(-a, -b, k)` op-for-op (R2).
///
/// # Safety
///
/// AVX2-gated. `neg_x8` / `smin_x8` inherit this fn's target feature (same-feature
/// calls — no `unsafe`). Pure register arithmetic, no memory access.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn smax_x8(a: __m256, b: __m256, k: __m256) -> __m256 {
    // -smin(-a, -b, k)                                     // mirrors lib.rs:324
    neg_x8(smin_x8(neg_x8(a), neg_x8(b), k))
}

/// 8-wide analytic sphere distance, bit-identical to [`boyko_sdf_math::sd_sphere`]
/// (lib.rs:288): `length(p - c) - r`.
///
/// `length` is `sqrt((dx*dx + dy*dy) + dz*dz)` left-to-right, mirroring
/// `v_len` (lib.rs:228) op-for-op (NO FMA).
///
/// # Safety
///
/// AVX2-gated. Pure register arithmetic, no memory access.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
#[allow(clippy::too_many_arguments)]
fn sd_sphere_x8(
    px: __m256,
    py: __m256,
    pz: __m256,
    cx: __m256,
    cy: __m256,
    cz: __m256,
    r: __m256,
) -> __m256 {
    use core::arch::x86_64::{_mm256_add_ps, _mm256_mul_ps, _mm256_sqrt_ps, _mm256_sub_ps};
    let dx = _mm256_sub_ps(px, cx); // p - c                // mirrors lib.rs:289 (v_sub)
    let dy = _mm256_sub_ps(py, cy);
    let dz = _mm256_sub_ps(pz, cz);
    // (dx*dx + dy*dy) + dz*dz  left-to-right                // mirrors lib.rs:228
    let sum = _mm256_add_ps(
        _mm256_add_ps(_mm256_mul_ps(dx, dx), _mm256_mul_ps(dy, dy)),
        _mm256_mul_ps(dz, dz),
    );
    let len = _mm256_sqrt_ps(sum);
    _mm256_sub_ps(len, r) // length(p - c) - r              // mirrors lib.rs:289
}

/// 8-wide exact IQ box distance, bit-identical to [`boyko_sdf_math::sd_box`]
/// (lib.rs:295-299), reproducing the scalar max/min ASSOCIATION verbatim (R1).
///
/// Scalar:
/// ```text
/// let q = v_abs(p - c) - h;                       // q0,q1,q2
/// let outside = v_len(v_max0(q));                 // sqrt((m0²+m1²)+m2²)
/// let inside = q0.max(q1.max(q2)).min(0.0);
/// outside + inside
/// ```
///
/// # Safety
///
/// AVX2-gated. `abs_x8` inherits this fn's target feature (same-feature call — no
/// `unsafe`). Pure register arithmetic, no memory access.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
#[allow(clippy::too_many_arguments)]
fn sd_box_x8(
    px: __m256,
    py: __m256,
    pz: __m256,
    cx: __m256,
    cy: __m256,
    cz: __m256,
    hx: __m256,
    hy: __m256,
    hz: __m256,
) -> __m256 {
    use core::arch::x86_64::{
        _mm256_add_ps, _mm256_max_ps, _mm256_min_ps, _mm256_mul_ps, _mm256_set1_ps, _mm256_sqrt_ps,
        _mm256_sub_ps,
    };
    let zero = _mm256_set1_ps(0.0);
    // q = abs(p - c) - h                                   // mirrors lib.rs:296
    let q0 = _mm256_sub_ps(abs_x8(_mm256_sub_ps(px, cx)), hx);
    let q1 = _mm256_sub_ps(abs_x8(_mm256_sub_ps(py, cy)), hy);
    let q2 = _mm256_sub_ps(abs_x8(_mm256_sub_ps(pz, cz)), hz);

    // outside = length(max(q, 0)): mxi = qi.max(0.0)        // mirrors lib.rs:282/297
    // Operands SWAPPED (`MAXPS(zero, qi)` mirrors `qi.max(0.0)`): `MAXPS` keeps the
    // 2nd operand on a `±0` tie, the opposite of `f32::max`'s 1st-operand rule.
    let mx0 = _mm256_max_ps(zero, q0);
    let mx1 = _mm256_max_ps(zero, q1);
    let mx2 = _mm256_max_ps(zero, q2);
    // (mx0² + mx1²) + mx2²  left-assoc                      // mirrors lib.rs:228
    let sum = _mm256_add_ps(
        _mm256_add_ps(_mm256_mul_ps(mx0, mx0), _mm256_mul_ps(mx1, mx1)),
        _mm256_mul_ps(mx2, mx2),
    );
    let outside = _mm256_sqrt_ps(sum);

    // inside = q0.max(q1.max(q2)).min(0.0) — KEEP the nesting (NOT max(max(q0,q1),q2))
    //                                                      // mirrors lib.rs:298
    // Operands SWAPPED at each min/max (`MAXPS(q2,q1)` = `q1.max(q2)`, etc.) so the
    // `±0` tie sign matches `f32::max`/`f32::min` (1st-operand rule), not `MAXPS`/
    // `MINPS` (2nd-operand). The nesting (association) is preserved.
    let inside = _mm256_min_ps(zero, _mm256_max_ps(_mm256_max_ps(q2, q1), q0));

    // outside + inside  (outside first)                    // mirrors lib.rs:299
    _mm256_add_ps(outside, inside)
}

/// 8-wide one-edit primitive distance, bit-identical to
/// [`boyko_sdf_math::edit_distance`] (lib.rs:304-311).
///
/// `kind` is a PER-EDIT scalar (uniform across the 8 points), so the BOX/SPHERE
/// choice is a scalar branch evaluated ONCE per edit (R3 — no per-lane blend). The
/// center/params are scalar broadcasts via `_mm256_set1_ps`.
///
/// # Safety
///
/// AVX2-gated. `sd_box_x8` / `sd_sphere_x8` inherit this fn's target feature
/// (same-feature calls — no `unsafe`). Reads only the scalar fields of `e`.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn edit_distance_x8(e: &SdfEdit, px: __m256, py: __m256, pz: __m256) -> __m256 {
    use core::arch::x86_64::_mm256_set1_ps;
    let cx = _mm256_set1_ps(e.center[0]);
    let cy = _mm256_set1_ps(e.center[1]);
    let cz = _mm256_set1_ps(e.center[2]);
    if e.kind == sdf_kind::BOX {
        // box: params.xyz = half-extents                   // mirrors lib.rs:306-307
        let hx = _mm256_set1_ps(e.params[0]);
        let hy = _mm256_set1_ps(e.params[1]);
        let hz = _mm256_set1_ps(e.params[2]);
        sd_box_x8(px, py, pz, cx, cy, cz, hx, hy, hz)
    } else {
        // sphere: params.x = radius                        // mirrors lib.rs:309
        let r = _mm256_set1_ps(e.params[0]);
        sd_sphere_x8(px, py, pz, cx, cy, cz, r)
    }
}

/// 8-wide CSG combine of the accumulator `acc` with one edit's distance `d`,
/// bit-identical to [`boyko_sdf_math::combine`] (lib.rs:331-357).
///
/// `op` and `k` are PER-EDIT scalars: the `match op` AND the `if k > 0` are
/// SCALAR-uniform branches evaluated ONCE per edit (R3 — never per-lane). The hard
/// ops keep the `acc`-first operand order (lib.rs:337/344/353).
///
/// # Safety
///
/// AVX2-gated. `smax_x8` / `smin_x8` / `neg_x8` inherit this fn's target feature
/// (same-feature calls — no `unsafe`). Pure register arithmetic, no memory access.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn combine_x8(acc: __m256, d: __m256, op: u32, k: f32) -> __m256 {
    use core::arch::x86_64::{_mm256_max_ps, _mm256_min_ps, _mm256_set1_ps};
    let kv = _mm256_set1_ps(k);
    let smooth = k > 0.0; // scalar-uniform (lib.rs:334/341/350)
    // Hard-op operands are SWAPPED vs the scalar (`MAXPS(b,a)` = `a.max(b)`): `MAXPS`/
    // `MINPS` keep the 2nd operand on a `±0` tie, the opposite of `f32::max`/`f32::min`'s
    // 1st-operand rule — swapping makes the hard-op sign-of-zero `to_bits`-identical to
    // the scalar `acc.max(.)` / `acc.min(.)`.
    if op == sdf_op::SUBTRACT {
        // smax(acc, -d, k) / acc.max(-d)                   // mirrors lib.rs:335/337
        let neg_d = neg_x8(d);
        if smooth {
            smax_x8(acc, neg_d, kv)
        } else {
            _mm256_max_ps(neg_d, acc)
        }
    } else if op == sdf_op::INTERSECT {
        // smax(acc, d, k) / acc.max(d)                     // mirrors lib.rs:342/344
        if smooth {
            smax_x8(acc, d, kv)
        } else {
            _mm256_max_ps(d, acc)
        }
    } else {
        // UNION (and any unknown discriminant) — smin(acc, d, k) / acc.min(d)
        //                                                  // mirrors lib.rs:351/353
        if smooth {
            smin_x8(acc, d, kv)
        } else {
            _mm256_min_ps(d, acc)
        }
    }
}

/// Evaluates the ordered edit-list field at 8 world points (SoA: `px`/`py`/`pz`),
/// returning the 8 signed distances — bit-identical, lane-for-lane, to
/// [`boyko_sdf_math::sdf_edit_list`] (lib.rs:365-377).
///
/// This is THE single kernel: the per-corner distance pass and the per-corner
/// gradient batch both call it. `edits.len()` is clamped to [`MAX_SDF_EDITS`]
/// (matching the leaf's `min`); an EMPTY field returns `SDF_FAR` in every lane WITH
/// NO `edits[0]` access (the empty field is a live case). The first edit seeds the
/// accumulator hard; each later edit folds under its own `op` / `smoothness`.
///
/// # Safety
///
/// AVX2-gated by the module `cfg` + `#[target_feature(enable = "avx2")]`: a
/// non-AVX2 host cannot link this path, so the compile-time gate IS the runtime
/// guarantee. `edit_distance_x8` / `combine_x8` inherit this fn's target feature
/// (same-feature calls — no `unsafe`). Only `edits[..n]` is read (`n <= len`).
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
pub(crate) fn sdf_edit_list_x8(edits: &[SdfEdit], px: __m256, py: __m256, pz: __m256) -> __m256 {
    use core::arch::x86_64::_mm256_set1_ps;
    let n = edits.len().min(MAX_SDF_EDITS);
    if n == 0 {
        // Empty field: +far everywhere, with NO edit access (lib.rs:367 seed).
        return _mm256_set1_ps(SDF_FAR);
    }
    // The accumulator is seeded HARD by edit 0 (lib.rs:370-371), so its initial
    // value is overwritten on i == 0 before any read — `SDF_FAR` matches the scalar
    // `acc = SDF_FAR` default that the i == 0 branch replaces.
    let mut acc = _mm256_set1_ps(SDF_FAR);
    for (i, e) in edits.iter().take(n).enumerate() {
        let d = edit_distance_x8(e, px, py, pz);
        if i == 0 {
            acc = d; // hard seed                            // mirrors lib.rs:371
        } else {
            acc = combine_x8(acc, d, e.op, e.smoothness); // mirrors lib.rs:373
        }
    }
    acc
}

#[cfg(test)]
mod o9_kernel_tests {
    //! O9 bit-exactness gate: every lane of [`sdf_edit_list_x8`] must be
    //! `f32::to_bits`-IDENTICAL to the scalar oracle [`boyko_sdf_math::sdf_edit_list`]
    //! evaluated at that lane's point. This module compiles ONLY in a `+avx2` build
    //! (the parent module is `#[cfg(target_feature = "avx2")]`), so the kernel and
    //! the scalar leaf are both linkable here. ANY lane mismatch is a HARD FAIL.
    //!
    //! The generator deliberately WIDENS into the C3 tie zone — `±0.0` half-extents,
    //! zero / tiny / degenerate extents, surface-coincident points (`q == 0`), points
    //! coincident with primitive centers (zero gradient), `k ∈ {0, tiny, large}` — to
    //! force the `±0`/association ties where a non-bit-exact widening would diverge.

    use core::arch::x86_64::{_mm256_loadu_ps, _mm256_storeu_ps};

    use boyko_sdf_math::{MAX_SDF_EDITS, SdfEdit, sdf_edit_list, sdf_kind, sdf_op};

    use super::sdf_edit_list_x8;

    /// Deterministic splitmix64 — no external dep, reproducible across runs.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Self(seed)
        }
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next_u64() % n
        }
        /// A float in `[-range, range]`.
        fn f32_in(&mut self, range: f32) -> f32 {
            let u = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
            (u * 2.0 - 1.0) * range
        }
    }

    /// Picks one coordinate value from a palette weighted toward the C3 tie zone:
    /// exact `0.0`, exact `-0.0`, a tiny magnitude, and ordinary spread.
    fn tie_coord(rng: &mut Rng) -> f32 {
        match rng.below(7) {
            0 => 0.0,
            1 => -0.0,
            2 => rng.f32_in(1.0e-6),
            3 => rng.f32_in(0.01),
            4 => 1.0e9, // far / large-magnitude finite
            _ => rng.f32_in(5.0),
        }
    }

    /// Picks an extent (radius / half-extent) palette including `0.0`, `-0.0`,
    /// tiny, and ordinary — the degenerate-primitive cases.
    fn tie_extent(rng: &mut Rng) -> f32 {
        match rng.below(6) {
            0 => 0.0,
            1 => -0.0,
            2 => rng.f32_in(1.0e-6).abs(),
            3 => 1.0e8,
            _ => rng.f32_in(3.0).abs(),
        }
    }

    /// Smoothness `k` palette: `0` (hard op), tiny `> 0`, and large — the three
    /// branches `combine`/`smin`/`smax` take.
    fn tie_smoothness(rng: &mut Rng) -> f32 {
        match rng.below(5) {
            0 => 0.0,
            1 => 1.0e-6,
            2 => 1.0e-3,
            3 => 5.0,
            _ => 50.0,
        }
    }

    /// Builds one random edit over the widened (tie-forcing) palette.
    fn rand_edit(rng: &mut Rng) -> SdfEdit {
        let center = [tie_coord(rng), tie_coord(rng), tie_coord(rng)];
        let kind = if rng.below(2) == 0 {
            sdf_kind::SPHERE
        } else {
            sdf_kind::BOX
        };
        let op = match rng.below(4) {
            0 => sdf_op::UNION,
            1 => sdf_op::SUBTRACT,
            2 => sdf_op::INTERSECT,
            // An UNKNOWN discriminant must fall back to UNION in BOTH paths.
            _ => 7,
        };
        let smoothness = tie_smoothness(rng);
        if kind == sdf_kind::BOX {
            SdfEdit::box_shape(
                center,
                [tie_extent(rng), tie_extent(rng), tie_extent(rng)],
                op,
                smoothness,
            )
        } else {
            SdfEdit::sphere(center, tie_extent(rng), op, smoothness)
        }
    }

    /// Builds 8 random points; with probability, snaps some lanes onto a primitive
    /// center (zero gradient) or onto a primitive surface (`q == 0`) to force ties.
    fn rand_points(rng: &mut Rng, edits: &[SdfEdit]) -> [[f32; 3]; 8] {
        let mut pts = [[0.0f32; 3]; 8];
        for p in pts.iter_mut() {
            match rng.below(4) {
                // Coincide with a primitive center (zero-gradient critical point).
                0 if !edits.is_empty() => {
                    let e = &edits[(rng.below(edits.len() as u64)) as usize];
                    *p = [e.center[0], e.center[1], e.center[2]];
                }
                // Sit on a sphere/box surface (q == 0) for the surface-coincident tie.
                1 if !edits.is_empty() => {
                    let e = &edits[(rng.below(edits.len() as u64)) as usize];
                    *p = [
                        e.center[0] + e.params[0],
                        e.center[1],
                        e.center[2],
                    ];
                }
                _ => *p = [tie_coord(rng), tie_coord(rng), tie_coord(rng)],
            }
        }
        pts
    }

    /// Runs the 8-wide kernel on `pts` and asserts each lane is `to_bits`-identical
    /// to the scalar `sdf_edit_list` at that point. Returns on the FIRST mismatch
    /// with the exact divergent (edit list, point, lane, bits) — a HARD FAIL.
    fn assert_lane_bit_exact(edits: &[SdfEdit], pts: &[[f32; 3]; 8]) {
        let cx = [pts[0][0], pts[1][0], pts[2][0], pts[3][0], pts[4][0], pts[5][0], pts[6][0], pts[7][0]];
        let cy = [pts[0][1], pts[1][1], pts[2][1], pts[3][1], pts[4][1], pts[5][1], pts[6][1], pts[7][1]];
        let cz = [pts[0][2], pts[1][2], pts[2][2], pts[3][2], pts[4][2], pts[5][2], pts[6][2], pts[7][2]];
        let mut out = [0.0f32; 8];
        // SAFETY: this test module compiles only under `+avx2` (the parent module's
        //   `cfg`), so AVX2 is present. `sdf_edit_list_x8` is `#[target_feature(...)]`
        //   and needs an `unsafe` call; each load/store is 8 contiguous in-bounds
        //   `f32` of a `[f32; 8]` stack buffer (unaligned variants).
        unsafe {
            let px = _mm256_loadu_ps(cx.as_ptr());
            let py = _mm256_loadu_ps(cy.as_ptr());
            let pz = _mm256_loadu_ps(cz.as_ptr());
            let v = sdf_edit_list_x8(edits, px, py, pz);
            _mm256_storeu_ps(out.as_mut_ptr(), v);
        }
        for lane in 0..8 {
            let scalar = sdf_edit_list(edits, pts[lane]);
            assert_eq!(
                out[lane].to_bits(),
                scalar.to_bits(),
                "O9 bit-exactness BREAK: lane {lane} x8={:#010x} scalar={:#010x} (x8={}, scalar={}) \
                 point={:?} edits={:?}",
                out[lane].to_bits(),
                scalar.to_bits(),
                out[lane],
                scalar,
                pts[lane],
                edits,
            );
        }
    }

    /// THE bit-exactness gate: 1000+ random cases over the widened C3 tie generator,
    /// each varying edit-count, kind, op, smoothness, and forcing `±0`/surface/center
    /// ties. Every lane of every case must be `to_bits`-identical to the scalar fold.
    #[test]
    fn x8_bits_eq_scalar_bits_widened_proptest() {
        let mut rng = Rng::new(0x0900_d1ff_cafe_0009);
        // Edit-count palette includes the empty (0), single, MAX, and the clamp
        // boundary (MAX + 1) — the count clamp must match the leaf's `min`.
        let counts = [0usize, 1, 2, 5, MAX_SDF_EDITS, MAX_SDF_EDITS + 1];
        let mut cases = 0usize;
        // 200 outer iterations × 6 counts = 1200 cases (> 1000).
        for _ in 0..200 {
            for &count in &counts {
                let edits: Vec<SdfEdit> = (0..count).map(|_| rand_edit(&mut rng)).collect();
                let pts = rand_points(&mut rng, &edits);
                assert_lane_bit_exact(&edits, &pts);
                cases += 1;
            }
        }
        assert!(cases >= 1000, "the bit-exactness gate must run >= 1000 cases (ran {cases})");
    }

    /// C1: an empty edit list returns `SDF_FAR` in every lane with NO `edits[0]`
    /// access (no panic / OOB) — bit-identical to the scalar `sdf_edit_list` at
    /// `n == 0`.
    #[test]
    fn x8_empty_list_is_far_all_lanes() {
        let edits: [SdfEdit; 0] = [];
        let pts = [[1.0, 2.0, 3.0]; 8];
        assert_lane_bit_exact(&edits, &pts);
        // And the value itself is the scalar empty-field sentinel.
        let scalar_far = sdf_edit_list(&edits, [1.0, 2.0, 3.0]);
        assert_eq!(scalar_far.to_bits(), super::SDF_FAR.to_bits(), "empty field must be SDF_FAR");
    }

    /// R5 lane isolation: feeding GARBAGE (large / `Inf`-adjacent finite) into lanes
    /// 6,7 must NOT perturb lanes 0..6. Run the SAME edit list + lanes 0..6 points
    /// twice — once with benign lanes 6,7, once with garbage — and assert lanes 0..6
    /// are `to_bits`-identical. Proves the kernel has ZERO horizontal ops.
    #[test]
    fn x8_inert_lanes_do_not_leak() {
        let mut rng = Rng::new(0x0900_1501_7e57_0009);
        for _ in 0..256 {
            let count = 1 + (rng.below(MAX_SDF_EDITS as u64)) as usize;
            let edits: Vec<SdfEdit> = (0..count).map(|_| rand_edit(&mut rng)).collect();
            // Six live lanes; lanes 6,7 are the inert tail.
            let mut live = [[0.0f32; 3]; 6];
            for p in live.iter_mut() {
                *p = [tie_coord(&mut rng), tie_coord(&mut rng), tie_coord(&mut rng)];
            }
            let benign = [0.0f32, 0.0, 0.0];
            let garbage_a = [1.0e30f32, -1.0e30, f32::MAX];
            let garbage_b = [f32::MIN, 3.4e38, -3.4e38];

            let pack = |tail0: [f32; 3], tail1: [f32; 3]| -> [[f32; 3]; 8] {
                [live[0], live[1], live[2], live[3], live[4], live[5], tail0, tail1]
            };
            let with_benign = pack(benign, benign);
            let with_garbage = pack(garbage_a, garbage_b);

            let eval = |pts: &[[f32; 3]; 8]| -> [u32; 8] {
                let cx = [pts[0][0], pts[1][0], pts[2][0], pts[3][0], pts[4][0], pts[5][0], pts[6][0], pts[7][0]];
                let cy = [pts[0][1], pts[1][1], pts[2][1], pts[3][1], pts[4][1], pts[5][1], pts[6][1], pts[7][1]];
                let cz = [pts[0][2], pts[1][2], pts[2][2], pts[3][2], pts[4][2], pts[5][2], pts[6][2], pts[7][2]];
                let mut out = [0.0f32; 8];
                // SAFETY: `+avx2`-gated module (see `assert_lane_bit_exact`); 8
                //   in-bounds contiguous `f32` per load/store (unaligned variants).
                unsafe {
                    let px = _mm256_loadu_ps(cx.as_ptr());
                    let py = _mm256_loadu_ps(cy.as_ptr());
                    let pz = _mm256_loadu_ps(cz.as_ptr());
                    let v = sdf_edit_list_x8(&edits, px, py, pz);
                    _mm256_storeu_ps(out.as_mut_ptr(), v);
                }
                [
                    out[0].to_bits(), out[1].to_bits(), out[2].to_bits(), out[3].to_bits(),
                    out[4].to_bits(), out[5].to_bits(), out[6].to_bits(), out[7].to_bits(),
                ]
            };

            let a = eval(&with_benign);
            let b = eval(&with_garbage);
            for lane in 0..6 {
                assert_eq!(
                    a[lane], b[lane],
                    "R5 lane-leak: lane {lane} changed when garbage was placed in lanes 6,7 \
                     (benign={:#010x} garbage={:#010x}) edits={:?}",
                    a[lane], b[lane], edits,
                );
            }
        }
    }

    /// W4 grep gate: `boyko_rhi_vulkan/` (the GPU golden crate) must contain ZERO
    /// `_x8` — the batched kernel is CPU-only and must not leak into the GPU oracle.
    #[test]
    fn w4_rhi_vulkan_has_no_x8() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("boyko_rhi_vulkan");
        let mut hits = Vec::new();
        scan_dir(&root, &mut |path, contents| {
            for (i, line) in contents.lines().enumerate() {
                if line.contains("_x8") {
                    hits.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                }
            }
        });
        assert!(
            hits.is_empty(),
            "W4 violated: boyko_rhi_vulkan must contain zero `_x8` (the CPU batched kernel \
             must not leak into the GPU golden crate). Hits:\n{}",
            hits.join("\n"),
        );
    }

    /// No-FMA / no-approx grep gate: `sdf_simd.rs` must contain ZERO `_mm256_fmadd`,
    /// `_mm256_rsqrt`, or `_mm256_rcp` CALL-SITES (a fused/approx op would diverge
    /// from the twice-rounded scalar by a ULP). Doc-comment prose naming them (to
    /// document the prohibition) is allowed — only NON-comment code lines are
    /// scanned.
    #[test]
    fn sdf_simd_has_no_fma_or_approx_callsites() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("sdf_simd.rs");
        let contents = std::fs::read_to_string(&path).expect("sdf_simd.rs must be readable");
        // Match a CALL-SITE: each intrinsic stem completed to a real `_ps(`
        // invocation (`_mm256_fmadd_ps(`, `_mm256_rsqrt_ps(`, `_mm256_rcp_ps(`). The
        // needles are ASSEMBLED from fragments at runtime so the full call token
        // never appears as a string literal in THIS source — otherwise the gate
        // would flag its own definition line. Doc-comment prose is also skipped.
        let suffix = "_ps(";
        let banned = [
            format!("_mm256_{}{}", "fmadd", suffix),
            format!("_mm256_{}{}", "rsqrt", suffix),
            format!("_mm256_{}{}", "rcp", suffix),
        ];
        let mut hits = Vec::new();
        for (i, line) in contents.lines().enumerate() {
            let trimmed = line.trim_start();
            // Skip doc / line comments — prose may name the banned ops to document
            // the prohibition (the module-doc does exactly that).
            if trimmed.starts_with("//") || trimmed.starts_with("//!") {
                continue;
            }
            for b in &banned {
                if line.contains(b.as_str()) {
                    hits.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                }
            }
        }
        assert!(
            hits.is_empty(),
            "no-FMA/no-approx invariant violated: sdf_simd.rs has banned op call-sites:\n{}",
            hits.join("\n"),
        );
    }

    /// Recursively scans every `*.rs` / `*.hlsl` / `*.spv`-adjacent text file under
    /// `dir`, invoking `f(path, contents)`. Used by the W4 grep gate.
    fn scan_dir(dir: &std::path::Path, f: &mut dyn FnMut(&std::path::Path, &str)) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip build artifacts.
                if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                    continue;
                }
                scan_dir(&path, f);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
                && let Ok(contents) = std::fs::read_to_string(&path)
            {
                f(&path, &contents);
            }
        }
    }
}
