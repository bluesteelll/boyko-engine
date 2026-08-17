//! The rate limiter — the two policies that need shared state, and the two that do not.
//!
//! # Half of `RatePolicy` never reaches this file, and that is the design
//!
//! | Policy | State | Scope | Steady-state cost |
//! |---|---|---|---|
//! | [`Every`](RatePolicy::Every) | none | — | nothing; the caller does not even ask |
//! | [`Once`](RatePolicy::Once) | a site-local [`OnceSite`](crate::codes::OnceSite) | **per site** | one `Relaxed` load of a private line, **no store** |
//! | [`OnceCounted`](RatePolicy::OnceCounted) | the same latch + one counter | **per site** | the load, plus one RMW per *suppressed* occurrence |
//! | [`EveryN(n)`](RatePolicy::EveryN) | [`RATE`] | **per code** | one RMW |
//! | [`MinIntervalMs(ms)`](RatePolicy::MinIntervalMs) | [`RATE`] | **per code** | one load, and one RMW only when the window opens |
//!
//! `Once` is per **site**, not per code, and the reason is a measured defect rather than a
//! preference: `RATE` is indexed by `code_idx`, so a code-scoped `Once` fires once per *code*.
//! Three independent capability degradations in `boyko_rhi_vulkan` share one `Once` code, and a
//! device lacking all three would have reported one and silently lost two — uncounted, because
//! `Once` deliberately does not count. The latch therefore lives at the site, where the
//! diagnostic value is, and this file never sees those two policies at all.
//!
//! # Why the shared slots are still worth having
//!
//! `EveryN` and `MinIntervalMs` are *aggregate* policies: "one in n of this condition, wherever it
//! happens" is a statement about the code, not about a call site. Giving them per-site state would
//! change what they mean.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::codes::{CODE_IDX_EXHAUSTED, RatePolicy};

/// Rate slots. One per dense `code_idx`, one cache line each.
///
/// 512 slots × 64 B = 32 KiB of `.bss`, demand-zero and untouched until a code with a
/// shared-state policy actually fires. The engine's own registry declares `Every` or `Once` on
/// every row today, so in the shipped default this array is never written at all.
pub const MAX_RATE_SLOTS: usize = 512;

/// One code's shared rate state.
///
/// A full cache line per code, because the alternative is two unrelated codes sharing one during
/// the storm the policy exists to damp — the same false-sharing argument the per-lane rings make,
/// at a different granularity.
///
/// **There is no `fired` field.** v3 had one; it went dead the moment `Once` stopped using `RATE`,
/// and a field nothing reads is a field a reader has to disprove.
#[repr(C, align(64))]
struct RateSlot {
    /// Occurrences seen, for `EveryN`. Wraps; only the low bits are ever tested.
    count: AtomicU32,
    /// Millisecond stamp of the last admission, for `MinIntervalMs`.
    last_ms: AtomicU64,
    _pad: [u8; 48],
}

impl RateSlot {
    const fn new() -> RateSlot {
        RateSlot { count: AtomicU32::new(0), last_ms: AtomicU64::new(0), _pad: [0; 48] }
    }
}

const _: () = assert!(core::mem::size_of::<RateSlot>() == 64);

static RATE: [RateSlot; MAX_RATE_SLOTS] = [const { RateSlot::new() }; MAX_RATE_SLOTS];

/// Suppressed occurrences, summed over every code. Reported by the census.
///
/// One counter rather than one per code, because the per-code breakdown a reader wants is the
/// per-**site** one, and that is the `ONCE_SITES` walk's answer rather than this array's.
static SUPPRESSED: AtomicU64 = AtomicU64::new(0);

/// Occurrences that ran with no rate state because the code index space was exhausted.
///
/// **Never an aliased slot.** [`code_idx_of`](crate::codes::code_idx_of) returns
/// [`CODE_IDX_EXHAUSTED`] rather than wrapping, so an unregistered code degrades to
/// [`Every`](RatePolicy::Every) — it is delivered, and the degradation is counted here — instead
/// of quietly inheriting some other code's throttle.
static UNINDEXED: AtomicU64 = AtomicU64::new(0);

