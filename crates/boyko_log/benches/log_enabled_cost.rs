//! The enabled path's per-record cost: five rows of the ladder's bench table, one sitting.
//!
//! | row | bound | control |
//! |---|---|---|
//! | `log_enabled_0args` | ≤ 15 ns | runtime-disabled |
//! | `log_enabled_2u32` | ≤ 20 ns | runtime-disabled |
//! | `log_enabled_str32` | ≤ 30 ns | runtime-disabled |
//! | `log_enabled_rate_once_fired` | ≤ 5 ns, no store | the `Every` policy |
//! | `log_enabled_sampled_out` | ≤ 6 ns | the same site at shift 0 |
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

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log, info, warn};

/// Records per timed block. Bounded by the 16 KiB lane, not by taste: see the module header.
const CALLS: u32 = 256;

/// Rounds in the sitting. Legs interleave, so drift lands on all of them.
const ROUNDS: usize = 41;

/// A 32-byte string, the `_str32` row's subject.
const S32: &str = "0123456789abcdef0123456789abcde";

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in a timing sample"));
    v[v.len() / 2]
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    let idx = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[idx]
}

/// Median and a spread floor (`03-STATISTICS.md` S2): a control whose expected value is zero
/// measures DRIFT, not resolution, so the floor is mandatory rather than decorative.
fn se_floor(samples: &mut [f64]) -> (f64, f64) {
    let med = median(samples);
    let iqr = quantile(samples, 0.75) - quantile(samples, 0.25);
    let se = iqr / (samples.len() as f64).sqrt();
    (med, se.max(med * 0.02))
}

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
    }
    arm(Level::Off, 0);

    let (med_dis, se_dis) = se_floor(&mut disabled);
    let (med_0, se_0) = se_floor(&mut args0);
    let (med_2, se_2) = se_floor(&mut args2);
    let (med_s, se_s) = se_floor(&mut str32);
    let (med_once, se_once) = se_floor(&mut once_fired);
    let (med_every, se_every) = se_floor(&mut every);
    let (med_samp, se_samp) = se_floor(&mut sampled);
    let (med_unsamp, _) = se_floor(&mut unsampled);

    println!("control  runtime-disabled     : {med_dis:7.2} ns  (se {se_dis:.2})");
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
        if delta.abs() < se {
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
    verdict("log_enabled_sampled_out", med_samp, 6.0, med_unsamp, se_samp);
}
