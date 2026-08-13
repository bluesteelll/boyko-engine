//! The profiler's diagnostic channel: the `DiagFlag` -> code table, the nine emitters, and the
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
//! | `ZoneRegistryExhausted` | `W9201` | A zone runs unregistered; its samples carry no resolvable id |
//! | `ZoneRegistryNearFull` | `W9208` | Nothing is lost yet, and this is what stops exhaustion being the first news of it |
//! | `ClockUncalibrated` | **none, deliberately** | see below |
//!
//! The `match` in [`flag_code`] is deliberately not `_`-terminated. Adding a variant to
//! `DiagFlag` therefore **fails to compile** here, in the emitter, which is what makes "every flag
//! has exactly one paired report" a property of the build rather than of somebody's diligence.
//! Measured: the two rows above were added by a compile error, not by remembering.
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
use boyko_log::codes::{
    E9204, E9213, OnceSite, W9201, W9202, W9203, W9205, W9206, W9207, W9208, W9209, W9210, W9211,
    W9212, W9214, W9215, W9216, W9217, W9218,
};
use boyko_log::target::Profiling;
use boyko_log::{error, warn};

/// The codes this rung emits, in registry order. The census is indexed by position here.
///
/// A slice rather than a range: a reader who has to compute which of `9201..=9218` are live from a
/// pair of bounds has been handed arithmetic instead of a list.
///
/// # TWO REPORTERS COULD NOT EMIT, and this list is why *(found at L8c)*
///
/// `claim` resolves a code to its slot here and returns `false` when there is none — after firing
/// a `debug_assert`. Profiling rung 10 added `report_user_budget_exhausted` and
/// `report_engine_scope_refused`, gave them `flag_code` arms, `Live` registry rows and doc pages,
/// and **did not add `W9210` or `W9212` here**. So both were complete-looking emitters that
/// panicked in debug and emitted nothing in release, for three rungs.
///
/// Nothing could see it. The registry's orphan check found their identifiers; its doc-page check
/// found their pages; `flag_code`'s table test did not cover their arms (it asserted four of nine
/// rows until L8c); and no test drove either condition. It surfaced only because L8c added four
/// codes to this array and the length pin moved.
///
/// The list is now the whole block: every `92xx` code that has an emitter has a slot.
pub const LIVE_CODES: [u16; 18] = [
    W9201.number(), // engine zone registry exhausted
    W9202.number(), // GPU timestamp pair budget exhausted            (L8c)
    W9203.number(), // region overflow / unclaimed drops
    E9204.number(), // profiler already bound to another world
    W9205.number(), // zones lost in this window                      (L8c)
    W9206.number(), // a contrast could not be resolved               (L8c)
    W9207.number(), // invariant TSC absent
    W9208.number(), // engine zone registry at 90 % occupancy
    W9209.number(), // late samples dropped
    W9210.number(), // user zone budget / dynamic name arena exhausted
    W9211.number(), // fold working set exceeds L1d
    W9212.number(), // register_zone refused an engine scope
    E9213.number(), // re-arm with a different geometry
    W9214.number(), // the telemetry path is unwritable
    W9215.number(), // a telemetry write failed and streaming was disabled
    W9216.number(), // clock epoch break
    W9217.number(), // GPU slots abandoned at teardown                (L8c)
    W9218.number(), // a telemetry quantile subscription was refused past the cap
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
        DiagFlag::ZoneRegistryExhausted => Some(W9201.number()),
        DiagFlag::ZoneRegistryNearFull => Some(W9208.number()),
        // Profiling rung 10. Two codes rather than one for a reason the corpus states as the
        // difference between an engine defect and a configuration fact: `W9210` is a game meeting
        // a budget the host chose, `W9212` is a game asking for a scope it may not have.
        DiagFlag::UserZoneBudgetExhausted => Some(W9210.number()),
        DiagFlag::EngineScopeRefused => Some(W9212.number()),
        // Profiling rung 13. Three codes rather than one, and the split is the same kind the rung
        // 10 pair above makes: `W9214` leaves NO file, `W9215` leaves a file with a stated end, and
        // `W9218` is not a fault at all — it is a budget, and the zone still streams everything
        // except its two quantiles.
        DiagFlag::TelemetryPathUnwritable => Some(W9214.number()),
        DiagFlag::TelemetryWriteFailed => Some(W9215.number()),
        DiagFlag::TelemetryZonesRefused => Some(W9218.number()),
        // L8c. The only one of that rung's four conditions that RAISES a flag rather than calling
        // its reporter directly -- see the variant's own doc for why this one has no other route.
        DiagFlag::GpuPairBudgetExhausted => Some(W9202.number()),
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
        DiagFlag::ZoneRegistryExhausted,
        DiagFlag::ZoneRegistryNearFull,
        DiagFlag::UserZoneBudgetExhausted,
        DiagFlag::EngineScopeRefused,
        DiagFlag::TelemetryPathUnwritable,
        DiagFlag::TelemetryWriteFailed,
        DiagFlag::TelemetryZonesRefused,
        DiagFlag::GpuPairBudgetExhausted,
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
            DiagFlag::ZoneRegistryExhausted => report_registry_exhausted(),
            DiagFlag::ZoneRegistryNearFull => report_registry_near_full(),
            DiagFlag::UserZoneBudgetExhausted => report_user_budget_exhausted(),
            DiagFlag::EngineScopeRefused => report_engine_scope_refused(),
            DiagFlag::TelemetryPathUnwritable => report_telemetry_path_unwritable(),
            DiagFlag::TelemetryWriteFailed => report_telemetry_write_failed(),
            DiagFlag::TelemetryZonesRefused => report_telemetry_zones_refused(),
            DiagFlag::GpuPairBudgetExhausted => report_gpu_pair_budget_exhausted(),
            DiagFlag::ClockUncalibrated => {
                debug_assert!(false, "invariant: flag_code(ClockUncalibrated) is None");
            }
        }
    }
}

