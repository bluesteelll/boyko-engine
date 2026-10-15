//! Render wave E0 — the host gates of the five shared primitives `Cf::{floor, frac, log2, exp2,
//! pow}` (AD-7, `plan/H1-E0-edsl-primitives.md`) and the `exp` / `ln` leaf helpers.
//!
//! - **Eval, exact ops** — `floor` / `frac` are bit-exact against an `f64` reference over a grid
//!   with every edge class (negatives, `±0`, `±∞`, NaN, subnormals, `±2²³`, `±(2²³ ± 0.5)`, the
//!   `[-2⁻²⁴, 0)` band where `frac` rounds to `1.0`).
//! - **Eval, approximate ops** — `log2` / `exp2` / `pow` within 1 ULP of an `f64` reference
//!   rounded once to `f32`, over 2¹⁶ log-spaced points per op. The metric is
//!   `boyko_shaderdsl::prims_probe::ulp_distance` (sign-folded monotone integer images), which is
//!   defined across subnormals and `±0`; a RELATIVE metric would blow up at the subnormal end.
//!   Not a tautology: the reference is the `f64` function, never the `f32` one the shim calls.
//! - **Leaf helpers** — `exp` / `ln` against `f64` with their stated, conditioning-scaled bounds.
//! - **The frozen axis** — `FieldScalar` (the physics leaf) carries exactly its 23 methods; an op
//!   slipped onto it is red here, by name.
//! - **Emit** (`feature = "emit"`) — each node prints exactly its intrinsic, operand order
//!   included, on BOTH axes that carry it (`Cf` and `InterpBackend`).

use boyko_shaderdsl::cf::{self, Cf, EvalCf};
use boyko_shaderdsl::prims_probe::{E0Op, e0_eval, ulp_distance};

/// `2⁻²⁴` — the f32 unit roundoff.
const U: f64 = 1.0 / 16_777_216.0;
/// `2⁻¹²⁶` — the smallest normal f32, the absolute floor of every relative tolerance here.
const FLOOR: f64 = 1.175_494_350_822_287_5e-38;
/// Points per approximate op (the rung's `2¹⁶`).
const POINTS: usize = 1 << 16;

/// SplitMix64 — a deterministic, dependency-free sample source.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// The `floor` / `frac` grid: the named edge values, a dense sweep of `[-2⁻²⁴, 0)` (where `frac`
/// is exactly `1.0` up to `-2⁻²⁵` and `1 - 2⁻²⁴` at `-2⁻²⁴`), the whole subnormal and
/// `[-2²⁴, 2²⁴]` neighbourhoods of the integers, and 2¹⁶ uniformly random bit patterns (every
/// exponent, both signs, NaN payloads included).
fn floor_frac_grid() -> Vec<f32> {
    let mut g = vec![
        0.0,
        -0.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        -f32::NAN,
        f32::MIN_POSITIVE,
        -f32::MIN_POSITIVE,
        f32::from_bits(1),
        -f32::from_bits(1),
        f32::from_bits(0x007F_FFFF),
        -f32::from_bits(0x007F_FFFF),
        f32::MAX,
        f32::MIN,
        8_388_608.0,
        -8_388_608.0,
        8_388_607.5,
        -8_388_607.5,
        (8_388_608.5f64) as f32,
        (-8_388_608.5f64) as f32,
        -0.25,
        0.25,
        -0.5,
        0.5,
        -1.0,
        1.0,
        -1.5,
        2.5,
        -2.5,
        -(2.0f32.powi(-24)),
        -(2.0f32.powi(-25)),
        -(2.0f32.powi(-26)),
    ];
    // The `[-2⁻²⁴, 0)` band, 4096 evenly spaced bit patterns of its magnitude range.
    let lo = 2.0f32.powi(-24).to_bits();
    for k in 0..4096u32 {
        g.push(-f32::from_bits(lo - k * (lo / 4096)));
    }
    // Around every power of two up to 2²⁴, both signs: the integer and its two neighbours.
    for e in 0..=24 {
        let p = 2.0f32.powi(e);
        for v in [p, f32::from_bits(p.to_bits() - 1), f32::from_bits(p.to_bits() + 1)] {
            g.push(v);
            g.push(-v);
        }
    }
    let mut rng = Rng(0xE0_F100);
    for _ in 0..POINTS {
        g.push(f32::from_bits(rng.next_u64() as u32));
    }
    g
}

