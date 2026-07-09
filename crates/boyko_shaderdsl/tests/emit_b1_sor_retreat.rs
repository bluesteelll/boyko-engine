//! Increment 4f — the B1 over-relaxation SOR-FAIL-RETREAT STEP span tests.
//!
//! Two oracles, two scopes:
//!
//! (a) The MULTI-ITERATION EVAL CONTROL-FLOW oracle — a frozen `host_sor_loop` transcribing the
//!     committed `if (omega > 1.0) { ... } else { ... }` step span VERBATIM inside a real
//!     `for it in 0..N` loop carrying `(t, omega, safe_t, sor_prev, sor_step_prev, exhausted)`,
//!     driven by an LCG `d`-sequence (~10k cases × 2 d-generators). At EACH iteration it asserts the
//!     FULL carried tuple (all `to_bits` + `exhausted`) AND the continue/break/fall-through
//!     disposition bit-identical vs `b1_sor_retreat_body::<EvalCf>` driven in the SAME loop with the
//!     SAME `Cell`s. Plus 5 hand-built witnesses for the design edge cases.
//!
//! (b) The Emit GENERATOR STRUCTURE guard (`feature = "emit"`) — the brace-matched golden of the
//!     emitted span (the committed L1459-1498 with comments stripped + the R1 `t = t + …` form),
//!     mirroring `emit_b1_exhaustion_remarch`'s golden. The CRITICAL assertion is the UNPARENTHESIZED
//!     `it > 0u && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev` condition.
//!
//! (c) A `ugt` / `and2` / `uint_lit` round-trip on `EvalCf`.

use boyko_shaderdsl::cf::{Cf, EvalCf, Flow, LoopOp};
use boyko_shaderdsl::sor::{FIELD_LIPSCHITZ_L, T_MAX, b1_sor_retreat_body};

// ---- (a) The multi-iteration Eval control-flow oracle ---------------------------------

/// One iteration's disposition — the loop control the step span yields, used to drive the test's
/// reference `for` loop the SAME way `runtime_for`/the IIFE's `?` drives the body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Disp {
    /// Fell through the step (ran the over-relax/plain step, no `t > T_MAX`) → next iteration.
    FallThrough,
    /// The sor-fail retreat `continue` (`t = safe_t + sor_prev; omega = 1.0; continue;`).
    Continue,
    /// The `t > T_MAX` miss `break` (`exhausted = false; break;`).
    Break,
}

/// The carried B1 marcher state across the over-relaxation loop (the 5 floats + the bool).
#[derive(Clone, Copy, Debug)]
struct Carry {
    t: f32,
    omega: f32,
    safe_t: f32,
    sor_prev: f32,
    sor_step_prev: f32,
    exhausted: bool,
}

impl Carry {
    /// Asserts bit-equality (all floats by `to_bits`, the bool directly) — the FULL tuple.
    fn assert_eq_bits(&self, other: &Carry, ctx: &str) {
        assert_eq!(self.t.to_bits(), other.t.to_bits(), "{ctx}: t bits");
        assert_eq!(self.omega.to_bits(), other.omega.to_bits(), "{ctx}: omega bits");
        assert_eq!(self.safe_t.to_bits(), other.safe_t.to_bits(), "{ctx}: safe_t bits");
        assert_eq!(self.sor_prev.to_bits(), other.sor_prev.to_bits(), "{ctx}: sor_prev bits");
        assert_eq!(
            self.sor_step_prev.to_bits(),
            other.sor_step_prev.to_bits(),
            "{ctx}: sor_step_prev bits"
        );
        assert_eq!(self.exhausted, other.exhausted, "{ctx}: exhausted");
    }
}

