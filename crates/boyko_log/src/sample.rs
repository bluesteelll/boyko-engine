//! Per-target sampling: deliver 1 record in `2^k`, and count the rest *(L12)*.
//!
//! # What sampling suppresses, and what it CANNOT
//!
//! **Sampling suppresses DELIVERY. It never suppresses argument evaluation.** The macro's gate
//! chain is a *level* test, so by the time a record reaches this module its arguments have already
//! been evaluated — they were evaluated to build the tuple handed to `emit_impl`. A caller with a
//! side-effecting argument gets that side effect on every call regardless of the shift.
//!
//! That is not a defect to be fixed by moving the decision earlier: the decision needs the target's
//! control byte and a per-lane counter, and hoisting either into the macro would put a load and an
//! RMW at every site in the engine, including the ones the compile ceiling deletes. The honest
//! answer is to state the property, which is why `G10(e)` asserts **both numbers together** — 1000
//! argument evaluations AND 500 deliveries over 1000 emits at shift 1. Asserting only the second
//! would let a design that moved evaluation behind the sample decision pass, and that design breaks
//! the documented "arguments are evaluated exactly once per call" contract.
//!
//! # Why the counter is per (LANE, TARGET) and not per target
//!
//! A single counter per target is a contended atomic RMW on the enabled hot path, shared by every
//! thread emitting to that category — the false-sharing argument the lane rings already make, one
//! level up. Per-lane rows make the increment single-writer: the lane index is unique per live
//! thread, so the counter is written by one thread and read by nobody until the census.
//!
//! The cost is `LANE_COUNT × MAX_TARGETS × 2 B` of `.bss`, demand-zero and untouched by any target
//! whose shift is zero — which is every target in the shipped default.

use core::sync::atomic::{AtomicU16, Ordering};

use boyko_diag::lane::LANE_COUNT;

use crate::target::MAX_TARGETS;

/// One counter per `(lane, target)`. 80 × 256 × 2 B = 40 KiB, demand-zero.
static SAMPLE_CTR: [AtomicU16; LANE_COUNT as usize * MAX_TARGETS] =
    [const { AtomicU16::new(0) }; LANE_COUNT as usize * MAX_TARGETS];

/// Should this record be delivered under `shift`?
///
/// `shift == 0` is the whole-traffic case and returns `true` **without touching the counter**, so a
/// target that never asked for sampling pays nothing beyond the branch — and the array stays
/// untouched `.bss` in the shipped default.
///
/// Otherwise the counter advances and the record is delivered when the low `shift` bits are zero:
/// exactly `n >> shift` of `n` records, with no drift, because the test is `count & (2^k − 1)` over
/// a counter that wraps at a power of two.
#[inline]
#[must_use]
pub fn admits(lane: u16, target: u16, shift: u8) -> bool {
    if shift == 0 {
        return true;
    }
    let idx = lane as usize * MAX_TARGETS + target as usize;
    let Some(cell) = SAMPLE_CTR.get(idx) else {
        // Out of range cannot happen -- `lane < LANE_COUNT` and `target < MAX_TARGETS` are both
        // upheld by their own constructors -- and if it ever did, DELIVERING is the safe answer:
        // an over-delivered record is visible, a wrongly suppressed one is not.
        return true;
    };
    let n = cell.fetch_add(1, Ordering::Relaxed);
    let mask = (1u16 << shift.min(15)) - 1;
    let deliver = n & mask == 0;
    if !deliver {
        report_sampling_active();
    }
    deliver
}

/// `boyko-W0113`, once per process: sampling is discarding records.
///
/// **The point is not that sampling happened** — an operator who set a shift knows they set one.
/// It is that from here on **`delivered` is no longer a total**, so a reader comparing counts
/// across targets is comparing a sampled number with an unsampled one unless they know. The census
/// says the same thing structurally with `UNPROVEN(sampled)`; this is the line that reaches a log
/// nobody is reading the census of.
///
/// `Once` per PROCESS rather than per target: one line stating that counts have stopped being
/// totals is the whole job, and one per target would be a storm about a setting the operator chose.
#[cold]
#[inline(never)]
fn report_sampling_active() {
    static SITE: crate::codes::OnceSite = crate::codes::OnceSite::new();
    if SITE.claim() {
        crate::warn!(
            crate::Log,
            crate::codes::W0113,
            "sampling is discarding records; delivered counts are no longer totals -- read the              census `sampled_out` column beside every `delivered`"
        );
    }
}

/// Reset one target's counters across every lane.
///
/// **Real public surface, not `#[cfg(test)]`-gated, and that is forced rather than lax.** An
/// integration test links this library compiled WITHOUT `cfg(test)` and cannot see a gated item —
/// the wall rung L7b hit and answered with a private copy, which the `test-probe` feature was
/// added to replace. A phase reset is harmless (it changes which records a *future* window
/// delivers, never what was counted), so it is exposed rather than duplicated.
///
/// It is also the honest console verb: an operator who changes a target's shift mid-session and
/// wants the new rate to start from a known phase has no other way to ask.
pub fn reset_counters(target: u16) {
    for lane in 0..LANE_COUNT as usize {
        SAMPLE_CTR[lane * MAX_TARGETS + target as usize].store(0, Ordering::Relaxed);
    }
}
