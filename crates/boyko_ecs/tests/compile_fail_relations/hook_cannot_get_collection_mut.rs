// Relations v1 (R5 / W2): the STRUCTURAL cascade-soundness guard. A hook body's
// only world handle is `DeferredEcsMaster`, which exposes NO `&mut`-into-storage
// method (`get_component` returns `&T` only). A hook therefore cannot obtain a
// `&mut`-into a `RelationshipTarget` collection — the `*_risky` mutators require
// `&mut Self`, reachable ONLY inside a `Command::apply` under `&mut EcsMaster`.
// This turns the structural argument into a regression-gated FACT.
//
// Expected diagnostic: no method named `get_component_mut` found for
// `DeferredEcsMaster` (the missing capability IS the soundness guarantee).

use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::hooks::HookContext;

// A hook body attempting to reach a &mut-into a RelationshipTarget collection.
unsafe fn bad_hook(mut w: DeferredEcsMaster<'_>, ctx: HookContext) {
    // `get_component_mut` does NOT exist on the read-only view — a hook can never
    // construct an aliasing &mut into a reverse-index collection.
    let _ = w.get_component_mut(ctx.entity);
}

fn main() {
    let _ = bad_hook as unsafe fn(DeferredEcsMaster<'_>, HookContext);
}
