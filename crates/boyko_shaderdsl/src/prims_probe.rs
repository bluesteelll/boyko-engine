//! Render wave E0 — the probe bodies of the five shared primitives (`Cf::{floor, frac, log2,
//! exp2, pow}`, AD-7) and the metric their host oracles measure with.
//!
//! Each body applies exactly ONE primitive and returns it, so an instantiation measures exactly
//! that node's lowering and nothing else:
//!
//! - `<EvalCf>` — the host values the `tests/e0_prims.rs` oracles sweep against an `f64`
//!   reference;
//! - `<EmitCf>` — the HLSL the printer spells (`crate::emit::emit_hlsl_e0_op`), the functions
//!   of the test-only kernel `boyko_rhi_vulkan/shaders/e0_prims_probe.comp.hlsl`, which the
//!   device probe dispatches.
//!
//! No physics-reachable code names this module; generic bodies emit no object code until a
//! test or a generator instantiates them.

use core::cell::Cell;

use crate::cf::{Cf, EvalCf, Flow};

/// One of the five E0 primitives — the probe's op axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum E0Op {
    /// `floor(x)`.
    Floor,
    /// `frac(x)`.
    Frac,
    /// `log2(x)`.
    Log2,
    /// `exp2(x)`.
    Exp2,
    /// `pow(x, y)`.
    Pow,
}

impl E0Op {
    /// Every op, in the probe kernel's output-word order.
    pub const ALL: [E0Op; 5] = [E0Op::Floor, E0Op::Frac, E0Op::Log2, E0Op::Exp2, E0Op::Pow];

    /// The op's short name (`floor`, …) — also the suffix of its generated probe function
    /// (`e0_floor`).
    pub const fn name(self) -> &'static str {
        match self {
            E0Op::Floor => "floor",
            E0Op::Frac => "frac",
            E0Op::Log2 => "log2",
            E0Op::Exp2 => "exp2",
            E0Op::Pow => "pow",
        }
    }

    /// The `GLSL.std.450` extended instruction the op lowers to (`Fract` for `frac`).
    pub const fn glsl_std_450(self) -> &'static str {
        match self {
            E0Op::Floor => "Floor",
            E0Op::Frac => "Fract",
            E0Op::Log2 => "Log2",
            E0Op::Exp2 => "Exp2",
            E0Op::Pow => "Pow",
        }
    }

    /// The number of operands (`pow` takes two, the others one).
    pub const fn arity(self) -> usize {
        match self {
            E0Op::Pow => 2,
            _ => 1,
        }
    }
}

/// `return floor(x);`
#[inline]
pub fn e0_floor_body<C: Cf>(x: C::Scalar, ret_out: &C::RetCellF) -> Flow {
    C::ret_f(ret_out, C::floor(x))
}

/// `return frac(x);`
#[inline]
pub fn e0_frac_body<C: Cf>(x: C::Scalar, ret_out: &C::RetCellF) -> Flow {
    C::ret_f(ret_out, C::frac(x))
}

/// `return log2(x);`
#[inline]
pub fn e0_log2_body<C: Cf>(x: C::Scalar, ret_out: &C::RetCellF) -> Flow {
    C::ret_f(ret_out, C::log2(x))
}

/// `return exp2(x);`
#[inline]
pub fn e0_exp2_body<C: Cf>(x: C::Scalar, ret_out: &C::RetCellF) -> Flow {
    C::ret_f(ret_out, C::exp2(x))
}

/// `return pow(x, y);`
#[inline]
pub fn e0_pow_body<C: Cf>(x: C::Scalar, y: C::Scalar, ret_out: &C::RetCellF) -> Flow {
    C::ret_f(ret_out, C::pow(x, y))
}

/// Host convenience: the probe body of `op` over the Eval backend. `y` is read only by
/// [`E0Op::Pow`].
#[inline]
pub fn e0_eval(op: E0Op, x: f32, y: f32) -> f32 {
    let cell = Cell::new(0.0f32);
    let _ = match op {
        E0Op::Floor => e0_floor_body::<EvalCf>(x, &cell),
        E0Op::Frac => e0_frac_body::<EvalCf>(x, &cell),
        E0Op::Log2 => e0_log2_body::<EvalCf>(x, &cell),
        E0Op::Exp2 => e0_exp2_body::<EvalCf>(x, &cell),
        E0Op::Pow => e0_pow_body::<EvalCf>(x, y, &cell),
    };
    cell.get()
}

