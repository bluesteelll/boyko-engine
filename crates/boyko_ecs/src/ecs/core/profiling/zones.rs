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
//! | [`ROUND`] / [`ROUND_WIDTH`] | one executor round that dispatched at least one system | one pair per dispatching round |
//!
//! # Dispatch shape is TWO ZONES, and not a `RoundRecord` column
//!
//! The corpus specifies `RoundRecord { frame, round, dispatched, begin, end }` — 24 B × 121 × 32 =
//! **90.8 KiB** of the reservation — to keep *"dispatch shape only: rounds per frame, wave width,
//! round span"*. All three of those are per-frame statistics, and all three fall out of two zone
//! cells the store already has:
//!
//! | Corpus quantity | Where it is read |
//! |---|---|
//! | rounds per frame | [`ROUND`]'s `count` |
//! | round span | [`ROUND`]'s `total` / `min` / `max` |
//! | wave width | [`ROUND_WIDTH`]'s `total` / `min` / `max` (a `Counter`, so `total` is Σ dispatched) |
//!
//! What the column would have bought over this is the **correlation**: whether the widest round was
//! also the longest one. Nothing in this corpus asks that question, and the price of being able to
//! is 90.8 KiB plus `MAX_ROUNDS_PER_FRAME = 32` truncation with its own drop class — a schedule
//! whose dependency chain is 33 rounds deep would have been *counted as dropped* rather than
//! measured. The pair of zones truncates at nothing.
//!
//! The decisive half is not the arithmetic, though. A column write needs a path from the dispatcher
//! into the reservation, and the dispatcher does **not** hold `&mut EcsMaster` while a round is in
//! flight — the cell it minted is shared with the workers. Reaching the store from there means
//! either a second published pointer into the reservation, written by a thread the fold's `&mut`
//! does not cover, or a per-schedule scratch buffer flushed later — profiling state owned by the
//! scheduler. The lane push has neither problem: it is the mechanism the substrate exists to be,
//! and it is what [`SystemSpan`] already does one level down.
//!
//! **This is a deviation from the corpus and is recorded as one**, in
//! `docs/diagnostics/profiling/05-LADDER-GATES.md` and in `docs/OPEN-QUESTIONS.md`. The lost
//! correlation is named there rather than left to be discovered.
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

declare_zone!(
    ROUND,
    name = "__round",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Deep,
);

declare_zone!(
    ROUND_WIDTH,
    name = "__round_width",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Deep,
);

/// Whether this build compiles the dispatch-round probe. Both round zones are `Deep`, so one
/// `const` answers for the pair.
pub const ROUND_ZONES_COMPILED: bool =
    (ROUND::TIER as u8) <= (boyko_diag::profiling_abi::GLOBAL_TIER as u8);

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

/// One executor dispatch round, measured on the dispatcher.
///
/// # Why this is not a `Drop` guard, unlike [`SystemSpan`]
///
/// A round's **width** is only known when [`try_dispatch_ready`] returns, and a round that
/// dispatched nothing is not a wave — it is the executor parking while workers finish, and
/// recording it would put zeroes into `min` and make the width distribution report a wave width
/// this schedule never had. `Drop` cannot express "record, but only given a value I do not have
/// yet", so the close takes the count and consumes the probe.
///
/// The cost of that choice is stated rather than hidden: a round whose systems panic unwinds past
/// [`close`](Self::close) and contributes no record. That frame is being torn down by a re-raised
/// worker panic anyway, and a `Drop` that recorded a width of zero on the way out would be worse
/// than a missing round.
///
/// [`try_dispatch_ready`]: crate::ecs::core::schedule::schedule::Schedule
pub struct RoundProbe {
    /// The clock at the round's open.
    opened: u64,
}

impl RoundProbe {
    /// Start measuring a round, or `None` when this build or this session is not measuring.
    ///
    /// The gates are A1's, in A1's order: a `const` the compiler folds, then one `.bss` load with
    /// a predicted branch. With the tier folded the call and its `close` are deleted outright; with
    /// the profiler disarmed the whole round costs one load and one not-taken branch, and in
    /// particular **no `rdtsc`** — which is why the clock read is inside the `Some`, after both
    /// gates, and not at the call site.
    #[inline]
    #[must_use]
    pub fn open() -> Option<RoundProbe> {
        if !ROUND_ZONES_COMPILED || !boyko_diag::profiling_abi::scope_armed(ROOT_SCOPE) {
            return None;
        }
        Some(RoundProbe { opened: boyko_diag::clock::ticks() })
    }

    /// Close the round, recording its span and its width — or nothing, if it dispatched nothing.
    ///
    /// Both samples carry the **same `stamp`**, so the fold attributes them to the same frame by
    /// the same walk. A width recorded against a different frame from its own span is two
    /// statements that cannot be joined.
    #[inline]
    pub fn close(self, dispatched: usize) {
        if dispatched == 0 {
            return;
        }
        let elapsed = boyko_diag::clock::ticks().wrapping_sub(self.opened);
        // Refusals are already counted by the region's own `overflow`, so the results are dropped
        // rather than escalated — the same reasoning as `SystemSpan`'s, and for the same reason:
        // there is nothing the executor could usefully do about a full ring mid-frame.
        let _ = boyko_diag::sample::push(
            crate::__BOYKO_ZONE_PARTITION,
            boyko_diag::sample::Sample {
                stamp: self.opened,
                value: elapsed,
                zone: boyko_diag::profiling_abi::zone_id(&ROUND),
                flags: boyko_diag::sample::SampleKind::Span as u16,
                _pad: 0,
            },
        );
        let _ = boyko_diag::sample::push(
            crate::__BOYKO_ZONE_PARTITION,
            boyko_diag::sample::Sample {
                stamp: self.opened,
                // `usize -> u64` is widening on every target this engine builds for, so the wave
                // width reaches the store without a cast that could narrow it.
                value: dispatched as u64,
                zone: boyko_diag::profiling_abi::zone_id(&ROUND_WIDTH),
                flags: boyko_diag::sample::SampleKind::Counter as u16,
                _pad: 0,
            },
        );
    }
}
