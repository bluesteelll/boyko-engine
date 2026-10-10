//! **G8(d)** — does a *dynamic* site's missing compile-time gate cost anything measurable?
//!
//! # What this is, and the state of the table it belongs to
//!
//! `05-LADDER-GATES.md` carries a twelve-row bench table with hard targets (≤ 3 ns, ≤ 4 ns,
//! ≤ 15/20/30 ns) and named controls. **Measured at L10-C: not one of those twelve existed.**
//! `boyko_log` had no `benches/` directory and no bench by any of those names existed anywhere
//! under `crates/`. Eleven rungs were reported as gated while the performance half of their gate
//! table was empty — the same defect class the rest of this campaign keeps finding, in its purest
//! form: not a gate that could not fire, but a gate that was never built.
//!
//! This file builds **two** of the twelve, plus the control the comparison needs. The other ten
//! remain unbuilt and are not implied to be satisfied by this one running.
//!
//! # The claim, and that it can be withdrawn
//!
//! A `dyn_*!` site has **two** gates where a static site has three: `T::STATIC_CEILING` is a
//! `const` on a trait impl and a dynamic target is a value, so there is no impl to read one from.
//! Decision 18 states the cost rather than smoothing it, and G8(d) turns it into a claim that can
//! be **withdrawn**: if `log_dyn_disabled − log_disabled_runtime` does not resolve above this
//! sitting's floor, *"Decision 2's claim that gate (a) buys anything is STRUCK from this corpus"*
//! rather than restated.
//!
//! Written down before the first run, so it cannot be retrofitted to whatever came out: with the
//! target `Off` both forms fold at gate (c). The static form's `T::ID` is a compile-time constant;
//! the dynamic form must evaluate its `id` operand first. The expected delta is on the order of a
//! single load, and whether one load resolves above this box's floor is genuinely open.
//!
//! # The statistics are the corpus's, the artifact types are not
//!
//! `03-STATISTICS.md` specifies the band, and this file implements that **formula**:
//! `SE(median) ≈ MEDIAN_SE_FACTOR·σ/√n` with `σ̂ ≈ (p95 − median)/Z95`.
//!
//! It deliberately does **not** reuse `boyko_app::profiling::contrast::resolve`. That instrument's
//! `LegSummary` carries a `WorkloadTag` and the `SessionId` halves of the process it came from, and
//! its `Floor` is built from a repetition *file* — it is built for profiler-zone medians across
//! artifact sessions. Feeding it a microbenchmark of a one-nanosecond gate would mean fabricating a
//! workload tag and a session id to satisfy a type, which distorts the instrument worse than
//! reusing ten lines of arithmetic. The formula is shared; the artifact machinery is not.
//!
//! # Why the twin is leg A measured TWICE
//!
//! S2 is explicit and was learned the expensive way: *"a zero control whose expected value is
//! exactly zero measures DRIFT, not RESOLUTION"* — P4-6's twin read 0 on all ten passes, the rule
//! silently became "is nonzero", and it reported a false RESOLVED. So the control here is **not**
//! an empty loop. It is `log_disabled_runtime` measured a second time in the same round: two legs
//! that are the same code, whose difference is expected to be exactly zero, and whose observed
//! spread therefore measures what this sitting's noise can manufacture. The empty loop is measured
//! too, but only to subtract loop overhead from the reported per-call figures — it is never the
//! band.
//!
//! Rounds are **counterbalanced** (the leg order reverses on odd rounds), so a monotone drift over
//! the sitting cannot masquerade as a difference between legs.

use std::hint::black_box;
use std::time::Instant;

use boyko_log::target::{
    LogTarget, TargetControl, TargetId, register_dynamic_target, set_target_control,
};
use boyko_log::{Ecs, Level};

/// `√(π/2)`, the median's standard error against the mean's. `03-STATISTICS.md`'s constant.
const MEDIAN_SE_FACTOR: f64 = 1.253_314_1;
/// The 95th-percentile z-score, for recovering σ̂ from `(p95 − median)`.
const Z95: f64 = 1.644_853_6;

