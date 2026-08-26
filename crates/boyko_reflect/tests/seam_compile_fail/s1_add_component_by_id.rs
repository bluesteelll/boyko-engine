//! **EG0, not-yet-reachable item S1 — `EcsMaster::add_component_by_id`.**
//!
//! `docs/REFLECTION-PLAN-ECS.md` §4: F9 + F11. The ZST attach helper
//! `debug_assert!`s that every added id is size-0; the only data attach helper is generic
//! over `Bundle` and takes it by value. **Nothing in the tree attaches a data component by
//! id**, which is why `add_default` cannot *"route through the existing structural insert"*
//! as the analysis's §4 claims.
//!
//! This fixture **flips to `pass` at EG2** — it is not a refusal, it is a not-yet. The call
//! is spelled path-qualified on purpose: the item's exact `Type::fn` spelling is then echoed
//! into the diagnostic, and `tests/seam_census.rs` binds the plan's row to *that*, so the
//! binding is the compiler's word and not a filename or a comment.

use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::EcsMaster;

fn main() {
    let mut ecs = EcsMaster::new();
    let entity = ecs.spawn_empty();
    let id: ComponentId = ecs.register_tag("eg0_s1_probe").component_id();
    let bytes: &[u8] = &[];

    let _ = EcsMaster::add_component_by_id(&mut ecs, entity, id, bytes);
}