/// `W9214` — the telemetry path could not be opened, so this session streams nothing.
///
/// **Raised where the path is first opened, which is inside `arm` with a telemetry config**, so a
/// run that never arms the profiler opens no file and can never reach this. The message names the
/// consequence rather than the errno: an operator who sees it wants to know that the session has no
/// stream, and the `io::Error` that produced it is the caller's to log with its own context.
#[cold]
#[inline(never)]
pub(crate) fn report_telemetry_path_unwritable() {
    debug_assert_eq!(flag_code(DiagFlag::TelemetryPathUnwritable), Some(W9214.number()));
    if claim(W9214.number()) {
        warn!(
            Profiling,
            W9214.number(),
            "the telemetry path could not be opened; this session streams nothing"
        );
    }
}

/// `W9215` — a telemetry write failed, so streaming is off for the rest of the session.
///
/// **Never a retry and never a panic.** A write that failed once inside a frame will fail again
/// inside the next one, and retrying would put an unbounded number of failing syscalls on the
/// dispatcher at exactly the moment the machine is in trouble. The file keeps every whole block
/// written before the failure, which is what makes the loss bound "one window" true here too.
#[cold]
#[inline(never)]
pub(crate) fn report_telemetry_write_failed() {
    debug_assert_eq!(flag_code(DiagFlag::TelemetryWriteFailed), Some(W9215.number()));
    if claim(W9215.number()) {
        warn!(
            Profiling,
            W9215.number(),
            "a telemetry write failed; streaming is disabled for the rest of this session"
        );
    }
}

/// `W9218` — a quantile subscription was refused past the per-session cap.
///
/// The one condition in this group that is **not** a fault. `median` and `p95` cost a strided
/// gather of the whole retained window plus a sort, per zone, and the cap is what keeps the window
/// reduction inside its budget (M7). A refused zone still streams `count`, `total`, `min` and
/// `max`; only its two quantile fields are absent, and the record says so with a flag rather than
/// with a zero.
#[cold]
#[inline(never)]
pub(crate) fn report_telemetry_zones_refused() {
    debug_assert_eq!(flag_code(DiagFlag::TelemetryZonesRefused), Some(W9218.number()));
    if claim(W9218.number()) {
        warn!(
            Profiling,
            W9218.number(),
            "a telemetry quantile subscription was refused past the per-session cap; the zone              streams without a median or a p95"
        );
    }
}

