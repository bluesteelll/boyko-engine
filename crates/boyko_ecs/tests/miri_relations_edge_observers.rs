//! Relation-edge / Down-broadcast observers — Miri (Tree Borrows) RE-ENTRANCY
//! coverage (the highest-risk surface of the observer-side relation DSL).
//!
//! THE soundness gate for the W3/W2 fence + the apply-window drain when an edge
//! or broadcast observer ITSELF enqueues structural commands. The fire happens
//! synchronously inside `LinkCommand::apply` / `UnlinkCommand::apply` (already
//! under `&mut EcsMaster` at the apply window); a runner that calls
//! `commands().spawn()/insert()/despawn()` pushes into `deferred_hook_queue`,
//! drained at the OUTERMOST boundary on the NEXT turn. This must stay
//! Tree-Borrows-clean (the `apply_via_raw_twin` / BUG-P19-TB-1 mem::take'd
//! stack-local discipline generalizes to the edge-fire re-entry).
//!
//! Run via (NOTE `-Zmiri-ignore-leaks` — the by-design bounded `BundleColumnCache`
//! `Box::leak`, #53 NOT-A-BUG, is orthogonal to Tree Borrows; matches the sibling
//! Miri suites):
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! rustup run nightly-x86_64-pc-windows-gnu cargo miri test -p boyko-ecs --test miri_relations_edge_observers
//! ```
//!
//! `#![cfg(miri)]` — only compiles under Miri; native runs ignore this file (the
//! `relations_edge_observers` / `relations_broadcast_down` suites cover the same
//! semantics end-to-end natively). Entity counts are kept TINY (Miri is ~100x
//! slower).

#![cfg(miri)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::traversal::{ChildOfTraversal, PropagationMode};
use boyko_ecs::ecs::core::component::observers::trigger::{Trigger, TriggerContext};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::relationship::{OnLink, OnUnlink, Relationship};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Component, Relationship as RelationshipDerive, RelationshipTarget};

const SEQ: Ordering = Ordering::SeqCst;

#[derive(Component, Clone, Copy, RelationshipDerive)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, retain_empty)]
struct LikedBy(Vec<Entity>);

/// The component a re-entrant deferred spawn creates — its own on_add proves the
/// deferred command applied exactly once.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Spawned(u32);

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct MTag(u32);

/// Spawns `n` markers; returns now-live handles (one apply window). Tiny `n`.
fn spawn_entities(ecs: &mut EcsMaster, n: usize) -> Vec<Entity> {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::with_capacity(n)));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut local = probe.lock().expect("probe lock");
        for i in 0..n {
            local.push(cmds.spawn(MTag(i as u32)).id());
        }
    });
    sink.lock().expect("probe lock").clone()
}

// ════════════════════════════════════════════════════════════════════════════
// D.1 — an OnLink observer that re-entrantly enqueues a deferred SPAWN; the
//       outermost drain applies it exactly once. TB-clean (no Drop-cached
//       NonNull<EcsMaster> aliasing the apply window).
// ════════════════════════════════════════════════════════════════════════════

static D1_LINK_FIRES: AtomicUsize = AtomicUsize::new(0);
static D1_SPAWNED_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn d1_on_link(mut w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    D1_LINK_FIRES.fetch_add(1, SEQ);
    // Re-entrantly enqueue a deferred spawn from INSIDE the synchronous edge fire.
    w.commands().spawn(Spawned(7));
}
unsafe fn d1_spawned_add(_w: DeferredEcsMaster<'_>, _c: boyko_ecs::ecs::core::component::observers::ObserverContext) {
    D1_SPAWNED_ADD.fetch_add(1, SEQ);
}

