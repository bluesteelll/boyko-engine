//! The profiler's diagnostic channel: the `DiagFlag` -> code table, the seven emitters, and the
//! census that makes an emission observable by a test.
//!
//! # This module is the substrate's mouth
//!
//! `boyko_diag` emits nothing — no code, no print, no panic hook — so every condition it observes
//! is a sticky bit ([`boyko_diag::loss::raise`]) plus a counter, and somebody above it has to read
//! them. **This is that somebody**, and it is the only one: the corpus states that
//! `boyko_ecs::…::profiling::fold` is the sole emitter of the `92xx` block, which is what keeps a
//! profiler drop reported as *a counter read* rather than as a log record that can itself be
//! dropped under exactly the load that produced the drop.
//!
//! # Q4, answered here because this rung is the first emitter
//!
//! `boyko_diag::loss` leaves the flag-to-code pairing open — *"owed by whichever plan lands its
//! emitter first"*. Nothing called [`take_raised`](boyko_diag::loss::take_raised) before this rung
//! (measured: zero callers outside the substrate's own docs), so the table is this file's:
//!
//! | `DiagFlag` | Code | Why |
//! |---|---|---|
//! | `ClockEpochBreak` | `W9216` | Ticks either side are incomparable; the window is discarded |
//! | `LaneExhausted` | `W9203` | A producer runs unlaned, so its samples are refused and land on the substrate's un-laned row — the same condition `W9203`'s second half names |
//! | `ClockUncalibrated` | **none, deliberately** | see below |
//!
//! **`ClockUncalibrated` gets no code, and that is a decision rather than an omission.** The
//! `92xx` block is exactly eighteen rows, dense and consecutive, and check 1 of the code registry
//! makes a nineteenth un-addable without moving the block — so "invent a code for it" is not free.
//! More to the point, it does not *want* a code: the condition is "a tick was read before the
//! scale was probed", whose consequence is that the window's magnitudes are unscaled. That is a
//! **status on the data**, and the corpus already has a vocabulary for exactly that
//! ([`LossStatus::Unproven`](boyko_diag::loss::LossStatus)). So it is reported as
//! [`FRAME_FLAG_CLOCK_UNCALIBRATED`](crate::ecs::core::profiling::FRAME_FLAG_CLOCK_UNCALIBRATED)
//! on every frame record of the affected window, where a reader who is about to compare two
//! numbers can see it. **Not every raised flag deserves a code; a flag whose consequence is a
//! status on the data is reported as that status.**
//!
//! # Why there is a census here at all
//!
//! Every `Live` registry row owes *"one test that observes the code being emitted"*. Observing it
//! through `boyko_log` would make each such test depend on the logger being enabled, a sink being
//! configured and a drain having run — three preconditions that have nothing to do with the claim,
//! and whose absence would make the test pass by observing nothing.
//!
//! [`report_count`] therefore counts **this module taking the emit path**. It is deliberately not
//! a claim that a record reached a sink; that is the logger's own accounting and has its own
//! counters. Two numbers that measure different things do not disagree.
//!
//! The counters are **release-live**, like every one of the eighteen drop classes: a reporting
//! obligation that vanishes in release is the vacuous-gate pattern by another route.

use core::sync::atomic::{AtomicU32, Ordering};

use boyko_diag::loss::DiagFlag;
use boyko_log::codes::{E9204, E9213, OnceSite, W9203, W9207, W9209, W9211, W9216};
use boyko_log::target::Profiling;
use boyko_log::{error, warn};

/// The codes this rung emits, in registry order. The census is indexed by position here.
///
/// A slice rather than a range: the seven are not consecutive, and a reader who has to compute
/// which of `9201..=9218` are live from a pair of bounds has been handed arithmetic instead of a
/// list.
pub const LIVE_CODES: [u16; 7] = [
    9203, // region overflow / unclaimed drops
    9204, // profiler already bound to another world
    9207, // invariant TSC absent
    9209, // late samples dropped
    9211, // fold working set exceeds L1d
    9213, // re-arm with a different geometry
    9216, // clock epoch break
];

/// One counter per [`LIVE_CODES`] entry. `.bss`, so a process that emits nothing writes no page.
static REPORTS: [AtomicU32; LIVE_CODES.len()] = [const { AtomicU32::new(0) }; LIVE_CODES.len()];

/// The once-latches. `RatePolicy::Once` is declared in the registry but is **not** enforced on the
/// emission path at this rung — `boyko_log::rate::admit` exists and nothing calls it from
/// `emit_impl` yet — so a site that owes "once" honours it with its own latch, exactly as the file
/// sink does for `W0103`. When the rate table is wired in, these become redundant rather than
/// wrong, and deleting them is a one-line change per site.
static LATCHES: [OnceSite; LIVE_CODES.len()] = [const { OnceSite::new() }; LIVE_CODES.len()];