/// The sign-folded monotone integer image of an `f32` bit pattern: `+x` maps to its bits,
/// `-x` to the negated magnitude bits, so `±0` both map to 0 and adjacent floats (subnormals
/// included, across zero) map to adjacent integers. Undefined for NaN (callers exclude it).
#[inline]
pub const fn ordered_bits(x: f32) -> i64 {
    let b = x.to_bits();
    let mag = (b & 0x7FFF_FFFF) as i64;
    if b >> 31 == 1 { -mag } else { mag }
}

/// The ULP distance of two non-NaN `f32`s — the distance of their [`ordered_bits`] images (the
/// render-wave E0 metric, D-E5): well defined across subnormals and across `±0`, `0` iff the two
/// are equal as values, `1` for neighbours.
#[inline]
pub const fn ulp_distance(a: f32, b: f32) -> u64 {
    ordered_bits(a).abs_diff(ordered_bits(b))
}

// ---- The probe kernel's buffer layout (the generator prints these; the device test reads them) --

/// Elements the device probe evaluates (the rung's `64 k`).
pub const E0_PROBE_ELEMENTS: u32 = 65_536;
/// The probe kernel's workgroup width (`[numthreads(64, 1, 1)]`).
pub const E0_PROBE_LOCAL_SIZE: u32 = 64;
/// Words per element in the probe's one storage buffer: `asuint(x)`, `asuint(y)`, the five op
/// results in [`E0Op::ALL`] order, one unused word (a 32-byte stride).
pub const E0_PROBE_STRIDE: u32 = 8;
/// The word of an element holding the first result (`floor`); op `k` of [`E0Op::ALL`] is at
/// `E0_PROBE_OUT_BASE + k`.
pub const E0_PROBE_OUT_BASE: u32 = 2;

const _: () = assert!(E0_PROBE_ELEMENTS.is_multiple_of(E0_PROBE_LOCAL_SIZE));
const _: () = assert!(E0_PROBE_OUT_BASE as usize + E0Op::ALL.len() <= E0_PROBE_STRIDE as usize);

// ---- "E0's bound": the device precision the five ops may be held to (D-E3) -------------------
//
// Source: the Vulkan specification, appendix "Vulkan Environment for SPIR-V", precision table
// (fetched 2026-10-14): `floor` and `fract` are correctly rounded; `log2` is 3 ULP outside
// `[0.5, 2]` and absolute `2⁻²¹` inside; `exp2` is `3 + 2·|x|` ULP; `pow` is inherited from
// `exp2(y · log2(x))`. Later oracles (POSTFX P22, PX0's `--eval-post`) cite these functions as
// "E0's bound"; the device's MEASURED maxima are a report (the manifest row, the GL-E0 log), never a
// gate value.
//
// Two environment rules shape the helpers below, because the engine's shaders declare no
// float-controls execution mode:
//
// - **Rounding mode.** Without `RoundingModeRTE` / `RoundingModeRTZ`, a "correctly rounded" result
//   is rounded with an implementation-defined mode, so an inexact correctly-rounded result may be
//   EITHER bracketing f32 ([`frac_bracket`]), and a rounded intermediate is charged a full ULP, not
//   half ([`vk_pow_bound`]'s product term).
// - **Denormals and special values.** Denormals may be flushed to zero, and without
//   `SignedZeroInfNanPreserve` NaN, `±Inf` and the sign of zero are not preserved. A consumer gates
//   finite normal rows and reports the rest (`boyko_rhi_vulkan/tests/e0_prims_device.rs`).
//
// Every helper here is `core`-only (no transcendental): the caller passes the `f64` reference it
// already computed.

/// `log2`'s bound outside `[0.5, 2]`, in ULPs of the result.
pub const VK_LOG2_ULP_OUTSIDE: f64 = 3.0;
/// `log2`'s ABSOLUTE bound inside `[0.5, 2]` — `2⁻²¹`.
pub const VK_LOG2_ABS_INSIDE: f64 = 1.0 / 2_097_152.0;
/// The closed interval of inputs where [`VK_LOG2_ABS_INSIDE`] applies.
pub const VK_LOG2_ABS_RANGE: (f32, f32) = (0.5, 2.0);

/// `exp2`'s bound at input `x`, in ULPs of the result: `3 + 2·|x|`.
#[inline]
pub const fn vk_exp2_ulp(x: f64) -> f64 {
    3.0 + 2.0 * x.abs()
}

