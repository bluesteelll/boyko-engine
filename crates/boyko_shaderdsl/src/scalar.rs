//! The `FieldScalar` backend trait + the `f32` Eval implementation.
//!
//! `FieldScalar` is the scalar ABSTRACTION the generic field body
//! ([`crate::field`]) is written against. It exposes EXACTLY the op-set the SDF
//! edit-list field needs (audited from `boyko_sdf_math::{smin,smax,combine,
//! sd_sphere,sd_box,edit_distance,sdf_edit_list}`) — no more, no less. Two
//! backends instantiate it:
//!
//! - `impl FieldScalar for f32` (here) — the **Eval** backend. Every method is a
//!   single `core` f32 op, BYTE-MIRRORING the frozen `boyko_sdf_math` bodies, so a
//!   monomorphization over `f32` produces the same machine code (and the same
//!   floating-point result) as the hand-written field.
//! - `impl FieldScalar for Emit` (`crate::emit`, `feature = "emit"`) — the HLSL
//!   SSA recorder: each method pushes one SSA node and returns its handle.
//!
//! # `no_std`
//!
//! This module is `#![no_std]`-clean (the Eval path is the physics leaf). The one
//! op stable `core` lacks is `sqrt`; it is feature-gated exactly as in
//! `boyko_sdf_math` (the `nightly` feature uses `core::intrinsics::sqrtf32`, else
//! `std`'s `f32::sqrt`). Both lower to the hardware `sqrtss`, so the Eval result is
//! bit-identical in either mode.

/// The scalar backend the generic SDF field body is written against.
///
/// A backend supplies an associated [`Vec3`](Self::Vec3) (the 3-component vector
/// the primitive distances operate on) and [`Mask`](Self::Mask) (the boolean the
/// finite op-dispatch selects on), plus the exact scalar/vector op-set the field
/// folds. Every op MUST match the frozen `boyko_sdf_math` arithmetic operand-for-
/// operand so the `f32` instantiation is byte-identical and the `Emit`
/// instantiation reproduces the same op tree.
///
/// All methods are `#[inline]`: the Eval backend lowers each to a single hardware
/// instruction, and inlining is what lets the monomorphized field collapse to the
/// hand-written code (the zero-cost guarantee).
pub trait FieldScalar: Copy {
    /// The 3-component vector type the primitive distances (`sd_sphere`/`sd_box`)
    /// operate on. `[Self; 3]` for both backends (component-wise).
    type Vec3: Copy;
    /// The boolean produced by the comparisons ([`gt`](Self::gt) /
    /// [`eq_u`](Self::eq_u)) and consumed by [`select`](Self::select). `bool` for
    /// the Eval backend; an SSA node for `Emit`.
    type Mask: Copy;
    /// The INTEGER value type the bit/brick leaves ([`crate::brick`]) operate on — a
    /// packed atlas byte / index. `i32` for the Eval backend (so the snorm `q as f32`
    /// numeric cast and the `q == i8::MIN` sentinel compare are byte-mirrors of the
    /// host); a `uint` SSA node for `Emit`. The A2 leaf only uses
    /// [`int_lit`](Self::int_lit) / [`int_eq`](Self::int_eq) /
    /// [`int_to_float`](Self::int_to_float); the
    /// bitwise AND / shift the printer already supports are added to the trait when
    /// A3's brick-index math needs them.
    type Int: Copy;

    /// A floating-point literal lifted into the backend (`x` on Eval; a constant
    /// SSA node on `Emit`).
    fn lit(x: f32) -> Self;

    /// An INTEGER literal lifted into the backend (`x` on Eval; a `uint` constant SSA
    /// node on `Emit`) — the snorm sentinel / a bit mask.
    fn int_lit(x: i32) -> Self::Int;

    /// `a == b` over two [`Int`](Self::Int)s, producing a [`Mask`](Self::Mask) — the
    /// snorm `q == i8::MIN` sentinel test.
    fn int_eq(a: Self::Int, b: Self::Int) -> Self::Mask;

    /// The NUMERIC (value-preserving) `Int -> Self` conversion — HLSL `(float)q`, NOT
    /// `asfloat` (a bit-reinterpret). Mirrors the host `q as f32`.
    fn int_to_float(a: Self::Int) -> Self;

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