/// The frozen reference step, transcribed VERBATIM from the committed
/// `sdf_gbuffer_composite.hlsl:1459-1498` (the over-relax step + the `t > T_MAX` miss). Mutates `c`
/// in place and RETURNS the iteration disposition. `d` is THIS iteration's sampled field value, `it`
/// the loop induction var (matching the body's parameters). The committed spells `t += step_len` /
/// `t += d` (R1: the body emits `t = t + …`; both round IDENTICALLY — one add — so the host uses the
/// clippy-clean compound-assign).
fn host_step(c: &mut Carry, d: f32, it: u32) -> Disp {
    if c.omega > 1.0 {
        let step_len = d * c.omega;
        if it > 0 && c.sor_prev + d < FIELD_LIPSCHITZ_L * c.sor_step_prev {
            c.t = c.safe_t + c.sor_prev;
            c.omega = 1.0;
            return Disp::Continue;
        }
        c.safe_t = c.t;
        c.sor_prev = d;
        c.sor_step_prev = step_len;
        c.t += step_len;
    } else {
        c.t += d;
    }
    if c.t > T_MAX {
        c.exhausted = false;
        return Disp::Break;
    }
    Disp::FallThrough
}

/// Drives `host_step` over a real `for it in 0..n` carrying `c`, with the `continue`/`break`
/// disposition steering the loop EXACTLY as the GPU `for (uint it)` does, recording each iteration's
/// disposition. Returns the final carry + the per-iteration disposition trace.
fn host_sor_loop(mut c: Carry, ds: &[f32]) -> (Carry, Vec<Disp>) {
    let mut trace = Vec::with_capacity(ds.len());
    for (it, &d) in ds.iter().enumerate() {
        let disp = host_step(&mut c, d, it as u32);
        trace.push(disp);
        match disp {
            Disp::FallThrough | Disp::Continue => {}
            Disp::Break => break,
        }
    }
    (c, trace)
}

/// Drives the GENERIC `b1_sor_retreat_body::<EvalCf>` over the SAME `for it in 0..n`, with the SAME
/// carried `Cell`s, mapping the returned `Flow` to a `Disp` and steering the loop the same way.
fn body_sor_loop(c0: Carry, ds: &[f32]) -> (Carry, Vec<Disp>) {
    // The carried state as EvalCf vars (the same `Cell` shape the GPU preamble decls would carry).
    let t = EvalCf::decl_param("t", c0.t);
    let omega = EvalCf::decl_param("omega", c0.omega);
    let safe_t = EvalCf::decl_param("safe_t", c0.safe_t);
    let sor_prev = EvalCf::decl_param("sor_prev", c0.sor_prev);
    let sor_step_prev = EvalCf::decl_param("sor_step_prev", c0.sor_step_prev);
    let exhausted = EvalCf::decl_bool_param("exhausted", c0.exhausted);

    let mut trace = Vec::with_capacity(ds.len());
    for (it, &d) in ds.iter().enumerate() {
        let flow = b1_sor_retreat_body::<EvalCf>(
            d,
            it as u32,
            &t,
            &omega,
            &safe_t,
            &sor_prev,
            &sor_step_prev,
            &exhausted,
        );
        let disp = match flow {
            Flow::Continue(()) => Disp::FallThrough,
            Flow::Break(LoopOp::Continue) => Disp::Continue,
            Flow::Break(LoopOp::Break) => Disp::Break,
            Flow::Break(LoopOp::Return) => {
                panic!("the sor-retreat span has no function return")
            }
        };
        trace.push(disp);
        match disp {
            Disp::FallThrough | Disp::Continue => {}
            Disp::Break => break,
        }
    }

    let c = Carry {
        t: EvalCf::get_var(&t),
        omega: EvalCf::get_var(&omega),
        safe_t: EvalCf::get_var(&safe_t),
        sor_prev: EvalCf::get_var(&sor_prev),
        sor_step_prev: EvalCf::get_var(&sor_step_prev),
        exhausted: EvalCf::get_bool_var(&exhausted),
    };
    (c, trace)
}

/// The Numerical Recipes LCG (the same generator the other eDSL suites use).
struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Lcg(seed)
    }
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }
    /// A float in roughly `[-range, range]`.
    fn next_f32(&mut self, range: f32) -> f32 {
        let u = (self.next_u32() as f32) / (u32::MAX as f32); // [0, 1]
        (u * 2.0 - 1.0) * range
    }
    /// A non-negative float in `[0, range]`.
    fn next_pos_f32(&mut self, range: f32) -> f32 {
        let u = (self.next_u32() as f32) / (u32::MAX as f32); // [0, 1]
        u * range
    }
}