/// The f32 ULP at magnitude `|v|`: the spacing of the f32 binade that contains `|v|` rounded to
/// f32, and `2⁻¹⁴⁹` at and below the normal range (the subnormal spacing). Finite `v` only.
#[inline]
pub const fn ulp_f32_at(v: f64) -> f64 {
    let a = v.abs() as f32;
    let biased = ((a.to_bits() >> 23) & 0xFF) as i64;
    // The f32 binade [2^(b-127), 2^(b-126)) has spacing 2^(b-150); the subnormals (b = 0) share
    // the b = 1 spacing 2^-149.
    let e = if biased == 0 { -149 } else { biased - 150 };
    f64::from_bits(((e + 1023) as u64) << 52)
}

/// The absolute `log2` bound at input `x` whose exact `log2` is `log2_x` (Vulkan: absolute `2⁻²¹`
/// inside `[0.5, 2]`, else 3 ULP of the result).
#[inline]
pub const fn vk_log2_bound(x: f32, log2_x: f64) -> f64 {
    if x >= VK_LOG2_ABS_RANGE.0 && x <= VK_LOG2_ABS_RANGE.1 {
        VK_LOG2_ABS_INSIDE
    } else {
        VK_LOG2_ULP_OUTSIDE * ulp_f32_at(log2_x)
    }
}

/// The absolute `exp2` bound at input `x` whose exact `exp2` is `exp2_x`.
#[inline]
pub const fn vk_exp2_bound(x: f32, exp2_x: f64) -> f64 {
    vk_exp2_ulp(x as f64) * ulp_f32_at(exp2_x)
}

/// An upper bound of `2^e − 1` for `0 ≤ e ≤ 1` with no transcendental: `e·ln 2·(1 + e)` (the
/// series `e·ln 2 + (e·ln 2)²/2! + …` divided by `e·ln 2` is `1 + 0.35·e + …`, below `1 + e` on the
/// whole interval). Returns `+∞` past `e = 1` — a bound that wide is no bound, and no reachable
/// `pow` row of the probe's domain (`|y| ≤ 8`) gets near it.
#[inline]
const fn exp2m1_upper(e: f64) -> f64 {
    if e > 1.0 { f64::INFINITY } else { e * core::f64::consts::LN_2 * (1.0 + e) }
}

/// The absolute `pow(x, y)` bound "inherited from `exp2(y × log2(x))`", for `x > 0` with exact
/// `log2 x = log2_x` and exact `x^y = pow_xy`:
///
/// `|Δ| ≤ |pow|·(2^{e_t} − 1) + (3 + 2·|t|)·ulp(pow)`, with `t = y·log2 x` and
/// `e_t = |y|·b_log2(x) + ulp(t)` — the `log2` error scaled by `|y|`, plus the product's rounding
/// charged a FULL ULP (the rounding mode is implementation-defined, see the section doc), carried
/// through `exp2` as a relative factor, plus `exp2`'s own bound at `t`.
#[inline]
pub const fn vk_pow_bound(x: f32, y: f32, log2_x: f64, pow_xy: f64) -> f64 {
    let t = y as f64 * log2_x;
    let e_t = (y as f64).abs() * vk_log2_bound(x, log2_x) + ulp_f32_at(t);
    pow_xy.abs() * exp2m1_upper(e_t) + vk_exp2_ulp(t) * ulp_f32_at(pow_xy)
}

/// The two f32 values a correctly rounded `frac(x)` may return under an implementation-defined
/// rounding mode: the f32 neighbours below and above the exact `x − floor(x)` (equal when it is
/// exact). Finite `x` only.
///
/// `frac` is inexact only on `x ∈ (−0.5, 0)`, where its result `1 + x` lies in `(0.5, 1)`, whose
/// f32 spacing is coarser than `x`'s. For `|x| ≥ 2⁻³⁰` the f64 difference is exact; below, `1 + x`
/// needs more than 53 bits, lies in `(1 − 2⁻³⁰, 1)` and is bracketed by `1 − 2⁻²⁴` and `1`, which
/// is spelled directly.
#[inline]
pub fn frac_bracket(x: f32) -> (f32, f32) {
    let one_below = f32::from_bits(1.0f32.to_bits() - 1);
    if x < 0.0 && x > -(1.0 / 1_073_741_824.0) {
        return (one_below, 1.0);
    }
    let exact = x as f64 - <EvalCf as Cf>::floor(x) as f64;
    let r = exact as f32;
    let rf = r as f64;
    if rf == exact {
        (r, r)
    } else if rf < exact {
        (r, f32::from_bits(r.to_bits() + 1))
    } else {
        (f32::from_bits(r.to_bits() - 1), r)
    }
}
