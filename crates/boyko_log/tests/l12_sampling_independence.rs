//! `G10(c)` — eight threads, eight targets, and every `(lane, target)` pair sampling independently.
//!
//! # The leg the L12 gate was missing
//!
//! `G10` has five legs. (a) exactness, (b) the shift-0 control and (e) the argument-evaluation
//! split all live in `l12_sampling.rs`. **(c) did not exist**, and it is the one that fails if the
//! counters are shared — the corpus states its own RED: *"share one counter across lanes ⇒ (c)
//! fails"*.
//!
//! # Why the claim needs real threads and not a loop over indices
//!
//! `SAMPLE_CTR` is `LANE_COUNT × MAX_TARGETS` cells, and the design's whole argument for that shape
//! is that the increment is **single-writer**: the lane index is unique per live thread, so no two
//! threads ever touch one cell. A single-threaded loop over `(lane, target)` pairs would exercise
//! the indexing and prove nothing about the property the indexing exists for.
//!
//! # What "independent" is asserted to mean
//!
//! Each of the 64 pairs must deliver **exactly** `N >> k` of `N`. Not "about", and not "in total":
//! a shared counter still delivers the right TOTAL across pairs, which is why the assertion is per
//! pair. That is the shape a wrong implementation passes if you only sum.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use boyko_log::sample::admits;

/// Threads, and lanes: one lane per thread, which is the substrate's own invariant.
const THREADS: u16 = 8;
/// Targets driven per thread.
const TARGETS: u16 = 8;
/// Occurrences per `(lane, target)` pair. A multiple of `1 << SHIFT`, so `N >> k` is exact and the
/// assertion is an equality rather than a range.
const N: u32 = 4096;
/// One in eight.
const SHIFT: u8 = 3;

#[test]
fn eight_lanes_by_eight_targets_each_sample_independently() {
    // One cell per pair, so a shared counter shows up as the WRONG PER-PAIR number rather than as a
    // right total.
    let delivered: Arc<Vec<AtomicU32>> =
        Arc::new((0..THREADS as usize * TARGETS as usize).map(|_| AtomicU32::new(0)).collect());

    let mut handles = Vec::with_capacity(THREADS as usize);
    for lane in 0..THREADS {
        let d = Arc::clone(&delivered);
        handles.push(std::thread::spawn(move || {
            for target in 0..TARGETS {
                let mut hits = 0u32;
                for _ in 0..N {
                    if admits(lane, target, SHIFT) {
                        hits += 1;
                    }
                }
                d[lane as usize * TARGETS as usize + target as usize].store(hits, Ordering::Relaxed);
            }
        }));
    }
    for h in handles {
        h.join().expect("a sampling thread must not panic");
    }

    let want = N >> SHIFT;
    let mut wrong = Vec::new();
    for lane in 0..THREADS {
        for target in 0..TARGETS {
            let got = delivered[lane as usize * TARGETS as usize + target as usize]
                .load(Ordering::Relaxed);
            if got != want {
                wrong.push(format!("(lane {lane}, target {target}) delivered {got}, wanted {want}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "sampling is not independent per (lane, target): {} of {} pairs are wrong. A counter \
         shared across lanes still delivers the right TOTAL, which is why this asserts per pair.\n  {}",
        wrong.len(),
        THREADS as usize * TARGETS as usize,
        wrong.join("\n  ")
    );

    // ── THE CONTROL: shift 0 delivers everything, on the same pairs ─────────────────────────
    //
    // Without it, an `admits` that always refused would make every pair read 0 -- equal to each
    // other, and equally wrong. This is leg (b)'s claim re-taken on the concurrent shape, because
    // (b) as it stands is single-threaded.
    for lane in 0..THREADS {
        for target in 0..TARGETS {
            let mut hits = 0u32;
            for _ in 0..64 {
                if admits(lane, target, 0) {
                    hits += 1;
                }
            }
            assert_eq!(hits, 64, "shift 0 must deliver every record for (lane {lane}, target {target})");
        }
    }
}
