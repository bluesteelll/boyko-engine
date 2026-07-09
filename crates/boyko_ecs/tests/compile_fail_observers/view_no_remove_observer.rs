// Phase 14b — a runner's `DeferredEcsMaster` view must NOT expose
// `remove_observer` (registry mutation is `&mut EcsMaster`-only; O1).
//
// Expected diagnostic: no method named `remove_observer` found for
// `DeferredEcsMaster`.

use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::ObserverContext;

unsafe fn bad_runner(mut w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    // Registry mutation must be unreachable from a runner.
    w.remove_observer();
}

fn main() {
    let _ = bad_runner as unsafe fn(DeferredEcsMaster<'_>, ObserverContext);
}