/// Should this occurrence be delivered?
///
/// Handles **only** the two shared-state policies. `Every` short-circuits before touching
/// anything; `Once` and `OnceCounted` are answered by the caller's site-local latch and reaching
/// this function with either is a caller error, so they are treated as `Every` and
/// `debug_assert`ed rather than silently given code-scoped semantics.
///
/// `now_ms` is passed in rather than read here: the clock belongs to `boyko_diag`, and a policy
/// decision that reads a clock is a policy decision that cannot be tested without one.
#[inline]
pub fn admit(code_idx: u32, policy: RatePolicy, now_ms: u64) -> bool {
    match policy {
        RatePolicy::Every => true,
        RatePolicy::Once | RatePolicy::OnceCounted => {
            debug_assert!(
                false,
                "invariant: Once/OnceCounted are answered by the site-local latch, not by RATE"
            );
            true
        }
        RatePolicy::EveryN(n) => {
            let Some(slot) = slot_of(code_idx) else { return degraded() };
            debug_assert!(n.is_power_of_two(), "invariant: EveryN(n) has a power-of-two n");
            // The RMW returns the PREVIOUS value, so the first occurrence at count 0 is admitted.
            // A wrap at 2^32 keeps the phase, because `n` divides 2^32 for every power of two.
            let seen = slot.count.fetch_add(1, Ordering::Relaxed);
            if seen & (n - 1) == 0 {
                true
            } else {
                SUPPRESSED.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
        RatePolicy::MinIntervalMs(ms) => {
            let Some(slot) = slot_of(code_idx) else { return degraded() };
            let last = slot.last_ms.load(Ordering::Relaxed);
            // `last == 0` is the never-fired state and admits: a real stamp of 0 would mean the
            // clock's own epoch, which `boyko_diag::clock` does not hand out after calibration.
            if last != 0 && now_ms.saturating_sub(last) < u64::from(ms) {
                SUPPRESSED.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            // A lost race admits twice inside one window. That is the correct trade: the
            // alternative is a CAS loop on the storm path to save one duplicate line, and the
            // policy's promise is "at most one per window per code" as a damping statement, not as
            // an exactness one. Written down because the weaker promise is easy to read as a bug.
            slot.last_ms.store(now_ms, Ordering::Relaxed);
            true
        }
    }
}

/// The slot for `code_idx`, or `None` when the code has no registry row.
#[inline]
fn slot_of(code_idx: u32) -> Option<&'static RateSlot> {
    if code_idx == CODE_IDX_EXHAUSTED {
        return None;
    }
    RATE.get(code_idx as usize)
}

/// An occurrence with no rate state: delivered, and the degradation counted.
#[cold]
#[inline(never)]
fn degraded() -> bool {
    UNINDEXED.fetch_add(1, Ordering::Relaxed);
    true
}

/// Occurrences the limiter suppressed, summed over every code. Cumulative for the process.
#[must_use]
pub fn suppressed() -> u64 {
    SUPPRESSED.load(Ordering::Relaxed)
}

/// Occurrences that ran with no rate state because the code index space was exhausted.
#[must_use]
pub fn unindexed() -> u64 {
    UNINDEXED.load(Ordering::Relaxed)
}

// ── L11a: the index space's two reports ──────────────────────────────────────────────────────
//
// THEY LIVE HERE AND NOT IN `codes.rs`, AND THE GATE IS WHY. `code_registry.rs` excludes `codes.rs`
// from its CODE stream on purpose -- that file DEFINES every identifier, so including it would make
// every row "observed by an emitter" by construction. A reporter written there is therefore a
// reporter check 3a cannot see, and the check said so the moment these two rows went `Live`.
//
// The layering it forces is the better one anyway: this module owns the slots, so this module
// reports on their exhaustion. `codes.rs` hands out indices INTO the array declared here.

/// `boyko-W0114`, once: the downstream index space is nine tenths spent.
///
/// # Why emitting from inside the mint cannot recurse
///
/// This is a `warn!` raised on the path that hands out indices, which looks circular and is not:
/// `W0114` is an **engine** code, so its own `CodeIdx` is `Static` and resolving it never enters
/// `codes::mint`. The recursion would exist only if the engine's indices were minted too — which is the
/// second thing `CodeIdx`'s two variants buy, after the compile-time-constant invariant.
#[cold]
#[inline(never)]
pub(crate) fn report_space_nearly_full() {
    static SITE: crate::codes::OnceSite = crate::codes::OnceSite::new();
    if SITE.claim() {
        crate::warn!(
            crate::Log,
            crate::codes::W0114,
            "downstream diagnostic-code index space is {}/{} spent",
            crate::codes::code_occupancy(),
            crate::codes::MAX_CODES - crate::codes::DOWNSTREAM_IDX_BASE
        );
    }
}

/// `boyko-E0115`, once: the space is gone and this code has no rate slot.
///
/// The record still arrives — only its *rate state* is missing, and the site degrades to
/// [`RatePolicy::Every`] semantics. Reporting once and continuing is the whole design: the
/// alternative a table like this invites is aliasing a slot, which throttles an unrelated code's
/// storm and reports nothing at all.
#[cold]
#[inline(never)]
pub(crate) fn report_space_exhausted() {
    static SITE: crate::codes::OnceSite = crate::codes::OnceSite::new();
    if SITE.claim() {
        crate::error!(
            crate::Log,
            crate::codes::E0115,
            "downstream diagnostic-code index space exhausted at {} slots; later codes emit with              no rate state and are counted",
            crate::codes::MAX_CODES
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `EveryN` admits the first and then one in `n`, and counts what it refused.
    ///
    /// Uses a slot no registry row owns, so the counts are this test's alone — `RATE` is
    /// process-global and a slot shared with another test is the flake class this crate has paid
    /// for four times.
    #[test]
    fn every_n_admits_one_in_n_and_counts_the_rest() {
        const IDX: u32 = 500;
        let before = suppressed();
        let mut admitted = 0;
        for _ in 0..64 {
            if admit(IDX, RatePolicy::EveryN(8), 0) {
                admitted += 1;
            }
        }
        assert_eq!(admitted, 8, "64 occurrences at EveryN(8) must deliver 8");
        assert_eq!(suppressed() - before, 56, "the other 56 must be counted, not forgotten");
    }

    /// `MinIntervalMs` opens once per window, and the first occurrence always passes.
    #[test]
    fn min_interval_opens_once_per_window() {
        const IDX: u32 = 501;
        assert!(admit(IDX, RatePolicy::MinIntervalMs(100), 1_000), "the first must pass");
        assert!(!admit(IDX, RatePolicy::MinIntervalMs(100), 1_050), "inside the window");
        assert!(!admit(IDX, RatePolicy::MinIntervalMs(100), 1_099), "still inside");
        assert!(admit(IDX, RatePolicy::MinIntervalMs(100), 1_100), "the window has passed");
    }

    /// An unregistered code is DELIVERED, not silenced, and never borrows another code's slot.
    #[test]
    fn an_unindexed_code_degrades_to_every_and_is_counted() {
        let before = unindexed();
        assert!(admit(CODE_IDX_EXHAUSTED, RatePolicy::EveryN(2), 0));
        assert!(admit(CODE_IDX_EXHAUSTED, RatePolicy::EveryN(2), 0));
        assert!(admit(CODE_IDX_EXHAUSTED, RatePolicy::MinIntervalMs(1_000), 0));
        assert_eq!(unindexed() - before, 3, "every degraded occurrence must be counted");
    }

    /// An index past the array is the same case as an exhausted one.
    ///
    /// Separate from the test above because the two reach `degraded()` by different routes — a
    /// sentinel and a bounds check — and a `get` that silently wrapped would pass that one.
    #[test]
    fn an_out_of_range_index_does_not_wrap_into_a_slot() {
        let before = unindexed();
        assert!(admit(MAX_RATE_SLOTS as u32, RatePolicy::EveryN(2), 0));
        assert_eq!(unindexed() - before, 1);
    }
}