/// Calls per timed leg. Large enough that one `Instant` pair spans millions of gate evaluations,
/// so the clock's own resolution is not the thing being measured.
const CALLS: u32 = 2_000_000;
/// Rounds in the sitting. Every leg is measured once per round, and the legs are interleaved rather
/// than run in blocks — a block-per-leg layout charges each leg a different part of the drift.
const ROUNDS: usize = 41;

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in a duration sample"));
    v[v.len() / 2]
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    let i = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[i]
}

/// The corpus's floor term for one leg: `MEDIAN_SE_FACTOR · σ̂ / √n`, `σ̂ ≈ (p95 − median)/Z95`.
///
/// Mandatory, and mandatory *per leg*: a band built from the twin alone would shrink to nothing on
/// a quiet box and report RESOLVED for a difference finer than either leg is placed.
fn se_floor(samples: &mut [f64]) -> (f64, f64) {
    let med = median(samples);
    let p95 = quantile(samples, 0.95);
    let sigma = ((p95 - med) / Z95).abs();
    (med, MEDIAN_SE_FACTOR * sigma / (samples.len() as f64).sqrt())
}

/// ns per call for one leg, minus nothing — overhead is subtracted by the caller.
///
/// # The opacity barrier, and the first run that made it necessary
///
/// `runtime_ceiling` is a **`Relaxed` atomic load of a `.bss` byte that does not change during the
/// loop**, and LLVM's LICM hoists monotonic loads out of loops. The first version of this bench had
/// no barrier and measured `log_disabled_runtime` at **0.0008 ns above an empty loop** — the load
/// hoisted, the branch became loop-invariant, and the body emptied. It still printed `RESOLVED`,
/// because the dynamic leg happened to hoist less well: a verdict about which form the optimizer
/// prefers in a tight loop, wearing the words of a verdict about gate (a).
///
/// That is `03-STATISTICS.md`'s S2 in a new costume — an instrument whose subject was optimized
/// away reports a verdict without measuring anything — and it is why the number is not reported
/// from a run without this line.
///
/// `black_box` on a byte the loop also writes clobbers memory for the optimizer, so the load must
/// be re-issued each iteration. It is applied **identically to every leg, including the empty-loop
/// control**, so its own cost is in all three readings and cancels in the delta.
#[inline(never)]
fn time<F: FnMut()>(mut f: F) -> f64 {
    let mut barrier = 0u8;
    let t0 = Instant::now();
    for _ in 0..CALLS {
        black_box(&mut barrier);
        f();
    }
    let ns = t0.elapsed().as_nanos() as f64 / f64::from(CALLS);
    black_box(barrier);
    ns
}

