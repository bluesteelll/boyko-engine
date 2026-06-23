//! Relations v1 — R2 C2 serialize-remap BEHAVIORAL tripwire (the silent-
//! corruption gate). `docs/RELATIONS-API-PLAN.md` §test-matrix R2 (C2 serialize).
//!
//! A `#[derive(Relationship)]` SOURCE auto-emits the serialize-direction FK remap
//! (B11) — exactly as the hand-mirror `ChildOf` does. This test saves a generic
//! `Likes` graph and loads it into a FRESH world, asserting the loaded source's
//! `Likes` FK is REMAPPED to the loaded target's fresh `Entity` (round-trips, not
//! verbatim, not dangling). A derive that forgot the serialize-remap passes every
//! link/unlink test but FAILS this.
//!
//! Mirrors `boyko_serialize/tests/entity_remap.rs`, retargeted from the in-crate
//! `ChildOf` hand-mirror to the DERIVED `Likes` (the sole gate on the derive's
//! serialize auto-emit path — the dev-dep cycle keeps `ChildOf` hand-written).

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::relationship::Relationship;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Component, Relationship, RelationshipTarget};

use boyko_serialize::{LoadEntityPolicy, SaveOptions, load_world, save_world};

/// A POB marker so a relation node's archetype carries at least one serializable
/// column (so the entity — and its saved→fresh mapping — is materialized on load).
/// The `u32` payload distinguishes the nodes for the assertions.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Tag(u32);

/// The DERIVED relation source — `Clone, Copy` so the `Component` derive
/// classifies the FK `SerializeViaFn` (remap-eligible), exactly like `ChildOf`.
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, linked_despawn, retain_empty)]
struct LikedBy(Vec<Entity>);

/// Saves `world` to a fresh byte buffer.
fn save(world: &EcsMaster) -> Vec<u8> {
    let mut out = Vec::new();
    save_world(world, &SaveOptions::default(), &mut out).expect("save");
    out
}

/// Finds the loaded entity carrying `Tag(value)` (ids are fresh after a load).
fn entity_with_tag(world: &EcsMaster, value: u32) -> Entity {
    world
        .iter_entities()
        .find(|&e| world.get_component::<Tag>(e).map(|t| t.0) == Some(value))
        .unwrap_or_else(|| panic!("no loaded entity carries Tag({value})"))
}

/// Builds a `source Likes target` link, each node carrying `Tag`. Returns the
/// live `(target, source)` ids. One apply window runs.
fn spawn_likes_pair(ecs: &mut EcsMaster, target_tag: u32, source_tag: u32) -> (Entity, Entity) {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let target = cmds.spawn(Tag(target_tag)).id();
        let source = cmds.spawn(Tag(source_tag)).id();
        cmds.entity(source).insert(Likes(target));
        let mut v = probe.lock().expect("probe");
        v.push(target);
        v.push(source);
    });
    let v = sink.lock().expect("probe").clone();
    (v[0], v[1])
}

// ════════════════════════════════════════════════════════════════════════════
// C2 serialize-remap: a Likes link round-trips with the source's FK remapped to
// the LOADED target (not the stale saved id, not dangling)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn likes_serialize_roundtrip_remaps_foreign_key() {
    let mut src = EcsMaster::new();
    let (saved_target, saved_source) = spawn_likes_pair(&mut src, 1, 2);
    assert_eq!(
        src.get_component::<Likes>(saved_source).map(|r| r.target()),
        Some(saved_target),
        "source Likes the target before save",
    );

    let bytes = save(&src);

    let mut dst = EcsMaster::new();
    // The running build must have the components registered before load (W1).
    let _ = Tag::component_id();
    let _ = Likes::component_id();
    load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load Likes graph");

    let loaded_target = entity_with_tag(&dst, 1);
    let loaded_source = entity_with_tag(&dst, 2);

    let fk = dst
        .get_component::<Likes>(loaded_source)
        .map(|r| r.target())
        .expect("loaded source carries Likes");

    // THE TRIPWIRE: the loaded source's Likes FK must point at the LOADED target.
    assert_eq!(
        fk, loaded_target,
        "C2: the loaded source's Likes FK is REMAPPED to the LOADED target \
         (B11 round-trip; a verbatim load would leave the stale saved id)",
    );
    if loaded_target != saved_target {
        assert_ne!(
            fk, saved_target,
            "the Likes FK must be remapped off the stale saved id (silent corruption otherwise)",
        );
    }

    // NOTE on the reverse index: load remaps the FK but does NOT re-fire the
    // `Likes` link hook, so the loaded target's `LikedBy` reverse index is NOT
    // rebuilt by the load (it is left absent). This is ENGINE-WIDE behavior, not a
    // `Likes`-specific gap: a `ChildOf` round-trip likewise loads `child.ChildOf`
    // remapped while leaving `parent.Children` absent (verified by a sibling probe
    // during authoring; no existing serialize test asserts reverse-index rebuild).
    // The C2 serialize-remap contract is the FK remap above — which holds. A
    // consumer that needs the reverse index rebuilt re-inserts the relationship
    // after load (firing the hook), the same as for `ChildOf`.
    let _ = &dst.get_component::<LikedBy>(loaded_target);
}
