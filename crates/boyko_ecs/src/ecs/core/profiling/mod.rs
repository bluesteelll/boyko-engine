//! The profiling store — the ECS half of `boyko_diag::profiling_abi`.
//!
//! # Why this lives inside `boyko_ecs`
//!
//! The same reason [`log`](crate::ecs::core::log) does: the durable store is backed by
//! [`VmReservation`](crate::ecs::memory::vm::VmReservation), which is `pub(crate)` to this crate
//! and must stay so — its soundness rests on invariants (`base` is write-once, the frontier is
//! monotone, nothing is freed) a foreign crate could not be held to. Putting the store here is
//! what makes it engine storage rather than a `Box<[u8]>` side-store, which Principle 0 forbids
//! **even inside a `Resource`**.
//!
//! The dependency edge runs `boyko_ecs -> boyko_diag`, never back. The substrate knows nothing
//! about the ECS: producers write 24 B records into `.bss` lane rings and the fold here copies
//! them out.
//!
//! # This module is the substrate's only mouth
//!
//! `boyko_diag` and `profiling_abi` are diagnostically **mute** — no code, no print, no panic
//! hook. Every `92xx` condition is raised there as a sticky bit plus a counter, and
//! [`diag`] is where they become diagnostics. That is what makes the report of a profiler drop a
//! *counter read* rather than a log record that can itself be dropped under exactly the load that
//! produced the drop.
//!
//! # What this rung is, and what it is not
//!
//! **Profiling rungs 2 and 3.** What exists: [`Profiler`] on a process-lifetime reservation with
//! an arm-time [`zone_stride`](Profiler::zone_stride), [`fold`], [`arm`](Profiler::arm) /
//! [`disarm`](Profiler::disarm), [`ProfilerPlugin`] and its world bind, the `92xx` codes this rung
//! can emit, the engine's own [`zones`] (the four `App` brackets, the fold's own, the per-system
//! span and the dispatch-round pair), and — behind `feature = "profiling-analysis"` — the interval
//! ring and the [`analysis`] report that reads it against the schedule's conflict graph.
//!
//! What does **not** exist yet, by rung, each absent rather than present-and-zero:
//!
//! | Rung | Absent here |
//! |---|---|
//! | 5–7 | the GPU channel, and with it `FrameState::Partial` and three of the five cell labels |
//! | 8 | `LegSummary`, the window reducer, `resolve`, the artifact |
//!
//! A field that is structurally always zero is indistinguishable from a measurement of zero, and a
//! reader cannot tell the difference. So each arrives with the rung that can make it move.
//!
//! **Telemetry (rung 13) is no longer on that list, and it did not land here.** The wire format is
//! `boyko_diag::telemetry` and the writer is `boyko_app::profiling::stream`; what rung 13 added to
//! *this* store is one section — the observed-kind map, `Profiler::observed_kind` — because a
//! `ZoneDesc` carries no kind and the fold is the only party that can state whether a cell's `total`
//! is ticks or increments.

// ---------------------------------------------------------------------------------------------
// The one place the two halves of the build axis are made to agree — profiling rung 14 (J1)
// ---------------------------------------------------------------------------------------------
//
// `BOYKO_PROFILE` is an environment variable read by `crates/boyko_diag/build.rs`;
// `profiling-analysis` is a cargo FEATURE of this crate. Cargo resolves features before any build
// script runs, and `cargo::rustc-cfg` applies only to the crate that emitted it, so **no value the
// axis emits can switch this feature**. The axis publishes what the profile ADMITS and this line
// refuses a disagreement — which is the whole of it: a `shipping` build that still carries
// `ConcurrencyReport`, `resolve` and the TOML writer fails to build here, rather than shipping
// symbols its own profile says are absent and being caught (or not) by a symbol census afterwards.
//
// The direction is one-way on purpose. Analysis compiled OUT of a `dev` build is a developer who
// passed fewer flags than they meant to; analysis compiled INTO a `shipping` build is the profile
// being a lie. Only the second is an error.
const _: () = assert!(
    !(cfg!(feature = "profiling-analysis") && !boyko_diag::profile::ANALYSIS_ADMITTED),
    "this build selected a BOYKO_PROFILE that does not admit the analysis half of the profiler \
     (shipping, shipping-min or off), but the `profiling-analysis` cargo feature is enabled. The \
     axis cannot switch a cargo feature -- drop `--features boyko-ecs/profiling-analysis`. Note \
     that `--no-default-features` is NOT the answer and never was: nine sibling manifests depend \
     on boyko-ecs without `default-features = false`, so a default-on feature is re-enabled by \
     unification no matter what the command line says. That is why this feature is opt-in."
);

#[cfg(feature = "profiling-analysis")]
pub mod analysis;
pub mod diag;
pub mod ecs_control;
pub mod fold;
pub mod hist;
pub mod lifetime;
pub mod plugin;
pub mod store;
pub mod zones;

#[cfg(test)]
mod tests;

