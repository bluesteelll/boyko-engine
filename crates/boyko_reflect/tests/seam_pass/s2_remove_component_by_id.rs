//! **EG0, not-yet-reachable item S2 — `EcsMaster::remove_component_by_id`.**
//!
//! `docs/REFLECTION-PLAN-ECS.md` §4: F10 + F12. The data-general detach helper exists and is
//! correct — it collects retained bytes, fires `on_replace` / `on_remove` on the dying row,
//! and runs `drop_fn` exactly once per removed id — and it is `pub(crate)`. The public twin
//! that exists, `remove_tag`, carries a ZST restriction the helper never actually needed.
//!
//! **Flips to `pass` at EG2.** Path-qualified so the compiler echoes the spelling the census
//! binds against.
//!
//! **Flipped from `compile_fail` to `pass` at EG2 (gate 11).** The item landed, so this
//! fixture's blessed `.stderr` was **deleted**, never re-blessed: *"Expected test case to
//! fail to compile, but it succeeded"* is a FLIP, and a flip has no error output to bless.
//! A `t.pass()` case is compiled **and RUN**, so the body below asserts the return value --
//! the claim a bare compile check could not make.

use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::EcsMaster;

fn main() {
    let mut ecs = EcsMaster::new();
    let entity = ecs.spawn_empty();
    let id: ComponentId = ecs.register_tag("eg0_s2_probe").component_id();

    // The entity is in the EMPTY archetype and hosts nothing, so this is the ABSENT arm.
    // S2 carries no reason channel by design: a bare `bool`, exactly like `remove_tag`.
    assert!(
        !EcsMaster::remove_component_by_id(&mut ecs, entity, id),
        "S2 landed: detaching an id the entity does not host answers `false` -- the by-id \
         twin of `remove_tag`'s silent no-op"
    );
}