/// The census slot for `number`, or `None` for a code this rung does not emit.
#[inline]
#[must_use]
fn slot_of(number: u16) -> Option<usize> {
    let mut i = 0;
    while i < LIVE_CODES.len() {
        if LIVE_CODES[i] == number {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// How many times this module has taken the emit path for `number`.
///
/// **Not** "how many records reached a sink". See the module docs.
#[must_use]
pub fn report_count(number: u16) -> u32 {
    slot_of(number).map_or(0, |i| REPORTS[i].load(Ordering::Relaxed))
}

/// Claim the once-latch for `number` and count the attempt.
///
/// Returns `false` when the code has already been reported, in which case the caller emits
/// nothing. The count is bumped **only on the claiming call**, so `report_count` reads 1 for a
/// `Once` code no matter how often the condition recurs — which is the honest reading of a latched
/// site, and a counter that kept climbing would describe a channel that does not exist.
#[inline]
fn claim(number: u16) -> bool {
    let Some(i) = slot_of(number) else {
        debug_assert!(false, "invariant: every emitter's code is in LIVE_CODES");
        return false;
    };
    if !LATCHES[i].claim() {
        return false;
    }
    REPORTS[i].fetch_add(1, Ordering::Relaxed);
    true
}

/// Which code, if any, a clock provenance implies.
///
/// **Pure, and split out on purpose.** The probe it consumes
/// ([`invariant_tsc`](boyko_diag::clock::invariant_tsc)) is `true` on every x86-64 box this project
/// targets, so the *emission* of `W9207` has no reachable state on this machine — a gate asserting
/// it would be green forever and prove nothing. What **is** falsifiable here is the mapping, and
/// this function is the whole of it: delete the arm and the test reds.
///
/// Stated so it is not read as tested: `W9207`'s emission is UNPROVEN on this box; its selection
/// is MEASURED.
#[inline]
#[must_use]
pub const fn clock_code(invariant_tsc: bool) -> Option<u16> {
    if invariant_tsc { None } else { Some(W9207.number()) }
}

/// Which code, if any, a raised [`DiagFlag`] implies — the whole of Q4's table, as one function so
/// the mapping has exactly one statement.
///
/// `None` is a positive answer for `ClockUncalibrated`, not a gap; see the module docs.
#[inline]
#[must_use]
pub const fn flag_code(flag: DiagFlag) -> Option<u16> {
    match flag {
        DiagFlag::ClockEpochBreak => Some(W9216.number()),
        DiagFlag::LaneExhausted => Some(W9203.number()),
        DiagFlag::ClockUncalibrated => None,
    }
}

/// Report every condition the substrate raised since the previous fold.
///
/// Takes the word rather than calling `take_raised` itself, because the take is destructive and
/// the fold is its single consumer — a helper that took it too would make two clearers of one
/// word, which is the lost-update shape the substrate is built to rule out.
///
/// The `match` below enumerates the table a second time and there is no way around it: the
/// emission macro needs a `literal` format string and a `const` code **per site**, so one function
/// cannot serve two codes. Each arm therefore carries a `debug_assert_eq!` against
/// [`flag_code`], which is the single statement — a divergence fires at the first occurrence in
/// debug rather than being discovered by reading.
pub(crate) fn report_raised(bits: u32) {
    if bits == 0 {
        return;
    }
    for flag in [
        DiagFlag::ClockEpochBreak,
        DiagFlag::ClockUncalibrated,
        DiagFlag::LaneExhausted,
    ] {
        if bits & flag.as_bits() == 0 {
            continue;
        }
        // `None` is `ClockUncalibrated`'s positive answer: the fold reports it as a frame flag.
        if flag_code(flag).is_none() {
            continue;
        }
        match flag {
            DiagFlag::ClockEpochBreak => report_epoch_break(),
            DiagFlag::LaneExhausted => report_lane_exhausted(),
            DiagFlag::ClockUncalibrated => {
                debug_assert!(false, "invariant: flag_code(ClockUncalibrated) is None");
            }
        }
    }
}

/// `W9216` — the clock's epoch broke and the in-flight window was discarded.
#[cold]
#[inline(never)]
pub(crate) fn report_epoch_break() {
    debug_assert_eq!(flag_code(DiagFlag::ClockEpochBreak), Some(W9216.number()));
    if claim(W9216.number()) {
        warn!(
            Profiling,
            W9216.number(),
            "clock epoch break: the in-flight profiling window was discarded and the clock recalibrated"
        );
    }
}

/// `W9203`, the un-laned half — a producer holds no lane, so its samples cannot be attributed.
#[cold]
#[inline(never)]
pub(crate) fn report_lane_exhausted() {
    debug_assert_eq!(flag_code(DiagFlag::LaneExhausted), Some(W9203.number()));
    if claim(W9203.number()) {
        warn!(
            Profiling,
            W9203.number(),
            "a profiling producer holds no lane; its samples are refused and counted on the un-laned row"
        );
    }
}

/// `W9203`, the overflow half — a lane region filled and refused samples.
///
/// The same code as [`report_lane_exhausted`] because the registry row names both halves, and one
/// latch serves both: a reader who has been told "samples were discarded" once does not need to be
/// told which of the two mechanisms did it a second time. The magnitudes are in the drop counters,
/// which are what a reader compares.
#[cold]
#[inline(never)]
pub(crate) fn report_overflow(engine: u64, user: u64) {
    if claim(W9203.number()) {
        warn!(
            Profiling,
            W9203.number(),
            "a profiling lane region overflowed: {} engine and {} user samples discarded so far",
            engine,
            user
        );
    }
}

/// `W9209` — samples arrived after their frame had left the retained window.
#[cold]
#[inline(never)]
pub(crate) fn report_late(late: u64) {
    if claim(W9209.number()) {
        warn!(
            Profiling,
            W9209.number(),
            "{} profiling samples arrived after their frame had left the retained window",
            late
        );
    }
}

/// `W9211` — the fold's per-frame column row does not fit L1d at this zone stride.
///
/// Not a refusal: `arm` succeeds and the session runs. A game may legitimately want more zones
/// than fit a cache level and pay for them — it just gets told, with the measured figure rather
/// than with the threshold.
#[cold]
#[inline(never)]
pub(crate) fn report_working_set(bytes: u64, stride: u32) {
    if claim(W9211.number()) {
        warn!(
            Profiling,
            W9211.number(),
            "the profiling fold's column row is {} B at a zone stride of {}, over the L1d budget",
            bytes,
            stride
        );
    }
}

/// `W9207` — the CPU advertises no invariant TSC.
#[cold]
#[inline(never)]
pub(crate) fn report_no_invariant_tsc() {
    if claim(W9207.number()) {
        warn!(
            Profiling,
            W9207.number(),
            "no invariant TSC: profiling tick magnitudes are not trustworthy across cores or power states"
        );
    }
}

/// `E9213` — `arm` was called with a geometry the live session cannot change to.
#[cold]
#[inline(never)]
pub(crate) fn report_geometry_mismatch(live: u32, asked: u32) {
    if claim(E9213.number()) {
        error!(
            Profiling,
            E9213.number(),
            "profiler re-armed with zone stride {} while the live session is {}; the arm was refused",
            asked,
            live
        );
    }
}

/// `E9204` — a second world tried to bind the process-global profiler.
#[cold]
#[inline(never)]
pub(crate) fn report_second_world(live: u64, asked: u64) {
    if claim(E9204.number()) {
        error!(
            Profiling,
            E9204.number(),
            "the profiler is already bound to world {}; world {} was refused",
            live,
            asked
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole of Q4's table, asserted as a table. Deleting an arm reds here before it reds
    /// anywhere a condition has to be provoked.
    #[test]
    fn the_flag_to_code_table_is_the_one_stated_in_the_module_docs() {
        assert_eq!(flag_code(DiagFlag::ClockEpochBreak), Some(9216));
        assert_eq!(flag_code(DiagFlag::LaneExhausted), Some(9203));
        // A POSITIVE answer, not a gap: the condition is reported as a frame flag, because its
        // consequence is a status on the data rather than an event.
        assert_eq!(flag_code(DiagFlag::ClockUncalibrated), None);
    }

    /// `W9207`'s selection is measured here because its emission cannot be: `invariant_tsc()` is
    /// `true` on every box this project targets, so the false branch has no reachable state and a
    /// gate over the emission would be green forever.
    #[test]
    fn the_clock_code_is_selected_only_when_the_tsc_is_not_invariant() {
        assert_eq!(clock_code(false), Some(9207));
        assert_eq!(clock_code(true), None);
    }

    /// Every emitter's code must have a census slot, or `claim` would fire its `debug_assert` and
    /// silently emit nothing in release.
    #[test]
    fn every_live_code_has_a_census_slot_and_they_are_distinct() {
        for (i, n) in LIVE_CODES.iter().copied().enumerate() {
            assert_eq!(slot_of(n), Some(i));
        }
        assert_eq!(slot_of(9201), None, "a code this rung does not emit must have no slot");
        // A duplicate would make two codes share a latch, so one would silence the other.
        for (i, a) in LIVE_CODES.iter().copied().enumerate() {
            for b in LIVE_CODES.iter().copied().skip(i + 1) {
                assert_ne!(a, b);
            }
        }
    }
}
