//! [`ProfilerPlugin`] — the registration, and the world bind that makes "one profiler, one world"
//! enforced rather than assumed.

use crate::ecs::core::app::{App, Plugin};
use crate::ecs::core::profiling::diag;
use crate::ecs::core::profiling::store::{Profiler, bind_world};

/// Registers the profiling store and binds the process-global profiler to this world.
///
/// # What `build` deliberately does NOT do
///
/// It does not arm. No reservation, no commit, no calibration, no publication — `build` runs
/// before a host has read its launch flag, and a diagnostics subsystem may not make a syscall the
/// flag has not authorised. [`Profiler::arm`] is the enable path and the host calls it, because
/// the host is what parses the flag; a plugin that armed by being added would make "add the
/// plugin" and "turn profiling on" the same act.
///
/// # There is no system here
///
/// The fold is **not** a system. It runs at the top of `App::update_with_delta`, which is the
/// single funnel both frame entry points share and the only place that is *outside* the schedules
/// it measures. A system would be inside one of them, so the instrument would be inside its own
/// primary number.
#[derive(Default)]
pub struct ProfilerPlugin;

impl Plugin for ProfilerPlugin {
    fn build(&self, app: &mut App) {
        let id = app.world().world_id().get();
        if let Err(live) = bind_world(id) {
            // Refused, not panicked, and the resource is NOT inserted: the lane rings and the
            // reservation are process-global while worlds are not, so a second world folding the
            // same rings would take half the first world's samples and neither would say so. A
            // world without the resource simply has no profiler, which is a state the fold call
            // site already handles.
            diag::report_second_world(live, id);
            return;
        }
        app.insert_resource(Profiler::new());
    }

    fn name(&self) -> &'static str {
        "boyko_ecs::ProfilerPlugin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::profiling::store::{UNBOUND, bound_world, test_serial, unbind_world};

    /// `BOUND_WORLD` is process-global. The lock is the store module's — ONE lock over every
    /// global this module owns, so a bind test and an arm test cannot interleave either.
    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        let g = test_serial();
        unbind_world();
        g
    }

    /// The first world binds and gets the store.
    #[test]
    fn the_first_world_binds_and_receives_the_store() {
        let _g = exclusive();
        let mut app = App::new();
        app.add_plugin(ProfilerPlugin);
        let id = app.world().world_id().get();
        assert_eq!(bound_world(), id);
        assert!(app.world().contains_resource::<Profiler>());
    }

    /// A second world is **refused**, and refused by not receiving the store at all.
    ///
    /// The rings and the reservation are process-global while worlds are not, so two worlds
    /// folding the same rings would each take half the other's samples and neither would say so.
    /// Handing the second world a store and hoping it never folds would be exactly that failure
    /// with a comment attached.
    #[test]
    fn a_second_world_is_refused_with_e9204_and_gets_no_store() {
        let _g = exclusive();
        let mut first = App::new();
        first.add_plugin(ProfilerPlugin);
        let live = first.world().world_id().get();

        let mut second = App::new();
        second.add_plugin(ProfilerPlugin);
        assert_eq!(bound_world(), live, "the second world stole the binding");
        assert!(
            !second.world().contains_resource::<Profiler>(),
            "a refused world was handed a store it must never fold with"
        );
        // `>= 1`: `E9204` is `Once` and another test in this process may have claimed it first.
        assert!(diag::report_count(boyko_log::codes::E9204.number()) >= 1, "the refusal was silent");
    }

    /// The same world twice is a host's duplicate plugin registration, not a second world.
    #[test]
    fn binding_the_same_world_twice_succeeds() {
        let _g = exclusive();
        let mut app = App::new();
        app.add_plugin(ProfilerPlugin);
        let id = app.world().world_id().get();
        assert!(bind_world(id).is_ok(), "one world cannot collide with itself");
        assert_eq!(bound_world(), id);
    }

    /// The unbound sentinel is not a world id anything can mint, so "no world" cannot be confused
    /// with "world `u64::MAX`".
    #[test]
    fn the_unbound_sentinel_is_distinct_from_every_world() {
        let _g = exclusive();
        assert_eq!(bound_world(), UNBOUND);
        let app = App::new();
        assert_ne!(app.world().world_id().get(), UNBOUND);
    }
}
