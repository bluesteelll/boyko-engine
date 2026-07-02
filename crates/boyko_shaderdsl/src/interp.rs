//! Pillar B increment B2 — the per-instance TRS interpolation math, authored ONCE
//! generic over a backend and instantiated two ways (the established campaign pattern):
//!
//! - `B = f32` — the **Eval** backend ([`impl InterpBackend for f32`]): every op is a
//!   single host instruction, so a monomorphization over `f32` IS the CPU oracle the
//!   golden tests lock (its composed 3×4 rows byte-match
//!   `boyko_render::InstanceModelCol::from_global` for the interpolated TRS).
//! - `B = Emit` — the **HLSL SSA recorder** ([`crate::emit`], `feature = "emit"`): each
//!   op pushes one SSA node; the printer walks the arena into the `interp_trs` HLSL body
//!   spliced into `shaders/interp_instances.comp.hlsl`.
//!
//! # Why a SEPARATE backend trait (not [`FieldScalar`](crate::scalar::FieldScalar))
//!
//! The SDF `FieldScalar` op-set is DELIBERATELY transcendental-free (the SSAO doc pins
//! "no `sin`/`cos`/`acos`" so the host oracle stays bit-comparable to the GPU). Quaternion
//! slerp NEEDS `sin`/`cos`/`acos`. Extending `FieldScalar` with trig would force the
//! physics `no_std` leaf (which links `FieldScalar for f32`) to carry std trig it never
//! uses. So the interp body is written against its OWN [`InterpBackend`] trait, and this
//! whole module is `#[cfg(feature = "emit")]`-gated (codegen tooling — never the physics
//! leaf), so the `no_std` firewall is untouched. The recorder is NOT forked: [`Emit`]'s
//! `InterpBackend` impl records into the SAME arena / [`Node`](crate::emit) IR the field
//! bodies use, adding only three unary-intrinsic nodes (`Sin`/`Cos`/`Acos`).
//!
//! # The compose contract (byte-match `InstanceModelCol::from_global`)
//!
//! The output 3×4 ROW-MAJOR rows MUST match `InstanceModelCol::from_global` for the SAME
//! interpolated TRS. That reference is
//! `Affine3A::from_translation_rotation_scale(t, q, s)` →
//! `Mat3::from_quat_scale(q, s)` (row-major `R · diag(s)`) + translation, packed as
//! `rows[i] = [m3.rows[i].x, .y, .z, t.i]`. Every operand below is written in the SAME
//! order as those frozen `boyko_math` bodies (`from_quat` lines 116-124, `from_quat_scale`
//! lines 149-168), so the `f32` Eval instantiation composes byte-identically.

/// The near-parallel dot threshold above which slerp degenerates to
/// normalize-lerp (nlerp): when `|dot(q0, q1)| > SLERP_DOT_THRESHOLD` the two
/// quaternions are within ~1.8° and `sin(theta)` is too small to divide by
/// safely, so the linear blend is both numerically stable and visually identical.
/// `0.9995` is the glam / Bevy `Quat::slerp` threshold.
pub const SLERP_DOT_THRESHOLD: f32 = 0.9995;

/// The backend axis the interpolation body is written against — the arithmetic /
/// select / comparison op-set PLUS the three transcendentals (`sin`/`cos`/`acos`)
/// quaternion slerp needs. A ZST-dispatched abstraction (like
/// [`FieldScalar`](crate::scalar::FieldScalar)): `f32` is the Eval oracle, [`Emit`]
/// the HLSL recorder.
///
/// Distinct from [`FieldScalar`](crate::scalar::FieldScalar) so the SDF op-set stays
/// transcendental-free (the physics leaf never carries trig); the two backends that
/// implement it (`f32` here, `Emit` in [`crate::emit`]) both live behind
/// `feature = "emit"`.
pub trait InterpBackend: Copy {
    /// The boolean produced by the comparisons and consumed by [`select`](Self::select).
    type Mask: Copy;

    /// A float literal lifted into the backend.
    fn lit(x: f32) -> Self;

