//! The M2 FIRST-CROSSING CERTIFICATE's range math (OPEN-ITEMS 16, `oi1516/cut.md` D16-2).
//!
//! The M2 brick path commits a hit found by the trilinear reconstruction and refined onto the exact
//! field. The reconstruction is not a lower bound on the field (a sub-`R_MIN` subtractor, a thin
//! shell, a small primitive in front of a larger one), so the refine can converge on a surface
//! BEHIND the first one. The certificate proves the exact field positive on the uncertified
//! segment of the ray before the hit, from one value and one slope per edit at each segment end:
//!
//! - every primitive kind (sphere, box with `h >= 0`, capsule) is the exact signed distance of a
//!   CONVEX set, hence a convex function on R³ and along any ray `g(t) = f(o + rd·t)`;
//! - a convex `g` lies above its tangent at each end, so on `[0, L]` it is at least the lower of
//!   the two tangents' crossing ([`convex_seg_lo_body`]) — exact (`= g(L)`) when the target is
//!   approached from outside (`g'(L) <= 0`), however grazing;
//! - the CSG fold is monotone in each operand (the subtrahend after its negation; polynomial
//!   `smin`'s partials are `h` and `1 − h`, both in `[0, 1]`), so it carries `[lo, hi]` intervals.
//!
//! The per-edit VALUE + SLOPE leaves ([`sphere_value_slope_body`], [`box_value_slope_body`],
//! [`capsule_value_slope_body`]) and the segment bound ([`convex_seg_lo_body`]) are authored here
//! ONCE: the `f32` Eval instance is the host certificate (`boyko_rhi_vulkan::goldens`), the `Emit`
//! instance is the HLSL `sdf_m2_certify.hlsli` splices between its GENERATED sentinels. The edit
//! loop, the kind / op dispatch and the subdivision driver stay hand-written on both sides (runtime
//! control flow over the packed edit list, the `sdf_field.hlsli` `combine` precedent).
//!
//! References: Hart, "Sphere tracing" (The Visual Computer 12(10), 1996) — the Lipschitz ball the
//! march certifies its own steps with; Keinert et al., "Enhanced Sphere Tracing" (STAG 2014) — the
//! over-relaxed step the certificate's start point accounts for; Galin et al., "Segment Tracing
//! Using Local Lipschitz Bounds" (CGF 39(2), 2020) — segment-wise bounds, here sharpened by
//! primitive convexity; Knoll et al., "Fast Ray Tracing of Arbitrary Implicit Surfaces with
//! Interval and Affine Arithmetic" (CGF 28(1), 2009) — the interval certify-or-subdivide scheme.

use crate::field::CAPSULE_DENOM_EPS;
use crate::scalar::{self, FieldScalar};

/// The floor on a distance a slope divides by: at a sphere's centre or on a capsule's axis the
/// offset vector is zero, the function is at its minimum, and `0` (the result of `0 / floor`) is a
/// valid subgradient.
pub const RANGE_LEN_FLOOR: f32 = 1.0e-30;

/// The (negative) floor on the slope difference the tangent crossing divides by.
pub const RANGE_SLOPE_GAP_FLOOR: f32 = -1.0e-30;

/// `+1` for `x >= 0`, else `-1` — the sign of an offset as a subgradient of `|x|` (at `x == 0` any
/// value in `[-1, 1]` is a subgradient; `+1` is one).
#[inline]
fn sign_or_one<S: FieldScalar<Vec3 = [S; 3]>>(x: S) -> S {
    S::select(x.ge(S::lit(0.0)), S::lit(1.0), S::lit(-1.0))
}

/// A sphere edit's `[value, slope]` at `p` along the unit ray direction `rd`: `length(p − c) − r`
/// and the directional derivative `dot(q, rd) / |q|` (`q = p − c`).
#[inline]
pub fn sphere_value_slope_body<S: FieldScalar<Vec3 = [S; 3]>>(p: [S; 3], rd: [S; 3], c: [S; 3], r: S) -> [S; 2] {
    let q = scalar::v_sub(p, c);
    let len = scalar::v_len(q);
    let value = len.sub(r);
    let slope = scalar::v_dot(q, rd).div(len.max(S::lit(RANGE_LEN_FLOOR)));
    [value, slope]
}

