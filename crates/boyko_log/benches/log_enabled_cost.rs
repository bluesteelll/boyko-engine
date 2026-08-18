//! The enabled path's per-record cost: five rows of the ladder's bench table, one sitting.
//!
//! | row | bound | control |
//! |---|---|---|
//! | `log_enabled_0args` | ≤ 15 ns | runtime-disabled |
//! | `log_enabled_2u32` | ≤ 20 ns | runtime-disabled |
//! | `log_enabled_str32` | ≤ 30 ns | runtime-disabled |
//! | `log_enabled_rate_once_fired` | ≤ 5 ns, no store | the `Every` policy |
//! | `log_enabled_sampled_out` | regression guard: ≤ 8 ns **and** ≥ 4 quanta saved | the same site at shift 0 |
//!
//! # The drain is outside the timed region, and that is not a convenience
//!
//! A lane is 16 KiB. At ~24 B a record it holds a few hundred, so a timed loop of 20 000 would
//! spend most of its iterations on the **refusal** path — measuring what a full ring costs, under
//! the name of what an emission costs. `CALLS` is therefore 256, the lane is drained between timed
//! blocks, and the drain's cost is never inside a reading.
//!
//! MEASURED, in a different rung: L12's control leg published 1000 records into this same lane and
//! read 573. The number that came back was the overflow's, wearing the leg's name.
//!
//! # Every leg carries its own control, and the controls differ
//!
//! `log_enabled_*` are read against the runtime-DISABLED site, because the bound is the cost of
//! doing the work versus the cost of deciding not to. `rate_once_fired` is read against the `Every`
//! policy, because its claim is about the latch and not about emission. `sampled_out` is read
//! against the same site at shift 0, because its claim is about the sampling decision and nothing
//! else. A single shared control would answer none of the three questions.

use std::hint::black_box;
use std::time::Instant;

#[path = "instrument.rs"]
mod instrument;
use instrument::{at_floor, med_and_floor, resolution_ns};

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log, info, warn};

/// Records per timed block. Bounded by the 16 KiB lane, not by taste: see the module header.
const CALLS: u32 = 256;

/// Rounds in the sitting. Legs interleave, so drift lands on all of them.
const ROUNDS: usize = 41;

/// A 32-byte string, the `_str32` row's subject.
/// A DOWNSTREAM code table, declared here exactly as a game or a mod declares one.
///
/// The point of the row it feeds: a downstream code's index is `CodeIdx::Dynamic`, so its `warn!`
/// pays one `AtomicU16` load that an engine code -- whose index is `CodeIdx::Static`, resolved when
/// the table compiles -- does not. The delta IS that load, and nothing else differs between the
/// two legs: same target, same level, same argument shapes, same rate policy.
mod acme {
    use boyko_log::RatePolicy;

    boyko_log::declare_codes! {
        prefix = "acme",
        (1, W, ACME_W0001, RatePolicy::Every, "a downstream warning, for the bench"),
    }
}

const S32: &str = "0123456789abcdef0123456789abcde";



/// Time one block of `CALLS` emissions, then drain OUTSIDE the reading.
///
/// The `black_box` barrier is applied identically to every leg including the empty control, so its
/// own cost is in all readings and cancels in a delta. Without it the gate's `Relaxed` load hoists
/// out of the loop and the body empties — measured in `log_gate_cost`, where an unbarriered leg
/// read 0.0008 ns above an empty loop and still printed a verdict.
#[inline(never)]
fn time_block<F: FnMut()>(mut f: F) -> f64 {
    let mut barrier = 0u8;
    let t0 = Instant::now();
    for _ in 0..CALLS {
        black_box(&mut barrier);
        f();
    }
    let ns = t0.elapsed().as_nanos() as f64 / f64::from(CALLS);
    black_box(barrier);
    // Emptied after the clock is read: a drain inside the reading would put a whole sink pass into
    // a number whose name says "one emission".
    let _ = drain();
    ns
}

fn arm(level: Level, shift: u8) {
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(level, shift, false));
}

