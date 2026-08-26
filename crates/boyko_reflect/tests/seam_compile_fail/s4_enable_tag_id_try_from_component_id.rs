//! **EG0, not-yet-reachable item S4′ — `EnableTagId::try_from_component_id`.**
//!
//! `docs/REFLECTION-PLAN-ECS.md` §4: F16 + **F27**. `is_enabled_id` / `enable_id` /
//! `disable_id` are all public and all take an `EnableTagId` — `tests/seam_census.rs` pins
//! their signatures. What has **no constructor** is the reverse direction: an `EnableTagId`
//! is *"a proof that the id was minted as a bitset enable tag"*, and only the mint path can
//! issue one. The by-name re-mint route is not a substitute: on a *derived*
//! `#[component(storage = "bitset")]` id, `register_enable_tag(name)` mints a **different**
//! tag and toggles its bit while the original's stays set — returning `Ok(())`. Both halves,
//! the `&self` read and the `&mut self` write, therefore go through this item.
//!
//! **Flips to `pass` at EG2.** This fixture is the reason the item was moved from the
//! negative list to the positive one on 2026-08-21: it was landed to assert *"the item reds
//! the moment it becomes reachable"* — a good instrument pointed at the wrong item, which
//! would have fired at EG2 as a success signal misread as a regression.
//!
//! ⚠️ The path is `component_registry::EnableTagId`, **not** `component_registry::tags::…`:
//! `mod tags` is private and reachable only through the parent's `pub use tags::*`. The plan
//! says S4′ *"lands in `component_registry::tags`"*, which is true of the **source file** and
//! false of the **public path**.

use boyko_ecs::ecs::core::component::component_registry::EnableTagId;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::EcsMaster;

fn main() {
    let mut ecs = EcsMaster::new();
    let id: ComponentId = ecs.register_tag("eg0_s4_probe").component_id();

    let _ = EnableTagId::try_from_component_id(id);
}