    /// `min(self, rhs)` — HLSL `min`, which DXC lowers to GLSL.std.450 `NMin`: `rhs` if
    /// `rhs < self`, else `self`; a NaN operand yields the other one (`NMin`'s rule, and
    /// `f32::min`'s). On a `±0.0` tie the host returns `self`, the FIRST operand, in every build.
    /// That sign is a host convention, not `NMin`'s: the spec's "-0 compares less than +0" (so
    /// `-0.0` in either order) holds only under `SPV_KHR_float_controls2`, which no shader here
    /// declares, and without it a device may return either zero. The convention is the one the
    /// physics AVX2 narrowphase (`boyko_physics::sdf_simd`) reproduces by swapping `MINPS`'s
    /// operands, so the scalar oracle and that kernel pick the same zero.
    fn min(self, rhs: Self) -> Self;
    /// `max(self, rhs)` — HLSL `max`, which DXC lowers to GLSL.std.450 `NMax`: `rhs` if
    /// `self < rhs`, else `self`; a NaN operand yields the other one. On a `±0.0` tie the host
    /// returns `self`, the FIRST operand: the convention of [`FieldScalar::min`], where `NMax`
    /// gives `+0.0` under `SPV_KHR_float_controls2` and either zero without it.
    fn max(self, rhs: Self) -> Self;

    /// `clamp(self, 0.0, 1.0)` — the FIXED-bounds clamp `smin` uses for its blend
    /// weight `hh`. Fixed bounds (not a general 3-arg clamp) because that is the
    /// only clamp the field needs and it keeps the emitted HLSL a literal
    /// `clamp(_, 0.0, 1.0)`.
    fn clamp01(self) -> Self;

    /// `lerp(self, a, h)` == `self + (a - self) * h` — the linear blend `smin`
    /// uses, EMITTED AS TWO temps (`t = a - self`; `t = t * h`; `r = self + t`) to
    /// honor the two-rounding contract of HLSL's `lerp(self, a, h)`. On Eval this
    /// is the same `self + (a - self) * h` expression the frozen `smin` writes.
    fn lerp(self, a: Self, h: Self) -> Self;

    /// `abs(self)`.
    fn abs(self) -> Self;
    /// `sqrt(self)` (IEEE; lowers to `sqrtss`).
    fn sqrt(self) -> Self;

    /// `cond ? t : e` — the value select the finite op-dispatch lowers to (no
    /// data-dependent control flow; HLSL emits a ternary / `select`).
    fn select(cond: Self::Mask, t: Self, e: Self) -> Self;

    /// `self > rhs` — the `k > 0.0` smooth/hard test in `combine`.
    fn gt(self, rhs: Self) -> Self::Mask;

    /// `self < rhs` — the strict less-than comparison (HLSL `OpFOrdLessThan`). Added
    /// for the brick-marcher control-flow leaf ([`crate::brick::dist_to_brick_exit_body`]),
    /// whose final progress clamp is `exit < BRICK_EXIT_EPS ? EPS : exit`. The frozen
    /// field/normal/decode bodies use only [`gt`](Self::gt), so adding this is
    /// firewall-harmless (no existing traced body records an `Lt`).
    fn lt(self, rhs: Self) -> Self::Mask;

    /// `self <= rhs` — the less-than-or-equal comparison (HLSL `OpFOrdLessThanEqual`,
    /// a DISTINCT opcode from a swapped `>` — a swapped-`>` would emit `OpFOrdGreaterThan`
    /// and FORK the committed `.spv`). Added for the brick-marcher's per-axis skip guard
    /// `abs(dir) <= BRICK_EXIT_EPS` ([`crate::brick::dist_to_brick_exit_body`]).
    fn le(self, rhs: Self) -> Self::Mask;

    /// `self >= rhs` — the greater-than-or-equal comparison (HLSL `OpFOrdGreaterThanEqual`,
    /// a DISTINCT opcode from a swapped `<=` — a swapped-`<=` would emit `OpFOrdLessThanEqual`
    /// and FORK the committed `.spv`). Added for the B1 exhaustion re-march's mesh guard
    /// `t >= t_mesh` ([`crate::remarch::b1_exhaustion_remarch_body`]). Operand order is
    /// load-bearing: the committed shader spells `t >= t_mesh` (`t` LEFT), so the body calls
    /// `get_var(&t).ge(t_mesh)` to match the emitted comparand order byte-for-byte.
    fn ge(self, rhs: Self) -> Self::Mask;