#[test]
fn eval_oracle_bit_identical_over_lcg_sweep() {
    let mut lcg = Lcg::new(0x0B17_50A2_C4D9);
    const CASES: usize = 10_000;
    // The over-relax loop runs up to MAX_IT in production; a 24-step `d`-sequence per case is more
    // than enough to exercise the over-relax step, the sor-fail retreat (which flips omega to 1.0 →
    // the plain else-arm for the rest of the case), and the T_MAX miss — many times each across the
    // sweep, with the disposition steering the loop EXACTLY as the GPU does.
    const STEPS: usize = 24;
    for case in 0..CASES {
        // A random initial carry. `omega` straddles 1.0 (so both the over-relax arm and the plain
        // else-arm are entered); the floats span the field's working range; `safe_t`/`sor_prev`/
        // `sor_step_prev` are seeded so the Lipschitz `<` can both hold and fail across the sweep.
        let c0 = Carry {
            t: lcg.next_pos_f32(3.0),
            omega: 1.0 + lcg.next_f32(0.6), // ~[0.4, 1.6] — straddles the 1.0 gate
            safe_t: lcg.next_pos_f32(3.0),
            sor_prev: lcg.next_pos_f32(1.0),
            sor_step_prev: lcg.next_pos_f32(1.5),
            exhausted: (lcg.next_u32() & 1) == 0,
        };

        // Two d-generators (the brief's ≥2): a SMALL-magnitude positive-leaning sequence (lets the
        // over-relax step accumulate slowly so the loop runs long) and a WIDE signed sequence (`d`
        // can be large or negative, so a step can overshoot T_MAX quickly and the Lipschitz test
        // flips). Both share the carry's RNG stream (distinct cases).
        let gen_small: Vec<f32> = (0..STEPS).map(|_| lcg.next_pos_f32(0.8)).collect();
        let gen_wide: Vec<f32> = (0..STEPS).map(|_| lcg.next_f32(6.0)).collect();

        for (gi, ds) in [&gen_small, &gen_wide].into_iter().enumerate() {
            let (host_c, host_trace) = host_sor_loop(c0, ds);
            let (body_c, body_trace) = body_sor_loop(c0, ds);

            assert_eq!(
                host_trace, body_trace,
                "case {case} gen {gi}: disposition trace MISMATCH (host {host_trace:?} vs body \
                 {body_trace:?}) for c0={c0:?} ds={ds:?}"
            );
            host_c.assert_eq_bits(&body_c, &format!("case {case} gen {gi}"));
        }
    }
}