/// The `f64` reference of `floor` (exact: `floor` of an f32 is an f32).
fn floor_ref(x: f32) -> f32 {
    f64::from(x).floor() as f32
}

/// The `f64` reference of `frac = x - floor(x)`, rounded to f32. The f64 difference is exact for
/// every f32 except the negatives with `|x| < 2⁻³⁰` (whose `1 + x` needs more than 53 bits); there
/// the exact result lies in `(1 - 2⁻³⁰, 1)`, far from the f32 midpoint `1 - 2⁻²⁵`, so both
/// roundings land on `1.0` and the reference is the correctly rounded value on the whole domain.
fn frac_ref(x: f32) -> f32 {
    let x = f64::from(x);
    (x - x.floor()) as f32
}

/// Bit-exact agreement, with every NaN equal to every NaN (payloads are not part of the claim).
fn same_bits(a: f32, b: f32) -> bool {
    (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
}

#[test]
fn e0_floor_frac_eval_bit_exact() {
    let mut bad: Vec<String> = Vec::new();
    let grid = floor_frac_grid();
    for &x in &grid {
        let f = <EvalCf as Cf>::floor(x);
        let want_f = floor_ref(x);
        if !same_bits(f, want_f) && bad.len() < 8 {
            bad.push(format!("floor({x:e}) = {f:e} ({:#010x}), f64 reference {want_f:e} ({:#010x})", f.to_bits(), want_f.to_bits()));
        }
        let r = <EvalCf as Cf>::frac(x);
        let want_r = frac_ref(x);
        if !same_bits(r, want_r) && bad.len() < 8 {
            bad.push(format!("frac({x:e}) = {r:e} ({:#010x}), f64 reference {want_r:e} ({:#010x})", r.to_bits(), want_r.to_bits()));
        }
    }
    assert!(bad.is_empty(), "Eval floor/frac not bit-exact over {} points:\n{}", grid.len(), bad.join("\n"));
}

#[test]
fn e0_frac_negative_wraps_up() {
    assert_eq!(<EvalCf as Cf>::floor(-0.25), -1.0, "floor(-0.25) must round DOWN, not toward zero");
    assert_eq!(<EvalCf as Cf>::frac(-0.25), 0.75, "frac(-0.25) must wrap up to 0.75 (x - floor x)");
    assert_eq!(e0_eval(E0Op::Frac, -0.25, 0.0), 0.75, "the frac probe body must agree with Cf::frac");
}

/// One approximate op's sweep result: points checked, worst ULP distance and the first few
/// failures (each naming the op and the input).
struct Sweep {
    op: &'static str,
    checked: usize,
    worst: u64,
    failures: Vec<String>,
}

impl Sweep {
    fn new(op: &'static str) -> Self {
        Sweep { op, checked: 0, worst: 0, failures: Vec::new() }
    }

    fn record(&mut self, input: String, got: f32, want: f32) {
        self.checked += 1;
        let d = if got.is_nan() || want.is_nan() { u64::MAX } else { ulp_distance(got, want) };
        self.worst = self.worst.max(d);
        if d > 1 && self.failures.len() < 4 {
            self.failures.push(format!("{}({input}) = {got:e}, f64 reference {want:e}, {d} ULP", self.op));
        }
    }
}

#[test]
fn e0_log2_exp2_pow_eval_within_1ulp_of_f64() {
    let n = POINTS as f64;

    // log2 over [2⁻¹⁴⁹, 2¹²⁸), log-spaced (subnormal inputs included).
    let mut log2 = Sweep::new("log2");
    for k in 0..POINTS {
        let x = (-149.0 + 277.0 * k as f64 / n).exp2() as f32;
        if x == 0.0 || !x.is_finite() {
            continue;
        }
        let want = f64::from(x).log2() as f32;
        log2.record(format!("{x:e}"), <EvalCf as Cf>::log2(x), want);
    }

    // exp2 over [-149, 128), evenly spaced (subnormal results included).
    let mut exp2 = Sweep::new("exp2");
    for k in 0..POINTS {
        let x = (-149.0 + 277.0 * k as f64 / n) as f32;
        let want = f64::from(x).exp2() as f32;
        if !want.is_finite() {
            continue;
        }
        exp2.record(format!("{x:e}"), <EvalCf as Cf>::exp2(x), want);
    }

    // pow over x in (0, 2²⁰] (log-spaced) x y in [-8, 8], finite results only.
    let mut pow = Sweep::new("pow");
    let side = 256usize;
    for i in 0..side {
        let x = (-149.0 + 169.0 * (i + 1) as f64 / side as f64).exp2() as f32;
        for j in 0..side {
            let y = (-8.0 + 16.0 * j as f64 / (side - 1) as f64) as f32;
            let want = f64::from(x).powf(f64::from(y)) as f32;
            if x == 0.0 || !want.is_finite() {
                continue;
            }
            pow.record(format!("{x:e}, {y:e}"), <EvalCf as Cf>::pow(x, y), want);
        }
    }

    let sweeps = [log2, exp2, pow];
    for s in &sweeps {
        eprintln!("e0 {}: {} points, worst {} ULP", s.op, s.checked, s.worst);
        assert!(s.checked >= POINTS / 2, "{}: the sweep checked only {} points (vacuous grid)", s.op, s.checked);
    }
    let failing: Vec<String> = sweeps
        .iter()
        .filter(|s| !s.failures.is_empty())
        .map(|s| format!("{} (worst {} ULP):\n  {}", s.op, s.worst, s.failures.join("\n  ")))
        .collect();
    assert!(failing.is_empty(), "Eval ops beyond 1 ULP of the f64 reference:\n{}", failing.join("\n"));
}

#[test]
fn e0_exp_ln_helpers_match_f64() {
    let mut bad: Vec<String> = Vec::new();
    let n = POINTS as f64;
    let mut worst_exp = 0.0f64;
    let mut worst_ln = 0.0f64;
    // exp over [-87, 88]: finite, normal results. Bound: the product's and LOG2_E's roundings
    // are each amplified by |x| through the exponential, exp2 adds its own.
    for k in 0..POINTS {
        let x = (-87.0 + 175.0 * k as f64 / n) as f32;
        let got = f64::from(cf::exp::<EvalCf>(x));
        let want = f64::from(x).exp();
        let tol = 4.0 * (1.0 + f64::from(x).abs()) * U * want.abs() + FLOOR;
        worst_exp = worst_exp.max((got - want).abs() / tol);
        if (got - want).abs() > tol && bad.len() < 8 {
            bad.push(format!("exp({x:e}) = {got:e}, f64 {want:e}, |err| {:e} > tol {tol:e}", (got - want).abs()));
        }
    }
    // ln over [2⁻¹²⁶, 2¹²⁷], log-spaced.
    for k in 0..POINTS {
        let x = (-126.0 + 253.0 * k as f64 / n).exp2() as f32;
        let got = f64::from(cf::ln::<EvalCf>(x));
        let want = f64::from(x).ln();
        let tol = 4.0 * U * want.abs() + FLOOR;
        worst_ln = worst_ln.max((got - want).abs() / tol);
        if (got - want).abs() > tol && bad.len() < 8 {
            bad.push(format!("ln({x:e}) = {got:e}, f64 {want:e}, |err| {:e} > tol {tol:e}", (got - want).abs()));
        }
    }
    eprintln!("e0 exp: worst |err|/tol = {worst_exp:.3}; ln: worst |err|/tol = {worst_ln:.3}");
    assert!(bad.is_empty(), "exp/ln leaf helpers off their f64 references:\n{}", bad.join("\n"));
}

/// The 23 methods of `FieldScalar` (the physics leaf), in declaration order — the frozen axis.
const FIELD_SCALAR_FNS: [&str; 23] = [
    "lit", "int_lit", "int_eq", "int_to_float", "add", "sub", "mul", "div", "neg", "min", "max",
    "clamp01", "lerp", "abs", "sqrt", "select", "gt", "lt", "le", "ge", "eq_u", "grad_offsets",
    "v_normalize",
];

/// The associated types of `FieldScalar`, in declaration order.
const FIELD_SCALAR_TYPES: [&str; 3] = ["Vec3", "Mask", "Int"];

/// The `pub trait FieldScalar` block of `src/scalar.rs` (brace-matched), comments stripped.
fn field_scalar_block() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/scalar.rs");
    let src = std::fs::read_to_string(path).expect("invariant: src/scalar.rs exists");
    let code: String = src
        .lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let start = code.find("pub trait FieldScalar").expect("invariant: scalar.rs declares FieldScalar");
    let after = &code[start..];
    let open = after.find('{').expect("the trait has a body");
    let mut depth = 0i32;
    for (i, ch) in after[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return after[open + 1..open + i].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("the FieldScalar block never closed");
}

/// The identifiers following `keyword` at brace depth 0 of `block`, in order.
fn items_after(block: &str, keyword: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let toks: Vec<&str> = block.split(|c: char| c.is_whitespace()).filter(|t| !t.is_empty()).collect();
    for (i, t) in toks.iter().enumerate() {
        if depth == 0 && *t == keyword
            && let Some(next) = toks.get(i + 1)
        {
            let name: String = next.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            out.push(name);
        }
        depth += t.matches('{').count() as i32 - t.matches('}').count() as i32;
    }
    out
}

#[test]
fn e0_field_scalar_axis_frozen() {
    let block = field_scalar_block();
    let fns = items_after(&block, "fn");
    assert_eq!(
        fns, FIELD_SCALAR_FNS,
        "FieldScalar (the physics leaf) is byte-frozen: its methods changed. E0's ops live on Cf; \
         an op added to FieldScalar would be carried by every physics build"
    );
    let types = items_after(&block, "type");
    assert_eq!(types, FIELD_SCALAR_TYPES, "FieldScalar's associated types changed");
}

#[cfg(feature = "emit")]
mod emit {
    use boyko_shaderdsl::emit::{emit_hlsl_e0_interp_op, emit_hlsl_e0_op};
    use boyko_shaderdsl::interp::InterpBackend;
    use boyko_shaderdsl::prims_probe::E0Op;

    /// The exact HLSL intrinsic spelling of each op, operand order included.
    fn call(op: E0Op) -> &'static str {
        match op {
            E0Op::Floor => "floor(x)",
            E0Op::Frac => "frac(x)",
            E0Op::Log2 => "log2(x)",
            E0Op::Exp2 => "exp2(x)",
            E0Op::Pow => "pow(x, y)",
        }
    }

    #[test]
    fn e0_emit_spells_each_intrinsic() {
        let mut bad = Vec::new();
        for op in E0Op::ALL {
            let got = emit_hlsl_e0_op(op);
            let want = format!("    return {};\n", call(op));
            if got != want {
                bad.push(format!("{}: printed {got:?}, want {want:?}", op.name()));
            }
        }
        assert!(bad.is_empty(), "Cf E0 nodes misprinted:\n{}", bad.join("\n"));
    }

    #[test]
    fn e0_interp_backend_arms() {
        // Values: each hand-computed, chosen so the named mutation (trunc, ln, exp, swapped pow)
        // changes the result.
        let values = [
            ("floor(-0.25)", InterpBackend::floor(-0.25f32), -1.0f32),
            ("frac(-0.25)", InterpBackend::frac(-0.25f32), 0.75),
            ("log2(8)", InterpBackend::log2(8.0f32), 3.0),
            ("exp2(3)", InterpBackend::exp2(3.0f32), 8.0),
            ("pow(2, 10)", InterpBackend::pow(2.0f32, 10.0), 1024.0),
        ];
        let mut bad: Vec<String> = values
            .iter()
            .filter(|(_, got, want)| got != want)
            .map(|(what, got, want)| format!("f32 {what} = {got}, want {want}"))
            .collect();
        for op in E0Op::ALL {
            let got = emit_hlsl_e0_interp_op(op);
            let want = format!("    float t0 = {};\n    return t0;\n", call(op));
            if got != want {
                bad.push(format!("Emit {}: printed {got:?}, want {want:?}", op.name()));
            }
        }
        assert!(bad.is_empty(), "InterpBackend E0 arms wrong:\n{}", bad.join("\n"));
    }
}
