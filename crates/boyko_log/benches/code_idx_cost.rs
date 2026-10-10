//! `downstream_code_warn`'s real subject: **the `idx_cell` load**, measured where it resolves.
//!
//! # Why this is a separate bench from the emission it belongs to
//!
//! The row is *`downstream_code_warn` ≤ 18 ns, control the engine-code `warn!` in the same
//! sitting; the delta is the `idx_cell` load.* The ABSOLUTE half is measured on the emission path
//! in `log_enabled_cost`, where it passes at ~10 ns. **The delta is not measurable there**: an
//! enabled emission is capped at 256 calls per block by the lane, which puts a 100 ns tick's
//! resolution at 0.391 ns/call — and the delta reads exactly 0.00 or 0.39 ns, i.e. zero or one
//! quantum. That is the instrument, not the load.
//!
//! `resolve_idx` is callable on its own: it takes a `CodeIdx` and returns an index, touching no
//! lane and publishing nothing. So the same trick that made `log_disabled_warn` and `log_pod_12b`
//! measurable applies — a 2 000 000-call block at 0.00005 ns resolution, four orders below the
//! quantity.
//!
//! **This is the third row in this table whose subject was separable from the constraint that hid
//! it.** The 256-call cap is a lane limit; three rows in a row have turned out not to use the lane
//! for the thing they actually bound.
//!
//! # The two legs
//!
//! * **Dynamic** — a downstream code's index lives in an `AtomicU16` minted at first use, so
//!   resolving it is a load.
//! * **Static** — an engine code's index was resolved when the table compiled, so resolving it is
//!   reading a field.
//!
//! The delta between them is the whole cost a downstream table pays for existing.

use std::hint::black_box;
use std::time::Instant;

#[path = "instrument.rs"]
mod instrument;
use instrument::{at_floor, med_and_floor, resolution_ns};

use boyko_log::codes::resolve_idx;

/// A downstream table, declared as a game declares one. Its codes are `CodeIdx::Dynamic`.
mod acme {
    use boyko_log::RatePolicy;

    boyko_log::declare_codes! {
        prefix = "acme",
        (1, W, ACME_W0001, RatePolicy::Every, "a downstream warning, for the bench"),
    }
}

/// Calls per timed block. No lane, no drain, no cap.
const CALLS: u32 = 2_000_000;

/// Rounds in the sitting, interleaved A-B-A'.
const ROUNDS: usize = 21;

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
    // Mint the downstream index once, so the loop measures RESOLUTION and not the first-use mint.
    // Without this the first iteration would carry the mint and every later one would not, and the
    // median would report the steady state while the mean reported neither.
    let dynamic = acme::ACME_W0001.idx();
    let _ = resolve_idx(dynamic);
    let static_idx = boyko_log::codes::E0107.idx();

    let mut dy = Vec::with_capacity(ROUNDS);
    let mut st = Vec::with_capacity(ROUNDS);
    let mut dy2 = Vec::with_capacity(ROUNDS);

    for _ in 0..ROUNDS {
        dy.push(time(|| {
            black_box(resolve_idx(black_box(dynamic)));
        }));
        st.push(time(|| {
            black_box(resolve_idx(black_box(static_idx)));
        }));
        dy2.push(time(|| {
            black_box(resolve_idx(black_box(dynamic)));
        }));
    }

    let resolution = resolution_ns(CALLS);
    let (med_dy, se_dy) = med_and_floor(&mut dy, resolution);
    let (med_st, se_st) = med_and_floor(&mut st, resolution);
    let (med_dy2, _) = med_and_floor(&mut dy2, resolution);

    let twin_gap = (med_dy - med_dy2).abs();
    let delta = med_dy - med_st;

    println!("instrument: resolution {resolution:.6} ns/call over {CALLS}-call blocks");
    println!("resolve_idx  Dynamic (downstream) : {med_dy:8.4} ns  (se {se_dy:.5})");
    println!("resolve_idx  Static  (engine)     : {med_st:8.4} ns  (se {se_st:.5})");
    println!("  A-vs-A' twin gap = {twin_gap:.5} ns   delta (the idx_cell load) = {delta:.4} ns");

    if at_floor(med_dy, resolution) {
        println!("  verdict: NOT MEASURABLE (instrument): the dynamic leg is at the clock's floor");
        return;
    }
    if twin_gap > med_dy * 0.05 {
        println!("  verdict: NOT MEASURABLE (instrument): the A-vs-A' twin drifted over 5% of the leg");
        return;
    }
    let combined = se_dy + se_st;
    if delta.abs() < combined {
        println!(
            "  verdict: NOT RESOLVED -- delta {delta:.4} ns is inside the combined floor {combined:.4} ns; a dynamic index costs what a static one costs, at this resolution"
        );
    } else if delta > 0.0 {
        println!("  verdict: RESOLVED -- a downstream code's idx_cell load costs {delta:.4} ns");
    } else {
        println!(
            "  verdict: RESOLVED, WRONG SIGN -- the dynamic leg is {:.4} ns CHEAPER than the static one, which no story about an extra load explains; suspect the instrument or the optimizer, not the code",
            -delta
        );
    }
}
