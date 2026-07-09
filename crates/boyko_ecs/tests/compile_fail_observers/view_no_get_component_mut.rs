// Phase 14b — a runner's `DeferredEcsMaster` view must NOT expose
// `get_component_mut` (a `&mut`-into-pool is `&mut EcsMaster`-only; O1 — it
// would alias the storage the fire walk is reading).
//
// Expected diagnostic: no method named `get_component_mut` found for
// `DeferredEcsMaster`.

use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::ObserverContext;

unsafe fn bad_runner(mut w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    // A &mut-into-pool must be unreachable from a runner.
    let _ = w.get_component_mut(ctx.entity);
}

fn main() {
    let _ = bad_runner as unsafe fn(DeferredEcsMaster<'_>, ObserverContext);
}
