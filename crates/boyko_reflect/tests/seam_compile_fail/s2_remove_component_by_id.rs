//! **EG0, not-yet-reachable item S2 — `EcsMaster::remove_component_by_id`.**
//!
//! `docs/REFLECTION-PLAN-ECS.md` §4: F10 + F12. The data-general detach helper exists and is
//! correct — it collects retained bytes, fires `on_replace` / `on_remove` on the dying row,
//! and runs `drop_fn` exactly once per removed id — and it is `pub(crate)`. The public twin
//! that exists, `remove_tag`, carries a ZST restriction the helper never actually needed.
//!
//! **Flips to `pass` at EG2.** Path-qualified so the compiler echoes the spelling the census
//! binds against.

use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::EcsMaster;

fn main() {
    let mut ecs = EcsMaster::new();
    let entity = ecs.spawn_empty();
    let id: ComponentId = ecs.register_tag("eg0_s2_probe").component_id();

    let _ = EcsMaster::remove_component_by_id(&mut ecs, entity, id);
}
