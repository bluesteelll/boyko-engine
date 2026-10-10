//! Relations v1 — R4: custom-trigger bubbling along a NON-`ChildOf` relation
//! (`Toward<Likes>`). `docs/RELATIONS-API-PLAN.md` §test-matrix R4 / Decision 5.
//!
//! The generic `Traversal` machinery already drives `trigger_walk` per hop; the
//! `Toward<R>` bridge (`observers/traversal.rs`) lets ANY `Relationship` bubble.
//! These tests prove an `AUTO_PROPAGATE` event with `type Traversal =
//! Toward<Likes>` bubbles UP a `Likes` chain, firing the entity-targeted observer
//! at each hop — and STOPS at a node with no `Likes` (a `None` hop).
//!
//! # Why `static` counters
//!
//! `TriggerFn` is a bare `unsafe fn` pointer — it cannot capture. Each test owns a
//! private module-level `static AtomicUsize` so concurrently-running tests never
//! observe one another's fires (the registries are process-wide in the test
//! binary).
//!
//! Mirrors `feature2_observers_behavioral.rs`'s
//! `bubbling_trigger_walks_up_childof_to_grandparent`, retargeted to `Likes`.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::trigger::{Trigger, TriggerContext};
use boyko_ecs::ecs::core::component::observers::traversal::Toward;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Component, Relationship, RelationshipTarget};

const SEQ: Ordering = Ordering::SeqCst;

/// The relation we bubble along (NOT `ChildOf`).
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, linked_despawn, retain_empty)]
struct LikedBy(Vec<Entity>);

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Tag(u32);

/// An AUTO_PROPAGATE event that bubbles along `Likes` (toward the liked target).
struct LikeBubble;
impl Trigger for LikeBubble {
    const AUTO_PROPAGATE: bool = true;
    type Traversal = Toward<Likes>;
    // `Up` trigger — `Broadcast` is never read; an in-scope `Relationship`.
    type Broadcast = Likes;
}

/// Spawns `n` markers; returns now-live handles (one apply window).
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
        assert!(ecs.has_entity(e), "spawned live after apply");
    }
    out
}

/// Builds a `Likes` chain `a → b → c` (a Likes b, b Likes c), all live. Returns
/// `[a, b, c]`.
fn build_likes_chain(ecs: &mut EcsMaster) -> [Entity; 3] {
    let e = spawn_entities(ecs, 3);
    let (a, b, c) = (e[0], e[1], e[2]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(a).insert(Likes(b));
        cmds.entity(b).insert(Likes(c));
    });
    [a, b, c]
}

// ════════════════════════════════════════════════════════════════════════════
// R4.1 — bubble along Likes: trigger at A bubbles A → B → C (3 hops)
// ════════════════════════════════════════════════════════════════════════════

static R4_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn r4_count(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, _e: *const u8) {
    R4_FIRES.fetch_add(1, SEQ);
}

#[test]
fn bubble_along_likes_fires_at_each_hop() {
    let mut ecs = EcsMaster::new();
    let [a, b, c] = build_likes_chain(&mut ecs);
    R4_FIRES.store(0, SEQ);

    ecs.observe_entity_event::<LikeBubble>(a, r4_count);
    ecs.observe_entity_event::<LikeBubble>(b, r4_count);
    ecs.observe_entity_event::<LikeBubble>(c, r4_count);

    // Trigger at A → bubbles A → B → C along Likes = 3 entity-targeted fires.
    ecs.trigger::<LikeBubble>(a, LikeBubble);

    assert_eq!(
        R4_FIRES.load(SEQ),
        3,
        "an AUTO_PROPAGATE event fired at A bubbles along Likes to B and C (3 hops, NON-ChildOf)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// R4.2 — the walk STOPS at a node without Likes (a None hop)
// ════════════════════════════════════════════════════════════════════════════

static R4B_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn r4b_count(_w: DeferredEcsMaster<'_>, _ctx: TriggerContext, _e: *const u8) {
    R4B_FIRES.fetch_add(1, SEQ);
}

#[test]
fn bubble_stops_on_missing_relation() {
    let mut ecs = EcsMaster::new();
    // a → b (b has NO Likes — the chain ends at b).
    let e = spawn_entities(&mut ecs, 3);
    let (a, b, detached) = (e[0], e[1], e[2]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(a).insert(Likes(b));
    });
    R4B_FIRES.store(0, SEQ);

    // Observe all three; `detached` is never reachable from a's chain.
    ecs.observe_entity_event::<LikeBubble>(a, r4b_count);
    ecs.observe_entity_event::<LikeBubble>(b, r4b_count);
    ecs.observe_entity_event::<LikeBubble>(detached, r4b_count);

    // Trigger at A → A fires, bubbles to B (B has no Likes → None hop → STOP).
    // Detached never fires.
    ecs.trigger::<LikeBubble>(a, LikeBubble);

    assert_eq!(
        R4B_FIRES.load(SEQ),
        2,
        "the bubble fires at A and B then STOPS (B has no Likes — a None hop); \
         the detached node never fires"
    );
}
