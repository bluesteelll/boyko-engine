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
    let id: ComponentId = ecs.register_tag("eg0_s3_probe").component_id();

    // The absent arm: the id is `Table`-kind, and the EMPTY archetype owns no pool for it,
    // so the table arm's `pools.get_pool(id)` resolves to `None` and nothing is stamped.
    assert!(
        !EcsMaster::mark_component_changed(&mut ecs, entity, id),
        "S3 landed: stamping an id the entity does not host answers `false` rather than \
         writing a tick into a pool that is not there"
    );
}
