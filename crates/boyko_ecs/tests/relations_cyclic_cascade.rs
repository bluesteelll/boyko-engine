//! Relations v1 — R6: cyclic `LINKED_DESPAWN` cascade TERMINATES (the C1-fix
//! gate). `docs/RELATIONS-API-PLAN.md` §test-matrix R6 / §W4 / EC7.
//!
//! A generic relation admits cycles (unlike a tree hierarchy). A `LINKED_DESPAWN`
//! cascade over a cycle must TERMINATE — each cyclic entity is despawned exactly
//! once (a re-entered despawn of an already-dead entity is a generation-checked
//! no-op in `delete_entity_core`), and the call RETURNS (no hang, no unbounded
//! `deferred_hook_queue` growth, no double-free panic). The cross-level
//! `MAX_HOOK_DRAIN_TURNS` backstop in `drain_deferred_hook_queue` bounds a
//! pathological runaway; a real finite cycle terminates naturally.
//!
//! # Cycle construction
//!
//! `Likes` is `LINKED_DESPAWN`. `Likes(T)` on `S` ⇒ `T.LikedBy ∋ S`, and
//! despawning `T` cascade-despawns its `LikedBy` sources. A 2-cycle:
//! `A.Likes(B)` + `B.Likes(A)` ⇒ `A.LikedBy = {B}`, `B.LikedBy = {A}`. Despawning
//! `A` cascades to `B`; `B`'s cascade re-enters `A` (already dead → no-op). An
//! N-ring chains the same way.
//!
//! # Timeout
//!
//! Each test is small + cheap; a non-terminating cascade would HANG the test
//! binary (caught by the harness wall-clock / CI timeout). If a test here hangs,
//! that is a REPORTED C1 regression.

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::relationship::RelationshipTarget;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Component, Relationship, RelationshipTarget};

/// A `LINKED_DESPAWN` relation source (generic — not `ChildOf`).
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

/// The `LINKED_DESPAWN` reverse index.
#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, linked_despawn, retain_empty)]
struct LikedBy(Vec<Entity>);

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Tag(u32);

/// Spawns `n` marker entities; returns now-live handles (one apply window).
fn spawn_entities(ecs: &mut EcsMaster, n: usize) -> Vec<Entity> {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::with_capacity(n)));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut local = probe.lock().expect("probe lock");
        for i in 0..n {
            local.push(cmds.spawn(Tag(i as u32)).id());
        }
    });
    let out = sink.lock().expect("probe lock").clone();
    for &e in &out {
        assert!(ecs.has_entity(e), "spawned entity live after apply");
    }
    out
}

// ════════════════════════════════════════════════════════════════════════════
// R6.1 — a 2-cycle (A↔B) LINKED_DESPAWN cascade terminates; both despawned once
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn cyclic_linked_despawn_two_cycle_terminates() {
    const { assert!(LikedBy::LINKED_DESPAWN) };

    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (a, b) = (e[0], e[1]);

    // Build the cycle: A.Likes(B), B.Likes(A).
    // ⇒ B.LikedBy = {A}, A.LikedBy = {B}.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(a).insert(Likes(b));
        cmds.entity(b).insert(Likes(a));
    });
    assert_eq!(likes_target(&ecs, a), Some(b), "A Likes B");
    assert_eq!(likes_target(&ecs, b), Some(a), "B Likes A");

    // Despawn A. A's cascade despawns A.LikedBy = {B}; B's cascade re-enters A
    // (already dead → generation-checked no-op). MUST terminate (no hang).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(a).despawn();
    });

    assert!(!ecs.has_entity(a), "A despawned");
    assert!(!ecs.has_entity(b), "B cascaded (cyclic cascade reached B)");
    assert_eq!(ecs.entity_count(), 0, "both cyclic entities despawned exactly once (no survivor)");
}

// ════════════════════════════════════════════════════════════════════════════
// R6.2 — a longer ring (8 nodes) LINKED_DESPAWN cascade terminates; all once
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn cyclic_linked_despawn_eight_ring_terminates() {
    let mut ecs = EcsMaster::new();
    const N: usize = 8;
    let nodes = spawn_entities(&mut ecs, N);
    let ring = nodes.clone();

    // Ring: node[i].Likes(node[(i+1) % N]) for all i.
    // ⇒ node[(i+1)%N].LikedBy ∋ node[i]; despawning any node cascades around.
    ecs.run_system(move |mut cmds: Commands| {
        for i in 0..N {
            let next = ring[(i + 1) % N];
            cmds.entity(ring[i]).insert(Likes(next));
        }
    });
    for i in 0..N {
        assert_eq!(
            likes_target(&ecs, nodes[i]),
            Some(nodes[(i + 1) % N]),
            "node {i} Likes node {}",
            (i + 1) % N
        );
    }

    // Despawn one node — the cascade chases the ring and re-enters the origin
    // (dead → no-op). MUST terminate.
    let node0 = nodes[0];
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(node0).despawn();
    });

    for (i, &node) in nodes.iter().enumerate() {
        assert!(!ecs.has_entity(node), "ring node {i} despawned (cascade chased the ring)");
    }
    assert_eq!(ecs.entity_count(), 0, "the whole ring is despawned exactly once (terminated)");
}

// ════════════════════════════════════════════════════════════════════════════
// R6.3 — a self-loop built by RAW insert is rejected by the self-ref guard, so
//        a despawn of it is a plain (non-cyclic) despawn that terminates
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn self_like_is_guarded_then_despawn_terminates() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 1);
    let me = e[0];

    // Likes(self) is reactively removed by the on_insert self-ref guard, so no
    // self-cycle is ever formed.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(me).insert(Likes(me));
    });
    assert_eq!(likes_target(&ecs, me), None, "self-Likes rejected (no self-cycle)");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(me).despawn();
    });
    assert!(!ecs.has_entity(me), "the entity despawns (no self-cascade loop)");
    assert_eq!(ecs.entity_count(), 0, "terminated cleanly");
}

/// Reads `Likes`'s target FK, if present.
fn likes_target(ecs: &EcsMaster, s: Entity) -> Option<Entity> {
    use boyko_ecs::ecs::core::relationship::Relationship;
    ecs.get_component::<Likes>(s).map(|r| r.target())
}