#[test]
fn eval_oracle_covers_the_five_witnesses() {
    // Hand-built witnesses that the 5 design edge cases are EXERCISED (each asserts host == body AND
    // the expected control-flow outcome). A small explicit `d`-sequence drives each.

    // 1. sor-fail at it >= 1 resumes (t = safe_t + sor_prev, omega = 1.0, continue). it == 0 takes a
    //    normal over-relax step (seeding safe_t/sor_prev/sor_step_prev); it == 1 is engineered so
    //    `sor_prev + d < L * sor_step_prev` HOLDS → the retreat fires.
    {
        // c0: omega = 1.5 (over-relax). it=0: d=0.5 → step_len = 0.75, safe_t = t(=1.0), sor_prev =
        // 0.5, sor_step_prev = 0.75, t = 1.75. it=1: d = 0.01 → sor_prev(0.5) + d(0.01) = 0.51 <
        // L(1.41421356) * sor_step_prev(0.75) = 1.0607 → retreat: t = safe_t(1.0) + sor_prev(0.5) =
        // 1.5, omega = 1.0, continue.
        let c0 = Carry { t: 1.0, omega: 1.5, safe_t: 0.0, sor_prev: 0.0, sor_step_prev: 0.0, exhausted: true };
        let ds = [0.5_f32, 0.01];
        let (hc, ht) = host_sor_loop(c0, &ds);
        let (bc, bt) = body_sor_loop(c0, &ds);
        assert_eq!(ht, bt);
        hc.assert_eq_bits(&bc, "witness 1");
        assert_eq!(bt[1], Disp::Continue, "it==1 must take the sor-fail retreat continue");
        assert_eq!(bc.omega.to_bits(), 1.0f32.to_bits(), "retreat sets omega = 1.0");
        assert_eq!(bc.t.to_bits(), 1.5f32.to_bits(), "retreat sets t = safe_t + sor_prev = 1.5");
    }

    // 2. it == 0 guard SUPPRESSES a first-iter fail: even if the Lipschitz `<` would hold, `it > 0u`
    //    short-circuits FALSE on it==0 → NO retreat (a normal over-relax step instead).
    {
        // sor_step_prev large so `sor_prev + d < L * sor_step_prev` WOULD hold, but it == 0.
        let c0 = Carry { t: 1.0, omega: 1.5, safe_t: 9.0, sor_prev: 0.0, sor_step_prev: 100.0, exhausted: true };
        let ds = [0.2_f32];
        let (hc, ht) = host_sor_loop(c0, &ds);
        let (bc, bt) = body_sor_loop(c0, &ds);
        assert_eq!(ht, bt);
        hc.assert_eq_bits(&bc, "witness 2");
        assert_ne!(bt[0], Disp::Continue, "it==0 must NOT retreat (it > 0u short-circuits false)");
        // The normal over-relax step ran: safe_t captured t(=1.0), omega unchanged at 1.5.
        assert_eq!(bc.omega.to_bits(), 1.5f32.to_bits(), "no retreat → omega unchanged");
        assert_eq!(bc.safe_t.to_bits(), 1.0f32.to_bits(), "the over-relax step captured safe_t = old t");
    }

    // 3. omega == 1.0 0%-gate else-arm (t += d; safe_t/sor_prev/sor_step_prev UNCHANGED).
    {
        let c0 = Carry { t: 2.0, omega: 1.0, safe_t: 7.0, sor_prev: 8.0, sor_step_prev: 9.0, exhausted: true };
        let ds = [0.3_f32];
        let (hc, ht) = host_sor_loop(c0, &ds);
        let (bc, bt) = body_sor_loop(c0, &ds);
        assert_eq!(ht, bt);
        hc.assert_eq_bits(&bc, "witness 3");
        assert_eq!(bt[0], Disp::FallThrough, "omega == 1.0 takes the plain else-arm, no continue/break");
        assert_eq!(bc.t.to_bits(), 2.3f32.to_bits(), "else-arm is t = t + d");
        // The over-relax-only state is UNTOUCHED by the plain arm.
        assert_eq!(bc.safe_t.to_bits(), 7.0f32.to_bits(), "plain arm leaves safe_t unchanged");
        assert_eq!(bc.sor_prev.to_bits(), 8.0f32.to_bits(), "plain arm leaves sor_prev unchanged");
        assert_eq!(bc.sor_step_prev.to_bits(), 9.0f32.to_bits(), "plain arm leaves sor_step_prev unchanged");
    }

    // 4. write-order: safe_t captures the OLD t (BEFORE the step), asserted on the NEXT iteration.
    {
        // it=0: omega=1.5, t=1.0, d=0.4 → step_len=0.6, safe_t = OLD t = 1.0, t = 1.6. it=1: a sor-
        // fail engineered to fire → t = safe_t + sor_prev; if safe_t had captured the NEW t (1.6),
        // the retreat target would differ. sor_prev(0.4) + d(0.01) = 0.41 < L*sor_step_prev(0.6) =
        // 0.8485 → retreat: t = safe_t(1.0) + sor_prev(0.4) = 1.4 (NOT 1.6 + 0.4 = 2.0).
        let c0 = Carry { t: 1.0, omega: 1.5, safe_t: 0.0, sor_prev: 0.0, sor_step_prev: 0.0, exhausted: true };
        let ds = [0.4_f32, 0.01];
        let (hc, ht) = host_sor_loop(c0, &ds);
        let (bc, bt) = body_sor_loop(c0, &ds);
        assert_eq!(ht, bt);
        hc.assert_eq_bits(&bc, "witness 4");
        assert_eq!(bt[1], Disp::Continue, "it==1 retreat must fire");
        assert_eq!(
            bc.t.to_bits(),
            1.4f32.to_bits(),
            "safe_t captured the OLD t (1.0): retreat target = 1.0 + 0.4 = 1.4 (not 2.0)"
        );
    }

    // 5. miss break (t > T_MAX → exhausted = false, break). A big d overshoots T_MAX in one step.
    {
        let c0 = Carry { t: 5.0, omega: 1.5, safe_t: 0.0, sor_prev: 0.0, sor_step_prev: 0.0, exhausted: true };
        // it=0: step_len = (T_MAX+5)*1.5, t = 5 + that >> T_MAX → miss break.
        let ds = [T_MAX + 5.0];
        let (hc, ht) = host_sor_loop(c0, &ds);
        let (bc, bt) = body_sor_loop(c0, &ds);
        assert_eq!(ht, bt);
        hc.assert_eq_bits(&bc, "witness 5");
        assert_eq!(bt[0], Disp::Break, "the T_MAX overshoot must break");
        assert!(!bc.exhausted, "the miss break clears exhausted = false");
        assert!(bc.t > T_MAX, "t must overshoot T_MAX before the miss break");
    }
}