#[test]
fn miri_on_link_observer_reentrant_spawn_applies_once() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Likes>(d1_on_link);
    ecs.observe_on_add::<Spawned>(d1_spawned_add);
    let _ = Spawned::component_id();

    let e = spawn_entities(&mut ecs, 2);
    let (target, source) = (e[0], e[1]);
    D1_LINK_FIRES.store(0, SEQ);
    D1_SPAWNED_ADD.store(0, SEQ);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });

    assert_eq!(D1_LINK_FIRES.load(SEQ), 1, "OnLink fired once");
    assert_eq!(
        D1_SPAWNED_ADD.load(SEQ),
        1,
        "the re-entrant deferred spawn from the OnLink observer applied EXACTLY once",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// D.2 — an OnUnlink observer that re-entrantly enqueues a deferred DESPAWN of a
//       third entity; applied once on the next drain turn. TB-clean.
// ════════════════════════════════════════════════════════════════════════════

static D2_UNLINK_FIRES: AtomicUsize = AtomicUsize::new(0);
static D2_VICTIM: AtomicUsize = AtomicUsize::new(usize::MAX);

unsafe fn d2_on_unlink(mut w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    D2_UNLINK_FIRES.fetch_add(1, SEQ);
    let victim_id = D2_VICTIM.load(SEQ);
    if victim_id != usize::MAX {
        let victim = Entity::new(boyko_ecs::ecs::identifiers::primitives::EntityId(victim_id), 0);
        w.commands().entity(victim).despawn();
    }
}

#[test]
fn miri_on_unlink_observer_reentrant_despawn_applies() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_unlink::<Likes>(d2_on_unlink);

    let e = spawn_entities(&mut ecs, 3);
    let (target, source, victim) = (e[0], e[1], e[2]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });
    D2_UNLINK_FIRES.store(0, SEQ);
    D2_VICTIM.store(victim.id().0, SEQ);

    // Remove the FK → OnUnlink fires → the observer enqueues a despawn of victim.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).remove::<Likes>();
    });

    assert_eq!(D2_UNLINK_FIRES.load(SEQ), 1, "OnUnlink fired once");
    assert!(!ecs.has_entity(victim), "the re-entrant deferred despawn from OnUnlink applied");
    assert!(ecs.has_entity(source), "source survives");
}

// ════════════════════════════════════════════════════════════════════════════
// D.3 — RE-TARGET while an OnLink observer enqueues another relate: the unlink-
//       then-link retarget plus a re-entrant insert all drain soundly. TB-clean.
// ════════════════════════════════════════════════════════════════════════════

static D3_LINK_FIRES: AtomicUsize = AtomicUsize::new(0);
static D3_EXTRA_TARGET: AtomicUsize = AtomicUsize::new(usize::MAX);
static D3_EXTRA_SOURCE: AtomicUsize = AtomicUsize::new(usize::MAX);
static D3_DID_RELATE: AtomicUsize = AtomicUsize::new(0);

unsafe fn d3_on_link(mut w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    D3_LINK_FIRES.fetch_add(1, SEQ);
    // On the FIRST fire only, re-entrantly relate a spare source to a spare target
    // (a NEW edge enqueued from inside the edge fire).
    if D3_DID_RELATE.swap(1, SEQ) == 0 {
        let t = D3_EXTRA_TARGET.load(SEQ);
        let s = D3_EXTRA_SOURCE.load(SEQ);
        if t != usize::MAX && s != usize::MAX {
            let te = Entity::new(boyko_ecs::ecs::identifiers::primitives::EntityId(t), 0);
            let se = Entity::new(boyko_ecs::ecs::identifiers::primitives::EntityId(s), 0);
            w.commands().entity(se).insert(Likes(te));
        }
    }
}

