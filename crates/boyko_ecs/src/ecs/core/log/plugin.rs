//! [`LogPlugin`] and [`log_drain_system`] — the seam's registration and its one system.

use boyko_log::sink::ecs;

use crate::ecs::core::app::{App, Plugin};
use crate::ecs::core::log::ring::LogRing;
use crate::ecs::core::log::stats::LogStats;
use crate::ecs::core::schedule::system_set::SystemSet;
use crate::ecs::core::system::ResMut;

/// The set [`log_drain_system`] belongs to, so a host can order its own systems against the drain
/// without naming the system.
///
/// # There is no `Last` schedule in this engine, and this is what replaces it
///
/// The design this seam was specified against says "`log_drain_system` in `Last`". This engine's
/// [`CoreSchedule`](crate::ecs::core::app::CoreSchedule) is a closed set of two — `Main` and
/// `Fixed` — and its own doc states the intended answer: *"finer-grained structure WITHIN a
/// schedule is what Phase-15 sets are for."* So the drain runs in `Main`, in this set, and a host
/// that wants the frame's own records visible in the same frame writes
/// `.before(LogSet::Drain)` on its emitters.
///
/// **What that costs, stated rather than smoothed:** with no edge, the scheduler is free to place
/// the drain anywhere in the frame, so a record emitted after it lands in the next frame's ring.
/// The specified bound of "one frame" therefore holds only for hosts that add the edge; without
/// one it is two. [`LogPlugin::build`] interns the set so that edge resolves regardless of
/// plugin add-order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LogSet;

impl SystemSet for LogSet {}

/// Registers the logging seam: the resources, and the one system that feeds them.
///
/// # What `build` deliberately does NOT do
///
/// It performs no reservation and no commit. It runs before the runtime flag is read, and a
/// diagnostics subsystem may not make a syscall the flag has not authorised — so
/// [`LogRing::new`](crate::ecs::core::log::LogRing::new) is lazy and the one growth happens on the
/// first drain that carries a line. It also does not call `boyko_log::lifecycle::boot` or
/// `enable`: the host owns that decision, because it is the host that parses the launch flag, and
/// a plugin that enabled diagnostics by being added would make "add the plugin" and "turn
/// diagnostics on" the same act.
#[derive(Default)]
pub struct LogPlugin;

impl Plugin for LogPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(LogRing::new());
        app.insert_resource(LogStats::default());
        app.add_systems_cfg(|b| {
            // Interning the set is what makes a host's `.before(LogSet)` resolve even when the
            // host plugin is added after this one — the same reason `CameraPlugin` configures
            // `CameraSet::Control` with no edges of its own.
            b.configure_set(LogSet);
            b.add_system(log_drain_system).in_set(LogSet);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_ecs::LogPlugin"
    }
}

/// Copy every formatted line the drain token's holder published into [`LogRing`].
///
/// # The first statement is the flag check — argued, and NOT verifiable at this rung
///
/// With the ECS ring off, this returns after one `Relaxed` load and touches no column and no page
/// of the handoff ring. That is what makes the subsystem's "off costs address space, not resident
/// memory" claim true *below* the emission path, where it is usually argued.
///
/// **MEASURED: deleting the check leaves the L5 gate GREEN.** At this rung the system has one duty
/// — consuming the handoff — and an empty ring is a no-op with or without it. The check is here
/// because L16 adds two duties the system performs on its **own account** (the `TARGET_STATS`
/// snapshot and the per-frame `frame_epoch` record), and those would materialize the columns on
/// frame 1 in a process that never enabled logging. Writing it now means the hole is not left for
/// a later rung to fall into; saying so here means nobody reads it as tested. The L16 obligation
/// is to delete the check and confirm `tests/log_seam.rs`'s flag-off assertion reds.
pub fn log_drain_system(mut ring: ResMut<LogRing>, mut stats: ResMut<LogStats>) {
    if !boyko_log::lifecycle::ecs_ring_enabled() {
        return;
    }

    let ring = &mut *ring;
    // SAFETY: this system is the handoff ring's single consumer. The scheduler grants exactly one
    //   `ResMut<LogRing>` and refuses any sibling read or write of the same resource, so a second
    //   caller would need a second `&mut LogRing` the conflict analysis will not hand out. That
    //   borrow IS the exclusivity proof the function's contract asks for.
    unsafe {
        ecs::drain_into(|frame| ring.store(&frame));
    }

    // Mirrored, not accumulated: the ring's counter is the single source, and a second running
    // total here would be a number that can disagree with it.
    stats.handoff_lost = ecs::lost().0;
}