#[cfg(feature = "profiling-analysis")]
pub use analysis::{ConcurrencyReport, PairVerdict, concurrency, pair_overlap};
pub use diag::{LIVE_CODES, report_count};
// L8c: the three `92xx` conditions whose sites live in `boyko_app` and whose reports cannot go
// through the flag word, because `fold.rs` is its single consumer and these three become true
// after the last fold. Re-exported so the host can reach the emitter without becoming one.
pub use diag::{report_contrast_not_resolved, report_gpu_slots_abandoned, report_window_zones_lost};
pub use ecs_control::{
    ENGINE_SCOPE_BASE, LatencyTable, ProfiledZone, ProfilingScope, ProfilingScopeEnabled,
    ScopeError, minted_game_scopes, register_scope,
};
pub use fold::{any_armed, fold};
pub use hist::{HIST_BUCKETS, HistSlot, HistView, bucket_edges, bucket_of};
pub use lifetime::LifetimeAcc;
pub use plugin::ProfilerPlugin;
#[cfg(feature = "profiling-analysis")]
pub use store::{INTERVALS_PER_FRAME, Interval, OVERLAP_FRAMES};
pub use store::{
    ArmOutcome, Cell, CellLabel, COLUMN_BYTES_PER_ZONE, DropCounters, FOLD_L1D_ZONE_LIMIT,
    MAX_HIST_SLOTS,
    FRAME_FLAG_CLOCK_UNCALIBRATED, FrameRecord, FrameState, MAX_PLAUSIBLE_FRAME_TICKS, Profiler,
    ProfilerConfig, ROOT_SCOPE, UNBOUND, WINDOW, bind_world, bound_world,
};

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::profiling::zones::FOLD;

/// The zone id of something that has none.
///
/// **`0`, and that is not an arbitrary sentinel.** `boyko_diag`'s registry starts minting at 1
/// precisely because zero is the un-minted state of a `ZoneHandle`'s own `id` field, so the two
/// meanings coincide by construction instead of by a second convention somebody has to keep. A
/// `SystemMeta` whose `zone` is 0 was never minted, and a sample carrying zone 0 came from
/// nothing this engine registered.
pub const ZONE_ID_UNASSIGNED: u16 = 0;

/// The tier a per-system zone is declared at.
///
/// `Deep`: a span around every system, every run, is per-frame detail a shipped title does not
/// pay for. The gate is `const`, so at a ceiling below this the minting loop, the `zone` writes
/// and the id space they consume are all deleted from the build.
pub const SYSTEM_ZONE_TIER: boyko_diag::profiling_abi::ZoneTier =
    boyko_diag::profiling_abi::ZoneTier::Deep;

/// Whether this build mints zones for systems at all.
///
/// Read as a `const` at the one site that mints, so a folded tier costs the builder nothing —
/// not a branch, not a counter, not an id.
pub const SYSTEM_ZONES_COMPILED: bool =
    (SYSTEM_ZONE_TIER as u8) <= (boyko_diag::profiling_abi::GLOBAL_TIER as u8);

/// Run the fold for `world`, if it has a profiler and the profiler is armed.
///
/// **The mask read comes first**, before the resource lookup, so a process with profiling off pays
/// one `.bss` load and one predicted branch per frame and touches no resource slot. Doing the
/// lookup first would make the disarmed cost a hash-free array index plus a bounds test — still
/// small, and still more than nothing, in the one place the whole subsystem's off-cost claim is
/// measured.
#[inline]
pub fn fold_frame(world: &mut EcsMaster) {
    if !any_armed() {
        return;
    }
    fold_frame_cold(world);
}

/// The armed arm, out of line so the disarmed path is a load and a branch with nothing else in the
/// instruction stream.
#[cold]
#[inline(never)]
fn fold_frame_cold(world: &mut EcsMaster) {
    if !world.contains_resource::<Profiler>() {
        // Armed, but this world has no store. That is the second world of a two-world process:
        // `ProfilerPlugin::build` refused it with `E9204` and inserted nothing, and the mask is
        // process-global so it reads armed here regardless.
        return;
    }
    // The instrument measures itself, and the bracket is here rather than inside `fold` for one
    // reason: `__fold`'s own sample is pushed by the guard's `Drop`, so it must close AFTER the
    // fold has finished draining — otherwise the sample it produces would be drained by the same
    // fold that produced it and attributed to the frame it was measuring.
    let _z = boyko_diag::zone!(FOLD);

    // Step 0 (A8): publish the scope half of the mask from the ECS, BEFORE the drain. Inside the
    // `__fold` bracket deliberately — D16 says the instrument's own cost is disclosed rather than
    // hidden, and the projection is part of that cost.
    //
    // Before rather than after, because the two orders differ by a frame: a toggle projected after
    // the drain would gate the samples of the frame that is only just opening, so the gate a
    // reader sees would be one the previous frame's samples never passed through.
    ecs_control::project(world);

    fold(world.resource_mut::<Profiler>());
}