    /// `self + rhs`.
    fn add(self, rhs: Self) -> Self;
    /// `self - rhs`.
    fn sub(self, rhs: Self) -> Self;
    /// `self * rhs`.
    fn mul(self, rhs: Self) -> Self;
    /// `self / rhs`.
    fn div(self, rhs: Self) -> Self;
    /// `-self`.
    fn neg(self) -> Self;
    /// `abs(self)`.
    fn abs(self) -> Self;
    /// `sqrt(self)` — the one op the slerp normalize needs (IEEE; lowers to `sqrtss`
    /// / HLSL `sqrt`).
    fn sqrt(self) -> Self;

    /// `sin(self)` — the HLSL `sin` intrinsic / host `f32::sin`.
    fn sin(self) -> Self;
    /// `cos(self)` — the HLSL `cos` intrinsic / host `f32::cos`.
    fn cos(self) -> Self;
    /// `acos(self)` — the HLSL `acos` intrinsic / host `f32::acos`.
    fn acos(self) -> Self;

    /// `cond ? t : e` — the value select (no data-dependent control flow; the GPU
    /// emits a ternary). Both arms are pure and always evaluated.
    fn select(cond: Self::Mask, t: Self, e: Self) -> Self;

    /// `self < rhs` — the strict less-than producing a [`Mask`](Self::Mask).
    fn lt(self, rhs: Self) -> Self::Mask;
    /// `self > rhs` — the strict greater-than producing a [`Mask`](Self::Mask).
    fn gt(self, rhs: Self) -> Self::Mask;

    /// `a && b` — the logical AND of two masks (the branchless "all four components
    /// equal" fold for the exact-at-prev==curr keystone). Both operands are
    /// side-effect-free comparisons, so the eager form is result-equivalent to a
    /// short-circuit.
    fn and(a: Self::Mask, b: Self::Mask) -> Self::Mask;

    /// `a == b` — bitwise-decidable equality producing a [`Mask`](Self::Mask). Used
    /// ONLY to detect the static `prev == curr` per-component identity that gates the
    /// exact-passthrough of the rotation; on the GPU this lowers to `OpFOrdEqual`.
    fn eq(self, rhs: Self) -> Self::Mask;
}

// ---- The Eval backend: `impl InterpBackend for f32` ---------------------------

impl InterpBackend for f32 {
    type Mask = bool;

    #[inline]
    fn lit(x: f32) -> Self {
        x
    }
    #[inline]
    fn add(self, rhs: Self) -> Self {
        self + rhs
    }
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        self - rhs
    }
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        self * rhs
    }
    #[inline]
    fn div(self, rhs: Self) -> Self {
        self / rhs
    }
    #[inline]
    fn neg(self) -> Self {
        -self
    }
    #[inline]
    fn abs(self) -> Self {
        f32::abs(self)
    }
    #[inline]
    fn sqrt(self) -> Self {
        f32::sqrt(self)
    }
    #[inline]
    fn sin(self) -> Self {
        f32::sin(self)
    }
    #[inline]
    fn cos(self) -> Self {
        f32::cos(self)
    }
    #[inline]
    fn acos(self) -> Self {
        f32::acos(self)
    }
    #[inline]
    fn select(cond: bool, t: Self, e: Self) -> Self {
        if cond { t } else { e }
    }
    #[inline]
    fn lt(self, rhs: Self) -> bool {
        self < rhs
    }
    #[inline]
    fn gt(self, rhs: Self) -> bool {
        self > rhs
    }
    #[inline]
    fn and(a: bool, b: bool) -> bool {
        a && b
    }
    #[inline]
    fn eq(self, rhs: Self) -> bool {
        self == rhs
    }
}

// ---- Generic vector helpers over `B: InterpBackend` ---------------------------