// ───────────────────── L8c: four conditions that reserved codes and emitted nothing ─────────────
//
// All four were `CodeStatus::Pending` naming profiling rungs 5 and 8, both SHIPPED, and all four
// conditions were measured present in the tree and silent. What L8c had to decide was not whether
// to report them but HOW, and the answer is not uniform:
//
// * `W9202` RAISES a flag. Its site is `boyko_rhi_vulkan::present::gpu_zone::alloc_pair`, and that
//   crate neither depends on this one nor is depended on by it -- the flag word in `boyko_diag`,
//   which sits below both, is the ONLY route. It is also the only one raised under load, per
//   frame, which is the case the word's "a drop reported as a counter read cannot itself be
//   dropped" argument was made for.
//
// * `W9205`, `W9206` and `W9217` CALL THEIR REPORTER DIRECTLY, and that is a correctness
//   requirement rather than a shortcut. `fold.rs` is the single consumer of the flag word: it
//   calls `take_raised` once per fold. A contrast resolved after the run, or a teardown that
//   happens once the frame loop has stopped, would raise a bit NO FOLD EVER TAKES -- a report that
//   exists in the source and reaches nobody, which is the exact defect shape this campaign keeps
//   finding. Their sites are in `boyko_app`, which depends on this crate, so the call is available.
//
// The module stays the SOLE EMITTER of the `92xx` block either way: the rule is about which module
// emits, not about the flag word being the only door into it. Two doors, one room, and each
// condition takes the one its position in the graph and its frequency allow.

/// `W9202` — a GPU timestamp slot ran out of pair budget; further brackets are unrecorded.
///
/// Reached from the fold, via [`DiagFlag::GpuPairBudgetExhausted`]. The slot keeps every pair it
/// already allocated, so the frame's earlier zones are intact and its later ones are ABSENT rather
/// than wrong — which is why this is a `Warn` and why a reader needs it: an absent zone and a zone
/// that did not run are indistinguishable in the artifact without it.
#[cold]
#[inline(never)]
pub(crate) fn report_gpu_pair_budget_exhausted() {
    debug_assert_eq!(flag_code(DiagFlag::GpuPairBudgetExhausted), Some(W9202.number()));
    if claim(W9202.number()) {
        warn!(
            Profiling,
            W9202.number(),
            "a GPU timestamp slot exhausted its pair budget; further brackets in that frame are \
             unrecorded"
        );
    }
}

/// `W9205` — pairs were lost in this window, so its figures are folded from fewer samples.
///
/// **`pub`, and called directly by `boyko_app`'s reducer** — see the block above. A lost pair is
/// one the recorder bracketed and whose results never came back, which is a different statement
/// from `NotBracketed` (a leg that does not run that pass) and is why the label census carries
/// them apart.
#[cold]
#[inline(never)]
pub fn report_window_zones_lost(lost: u32, torn: u32, measured: u32) {
    if claim(W9205.number()) {
        warn!(
            Profiling,
            W9205.number(),
            "zones were lost in this window: {} lost, {} torn, {} measured -- the figures are \
             folded from fewer samples than the window ran",
            lost,
            torn,
            measured
        );
    }
}

/// `W9206` — a contrast could not be resolved, so the comparison has no verdict.
///
/// **`pub`, and called directly by `boyko_app`'s comparator.** `resolve` runs after a measurement,
/// often after the frame loop has stopped, so a raised flag would wait for a fold that never
/// comes.
///
/// The reason is an ARGUMENT rather than part of the message literal: `NotResolvedReason` is a
/// closed enum with a wire word apiece, and a reader who gets `floor_workload_mismatch` needs a
/// different thing from one who gets `below_band` — the first is a measurement that compared two
/// configurations, the second is a real result meaning "the difference is inside the band".
#[cold]
#[inline(never)]
pub fn report_contrast_not_resolved(reason: &str) {
    if claim(W9206.number()) {
        warn!(
            Profiling,
            W9206.number(),
            "a contrast could not be resolved ({}); the comparison carries no verdict",
            reason
        );
    }
}