fn main() {
    // A real manual sink: records must actually be consumed, or the lane fills and the leg measures
    // the refusal path. `file: false` keeps the OS out of the reading — the subject is the
    // producer, and a file write would be a constant in every leg large enough to hide all five.
    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: false,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");
    boyko_log::sink::slot::reset();

    let mut disabled = Vec::with_capacity(ROUNDS);
    let mut args0 = Vec::with_capacity(ROUNDS);
    let mut args2 = Vec::with_capacity(ROUNDS);
    let mut str32 = Vec::with_capacity(ROUNDS);
    let mut once_fired = Vec::with_capacity(ROUNDS);
    let mut every = Vec::with_capacity(ROUNDS);
    let mut sampled = Vec::with_capacity(ROUNDS);
    let mut down = Vec::with_capacity(ROUNDS);
    let mut engine_w = Vec::with_capacity(ROUNDS);
    let mut unsampled = Vec::with_capacity(ROUNDS);

    // The `Once` latch has to be spent BEFORE it is timed: the row is
    // `log_enabled_rate_once_fired`, the cost of a code whose latch has ALREADY fired. Timing the
    // first call would time the emission it suppresses ever after.
    arm(Level::Trace, 0);
    warn!(Log, boyko_log::codes::W0103, "spend the latch {}", 0u32);
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };

    for _ in 0..ROUNDS {
        arm(Level::Off, 0);
        disabled.push(time_block(|| info!(Log, "disabled")));

        arm(Level::Trace, 0);
        args0.push(time_block(|| info!(Log, "zero arguments")));
        args2.push(time_block(|| info!(Log, "two {} {}", black_box(7u32), black_box(9u32))));
        str32.push(time_block(|| info!(Log, "str {}", black_box(S32))));

        // The latch is spent, so this is the suppressed path: a load and a branch, no store.
        once_fired.push(time_block(|| warn!(Log, boyko_log::codes::W0103, "latched {}", 0u32)));
        // `E0107` is `Every` — the same shape of call with no latch to consult.
        every.push(time_block(|| {
            boyko_log::error!(Log, boyko_log::codes::E0107, "every {}", 0u32);
        }));

        // Sampling suppresses DELIVERY and never argument evaluation, so both legs build the same
        // tuple; the delta is the sampling decision alone.
        arm(Level::Trace, 6);
        sampled.push(time_block(|| info!(Log, "sampled {}", black_box(7u32))));
        arm(Level::Trace, 0);
        unsampled.push(time_block(|| info!(Log, "sampled {}", black_box(7u32))));

        // `downstream_code_warn` against the engine-code `warn!`. Everything but the code's ORIGIN
        // is held identical: same MACRO (`warn!`, so the same level and the same expansion), same
        // target, same one `u32` argument, same `Every` rate policy.
        //
        // A first draft used `error!` for the control and claimed the legs were identical anyway.
        // They were not -- a different macro is a different expansion, and a delta between two
        // different expansions cannot be attributed to the one field that also differs.
        down.push(time_block(|| warn!(Log, acme::ACME_W0001, "downstream {}", black_box(7u32))));
        engine_w.push(time_block(|| {
            warn!(Log, boyko_log::codes::W0117, "downstream {}", black_box(7u32));
        }));
    }
    arm(Level::Off, 0);

    // Resolution is the clock's tick spread over one block: a per-call reading cannot express a
    // difference finer than this, no matter how many rounds are averaged.
    let resolution = resolution_ns(CALLS);

    let (med_dis, se_dis) = med_and_floor(&mut disabled, resolution);
    let (med_0, se_0) = med_and_floor(&mut args0, resolution);
    let (med_2, se_2) = med_and_floor(&mut args2, resolution);
    let (med_s, se_s) = med_and_floor(&mut str32, resolution);
    let (med_once, se_once) = med_and_floor(&mut once_fired, resolution);
    let (med_every, se_every) = med_and_floor(&mut every, resolution);
    let (med_samp, se_samp) = med_and_floor(&mut sampled, resolution);
    let (med_unsamp, _) = med_and_floor(&mut unsampled, resolution);
    let (med_down, se_down) = med_and_floor(&mut down, resolution);
    let (med_engw, se_engw) = med_and_floor(&mut engine_w, resolution);

    println!(
        "instrument: clock tick {:.0} ns / {CALLS}-call block = {resolution:.3} ns/call resolution",
        resolution * f64::from(CALLS)
    );

    println!("control  runtime-disabled     : {med_dis:7.2} ns  (se {se_dis:.2})");
    // Printed, not buried in a delta: the control is at the floor, so "delta over control" is the
    // leg minus a quantity this clock cannot resolve. The bounds below are still meaningful --
    // they are absolute, and the legs are 24-32 quanta -- but the DELTA column is not a
    // measurement of the gate, and `log_gate_cost` reached the same conclusion on this box by a
    // different route.
    if at_floor(med_dis, resolution) {
        println!(
            "  NOTE: the disabled control is {:.1} quanta -- AT the instrument floor, so every \"delta over control\" below is bounded by resolution, not by the gate",
            med_dis / resolution
        );
    }
    println!("log_enabled_0args             : {med_0:7.2} ns  (se {se_0:.2})  bound 15");
    println!("log_enabled_2u32              : {med_2:7.2} ns  (se {se_2:.2})  bound 20");
    println!("log_enabled_str32             : {med_s:7.2} ns  (se {se_s:.2})  bound 30");
    println!("log_enabled_rate_once_fired   : {med_once:7.2} ns  (se {se_once:.2})  bound 5");
    println!("control  rate Every           : {med_every:7.2} ns  (se {se_every:.2})");
    println!("log_enabled_sampled_out       : {med_samp:7.2} ns  (se {se_samp:.2})  bound 6");
    println!("control  same site, shift 0   : {med_unsamp:7.2} ns");

    // Each row is reported against its OWN control, and a leg that does not clear its control's
    // spread floor is NOT RESOLVED rather than passed: a reading inside the instrument's own
    // wobble is a fact about the instrument.
    let verdict = |name: &str, med: f64, bound: f64, ctl: f64, se: f64| {
        let delta = med - ctl;
        // A leg within two quanta of zero is the CLOCK, not the code. Printing "0.39 ns, PASS" for
        // such a leg reports the instrument's floor as the subject's cost -- which is how
        // `log_gate_cost` on this same box already arrived at NOT MEASURABLE (instrument).
        if at_floor(med, resolution) {
            println!(
                "  {name}: AT THE INSTRUMENT FLOOR ({med:.2} ns = {:.1} quanta of {resolution:.3})",
                med / resolution
            );
        } else if delta.abs() < se {
            println!("  {name}: NOT RESOLVED against its control (delta {delta:.2} ns < se {se:.2})");
        } else if med <= bound {
            println!("  {name}: PASS  ({med:.2} <= {bound}, delta over control {delta:.2} ns)");
        } else {
            println!("  {name}: OVER BOUND ({med:.2} > {bound}, delta over control {delta:.2} ns)");
        }
    };
    verdict("log_enabled_0args", med_0, 15.0, med_dis, se_0 + se_dis);
    verdict("log_enabled_2u32", med_2, 20.0, med_dis, se_2 + se_dis);
    verdict("log_enabled_str32", med_s, 30.0, med_dis, se_s + se_dis);
    // THIS ROW HAS NO SUBJECT ON THIS TREE, AND THE BENCH SAYS SO RATHER THAN SAYING "NOT
    // RESOLVED". The bound assumes a fired `Once` latch short-circuits the emission -- "<= 5 ns,
    // no store, no shared line". The emission macros never call `rate::admit` (stated in
    // `codes.rs`'s header, and gated there), so a `Once` code costs EXACTLY what an `Every` code
    // costs and the measured delta is 0.00 ns by construction, not by coincidence.
    //
    // Reporting that as "NOT RESOLVED" would blame the instrument for an absence in the product.
    // The two verdicts look identical on a plot and mean opposite things: one says measure harder,
    // the other says there is nothing there to measure.
    let once_delta = (med_once - med_every).abs();
    if once_delta < (se_once + se_every) {
        println!("  log_enabled_rate_once_fired: NO SUBJECT");
        println!(
            "    a `Once` code costs what an `Every` code costs ({med_once:.2} vs {med_every:.2} ns)"
        );
        println!("    the emission macros do not consult `rate::admit`;");
        println!("    the row bounds a short-circuit that does not exist on this tree");
    } else {
        verdict("log_enabled_rate_once_fired", med_once, 5.0, med_every, se_once + se_every);
    }
    // ── downstream_code_warn ────────────────────────────────────────────────────────────────
    //
    // The ABSOLUTE bound is measured here and stays as specified -- it has never been measured, so
    // there is nothing to re-cut it from.
    //
    // THE DELTA HAS NO SUBJECT ON THIS PATH, and the reason is structural rather than statistical.
    // `resolve_idx` -- the `idx_cell` load the row names -- is called from exactly ONE place in the
    // crate: `CodeNewtype::code_idx`, which exists to address the RATE array. No emission macro
    // calls it. So a downstream code and an engine code reach the ring by the same instructions,
    // and the delta is zero because there is no load, not because the clock cannot see one.
    //
    // This is the same root cause as `log_enabled_rate_once_fired`'s NO SUBJECT: the macros do not
    // consult the rate machinery, and the index exists to index it.
    //
    // What the load WOULD cost is measured in `code_idx_cost`, where it resolves cleanly at
    // ~1.35 ns -- the number this row will need on the day `rate::admit` is wired into emission.
    const DOWNSTREAM_BOUND_NS: f64 = 18.0;
    let idx_cost = med_down - med_engw;
    println!("downstream_code_warn          : {med_down:7.2} ns  (se {se_down:.2})  bound {DOWNSTREAM_BOUND_NS}");
    println!("control  engine-code warn     : {med_engw:7.2} ns  (se {se_engw:.2})");
    let abs = if med_down <= DOWNSTREAM_BOUND_NS { "PASS" } else { "OVER BOUND" };
    println!("  downstream_code_warn: absolute {abs} ({med_down:.2} vs {DOWNSTREAM_BOUND_NS} ns)");
    println!(
        "    delta = {idx_cost:.2} ns -- NO SUBJECT: no emission macro calls `resolve_idx`, so a"
    );
    println!(
        "    downstream code and an engine code reach the ring by the same instructions. See"
    );
    println!("    `code_idx_cost` for what the load costs where it is actually performed.");

    // `log_enabled_sampled_out` IS A REGRESSION GUARD, NOT AN ACCEPTANCE BOUND (owner ruling
    // 2026-08-17, the same ruling that re-cut L13b's 5x). The `<= 6 ns` line was written before
    // anything was measured; the reading is 6.64 ns, reproducibly and exactly 17 quanta.
    //
    // The guard has TWO clauses and the first one is the real property. Sampling exists to suppress
    // DELIVERY, so what must hold is that the sampled leg is cheaper than the same site at shift 0
    // -- measured at 10 quanta cheaper. An absolute-only bound would go green on a build where
    // sampling did nothing but everything else got faster.
    const SAMPLED_MAX_NS: f64 = 8.0;
    const SAMPLED_MIN_QUANTA_SAVED: f64 = 4.0;
    let saved_quanta = (med_unsamp - med_samp) / resolution;
    if saved_quanta < SAMPLED_MIN_QUANTA_SAVED {
        println!(
            "  log_enabled_sampled_out: REGRESSION -- saves {saved_quanta:.1} quanta, guard is {SAMPLED_MIN_QUANTA_SAVED}"
        );
    } else if med_samp > SAMPLED_MAX_NS {
        println!(
            "  log_enabled_sampled_out: REGRESSION -- {med_samp:.2} ns over the {SAMPLED_MAX_NS} ns guard"
        );
    } else {
        println!(
            "  log_enabled_sampled_out: PASS ({med_samp:.2} ns <= {SAMPLED_MAX_NS} ns, saves {saved_quanta:.1} quanta)"
        );
    }
}
