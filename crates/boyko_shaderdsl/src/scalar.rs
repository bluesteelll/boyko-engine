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
//! - `impl FieldScalar for Emit` ([`crate::emit`], `feature = "emit"`) — the HLSL
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

    /// A floating-point literal lifted into the backend (`x` on Eval; a constant
    /// SSA node on `Emit`).
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

    /// `min(self, rhs)` (the IEEE `f32::min` / HLSL `min`).
    fn min(self, rhs: Self) -> Self;
    /// `max(self, rhs)` (the IEEE `f32::max` / HLSL `max`).
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

    /// `op == want` — the unsigned op-discriminant equality in `combine`'s
    /// dispatch. `op`/`want` are host `u32` constants (the edit's op + the
    /// `sdf_op::*` discriminant), NOT traced values: the dispatch over the op enum
    /// is a HOST branch that selects which finite formula to fold, exactly as the
    /// frozen `combine` does.
    fn eq_u(op: u32, want: u32) -> Self::Mask;
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

// ---- The Eval backend: `impl FieldScalar for f32` (byte-mirror of lib.rs) -------

impl FieldScalar for f32 {
    type Vec3 = [f32; 3];
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
    fn min(self, rhs: Self) -> Self {
        // `f32::min` — the same IEEE min `boyko_sdf_math` folds (lib.rs:665).
        f32::min(self, rhs)
    }
    #[inline]
    fn max(self, rhs: Self) -> Self {
        f32::max(self, rhs)
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
    fn eq_u(op: u32, want: u32) -> bool {
        op == want
    }
}