/// An axis-aligned box edit's `[value, slope]` at `p` along `rd` (centre `c`, half-extents
/// `h >= 0`): the IQ exact distance and a subgradient — outside (`|max(q, 0)| > 0`) the gradient
/// `sign(p − c) ⊙ max(q, 0) / |max(q, 0)|`, inside `sign(p − c)_k · e_k` of the argmax `k` of `q`
/// (`q = |p − c| − h`; ties take the lower axis).
#[inline]
pub fn box_value_slope_body<S: FieldScalar<Vec3 = [S; 3]>>(p: [S; 3], rd: [S; 3], c: [S; 3], h: [S; 3]) -> [S; 2] {
    let zero = S::lit(0.0);
    let pc = scalar::v_sub(p, c);
    let q = scalar::v_sub(scalar::v_abs(pc), h);
    let m = scalar::v_max0(q);
    let len = scalar::v_len(m);
    let q_max = q[0].max(q[1].max(q[2]));
    let value = len.add(q_max.min(zero));
    let s = [sign_or_one(pc[0]), sign_or_one(pc[1]), sign_or_one(pc[2])];
    let num = s[0]
        .mul(m[0])
        .mul(rd[0])
        .add(s[1].mul(m[1]).mul(rd[1]))
        .add(s[2].mul(m[2]).mul(rd[2]));
    let outside = num.div(len.max(S::lit(RANGE_LEN_FLOOR)));
    let in_x = s[0].mul(rd[0]);
    let in_y = s[1].mul(rd[1]);
    let in_z = s[2].mul(rd[2]);
    let inside = S::select(q[0].ge(q[1].max(q[2])), in_x, S::select(q[1].ge(q[2]), in_y, in_z));
    [value, S::select(len.gt(zero), outside, inside)]
}

/// A capsule edit's `[value, slope]` at `p` along `rd` (segment `[a, b]`, cap radius `r`): the IQ
/// exact distance and the gradient `w / |w|`, `w = (p − a) − (b − a)·h` the offset from the nearest
/// segment point (the same `h` clamp and `CAPSULE_DENOM_EPS` guard as the field's `sd_capsule`).
#[inline]
pub fn capsule_value_slope_body<S: FieldScalar<Vec3 = [S; 3]>>(
    p: [S; 3],
    rd: [S; 3],
    a: [S; 3],
    b: [S; 3],
    r: S,
) -> [S; 2] {
    let pa = scalar::v_sub(p, a);
    let ba = scalar::v_sub(b, a);
    let denom = scalar::v_dot(ba, ba).max(S::lit(CAPSULE_DENOM_EPS));
    let h = scalar::v_dot(pa, ba).div(denom).clamp01();
    let w = scalar::v_sub(pa, scalar::v_scale(ba, h));
    let len = scalar::v_len(w);
    [len.sub(r), scalar::v_dot(w, rd).div(len.max(S::lit(RANGE_LEN_FLOOR)))]
}