// ---- (c) The new condition leaves' control table (EvalCf) ------------------------------

#[test]
fn eval_ugt_and2_uint_lit_round_trip() {
    // `ugt` — the uint strict-`>` (the `it > 0u` guard).
    assert!(EvalCf::ugt(1, EvalCf::uint_lit(0)), "1 > 0u must be true");
    assert!(!EvalCf::ugt(0, EvalCf::uint_lit(0)), "0 > 0u must be false (the it==0 guard)");
    assert!(EvalCf::ugt(5, 3), "5 > 3 must be true");

    // `and2` — the eager logical AND (result-equivalent to the GPU short-circuit).
    assert!(EvalCf::and2(true, true), "true && true");
    assert!(!EvalCf::and2(true, false), "true && false");
    assert!(!EvalCf::and2(false, true), "false && true");
    assert!(!EvalCf::and2(false, false), "false && false");

    // `uint_lit` — the literal IS its value on Eval.
    assert_eq!(EvalCf::uint_lit(0), 0, "uint_lit(0) is the value 0");
    assert_eq!(EvalCf::uint_lit(42), 42, "uint_lit(42) is the value 42");
}

// ---- (b) The Emit generator structure guard (feature = "emit") ------------------------

#[cfg(feature = "emit")]
mod emit_structure {
    fn generated() -> String {
        boyko_shaderdsl::emit::emit_hlsl_b1_sor_retreat().replace("\r\n", "\n")
    }

    #[test]
    fn omega_gate_is_two_arm_if_else() {
        let g = generated();
        // The 0%-gate: `if (omega > 1.0) { ... } else { ... }` — a TWO-arm branch (`Cf::if_else`).
        assert!(
            g.contains("if (omega > 1.0) {"),
            "the over-relax gate must spell `if (omega > 1.0) {{`:\n{g}"
        );
        assert!(g.contains("} else {"), "the gate must have an `else` arm (the plain step):\n{g}");
    }

    #[test]
    fn condition_is_unparenthesized_ugt_and2_lt_mul() {
        let g = generated();
        // THE KEY assertion (the precedence risk): the condition prints FLAT, NO parens —
        // `it > 0u && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev`. And2 prints both operands
        // at Root; the comparisons fall to `_ => false`; the inner Add (left) / Mul (right) at Root
        // never wrap. A stray paren here would be a printer regression (the cmp-.spv would catch it,
        // but this fails loudly first).
        assert!(
            g.contains("if (it > 0u && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev) {"),
            "the retreat condition must print FLAT (no parens), `uint` `>` + `&&` + `<` + inline \
             `*`:\n{g}"
        );
        // Explicitly forbid the parenthesized shapes a printer regression would produce.
        for forbidden in [
            "(it > 0u)",
            "(sor_prev + d)",
            "(FIELD_LIPSCHITZ_L * sor_step_prev)",
            "it > 0u && (",
        ] {
            assert!(
                !g.contains(forbidden),
                "the condition must NOT parenthesize `{forbidden}`:\n{g}"
            );
        }
        // The `&&` is the logical AND node (NOT the bitwise `&`), and `0u` is a bare literal.
        assert!(g.contains(" && "), "must spell the logical `&&`:\n{g}");
        assert!(!g.contains(" & "), "must NOT spell the bitwise `&` (And2 != And):\n{g}");
        assert!(g.contains("it > 0u"), "`0u` must be a bare uint literal (uint_lit, not a symbol):\n{g}");
    }

