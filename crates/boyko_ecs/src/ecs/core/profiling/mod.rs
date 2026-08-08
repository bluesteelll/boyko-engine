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
//! **Profiling rung 2.** What exists: [`Profiler`] on a process-lifetime reservation with an
//! arm-time [`zone_stride`](Profiler::zone_stride), [`fold`], [`arm`](Profiler::arm) /
//! [`disarm`](Profiler::disarm), [`ProfilerPlugin`] and its world bind, and the seven `92xx` codes
//! this rung can emit.
//!
//! What does **not** exist yet, by rung, each absent rather than present-and-zero:
//!
//! | Rung | Absent here |
//! |---|---|
//! | 3 | `SystemMeta.zone`, tier-gated minting, the four `App` zones, `RoundRecord`, `sys_of` |
//! | 5–7 | the GPU channel, and with it `FrameState::Partial` and three of the five cell labels |
//! | 8 | `LegSummary`, the window reducer, `resolve`, the artifact |
//! | 10 | the dynamic registry, so `ProfilerConfig::user_zone_budget` has nothing to spend itself on |
//! | 11 | the scope projection — [`ROOT_SCOPE`] stands in, so the profiler arms as a whole |
//! | 12 | the lifetime accumulators and the histograms |
//! | 13 | telemetry |
//!
//! A field that is structurally always zero is indistinguishable from a measurement of zero, and a
//! reader cannot tell the difference. So each arrives with the rung that can make it move.

pub mod diag;
pub mod fold;
pub mod plugin;
pub mod store;

#[cfg(test)]
mod tests;

pub use diag::{LIVE_CODES, report_count};
pub use fold::{any_armed, fold};
pub use plugin::ProfilerPlugin;
pub use store::{
    ArmOutcome, Cell, CellLabel, COLUMN_BYTES_PER_ZONE, DropCounters, FOLD_L1D_ZONE_LIMIT,
    FRAME_FLAG_CLOCK_UNCALIBRATED, FrameRecord, FrameState, MAX_PLAUSIBLE_FRAME_TICKS, Profiler,
    ProfilerConfig, ROOT_SCOPE, UNBOUND, WINDOW, bind_world, bound_world,
};

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

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
    fold(world.resource_mut::<Profiler>());
}