    /// `op == want` — the unsigned op-discriminant equality in `combine`'s
    /// dispatch. `op`/`want` are host `u32` constants (the edit's op + the
    /// `sdf_op::*` discriminant), NOT traced values: the dispatch over the op enum
    /// is a HOST branch that selects which finite formula to fold, exactly as the
    /// frozen `combine` does.
    fn eq_u(op: u32, want: u32) -> Self::Mask;

    /// The three central-difference GRAD_H offset vectors `[e.xyy, e.yxy, e.yyx]`
    /// where `e = (GRAD_H, 0.0)` — the exact swizzles the frozen `sdf_normal` writes
    /// (`sdf_field.hlsli:194-198`).
    ///
    /// A backend hook (not a free function) because the two backends spell the
    /// offsets DIFFERENTLY and that spelling is load-bearing for the SPIR-V gate:
    /// - **Eval** (`f32`) returns the literal axis vectors `[h,0,0]`, `[0,h,0]`,
    ///   `[0,0,h]` (with `h = GRAD_H`) — the same triple `sdf_edit_list_normal` adds
    ///   to `p`, so the Eval result is byte-identical.
    /// - **Emit** records THREE swizzle nodes off one shared `float2 e =
    ///   float2(GRAD_H, 0.0)`, printed TEXTUALLY as `e.xyy` / `e.yxy` / `e.yyx`
    ///   (NOT decomposed into `float3(GRAD_H, 0.0, 0.0)`): the frozen HLSL uses the
    ///   `.xyy` swizzle form and DXC's SPIR-V is sensitive to it (an `OpVectorShuffle`
    ///   off `e` vs three scalar `OpCompositeConstruct`s would FORK the frozen
    ///   `sdf_field_probe.baseline.dis`). Returning the offsets through this hook is
    ///   the single point that keeps the swizzle spelling frozen.
    ///
    /// `GRAD_H` is `0.0005` — mirrors `boyko_sdf_math::SDF_GRAD_H` and the shader's
    /// `GRAD_H` (`sdf_field.hlsli:42`).
    fn grad_offsets() -> [Self::Vec3; 3];

    /// `a / length(a)` — the unit vector, the LAST op of the surface normal.
    ///
    /// The spelling is, again, backend-specific and load-bearing:
    /// - **Eval** (`f32`) is the GUARDED `v_normalize`: a zero / non-finite length
    ///   returns `[0, 0, 0]` (a field critical point) instead of `NaN`, BYTE-MIRRORING
    ///   `boyko_sdf_math::v_normalize` (lib.rs:575) so the Eval normal is identical to
    ///   `sdf_edit_list_normal`. The guard intercepts ONLY the exactly-zero /
    ///   non-finite path; every non-degenerate input is the byte-identical division.
    /// - **Emit** records the RAW HLSL `normalize(a)` intrinsic (no zero-check exists
    ///   at the op level — the guard is a value-level CPU concern the GPU goldens
    ///   never sample, since they evaluate the normal only at surface hits where
    ///   `|grad| ≈ 1`). This is what the frozen `sdf_normal` emits, so the SPIR-V
    ///   stays byte-identical.
    fn v_normalize(a: Self::Vec3) -> Self::Vec3;
}

/// `length(a)` over a backend `Vec3` — `sqrt(x*x + y*y + z*z)`. A free function
/// (not a trait method) because both backends represent `Vec3` as `[Self; 3]`, so
/// the norm is identical scalar arithmetic over the components.
#[inline]
pub fn v_len<S: FieldScalar<Vec3 = [S; 3]>>(a: [S; 3]) -> S {
    // sqrt(a.x*a.x + a.y*a.y + a.z*a.z) — the exact operand order of
    // `boyko_sdf_math::v_len` (lib.rs:540).
    let sum = a[0].mul(a[0]).add(a[1].mul(a[1])).add(a[2].mul(a[2]));
    sum.sqrt()
}