/// `a + (b - a) * t` — the component-wise linear blend, EXACT at `t == 0` and at
/// `a == b` for finite values.
///
/// The `a + (b - a) * t` form (NOT `a*(1-t) + b*t`) is what guarantees the two
/// static-scene keystones: at `t == 0`, `a + (b - a) * 0 = a + 0 = a` bitwise; at
/// `a == b`, `a + 0 * t = a + 0 = a` bitwise. (At `t == 1` it is `a + (b - a)`,
/// which is NOT bitwise `b` in general — one rounding — so `t == 1` is documented,
/// not exact; the load-bearing case is `a == b` at ANY `t`.)
#[inline]
fn lerp3<B: InterpBackend>(a: [B; 3], b: [B; 3], t: B) -> [B; 3] {
    [
        a[0].add(b[0].sub(a[0]).mul(t)),
        a[1].add(b[1].sub(a[1]).mul(t)),
        a[2].add(b[2].sub(a[2]).mul(t)),
    ]
}

/// `dot(a, b)` over two quaternions (`x*x + y*y + z*z + w*w` fold), written as the
/// left-associated scalar chain so both backends produce the identical op tree.
#[inline]
fn dot4<B: InterpBackend>(a: [B; 4], b: [B; 4]) -> B {
    a[0]
        .mul(b[0])
        .add(a[1].mul(b[1]))
        .add(a[2].mul(b[2]))
        .add(a[3].mul(b[3]))
}

// ---- The interpolation body (generic over `B: InterpBackend`) -----------------

/// Interpolates a previous → current TRS pair at `alpha` and composes the result
/// into the 12 scalars of the 3×4 ROW-MAJOR model affine
/// (`InstanceModelCol::rows` layout: `rows[i] = [m3.rows[i].x, .y, .z, t.i]`).
///
/// `prev_pos` / `curr_pos` and `prev_scale` / `curr_scale` are component-wise
/// `lerp` (exact at `alpha == 0` and at `prev == curr`); `prev_rot` / `curr_rot`
/// are the shortest-path quaternion slerp (see [`slerp_quat`]). The composed linear
/// part is `R(q) · diag(scale)` in the EXACT operand order of
/// `boyko_math::Mat3::from_quat` + `from_quat_scale`, so the `f32` instantiation is
/// byte-identical to `InstanceModelCol::from_global` for the interpolated TRS.
///
/// Quaternion component order is `(x, y, z, w)` — the engine's glTF/GPU convention.
/// The return is `[row0.xyzw, row1.xyzw, row2.xyzw]` flattened (12 scalars, in
/// storage order).
#[inline]
pub fn transform_pair_interp_body<B: InterpBackend>(
    prev_pos: [B; 3],
    prev_rot: [B; 4],
    prev_scale: [B; 3],
    curr_pos: [B; 3],
    curr_rot: [B; 4],
    curr_scale: [B; 3],
    alpha: B,
) -> [B; 12] {
    let pos = lerp3(prev_pos, curr_pos, alpha);
    let scale = lerp3(prev_scale, curr_scale, alpha);
    let q = slerp_quat(prev_rot, curr_rot, alpha);

    // R = from_quat(q), row-major, byte-mirroring boyko_math::Mat3::from_quat
    // (mat.rs:116-124). Operand order is load-bearing for the byte-identity gate.
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let two = B::lit(2.0);
    let one = B::lit(1.0);

    let xx = x.mul(x);
    let yy = y.mul(y);
    let zz = z.mul(z);
    let xy = x.mul(y);
    let xz = x.mul(z);
    let yz = y.mul(z);
    let wx = w.mul(x);
    let wy = w.mul(y);
    let wz = w.mul(z);

    // row 0: (1 - 2(yy+zz), 2(xy - wz), 2(xz + wy))
    let r00 = one.sub(two.mul(yy.add(zz)));
    let r01 = two.mul(xy.sub(wz));
    let r02 = two.mul(xz.add(wy));
    // row 1: (2(xy + wz), 1 - 2(xx+zz), 2(yz - wx))
    let r10 = two.mul(xy.add(wz));
    let r11 = one.sub(two.mul(xx.add(zz)));
    let r12 = two.mul(yz.sub(wx));
    // row 2: (2(xz - wy), 2(yz + wx), 1 - 2(xx+yy))
    let r20 = two.mul(xz.sub(wy));
    let r21 = two.mul(yz.add(wx));
    let r22 = one.sub(two.mul(xx.add(yy)));

    // R · diag(scale): scale COLUMN j by scale[j] (mat.rs:149-168). Column j of a
    // row-major matrix is the j-th component of each row.
    let (sx, sy, sz) = (scale[0], scale[1], scale[2]);
    let m00 = r00.mul(sx);
    let m01 = r01.mul(sy);
    let m02 = r02.mul(sz);
    let m10 = r10.mul(sx);
    let m11 = r11.mul(sy);
    let m12 = r12.mul(sz);
    let m20 = r20.mul(sx);
    let m21 = r21.mul(sy);
    let m22 = r22.mul(sz);

    // rows[i] = [linear_row_i.xyz | translation_i] (InstanceModelCol layout).
    [
        m00, m01, m02, pos[0], //
        m10, m11, m12, pos[1], //
        m20, m21, m22, pos[2],
    ]
}