#[test]
fn miri_retarget_during_on_link_observer_with_reentrant_relate() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_link::<Likes>(d3_on_link);

    let e = spawn_entities(&mut ecs, 4);
    let (t1, t2, source, spare) = (e[0], e[1], e[2], e[3]);
    // spare relates to t1 from inside the first OnLink fire.
    D3_EXTRA_TARGET.store(t1.id().0, SEQ);
    D3_EXTRA_SOURCE.store(spare.id().0, SEQ);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(t1));
    });
    D3_LINK_FIRES.store(0, SEQ);
    D3_DID_RELATE.store(1, SEQ); // suppress the re-entrant relate on the retarget

    // Re-target source: t1 → t2 (unlink old, link new). TB-clean drain.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(t2));
    });

    assert_eq!(
        ecs.get_component::<Likes>(source).map(|r| r.target()),
        Some(t2),
        "source re-targeted to t2 soundly",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// D.4 — SOURCE-DESPAWN firing OnUnlink whose observer enqueues a spawn: the
//       source's teardown drives the unlink fire; the re-entrant command drains.
// ════════════════════════════════════════════════════════════════════════════

static D4_UNLINK_FIRES: AtomicUsize = AtomicUsize::new(0);
static D4_SPAWNED_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn d4_on_unlink(mut w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    D4_UNLINK_FIRES.fetch_add(1, SEQ);
    w.commands().spawn(Spawned(9));
}
unsafe fn d4_spawned_add(_w: DeferredEcsMaster<'_>, _c: boyko_ecs::ecs::core::component::observers::ObserverContext) {
    D4_SPAWNED_ADD.fetch_add(1, SEQ);
}

#[test]
fn miri_source_despawn_on_unlink_observer_reentrant_spawn() {
    let mut ecs = EcsMaster::new();
    ecs.observe_on_unlink::<Likes>(d4_on_unlink);
    ecs.observe_on_add::<Spawned>(d4_spawned_add);
    let _ = Spawned::component_id();

    let e = spawn_entities(&mut ecs, 2);
    let (target, source) = (e[0], e[1]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).insert(Likes(target));
    });
    D4_UNLINK_FIRES.store(0, SEQ);
    D4_SPAWNED_ADD.store(0, SEQ);

    // Despawn the source. Its teardown removes Likes → OnUnlink fires (target
    // alive) → the observer enqueues a deferred spawn.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(source).despawn();
    });

    assert_eq!(
        D4_UNLINK_FIRES.load(SEQ),
        1,
        "the global OnUnlink fired for the edge destroyed by the source despawn (target alive)",
    );
    assert_eq!(
        D4_SPAWNED_ADD.load(SEQ),
        1,
        "the re-entrant spawn from the source-despawn OnUnlink observer applied once",
    );
    assert!(!ecs.has_entity(source), "source despawned");
}

// ════════════════════════════════════════════════════════════════════════════
// D.5 — a Down broadcast observer that re-entrantly enqueues a deferred spawn
//       at each visited node; all drain at the outermost boundary. TB-clean.
// ════════════════════════════════════════════════════════════════════════════

struct DownReentrant;
impl Trigger for DownReentrant {
    const PROPAGATION: PropagationMode = PropagationMode::Down;
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}

static D5_FIRES: AtomicUsize = AtomicUsize::new(0);
static D5_SPAWNED_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn d5_node(mut w: DeferredEcsMaster<'_>, _c: TriggerContext, _ev: *const u8) {
    D5_FIRES.fetch_add(1, SEQ);
    // Re-entrant deferred spawn at each broadcast node.
    w.commands().spawn(Spawned(5));
}
unsafe fn d5_spawned_add(_w: DeferredEcsMaster<'_>, _c: boyko_ecs::ecs::core::component::observers::ObserverContext) {
    D5_SPAWNED_ADD.fetch_add(1, SEQ);
}

#[test]
fn miri_down_broadcast_observer_reentrant_spawn() {
    let mut ecs = EcsMaster::new();
    ecs.observe::<DownReentrant>(d5_node);
    ecs.observe_on_add::<Spawned>(d5_spawned_add);
    let _ = Spawned::component_id();

    // root → c1, root → c2 (ChildOf). Down broadcast hits root + 2 = 3 nodes.
    let e = spawn_entities(&mut ecs, 3);
    let (root, c1, c2) = (e[0], e[1], e[2]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(root).add_child(c1);
        cmds.entity(root).add_child(c2);
    });
    D5_FIRES.store(0, SEQ);
    D5_SPAWNED_ADD.store(0, SEQ);

    ecs.trigger::<DownReentrant>(root, DownReentrant);
    // The broadcast walk runs SYNCHRONOUSLY; the re-entrant spawns are enqueued
    // into the deferred queue and must drain after. A direct `trigger` is not
    // inside an apply window, so we drain explicitly via a no-op system.
    ecs.run_system(move |mut _cmds: Commands| {});

    assert_eq!(D5_FIRES.load(SEQ), 3, "Down broadcast fired at root + 2 children (3 nodes)");
    assert_eq!(
        D5_SPAWNED_ADD.load(SEQ),
        3,
        "each broadcast node's re-entrant deferred spawn applied (3 total) — TB-clean drain",
    );

    // Keep these imports/types linked even if a case is feature-gated out.
    let _ = (OnLink::<Likes>::new(root), OnUnlink::<Likes>::new(root));
}
