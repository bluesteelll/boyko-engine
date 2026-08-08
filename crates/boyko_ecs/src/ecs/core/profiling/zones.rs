//! The engine's own zone sites — the frame driver's four, and the instrument's one.
//!
//! # Why the frame is FOUR zones and not one
//!
//! An earlier definition said the primary CPU number was *"the `Schedule::run` span"*. The host's
//! frame is `Time → events → Fixed×N → Main` — **two schedules, and `Fixed` runs N times** — so
//! "the `Schedule::run` span" is not one interval, and "the fold is outside the primary number" was
//! undefined across N+1 runs.
//!
//! | Zone | Bracket | Cardinality |
//! |---|---|---|
//! | [`FRAME`] | `update_with_delta` entry (**after** the fold returns) → exit | 1 per frame — **this is the primary CPU number** |
//! | [`EVENTS`] | step ③ `update_events` | 0 or 1 |
//! | [`FIXED_STEP`] | one `fixed.run(world)` inside step ④ | **N** per frame |
//! | [`MAIN_RUN`] | step ⑤ `schedule.run(world)` | 1 |
//! | [`FOLD`] | the fold itself | 1 per frame, **outside [`FRAME`] by construction** |
//!
//! # Nothing here is copied into `FrameRecord`, deliberately
//!
//! The corpus's `FrameRecord` carries `run_gross`, `fixed_total`, `main_total`,
//! `instrument_measured` and `fixed_steps`. Every one of them is **already** in a cell: `run_gross`
//! is `FRAME`'s `total` for that frame row, `fixed_total` is `FIXED_STEP`'s, `instrument_measured`
//! is `FOLD`'s, and `fixed_steps` — the substep count N — is `FIXED_STEP`'s `count`, because a
//! zone that opens N times per frame counts N.
//!
//! Copying them into the frame record would be a second statement of five facts the store already
//! holds, in a struct that is written by a different code path — which is how two numbers for one
//! quantity come to disagree. So `FrameRecord` does not grow at this rung, and the reducer reads
//! the cells like it reads every other zone's.
//!
//! # Tiers
//!
//! [`FRAME`] and [`FOLD`] are `Always`: frame time ships, and the instrument's own cost must be
//! measurable in the configuration a title actually runs — an instrument whose cost is only visible
//! in the build where it does not matter is not disclosed at all. The three inner brackets are
//! `Dev`: they are subsystem spans, and a shipped title pays nothing for them because a `const
//! false` in the gate's `&&` chain deletes the arm and its operands.

use boyko_diag::declare_zone;
use boyko_diag::profiling_abi::ZoneTier;

use crate::ecs::core::profiling::store::ROOT_SCOPE;

declare_zone!(
    FRAME,
    name = "__frame",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Always,
);

declare_zone!(
    EVENTS,
    name = "__events",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Dev,
);

declare_zone!(
    FIXED_STEP,
    name = "__fixed_step",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Dev,
);

declare_zone!(
    MAIN_RUN,
    name = "__main_run",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Dev,
);

declare_zone!(
    FOLD,
    name = "__fold",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Always,
);

/// One system's run, as a span.
///
/// # Why this is not `zone!`
///
/// [`zone!`](boyko_diag::zone) takes a bare identifier and reads a `static ZoneHandle` plus its
/// `mod` companion. A system has neither: its id is minted at `try_build` into
/// [`SystemMeta.zone`](crate::ecs::core::system::system_meta::SystemMeta::zone), and its name lives
/// in `SystemMeta` rather than in a `&'static ZoneDesc` — which is rung 3a's decision, taken so the
/// engine does not store a second copy of a string the schedule already owns.
///
/// So the bracket is written out: the tier gate is a `const` read from
/// [`SYSTEM_ZONES_COMPILED`](crate::ecs::core::profiling::SYSTEM_ZONES_COMPILED) instead of from a
/// companion module, and the id comes from the meta instead of from a handle. Everything else —
/// the `&&` chain, the runtime scope test, the one `rdtsc` at open and one at close — is A1's,
/// verbatim.
///
/// # Why a guard and not `open`/`close`
///
/// A system that panics must still close its span, or its interval is lost **and** every enclosing
/// span silently absorbs it. `Drop` is the only closing discipline the language enforces, and the
/// schedule has no `catch_unwind` of its own to fall back on.
pub struct SystemSpan {
    /// The system's minted zone id.
    zone: u16,
    /// The clock at open.
    opened: u64,
}

impl SystemSpan {
    /// Open a span for `zone`, or `None` when this build, this session or this system has none.
    ///
    /// Three gates, in the order that makes the disarmed cost smallest: a `const` the compiler
    /// folds, then one `.bss` load with a predicted branch, then a compare against the unassigned
    /// sentinel. With the tier folded the whole function is deleted along with its call sites'
    /// arguments; with the tier compiled and the profiler disarmed it is one load and one branch.
    #[inline]
    #[must_use]
    pub fn open(zone: u16) -> Option<SystemSpan> {
        if !crate::ecs::core::profiling::SYSTEM_ZONES_COMPILED
            || !boyko_diag::profiling_abi::scope_armed(ROOT_SCOPE)
            || zone == crate::ecs::core::profiling::ZONE_ID_UNASSIGNED
        {
            return None;
        }
        Some(SystemSpan { zone, opened: boyko_diag::clock::ticks() })
    }
}

impl Drop for SystemSpan {
    #[inline]
    fn drop(&mut self) {
        // `wrapping_sub` because the clock is a raw counter: a wrap must yield the interval rather
        // than a number near 2^64, which would panic in debug and poison a window in release.
        let elapsed = boyko_diag::clock::ticks().wrapping_sub(self.opened);
        // The push lands in the LANE OF THE THREAD THAT RAN THE SYSTEM — a worker's own lane on the
        // concurrent path, the dispatcher's on the inline one. That is the whole reason the lane is
        // resolved per-thread rather than passed in: a span recorded against the wrong lane would
        // be a sample the wrong producer is charged for, and the overlap analysis reads exactly
        // those two fields.
        //
        // A refusal is already counted by the region's own `overflow`, so the result is dropped
        // rather than escalated: there is nothing a system's epilogue could usefully do about a
        // full ring, and a branch here would be paid by every system, every run, forever.
        let _ = boyko_diag::sample::push(
            crate::__BOYKO_ZONE_PARTITION,
            boyko_diag::sample::Sample {
                stamp: self.opened,
                value: elapsed,
                zone: self.zone,
                flags: boyko_diag::sample::SampleKind::Span as u16,
                _pad: 0,
            },
        );
    }
}