/// `a + b` — component-wise vector addition. Used by [`crate::normal`] to form the
/// central-difference probe points `p ± offset` (the offset vectors are the GRAD_H
/// swizzle constructors, see [`FieldScalar::grad_offsets`]).
#[inline]
pub fn v_add<S: FieldScalar<Vec3 = [S; 3]>>(a: [S; 3], b: [S; 3]) -> [S; 3] {
    [a[0].add(b[0]), a[1].add(b[1]), a[2].add(b[2])]
}

/// `a - b` — component-wise vector subtraction.
#[inline]
pub fn v_sub<S: FieldScalar<Vec3 = [S; 3]>>(a: [S; 3], b: [S; 3]) -> [S; 3] {
    [a[0].sub(b[0]), a[1].sub(b[1]), a[2].sub(b[2])]
}

/// `abs(a)` — component-wise absolute value.
#[inline]
pub fn v_abs<S: FieldScalar<Vec3 = [S; 3]>>(a: [S; 3]) -> [S; 3] {
    [a[0].abs(), a[1].abs(), a[2].abs()]
}

/// `max(a, 0.0)` — component-wise max-with-zero (the box SDF's `max(q, 0.0)`).
#[inline]
pub fn v_max0<S: FieldScalar<Vec3 = [S; 3]>>(a: [S; 3]) -> [S; 3] {
    let zero = S::lit(0.0);
    [a[0].max(zero), a[1].max(zero), a[2].max(zero)]
}

/// `dot(a, b)` — the 3-component dot product, written as the EXPLICIT left-associated
/// scalar fold `(a.x*b.x + a.y*b.y) + a.z*b.z`. Mirrors `boyko_sdf_math::v_dot`
/// (lib.rs:544-545) operand-for-operand.
///
/// The capsule distance ([`crate::field::sd_capsule`]) needs two dot products. They are
/// spelled with this EXPLICIT scalar fold — NOT the HLSL `dot()` intrinsic — on BOTH
/// backends: DXC may lower `dot()` to an `OpDot` / a reassociated FMA chain that forks
/// the host f32 result from the GPU bytes, so the hand-written `sd_capsule` in
/// `sdf_field.hlsli` spells the same explicit `pa.x*ba.x + pa.y*ba.y + pa.z*ba.z` and
/// the two stay byte-identical (the `eval_byte_identity` + `field_probe_gate` tripwires).
#[inline]
pub fn v_dot<S: FieldScalar<Vec3 = [S; 3]>>(a: [S; 3], b: [S; 3]) -> S {
    a[0].mul(b[0]).add(a[1].mul(b[1])).add(a[2].mul(b[2]))
}

/// `a * s` — component-wise scalar multiply (the capsule's `ba * h` projection step).
#[inline]
pub fn v_scale<S: FieldScalar<Vec3 = [S; 3]>>(a: [S; 3], s: S) -> [S; 3] {
    [a[0].mul(s), a[1].mul(s), a[2].mul(s)]
}

// ---- The Eval backend: `impl FieldScalar for f32` (byte-mirror of lib.rs) -------

impl FieldScalar for f32 {
    type Vec3 = [f32; 3];
    type Mask = bool;
    // The Eval integer is `i32`: the host snorm decode reads an `i8` code, which
    // widens to `i32` losslessly, and `q as f32` is identical from either width.
    type Int = i32;

    #[inline]
    fn lit(x: f32) -> Self {
        x
    }

