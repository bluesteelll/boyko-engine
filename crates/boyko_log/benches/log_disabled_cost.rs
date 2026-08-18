//! `log_disabled_warn` against `log_disabled_runtime` — **the row S5 owes an answer to**.
//!
//! `05-LADDER-GATES.md`: *`log_disabled_warn` ≤ 4 ns, control `log_disabled_runtime` (an `info!`,
//! untouched by `sink_can_accept`) in the same sitting — the delta **is** S5's added load + branch.*
//!
//! # Why this bench can use a block four orders of magnitude larger than the others
//!
//! Every other bench in this crate times **enabled** sites, so its block is capped at 256 calls:
//! that is what a lane holds before it must be drained, and a drain inside the timed region would
//! be in the reading. The resulting per-call resolution is a 100 ns tick over 256 calls =
//! 0.391 ns — which is *the same order as the quantity this row bounds*. `log_gate_cost` and
//! `log_enabled_cost` both report the disabled path AT the instrument floor for exactly that
//! reason.
//!
//! **A disabled site publishes nothing.** No lane, no drain, no cap. So the block is 1 000 000
//! calls and the resolution is 0.0001 ns — four orders below the 4 ns bound instead of level with
//! it. The bound becomes measurable by removing a constraint that never applied to it, not by
//! measuring harder.
//!
//! That is the whole method: the earlier NOT MEASURABLE verdicts were true statements about an
//! instrument configured for a different subject.

use std::hint::black_box;
use std::time::Instant;

#[path = "instrument.rs"]
mod instrument;
use instrument::{at_floor, med_and_floor, resolution_ns};

use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Ecs, Level, Log, info, warn};

/// Calls per timed block. Bounded by patience, not by the lane: nothing is published.
const CALLS: u32 = 1_000_000;

/// Rounds in the sitting, interleaved A-B-A' so drift lands on both legs.
const ROUNDS: usize = 21;

/// The row's bound, unchanged: this is an ACCEPTANCE bound that has never been measured, so it
/// stays as specified until a reading exists to re-cut it from.
const BOUND_NS: f64 = 4.0;

#[inline(never)]
fn time<F: FnMut()>(mut f: F) -> f64 {
    // The barrier clobbers memory so the `Relaxed` ceiling load cannot be hoisted out of the loop.
    // Without it LLVM's LICM empties the body and the bench measures an empty loop against an empty
    // loop -- measured on this tree at 0.0008 ns, printing a verdict about nothing.
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
    // Both targets explicitly `Off`. `.bss` is already zero, but setting it means the leg does not
    // depend on nothing else in the process having armed them.
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Off, 0, false));
    set_target_control(<Ecs as LogTarget>::ID, TargetControl::new(Level::Off, 0, false));

    let mut w = Vec::with_capacity(ROUNDS);
    let mut i = Vec::with_capacity(ROUNDS);
    let mut w2 = Vec::with_capacity(ROUNDS);
    let mut empty = Vec::with_capacity(ROUNDS);

    for _ in 0..ROUNDS {
        w.push(time(|| warn!(Log, boyko_log::codes::W0103, "disabled warn {}", 1u32)));
        i.push(time(|| info!(Log, "disabled info {}", 1u32)));
        w2.push(time(|| warn!(Log, boyko_log::codes::W0103, "disabled warn {}", 1u32)));
        empty.push(time(|| {
            black_box(0u32);
        }));
    }

    let resolution = resolution_ns(CALLS);
    let (med_w, se_w) = med_and_floor(&mut w, resolution);
    let (med_i, se_i) = med_and_floor(&mut i, resolution);
    let (med_w2, _) = med_and_floor(&mut w2, resolution);
    let (med_e, _) = med_and_floor(&mut empty, resolution);

    let twin_gap = (med_w - med_w2).abs();
    let delta = med_w - med_i;

    println!("instrument: resolution {resolution:.5} ns/call over {CALLS}-call blocks");
    println!("log_disabled_warn        : {med_w:7.4} ns  (se {se_w:.4})  bound {BOUND_NS}");
    println!("control  log_disabled_runtime (info!) : {med_i:7.4} ns  (se {se_i:.4})");
    println!("control  empty loop      : {med_e:7.4} ns");
    println!("  A-vs-A' twin gap = {twin_gap:.4} ns   delta(warn - info) = {delta:.4} ns");

    // THE EMPTY-LOOP CONTROL DECIDES WHETHER THERE IS A SUBJECT. A disabled site that costs what an
    // empty loop costs has been folded away by the compiler, and every bound below it is vacuous --
    // the state `log_disabled_compile` was deleted for (B7).
    if med_w <= med_e + resolution * 2.0 {
        println!(
            "  verdict: NO SUBJECT -- the disabled warn costs an empty loop ({med_w:.4} vs \
             {med_e:.4}); the gate folded and there is nothing to bound"
        );
        return;
    }
    if at_floor(med_w, resolution) {
        println!("  verdict: NOT MEASURABLE (instrument): the leg is at the clock's floor");
        return;
    }
    if twin_gap > med_w * 0.05 {
        println!("  verdict: NOT MEASURABLE (instrument): the A-vs-A' twin drifted over 5% of the leg");
        return;
    }
    // THE CONTROL FOLDED, AND THAT CHANGES WHAT THE DELTA MEANS.
    //
    // MEASURED: the `info!` control reads 0.2475 ns against an empty loop's 0.2474 -- identical to
    // four decimals across three sittings. A runtime-disabled `info!` optimises to NOTHING on this
    // tree.
    //
    // The row says the delta IS S5's added load. That holds only while the control has a subject.
    // Write it out: warn = empty + s5 + gate_w, info = empty + gate_i. The delta is
    // `s5 + gate_w - gate_i`, and it equals S5's cost alone only if `gate_w == gate_i`. With the
    // control folded to the empty loop, `gate_i` is zero and the delta carries `gate_w` too --
    // so calling it "S5's added load" attributes to S5 whatever the warn gate itself costs.
    //
    // The number is still worth having: it is warn's whole residual cost above a folded control,
    // which is an upper bound on S5. It is reported as that, and not as the row's label.
    // Tested against the CONTROL'S OWN SPREAD, not against a count of quanta: "indistinguishable
    // from an empty loop" is a claim about this leg's noise, and a fixed number of clock ticks is
    // an answer to a different question.
    if med_i - med_e < se_i + resolution {
        println!(
            "  NOTE: the `info!` control folded to an empty loop ({med_i:.4} vs {med_e:.4}); the delta is warn's WHOLE residual -- an UPPER BOUND on S5"
        );
    }

    // S5's cost is the DELTA, and it is only a measurement if it clears the combined floor. Below
    // that the two legs are one reading wearing two names.
    let combined = se_w + se_i;
    if delta.abs() < combined {
        println!(
            "  verdict: S5's added load is NOT RESOLVED -- delta {delta:.4} ns is inside the \
             combined floor {combined:.4} ns"
        );
    } else if med_w <= BOUND_NS {
        println!(
            "  verdict: PASS ({med_w:.4} ns <= {BOUND_NS} ns; S5 <= {delta:.4} ns)"
        );
    } else {
        println!("  verdict: OVER BOUND ({med_w:.4} > {BOUND_NS}; S5 <= {delta:.4} ns)");
    }
}
