//! **EG0, the one item this plan REFUSES to add — `TagId::from_component_id`.**
//!
//! This fixture is the *other kind*. The four beside it flip to `pass` at EG2; this one must
//! stay red **forever**, and the two-kind split is load-bearing: a single undifferentiated
//! "negative list" is what let one item sit on it while a sibling plan declared the same item
//! mandatory.
//!
//! `docs/REFLECTION-PLAN-ECS.md` §4, *"What the seam does NOT include"*: a dynamic tag needs
//! no reverse constructor at all. Its id *came from* `try_register_tag_by_name`, so its name
//! **is** the key in `TAG_NAMES` (F20 + F27's second half) and `tag_by_name` — public, and
//! called by `tests/seam_census.rs` — resolves it without minting. The whole dynamic-tag
//! presence write path is `display_name(id)` → `tag_by_name` → `add_tag` / `remove_tag`, with
//! **zero** additions to `boyko_ecs`.
//!
//! If this fixture ever turns green, the seam grew an item nobody argued for.

use boyko_ecs::ecs::core::component::component_registry::TagId;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::EcsMaster;

fn main() {
    let mut ecs = EcsMaster::new();
    let id: ComponentId = ecs.register_tag("eg0_neg_probe").component_id();

    let _ = TagId::from_component_id(id);
}