    #[test]
    fn step_len_is_named_temp_and_threshold_is_inline_mul() {
        let g = generated();
        // `step_len` IS a named `float` temp (`d * omega`); the threshold `FIELD_LIPSCHITZ_L *
        // sor_step_prev` is an INLINE Mul in the condition (NOT a temp). The FMA pin.
        assert!(
            g.contains("float step_len = d * omega;"),
            "`step_len` must be a named float temp `float step_len = d * omega;`:\n{g}"
        );
        // The threshold is inline in the condition (asserted by the flat-condition test above); it
        // must NOT be materialized as its own temp.
        assert!(
            !g.contains("= FIELD_LIPSCHITZ_L * sor_step_prev;"),
            "the Lipschitz threshold must be INLINE in the condition, not its own temp:\n{g}"
        );
    }

    #[test]
    fn retreat_then_block_in_order_then_continue() {
        let g = generated();
        // The composite retreat then-block: `t = safe_t + sor_prev;` THEN `omega = 1.0;` THEN
        // `continue;`, in order.
        assert!(g.contains("t = safe_t + sor_prev;"), "retreat must plain-resume `t = safe_t + sor_prev;`:\n{g}");
        assert!(g.contains("omega = 1.0;"), "retreat must set `omega = 1.0;`:\n{g}");
        assert!(g.contains("continue;"), "retreat must `continue;`:\n{g}");
    }

    #[test]
    fn steps_are_set_var_plus_form_r1() {
        let g = generated();
        // R1: BOTH steps spell the `set_var` form (`t = t + step_len;` / `t = t + d;`), byte-
        // identical to the committed `t += step_len;` / `t += d;` in the `.spv` — no compound leaf.
        assert!(g.contains("t = t + step_len;"), "the over-relax step must spell `t = t + step_len;`:\n{g}");
        assert!(g.contains("t = t + d;"), "the plain else step must spell `t = t + d;`:\n{g}");
        assert!(!g.contains("t += "), "no compound-assign form (R1 is the set_var form):\n{g}");
    }

    #[test]
    fn miss_break_sets_exhausted_then_breaks() {
        let g = generated();
        // The `t > T_MAX` composite miss: `exhausted = false;` THEN `break;`.
        assert!(g.contains("if (t > T_MAX) {"), "the miss guard must spell `if (t > T_MAX) {{`:\n{g}");
        assert!(g.contains("exhausted = false;"), "the miss must clear `exhausted = false;`:\n{g}");
        assert!(g.contains("break;"), "the miss must `break;`:\n{g}");
    }

    #[test]
    fn span_has_no_redecl_or_loop_header_or_field_call() {
        let g = generated();
        // The span is INSIDE the hand-written `for (uint it)` loop with hand-written carried decls,
        // so it must NOT redeclare any carried var, re-emit the loop header / mesh-guard / sdf call.
        for forbidden in [
            "float t =",
            "float omega",
            "bool exhausted",
            "uint it",
            "for (",
            "[loop]",
            "sdf(",
            "t >= t_mesh",
        ] {
            assert!(
                !g.contains(forbidden),
                "the generated span must NOT emit the hand-written construct `{forbidden}`:\n{g}"
            );
        }
    }

    #[test]
    fn full_span_brace_matched_golden() {
        let g = generated();
        // The WHOLE span, brace-matched — the canonical generated text (the committed L1459-1498 with
        // comments stripped + the R1 `t = t + …` form), printed at DEPTH 2 (8-space `if (omega >
        // 1.0)` — the site nests main→`for (uint it)`→this step). A single golden so a structural
        // drift (a missing statement, a reordered line, a stray temp, a wrong indent, a stray paren)
        // fails loudly.
        const GOLDEN: &str = "        if (omega > 1.0) {
            float step_len = d * omega;
            if (it > 0u && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev) {
                t = safe_t + sor_prev;
                omega = 1.0;
                continue;
            }
            safe_t = t;
            sor_prev = d;
            sor_step_prev = step_len;
            t = t + step_len;
        } else {
            t = t + d;
        }
        if (t > T_MAX) {
            exhausted = false;
            break;
        }
";
        assert_eq!(g, GOLDEN, "the generated span must match the brace-matched golden");
    }
}