fn main() {
    // Both targets `Off` — this measures the DISABLED path, which is the row the table bounds and
    // the state a shipped game runs in. `.bss` is already zero, but it is set explicitly so the
    // leg does not depend on nothing else in the process having armed the target.
    set_target_control(<Ecs as LogTarget>::ID, TargetControl::OFF);
    let dyn_id: TargetId = register_dynamic_target("bench:dyn", TargetControl::OFF)
        .expect("a fresh band has room for one name");
    assert_eq!(
        boyko_log::target_control(dyn_id).level(),
        Level::Off,
        "the dynamic leg must measure a DISABLED site, or it is measuring the ring instead"
    );

    let mut a = Vec::with_capacity(ROUNDS);
    let mut b = Vec::with_capacity(ROUNDS);
    let mut a2 = Vec::with_capacity(ROUNDS);
    let mut z = Vec::with_capacity(ROUNDS);
    let mut sink = 0u32;

    for round in 0..ROUNDS {
        // Counterbalanced: the order reverses on odd rounds, so a monotone drift across the sitting
        // lands on both legs equally instead of on whichever ran last.
        let fwd = round % 2 == 0;
        let run_a = || time(|| boyko_log::info!(Ecs, "disabled {}", black_box(1u32)));
        let run_b = || time(|| boyko_log::dyn_info!(dyn_id, "disabled {}", black_box(1u32)));
        let mut run_z = || time(|| sink = sink.wrapping_add(black_box(1u32)));

        if fwd {
            a.push(run_a());
            b.push(run_b());
            a2.push(run_a());
            z.push(run_z());
        } else {
            z.push(run_z());
            a2.push(run_a());
            b.push(run_b());
            a.push(run_a());
        }
    }
    black_box(sink);

    let (med_a, se_a) = se_floor(&mut a);
    let (med_b, se_b) = se_floor(&mut b);
    let (med_a2, _) = se_floor(&mut a2);
    let (med_z, _) = se_floor(&mut z);

    // The twin: A against A'. Expected exactly zero, so what it reports is drift.
    let mut twin: Vec<f64> = a.iter().zip(a2.iter()).map(|(x, y)| (x - y).abs()).collect();
    twin.sort_by(|p, q| p.partial_cmp(q).expect("no NaN"));
    let twin_ns = quantile(&twin, 0.90).max((med_a - med_a2).abs());

    let band = se_a + se_b + twin_ns;
    let delta = med_b - med_a;

    // ── THE POSITIVE CONTROL: can this instrument address the question at all? ────────────────
    //
    // Adding a call to a loop cannot make the loop FASTER. A negative net cost is therefore not a
    // small number — it is proof that what separates these legs is not what they call. At this
    // scale (sub-nanosecond) the legs' loop bodies compile to different shapes, and the difference
    // between those shapes exceeds the difference between the gates.
    //
    // Without this check the bench reports `RESOLVED` with a *tight* band, because the sitting is
    // highly REPEATABLE — and repeatability is not accuracy. That is `03-STATISTICS.md`'s S2 with
    // the sign flipped: there, a control that could not move reported a false RESOLVED; here, an
    // instrument that cannot attribute reports a confident verdict on an impossible reading.
    //
    // The band is not widened to swallow it. An instrument that cannot measure a quantity says so.
    let net_a = med_a - med_z;
    let net_b = med_b - med_z;
    let measurable = net_a > -band && net_b > -band;
    let resolved = measurable && delta.abs() > band;

    println!("== G8(d): log_dyn_disabled vs log_disabled_runtime ==");
    println!("  rounds {ROUNDS}, {CALLS} calls per leg per round, counterbalanced");
    println!("  empty-loop control      {med_z:.4} ns/iter");
    println!("  log_disabled_runtime    {med_a:.4} ns/call  (net {:.4})", med_a - med_z);
    println!("  log_dyn_disabled        {med_b:.4} ns/call  (net {:.4})", med_b - med_z);
    println!("  delta                   {delta:.4} ns");
    println!("  band                    {band:.4} ns  = se_a {se_a:.4} + se_b {se_b:.4} + twin {twin_ns:.4}");
    println!("  twin (A vs A', expected 0) {twin_ns:.4} ns");
    let verdict = if !measurable {
        "NOT MEASURABLE (instrument)"
    } else if resolved {
        "RESOLVED"
    } else {
        "NOT RESOLVED"
    };
    println!("  VERDICT                 {verdict}");
    println!();
    if !measurable {
        println!("  A NET COST IS NEGATIVE: a leg measured FASTER than the empty loop, which is not a");
        println!("  small effect but an impossible one. What separates these legs at this scale is the");
        println!("  shape their loop bodies compile to, not the gates they evaluate. The instrument");
        println!("  cannot address G8(d) on this box, so it reports that instead of a verdict.");
        println!();
        println!("  Decision 2's claim about gate (a) is therefore UNPROVEN here -- NOT struck. The");
        println!("  corpus strikes it when the comparison RESOLVES APART and finds nothing; it has not");
        println!("  been made. See 04-GAME-FACING.md Decision 18 and 05-LADDER-GATES.md G8(d).");
    } else {
        println!("  If NOT RESOLVED, Decision 2's claim that gate (a) buys anything is STRUCK from the");
        println!("  corpus rather than restated -- see 04-GAME-FACING.md Decision 18 and G8(d).");
    }
}
