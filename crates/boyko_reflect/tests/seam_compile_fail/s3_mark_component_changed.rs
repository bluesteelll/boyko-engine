//! **EG0, not-yet-reachable item S3 — `EcsMaster::mark_component_changed`.**
//!
//! `docs/REFLECTION-PLAN-ECS.md` §4: F14 + F15. `get_component_changed_tick` is public — the
//! census calls it — but there is **no by-id change-tick WRITE**, so a table-path `set_field`
//! would be invisible to `Changed<T>`. Any by-id writer (scene apply, replication) has the
//! same hole; the read half being public and the write half not is the asymmetry EG5 cannot
//! close on its own.
//!
//! **Flips to `pass` at EG2.** Path-qualified so the compiler echoes the spelling the census
//! binds against.

use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::EcsMaster;

fn main() {
    let mut ecs = EcsMaster::new();
    let entity = ecs.spawn_empty();
    let id: ComponentId = ecs.register_tag("eg0_s3_probe").component_id();

    let _ = EcsMaster::mark_component_changed(&mut ecs, entity, id);
}