/// A lower bound of a convex `g` on a segment of length `len`, from its values `va`, `vb` and
/// slopes `sa`, `sb` at the two ends: `va` when `sa >= 0` (non-decreasing from the start), `vb`
/// when `sb <= 0` (non-increasing to the end), else the value of the start tangent at its crossing
/// with the end tangent (`(vb − va − sb·len) / (sa − sb)`, clamped to `[0, len]`) — and never above
/// either end value.
#[inline]
pub fn convex_seg_lo_body<S: FieldScalar<Vec3 = [S; 3]>>(va: S, vb: S, sa: S, sb: S, len: S) -> S {
    let zero = S::lit(0.0);
    let gap = sa.sub(sb).min(S::lit(RANGE_SLOPE_GAP_FLOOR));
    let x = vb.sub(va).sub(sb.mul(len)).div(gap).max(zero).min(len);
    let tangent = va.add(sa.mul(x));
    let lo = S::select(sa.ge(zero), va, S::select(sb.le(zero), vb, tangent));
    lo.min(va).min(vb)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sampled convex function's minimum on `[0, len]` (the bound must never exceed it).
    fn sampled_min(g: impl Fn(f32) -> f32, len: f32) -> f32 {
        (0..=4096).map(|i| g(len * i as f32 / 4096.0)).fold(f32::INFINITY, f32::min)
    }

    #[test]
    fn convex_seg_lo_four_slope_cases() {
        // sa >= 0: non-decreasing from the start → the start value.
        assert_eq!(convex_seg_lo_body::<f32>(0.5, 0.9, 0.2, 0.6, 1.0), 0.5);
        // sb <= 0: non-increasing to the end → the end value.
        assert_eq!(convex_seg_lo_body::<f32>(0.9, 0.5, -0.6, -0.2, 1.0), 0.5);
        // sa < 0 < sb: the tangents' crossing. g(t) = (t - 0.5)² + 0.1 on [0, 1]: va = vb = 0.35,
        // sa = -1, sb = 1 → crossing at 0.5, tangent value 0.35 - 0.5 = -0.15 <= the true 0.1.
        let lo = convex_seg_lo_body::<f32>(0.35, 0.35, -1.0, 1.0, 1.0);
        assert!((lo - -0.15).abs() < 1e-6, "{lo}");
        assert!(lo <= 0.1);
        // Never above either end.
        assert!(convex_seg_lo_body::<f32>(0.2, 0.3, -1e-9, 1e-9, 2.0) <= 0.2);
    }

    #[test]
    fn convex_seg_lo_is_exact_on_an_outside_sphere_approach() {
        // A ray toward a unit sphere's centre from 3 away: g(t) = 3 - t - 1 on [0, 1.5].
        let c = [0.0_f32, 0.0, 0.0];
        let rd = [0.0_f32, 0.0, -1.0];
        let a = sphere_value_slope_body::<f32>([0.0, 0.0, 3.0], rd, c, 1.0);
        let b = sphere_value_slope_body::<f32>([0.0, 0.0, 1.5], rd, c, 1.0);
        assert_eq!(convex_seg_lo_body::<f32>(a[0], b[0], a[1], b[1], 1.5), b[0]);
        assert_eq!(b[0], 0.5);
    }

    #[test]
    fn value_slope_leaves_bound_their_sampled_minima() {
        let mut seed = 0x0516_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        };
        for _ in 0..2000 {
            let o = [next() * 2.0, next() * 2.0, next() * 2.0];
            let d = scalar::v_scale([next(), next(), next()], 1.0);
            let dl = scalar::v_len(d).max(1e-3);
            let rd = [d[0] / dl, d[1] / dl, d[2] / dl];
            let len = (next() + 1.0) * 1.5;
            let at = |t: f32| [o[0] + rd[0] * t, o[1] + rd[1] * t, o[2] + rd[2] * t];
            let c = [next() * 0.5, next() * 0.5, next() * 0.5];
            let h = [next().abs() * 0.6, next().abs() * 0.6, next().abs() * 0.6];
            let cb = [next(), next(), next()];
            let r = next().abs() * 0.5 + 0.05;
            let kinds: [&dyn Fn([f32; 3]) -> [f32; 2]; 3] = [
                &|p| sphere_value_slope_body::<f32>(p, rd, c, r),
                &|p| box_value_slope_body::<f32>(p, rd, c, h),
                &|p| capsule_value_slope_body::<f32>(p, rd, c, cb, r),
            ];
            for f in kinds {
                let (va, vb) = (f(at(0.0)), f(at(len)));
                let lo = convex_seg_lo_body::<f32>(va[0], vb[0], va[1], vb[1], len);
                let min = sampled_min(|t| f(at(t))[0], len);
                assert!(lo <= min + 1e-5, "lo {lo} above the sampled minimum {min}");
                assert!(lo <= va[0].min(vb[0]));
            }
        }
    }
}
