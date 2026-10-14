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