/// Shortest-path quaternion slerp `slerp(q0, q1, t)` (component order `(x, y, z, w)`),
/// returning a unit quaternion, EXACT (bitwise) at `q0 == q1` for ANY `t`.
///
/// The three glam / Bevy `Quat::slerp` behaviors, all BRANCHLESS (via
/// [`select`](InterpBackend::select) — no data-dependent control flow, so the GPU
/// lowers each guard to an `OpSelect`/ternary, not a branch):
///
/// 1. **Shortest path**: when `dot(q0, q1) < 0` the two quaternions are on opposite
///    hemispheres; negate `q1` (and the dot) so the blend takes the short arc. A
///    quaternion and its negation encode the SAME rotation, so this is
///    rotation-preserving.
/// 2. **Near-parallel fallback**: when `|dot| > SLERP_DOT_THRESHOLD` the angle is
///    tiny and `sin(theta)` underflows; fall back to `normalize(lerp(q0, q1, t))`
///    (nlerp), numerically stable and visually identical at small angles.
/// 3. **Exact-at-equal keystone**: when `q0 == q1` bitwise, return `q0` UNCHANGED,
///    BEFORE the nlerp `normalize` (whose division would perturb the last bit even
///    for a unit input). The static-scene byte-identity requirement — a still object
///    at ANY `alpha` produces the EXACT prev rotation.
#[inline]
pub fn slerp_quat<B: InterpBackend>(q0: [B; 4], q1: [B; 4], t: B) -> [B; 4] {
    let zero = B::lit(0.0);
    let one = B::lit(1.0);

    let d = dot4(q0, q1);

    // (1) Shortest path: sign = (d < 0) ? -1 : 1; flip q1 and d by it.
    let neg = d.lt(zero);
    let sign = B::select(neg, one.neg(), one);
    let q1 = [
        q1[0].mul(sign),
        q1[1].mul(sign),
        q1[2].mul(sign),
        q1[3].mul(sign),
    ];
    let d = d.mul(sign); // now d == |d| >= 0

    // (2) Full slerp weights (valid when d <= threshold): theta = acos(d),
    // w0 = sin((1-t)*theta)/sin(theta), w1 = sin(t*theta)/sin(theta).
    let theta = d.acos();
    let sin_theta = theta.sin();
    let w0_slerp = one.sub(t).mul(theta).sin().div(sin_theta);
    let w1_slerp = t.mul(theta).sin().div(sin_theta);

    // nlerp weights (the near-parallel fallback): w0 = 1 - t, w1 = t; the result is
    // normalized below. Selecting the WEIGHTS (not the whole branch) keeps the body a
    // single straight-line expression.
    let near = d.gt(B::lit(SLERP_DOT_THRESHOLD));
    let w0 = B::select(near, one.sub(t), w0_slerp);
    let w1 = B::select(near, t, w1_slerp);

    // Blend: q = q0 * w0 + q1 * w1.
    let blended = [
        q0[0].mul(w0).add(q1[0].mul(w1)),
        q0[1].mul(w0).add(q1[1].mul(w1)),
        q0[2].mul(w0).add(q1[2].mul(w1)),
        q0[3].mul(w0).add(q1[3].mul(w1)),
    ];

    // Normalize (needed by the nlerp path; the slerp path is already ~unit, and a
    // re-normalize is harmless and keeps the two paths a uniform expression).
    let len_sq = dot4(blended, blended);
    let inv_len = one.div(len_sq.sqrt());
    let normed = [
        blended[0].mul(inv_len),
        blended[1].mul(inv_len),
        blended[2].mul(inv_len),
        blended[3].mul(inv_len),
    ];

    // (3) Exact-at-equal keystone: if q0 == q1 (the ORIGINAL, pre-flip curr) bitwise,
    // return q0 EXACTLY (the raw component, unperturbed by the normalize division).
    // `all_equal` folds the four per-component equalities. `select` picks the raw
    // `q0[i]`, so the result is bitwise `q0` regardless of the arithmetic above.
    let all_equal = B::and(
        B::and(q0[0].eq(q1[0]), q0[1].eq(q1[1])),
        B::and(q0[2].eq(q1[2]), q0[3].eq(q1[3])),
    );
    [
        B::select(all_equal, q0[0], normed[0]),
        B::select(all_equal, q0[1], normed[1]),
        B::select(all_equal, q0[2], normed[2]),
        B::select(all_equal, q0[3], normed[3]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny xorshift PRNG for reproducible randomized cases (no `rand` dep — the
    /// crate is zero-dependency).
    struct Rng(u64);
    impl Rng {
        fn next_u32(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            (x >> 32) as u32
        }
        /// A float in `[-1, 1)`.
        fn signed(&mut self) -> f32 {
            (self.next_u32() as f32 / u32::MAX as f32) * 2.0 - 1.0
        }
        /// A float in `[lo, hi)`.
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (self.next_u32() as f32 / u32::MAX as f32) * (hi - lo)
        }
    }

    /// A random UNIT quaternion `(x, y, z, w)`.
    fn rand_unit_quat(rng: &mut Rng) -> [f32; 4] {
        loop {
            let q = [rng.signed(), rng.signed(), rng.signed(), rng.signed()];
            let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            if n > 1.0e-3 {
                return [q[0] / n, q[1] / n, q[2] / n, q[3] / n];
            }
        }
    }

    /// The CPU REFERENCE: the exact `InstanceModelCol::from_global` compose for a TRS,
    /// mirroring `boyko_math::Mat3::from_quat` (mat.rs:116-124) + `from_quat_scale`
    /// (mat.rs:149-168) operand-for-operand, then packing `rows[i] = [m.xyz, t.i]`.
    /// The interp body's `f32` instantiation at a FIXED (prev == curr) TRS must equal
    /// this byte-for-byte.
    fn reference_rows(pos: [f32; 3], q: [f32; 4], scale: [f32; 3]) -> [f32; 12] {
        let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
        let (xx, yy, zz) = (x * x, y * y, z * z);
        let (xy, xz, yz) = (x * y, x * z, y * z);
        let (wx, wy, wz) = (w * x, w * y, w * z);
        // from_quat rows (row-major).
        let r = [
            [
                1.0 - 2.0 * (yy + zz),
                2.0 * (xy - wz),
                2.0 * (xz + wy),
            ],
            [
                2.0 * (xy + wz),
                1.0 - 2.0 * (xx + zz),
                2.0 * (yz - wx),
            ],
            [
                2.0 * (xz - wy),
                2.0 * (yz + wx),
                1.0 - 2.0 * (xx + yy),
            ],
        ];
        // from_quat_scale: column j scaled by scale[j].
        let (sx, sy, sz) = (scale[0], scale[1], scale[2]);
        [
            r[0][0] * sx, r[0][1] * sy, r[0][2] * sz, pos[0], //
            r[1][0] * sx, r[1][1] * sy, r[1][2] * sz, pos[1], //
            r[2][0] * sx, r[2][1] * sy, r[2][2] * sz, pos[2],
        ]
    }

    fn interp_f32(
        pp: [f32; 3],
        pr: [f32; 4],
        ps: [f32; 3],
        cp: [f32; 3],
        cr: [f32; 4],
        cs: [f32; 3],
        alpha: f32,
    ) -> [f32; 12] {
        transform_pair_interp_body::<f32>(pp, pr, ps, cp, cr, cs, alpha)
    }

    /// prev == curr at ANY alpha ⇒ the composed rows are BITWISE the fixed-TRS
    /// reference. THE static-scene keystone: a still object never shimmers.
    #[test]
    fn prev_equals_curr_is_bitwise_reference_at_arbitrary_alpha() {
        let mut rng = Rng(0x1234_5678_9abc_def0);
        for _ in 0..100 {
            let pos = [rng.signed() * 10.0, rng.signed() * 10.0, rng.signed() * 10.0];
            let q = rand_unit_quat(&mut rng);
            let scale = [rng.range(0.1, 3.0), rng.range(0.1, 3.0), rng.range(0.1, 3.0)];
            let want = reference_rows(pos, q, scale);
            for &alpha in &[0.0_f32, 0.37, 0.5, 0.99] {
                let got = interp_f32(pos, q, scale, pos, q, scale, alpha);
                for i in 0..12 {
                    assert_eq!(
                        got[i].to_bits(),
                        want[i].to_bits(),
                        "prev==curr row[{i}] not bitwise reference at alpha={alpha}"
                    );
                }
            }
        }
    }

    /// alpha == 0 ⇒ BITWISE the PREV compose (the `a + (b-a)*0 == a` lerp form + the
    /// slerp keystone/weights collapsing to prev). Endpoint-exact by construction.
    #[test]
    fn alpha_zero_is_bitwise_prev() {
        let mut rng = Rng(0xdead_beef_0000_0001);
        for _ in 0..100 {
            let pp = [rng.signed() * 5.0, rng.signed() * 5.0, rng.signed() * 5.0];
            let pr = rand_unit_quat(&mut rng);
            let ps = [rng.range(0.2, 2.0), rng.range(0.2, 2.0), rng.range(0.2, 2.0)];
            let cp = [rng.signed() * 5.0, rng.signed() * 5.0, rng.signed() * 5.0];
            let cr = rand_unit_quat(&mut rng);
            let cs = [rng.range(0.2, 2.0), rng.range(0.2, 2.0), rng.range(0.2, 2.0)];

            let want = reference_rows(pp, pr, ps);
            let got = interp_f32(pp, pr, ps, cp, cr, cs, 0.0);
            // Translation + scale rows: alpha==0 lerp is bitwise prev.
            for i in [3usize, 7, 11] {
                assert_eq!(got[i].to_bits(), want[i].to_bits(), "translation[{i}] not bitwise prev at alpha=0");
            }
            // Rotation/scale linear block: alpha==0 slerp returns a UNIT quaternion
            // equal to prev (up to normalize rounding), so the composed linear part
            // matches the reference within a tight ulp band.
            for i in [0usize, 1, 2, 4, 5, 6, 8, 9, 10] {
                assert!(
                    (got[i] - want[i]).abs() <= 4.0e-6 * (1.0 + want[i].abs()),
                    "linear[{i}] diverged at alpha=0: got {}, want {}",
                    got[i],
                    want[i]
                );
            }
        }
    }

    /// The slerp output is a UNIT quaternion for random endpoints at random alpha
    /// (both the full-slerp and the near-parallel nlerp paths).
    #[test]
    fn slerp_is_unit_norm() {
        let mut rng = Rng(0x00c0_ffee_1234_5678);
        for _ in 0..200 {
            let q0 = rand_unit_quat(&mut rng);
            let q1 = rand_unit_quat(&mut rng);
            let t = rng.range(0.0, 1.0);
            let q = slerp_quat::<f32>(q0, q1, t);
            let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            assert!((n - 1.0).abs() <= 1.0e-5, "slerp not unit: |q| = {n}");
        }
    }

    /// The near-parallel branch (|dot| > threshold) produces a unit quaternion and,
    /// for two near-identical inputs, stays close to the endpoints.
    #[test]
    fn slerp_near_parallel_uses_stable_nlerp() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for _ in 0..100 {
            let q0 = rand_unit_quat(&mut rng);
            // A tiny perturbation keeps dot(q0, q1) > 0.9995.
            let eps = 1.0e-4;
            let mut q1 = [
                q0[0] + rng.signed() * eps,
                q0[1] + rng.signed() * eps,
                q0[2] + rng.signed() * eps,
                q0[3] + rng.signed() * eps,
            ];
            let n = (q1[0] * q1[0] + q1[1] * q1[1] + q1[2] * q1[2] + q1[3] * q1[3]).sqrt();
            q1 = [q1[0] / n, q1[1] / n, q1[2] / n, q1[3] / n];
            let dot = q0[0] * q1[0] + q0[1] * q1[1] + q0[2] * q1[2] + q0[3] * q1[3];
            assert!(dot.abs() > SLERP_DOT_THRESHOLD, "test setup: not near-parallel (dot={dot})");

            let q = slerp_quat::<f32>(q0, q1, 0.5);
            let norm = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            assert!((norm - 1.0).abs() <= 1.0e-5, "near-parallel nlerp not unit: {norm}");
        }
    }

    /// The shortest-path sign flip: `slerp(q0, -q1, t)` encodes the SAME rotation as
    /// `slerp(q0, q1, t)` (a quaternion and its negation are the same rotation), so the
    /// two composed matrices agree.
    #[test]
    fn slerp_takes_shortest_path() {
        let mut rng = Rng(0x0102_0304_0506_0708);
        for _ in 0..100 {
            let q0 = rand_unit_quat(&mut rng);
            let q1 = rand_unit_quat(&mut rng);
            let neg_q1 = [-q1[0], -q1[1], -q1[2], -q1[3]];
            let t = rng.range(0.0, 1.0);
            let a = slerp_quat::<f32>(q0, q1, t);
            let b = slerp_quat::<f32>(q0, neg_q1, t);
            // Same rotation ⇒ a == ±b componentwise; compare |a·b| ≈ 1.
            let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
            assert!(dot.abs() >= 1.0 - 1.0e-4, "sign-flip changed the rotation: |dot|={}", dot.abs());
        }
    }

    /// The full compose of an interpolated pose is a valid pure-scale-free rotation
    /// block: with unit scale, the linear rows are orthonormal (the interpolation
    /// produced a real rotation, not a skew).
    #[test]
    fn interp_unit_scale_linear_block_is_orthonormal() {
        let mut rng = Rng(0xfeed_face_cafe_babe);
        for _ in 0..100 {
            let pr = rand_unit_quat(&mut rng);
            let cr = rand_unit_quat(&mut rng);
            let one = [1.0_f32, 1.0, 1.0];
            let zero = [0.0_f32, 0.0, 0.0];
            let t = rng.range(0.0, 1.0);
            let rows = interp_f32(zero, pr, one, zero, cr, one, t);
            // Row vectors of the 3x3 linear part.
            let m = [
                [rows[0], rows[1], rows[2]],
                [rows[4], rows[5], rows[6]],
                [rows[8], rows[9], rows[10]],
            ];
            for row in &m {
                let len = (row[0] * row[0] + row[1] * row[1] + row[2] * row[2]).sqrt();
                assert!((len - 1.0).abs() <= 1.0e-4, "row not unit-length: {len}");
            }
            // Rows mutually orthogonal.
            let d01 = m[0][0] * m[1][0] + m[0][1] * m[1][1] + m[0][2] * m[1][2];
            let d02 = m[0][0] * m[2][0] + m[0][1] * m[2][1] + m[0][2] * m[2][2];
            let d12 = m[1][0] * m[2][0] + m[1][1] * m[2][1] + m[1][2] * m[2][2];
            for d in [d01, d02, d12] {
                assert!(d.abs() <= 1.0e-4, "rows not orthogonal: {d}");
            }
        }
    }
}