/// `W9217` — GPU timestamp slots were still in flight at teardown and were abandoned.
///
/// **`pub`, and called directly by `boyko_app`'s teardown**, which is the clearest case of the
/// block above: by the time this is true the frame loop has stopped, so nothing will ever fold
/// again and a raised flag would be a report that cannot arrive.
///
/// `GpuZoneRecorder::flush` exists precisely to make this NOT happen — it force-retires every
/// in-flight slot as `Flushed` — so reaching this means the teardown path did not call it. The
/// commonest way is a run that ends before its measurement window completes: a closed window, or a
/// terminal device error.
#[cold]
#[inline(never)]
pub fn report_gpu_slots_abandoned(slots: u32) {
    if claim(W9217.number()) {
        warn!(
            Profiling,
            W9217.number(),
            "{} GPU timestamp slot(s) were still in flight at teardown and were abandoned; their \
             brackets are absent from this run's artifact",
            slots
        );
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

/// `W9201` — the engine zone registry is exhausted; further zones run unregistered.
///
/// **A warning, not an error, and not a panic.** A missing zone is a missing *measurement*, not a
/// wrong *answer* — and a profiler that aborts a build because a legal app has more systems than
/// it has slots has become the failure it exists to report.
#[cold]
#[inline(never)]
pub(crate) fn report_registry_exhausted() {
    debug_assert_eq!(flag_code(DiagFlag::ZoneRegistryExhausted), Some(W9201.number()));
    if claim(W9201.number()) {
        warn!(
            Profiling,
            W9201.number(),
            "the engine zone registry is exhausted at {} slots; further zones run unregistered",
            boyko_diag::profiling_abi::minted_zones()
        );
    }
}

/// `W9208` — the engine zone registry crossed 90 % occupancy.
///
/// Nothing is lost yet. This code exists so that exhaustion is not the first news of it: by the
/// time `W9201` fires, the zones that would have explained the problem are the ones missing.
#[cold]
#[inline(never)]
pub(crate) fn report_registry_near_full() {
    debug_assert_eq!(flag_code(DiagFlag::ZoneRegistryNearFull), Some(W9208.number()));
    if claim(W9208.number()) {
        warn!(
            Profiling,
            W9208.number(),
            "the engine zone registry is at {} of {} slots",
            boyko_diag::profiling_abi::minted_zones(),
            boyko_diag::profiling_abi::ENGINE_ZONE_SLOTS as u64
        );
    }
}

/// `W9210` — a `User`-partition crate's zone budget or the dynamic name arena is exhausted.
///
/// **The other side of `W9201`, and deliberately not the same code.** `W9201` says the ENGINE ran
/// out of its own slots, which a host cannot act on because it does not choose that number.
/// `W9210` says a GAME met `MAX_USER_BUDGET`, which is a number the host does choose — so the two
/// carry different advice and reporting them as one would give a host the wrong one.
///
/// Profiling rung 10.
#[cold]
#[inline(never)]
pub(crate) fn report_user_budget_exhausted() {
    debug_assert_eq!(
        flag_code(DiagFlag::UserZoneBudgetExhausted),
        Some(W9210.number())
    );
    if claim(W9210.number()) {
        warn!(
            Profiling,
            W9210.number(),
            "the user zone budget or name arena is exhausted at {} of {} slots; further game \
             zones run unregistered",
            boyko_diag::profiling_abi::minted_user_zones(),
            boyko_diag::profiling_abi::MAX_USER_BUDGET as u64
        );
    }
}

/// `W9212` — `register_zone` was asked for a scope inside the engine's reserved range.
///
/// The registration is **refused**, not clamped. A game whose zone silently moved to a neighbouring
/// scope would be armed and disarmed by a knob it never asked for, and its samples would arrive
/// interleaved with the engine's under a scope name that describes neither.
///
/// Profiling rung 10.
#[cold]
#[inline(never)]
pub(crate) fn report_engine_scope_refused() {
    debug_assert_eq!(flag_code(DiagFlag::EngineScopeRefused), Some(W9212.number()));
    if claim(W9212.number()) {
        warn!(
            Profiling,
            W9212.number(),
            "register_zone refused a scope below {}; scopes under that are the engine's",
            boyko_diag::profiling_abi::USER_SCOPE_BASE as u64
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
    ///
    /// # It asserted FOUR of nine rows until L8c, and the gap was invisible
    ///
    /// `flag_code`'s `match` covers all nine `DiagFlag` variants — the module doc explains at
    /// length that it is deliberately not `_`-terminated so a new variant fails to compile. This
    /// test covered the four in the doc's illustrative table and stopped there, so `W9210`,
    /// `W9212`, `W9214`, `W9215` and `W9218` had their mapping asserted **nowhere**.
    ///
    /// Two of them were also observed nowhere else, and the registry's check 5 did not say so
    /// because it read the raw text of test files: a *doc comment* naming `W9212` counted as a
    /// test naming it. L8c stripped comments from that corpus, and `W9212`/`W9214` fell out
    /// immediately. The mapping is what is falsifiable for both — an unwritable telemetry path and
    /// an engine-scope refusal are provoked by state this binary does not have — so this is where
    /// they are asserted, on the same argument `W9207`'s row below already carries.
    #[test]
    fn the_flag_to_code_table_is_the_one_stated_in_the_module_docs() {
        assert_eq!(flag_code(DiagFlag::ClockEpochBreak), Some(W9216.number()));
        assert_eq!(flag_code(DiagFlag::LaneExhausted), Some(W9203.number()));
        assert_eq!(flag_code(DiagFlag::ZoneRegistryExhausted), Some(W9201.number()));
        assert_eq!(flag_code(DiagFlag::ZoneRegistryNearFull), Some(W9208.number()));
        // Rung 10's pair. The split is the corpus's own: a budget the HOST set versus a scope the
        // game may not have, and reporting both as one code would tell a host to raise a limit it
        // does not control.
        assert_eq!(flag_code(DiagFlag::UserZoneBudgetExhausted), Some(W9210.number()));
        assert_eq!(flag_code(DiagFlag::EngineScopeRefused), Some(W9212.number()));
        // Rung 13's three. `W9214` leaves NO file, `W9215` leaves a file with a stated end, and
        // `W9218` is a budget rather than a fault -- three states a reader can act on differently,
        // which is why they are not one code.
        assert_eq!(flag_code(DiagFlag::TelemetryPathUnwritable), Some(W9214.number()));
        assert_eq!(flag_code(DiagFlag::TelemetryWriteFailed), Some(W9215.number()));
        assert_eq!(flag_code(DiagFlag::TelemetryZonesRefused), Some(W9218.number()));
        // L8c's one flag-routed condition. The other three of that rung call their
        // reporters directly and so have no arm here -- see the block above them.
        assert_eq!(flag_code(DiagFlag::GpuPairBudgetExhausted), Some(W9202.number()));
        // A POSITIVE answer, not a gap: the condition is reported as a frame flag, because its
        // consequence is a status on the data rather than an event.
        assert_eq!(flag_code(DiagFlag::ClockUncalibrated), None);
    }

    /// Every code the table can produce is DISTINCT, which the row-by-row assertions above cannot
    /// say on their own.
    ///
    /// Two flags mapped to one code would make an engine defect and a configuration fact arrive as
    /// the same record — the exact confusion the rung-10 and rung-13 splits were made to prevent —
    /// and every individual `assert_eq!` above would still pass.
    #[test]
    fn no_two_flags_share_a_code() {
        const FLAGS: [DiagFlag; 11] = [
            DiagFlag::ClockEpochBreak,
            DiagFlag::ClockUncalibrated,
            DiagFlag::LaneExhausted,
            DiagFlag::ZoneRegistryExhausted,
            DiagFlag::ZoneRegistryNearFull,
            DiagFlag::UserZoneBudgetExhausted,
            DiagFlag::EngineScopeRefused,
            DiagFlag::TelemetryPathUnwritable,
            DiagFlag::TelemetryWriteFailed,
            DiagFlag::TelemetryZonesRefused,
            DiagFlag::GpuPairBudgetExhausted,
        ];
        let mut seen: Vec<u16> = Vec::new();
        for f in FLAGS {
            if let Some(c) = flag_code(f) {
                assert!(!seen.contains(&c), "two flags map to code {c}");
                seen.push(c);
            }
        }
        assert_eq!(seen.len(), 10, "ten of the eleven flags carry a code; ClockUncalibrated does not");
    }

    /// `W9207`'s selection is measured here because its emission cannot be: `invariant_tsc()` is
    /// `true` on every box this project targets, so the false branch has no reachable state and a
    /// gate over the emission would be green forever.
    #[test]
    fn the_clock_code_is_selected_only_when_the_tsc_is_not_invariant() {
        assert_eq!(clock_code(false), Some(W9207.number()));
        assert_eq!(clock_code(true), None);
    }

    /// Every emitter's code must have a census slot, or `claim` would fire its `debug_assert` and
    /// silently emit nothing in release.
    #[test]
    fn every_live_code_has_a_census_slot_and_they_are_distinct() {
        for (i, n) in LIVE_CODES.iter().copied().enumerate() {
            assert_eq!(slot_of(n), Some(i));
        }
        // The one place in this module a code stays a BARE NUMBER, and deliberately: `W9202` is
        // still `CodeStatus::Pending`, and the registry's check 3b requires a `Pending` row to have
        // ZERO identifier uses. Importing it to write `W9202.number()` here would put the
        // identifier in a `use` line outside any `#[cfg(test)]`, which is exactly what that check
        // is looking for. Every other number in this module became its constant at L8c.
        // The negative control moved at L8c: `9202` used to be the code "this rung does not
        // emit", and it now does. Every `92xx` code with an emitter has a slot, so the
        // subject has to come from outside the block -- `W0103` is the file sink's and this
        // module will never emit it. A negative control whose subject became positive is a
        // control that stopped controlling.
        assert_eq!(slot_of(103), None, "a code this rung does not emit must have no slot");
        // A duplicate would make two codes share a latch, so one would silence the other.
        for (i, a) in LIVE_CODES.iter().copied().enumerate() {
            for b in LIVE_CODES.iter().copied().skip(i + 1) {
                assert_ne!(a, b);
            }
        }
    }
}

#[cfg(test)]
mod l8c_emitter_tests {
    use super::*;

    /// The four conditions logging rung L8c gave emitters, observed through the census that
    /// [`report_count`] exposes.
    ///
    /// **`>= 1`, not `== 1`**, and the module header already states why: the latches are process
    /// state and a sibling test in the same binary may have claimed one first. What the census can
    /// prove is that the emit path was TAKEN, which is the claim each of these rows owed.
    ///
    /// Each drives the PRODUCTION reporter — the same function its site calls — rather than a
    /// `warn!` written beside it. L8a paid for that lesson three times in one rung.
    #[test]
    fn the_four_l8c_conditions_all_reach_the_emit_path() {
        // `W9202` goes through the flag word, so it is driven the way the fold drives it: bits in,
        // report out. That also exercises the `report_raised` dispatch arm, which a direct call to
        // the reporter would skip.
        report_raised(DiagFlag::GpuPairBudgetExhausted.as_bits());
        assert!(report_count(W9202.number()) >= 1, "the pair-budget overflow was silent");

        report_window_zones_lost(7, 2, 51);
        assert!(report_count(W9205.number()) >= 1, "the lost window was silent");

        report_contrast_not_resolved("floor_workload_mismatch");
        assert!(report_count(W9206.number()) >= 1, "the refused contrast was silent");

        report_gpu_slots_abandoned(3);
        assert!(report_count(W9217.number()) >= 1, "the abandoned slots were silent");
    }

    /// **THE GATE THAT WOULD HAVE CAUGHT RUNG 10**: every code `flag_code` can produce has a
    /// census slot.
    ///
    /// `claim` resolves a code through `slot_of` and returns `false` when there is none, so a
    /// reporter whose code is missing from `LIVE_CODES` fires a `debug_assert` and then emits
    /// NOTHING — in release, silently. That is what `W9210` and `W9212` did for three rungs: two
    /// complete emitters with `flag_code` arms, `Live` registry rows and doc pages, structurally
    /// incapable of emitting.
    ///
    /// Nothing in the workspace could see it. The orphan check found their identifiers, the
    /// doc-page check found their pages, the flag table asserted four of nine rows, and no test
    /// drove either condition. It surfaced only because L8c changed this array's length and a pin
    /// moved. **This is the check that makes the link mechanical instead of incidental.**
    #[test]
    fn every_code_the_flag_table_can_produce_has_a_census_slot() {
        for f in [
            DiagFlag::ClockEpochBreak,
            DiagFlag::ClockUncalibrated,
            DiagFlag::LaneExhausted,
            DiagFlag::ZoneRegistryExhausted,
            DiagFlag::ZoneRegistryNearFull,
            DiagFlag::UserZoneBudgetExhausted,
            DiagFlag::EngineScopeRefused,
            DiagFlag::TelemetryPathUnwritable,
            DiagFlag::TelemetryWriteFailed,
            DiagFlag::TelemetryZonesRefused,
            DiagFlag::GpuPairBudgetExhausted,
        ] {
            let Some(code) = flag_code(f) else { continue };
            assert!(
                slot_of(code).is_some(),
                "the flag with bits {:#x} maps to code {code}, which has no LIVE_CODES slot --                  `claim` will refuse it and its reporter will emit nothing, in release without a                  sound",
                f.as_bits()
            );
        }
    }

    /// Every code L8c added has a census slot, and `LIVE_CODES` grew to match.
    ///
    /// Without a slot `claim` fires its `debug_assert` and the reporter emits NOTHING in release —
    /// a code that is `Live`, documented, called, and silent. The four are asserted by identifier
    /// so the registry's check 5 can see them.
    #[test]
    fn the_four_l8c_codes_have_census_slots() {
        for c in [W9202.number(), W9205.number(), W9206.number(), W9217.number()] {
            assert!(slot_of(c).is_some(), "code {c} has no census slot, so its reporter is silent");
        }
        assert_eq!(
            LIVE_CODES.len(),
            18,
            "twelve before L8c; four added for its own codes, and TWO for rung 10's reporters that \n             had no slot and therefore could not emit"
        );
    }
}
