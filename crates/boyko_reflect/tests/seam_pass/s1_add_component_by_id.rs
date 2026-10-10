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
//!
//! **Flipped from `compile_fail` to `pass` at EG2 (gate 11).** The item landed, so this
//! fixture's blessed `.stderr` was **deleted**, never re-blessed: *"Expected test case to
//! fail to compile, but it succeeded"* is a FLIP, and a flip has no error output to bless.
//! A `t.pass()` case is compiled **and RUN**, so the body below asserts the return value --
//! the claim a bare compile check could not make.

use boyko_ecs::ecs::core::ecs_master::AddOutcome;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::EcsMaster;

fn main() {
    let mut ecs = EcsMaster::new();
    let entity = ecs.spawn_empty();
    let id: ComponentId = ecs.register_tag("eg0_s1_probe").component_id();
    let bytes: &[u8] = &[];

    // Every refusal arm is MISSED here, and each for a reason worth naming: a dynamic
    // tag is `StorageKind::Table` with `size == 0`, so `bytes.len() == 0 == layout.size()`;
    // its residency is `Cpu`; and `spawn_empty` leaves the entity in the EMPTY archetype,
    // which hosts no id at all. So `Attached` is the one outcome S1 can produce here.
    assert_eq!(
        EcsMaster::add_component_by_id(&mut ecs, entity, id, bytes),
        AddOutcome::Attached,
        "S1 landed: a size-0 table attach onto an entity that does not host the id is \
         `Attached`"
    );
}