    #[inline]
    fn int_lit(x: i32) -> i32 {
        x
    }
    #[inline]
    fn int_eq(a: i32, b: i32) -> bool {
        a == b
    }
    #[inline]
    fn int_to_float(a: i32) -> f32 {
        // `a as f32` — the byte-mirror of the host `q as f32` (the `i8` code already
        // widened to `i32`, so the float value is identical).
        a as f32
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

    // NOT `f32::min`/`f32::max`: on rustc 1.98.1 they are `minimum_number_nsz`/
    // `maximum_number_nsz` ("if the inputs compare equal ... either input may be returned
    // non-deterministically"), so the SAME expression lowered at two call sites can return
    // opposite zeros — release CI run 5's `m2_brick_span` red (`max(+0.0, -0.0)` was `+0.0` in
    // the eDSL's unrolled copy and `-0.0` in the mirror's rolled loop, while a debug build gave
    // `-0.0` in both). Spelling the select keeps the NaN rule `f32::min`/`max` document and fixes
    // the tie to the first operand, in every build: the trait's host convention, not `NMin`/
    // `NMax`'s (they leave that sign to the device unless `SPV_KHR_float_controls2` is on, and put
    // `-0` below `+0` when it is). It is what `boyko_physics::sdf_simd`'s swapped `MINPS`/`MAXPS`
    // operands reproduce: with `f32::min`/`max` back here, that module's deferred
    // `x8_bits_eq_scalar_bits_widened_proptest` reds again (lane 6, `+0` against `-0`).
    #[inline]
    fn min(self, rhs: Self) -> Self {
        if self.is_nan() || rhs < self {
            rhs
        } else {
            self
        }
    }
    #[inline]
    fn max(self, rhs: Self) -> Self {
        if self.is_nan() || self < rhs {
            rhs
        } else {
            self
        }
    }

    #[inline]
    fn clamp01(self) -> Self {
        // `.clamp(0.0, 1.0)` — byte-identical to `smin`'s clamp (lib.rs:628).
        self.clamp(0.0, 1.0)
    }

    #[inline]
    fn lerp(self, a: Self, h: Self) -> Self {
        // `self + (a - self) * h` — the EXACT form `smin` writes as
        // `(b + (a - b) * hh)` (lib.rs:630), with `self == b`. Two roundings (the
        // `*` then the `+`), matching the emitted two-temp HLSL `lerp`.
        self + (a - self) * h
    }

    #[inline]
    fn abs(self) -> Self {
        f32::abs(self)
    }
    #[inline]
    fn sqrt(self) -> Self {
        // The sqrt shim MOVED from `boyko_sdf_math::sqrt` (lib.rs:55-67): the one
        // op stable `core` lacks. Lowers to `sqrtss` in both modes, so the Eval
        // result is bit-identical (the GPU goldens are unaffected).
        #[cfg(feature = "nightly")]
        {
            core::intrinsics::sqrtf32(self)
        }
        #[cfg(not(feature = "nightly"))]
        {
            f32::sqrt(self)
        }
    }

    #[inline]
    fn select(cond: bool, t: Self, e: Self) -> Self {
        // `if m { t } else { e }` — the value select; the Eval branch is a CMOV
        // (no UB, both operands already computed).
        if cond { t } else { e }
    }

    #[inline]
    fn gt(self, rhs: Self) -> bool {
        self > rhs
    }

    #[inline]
    fn lt(self, rhs: Self) -> bool {
        self < rhs
    }

    #[inline]
    fn le(self, rhs: Self) -> bool {
        self <= rhs
    }

    #[inline]
    fn ge(self, rhs: Self) -> bool {
        self >= rhs
    }

    #[inline]
    fn eq_u(op: u32, want: u32) -> bool {
        op == want
    }

    #[inline]
    fn grad_offsets() -> [[f32; 3]; 3] {
        // `e = (GRAD_H, 0.0)`; the offsets are `e.xyy`, `e.yxy`, `e.yyx` — i.e. the
        // axis vectors `[h,0,0]`, `[0,h,0]`, `[0,0,h]`. The SAME triple
        // `sdf_edit_list_normal` (lib.rs:698-702) adds/subtracts to `p`. `GRAD_H`
        // mirrors `boyko_sdf_math::SDF_GRAD_H` (lib.rs:97).
        const GRAD_H: f32 = 0.0005;
        [[GRAD_H, 0.0, 0.0], [0.0, GRAD_H, 0.0], [0.0, 0.0, GRAD_H]]
    }

    #[inline]
    fn v_normalize(a: [f32; 3]) -> [f32; 3] {
        // The GUARDED unit-vector, BYTE-MIRRORING `boyko_sdf_math::v_normalize`
        // (lib.rs:575-584): a zero / non-finite length (a field critical point)
        // returns ZERO so the physics seam-skip fires instead of a `NaN` normal; every
        // non-degenerate input takes the byte-identical `[a0/len, a1/len, a2/len]`.
        let len = v_len(a);
        if len <= f32::MIN_POSITIVE || !len.is_finite() {
            return [0.0, 0.0, 0.0];
        }
        [a[0] / len, a[1] / len, a[2] / len]
    }
}
