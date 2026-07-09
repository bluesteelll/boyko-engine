//! Feature 2 — Miri (Tree Borrows) coverage for the NEW observer unsafe paths.
//! Single-thread only (multi-thread Miri deferred per Phase 9.1).
//!
//! Run via:
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows"
//! cargo +nightly miri test -p boyko-ecs --test miri_feature2_observers
//! ```
//!
//! Per the 14a/14b lesson — Miri-TB caught soundness bugs that critic +
//! code-review APPROVED — **Miri-TB is the authoritative soundness oracle** for
//! the Feature-2 raw-pointer plumbing. The targets (plan "Soundness" section):
//!
//! 1. **The entity-observer fire loop** (OBS-FIRE-LOOP / F2). The per-entity
//!    store reconstructs an `ObserverFn`/`TriggerFn` from a stored `usize` and
//!    transmutes it back to a fn-ptr before the call; no `world`-derived `&`
//!    (incl. the store / registry `&`) may span the `DeferredEcsMaster` view
//!    mint or the runner call.
//! 2. **The custom-trigger walk + propagation** — `trigger` / `trigger_global`
//!    mint a `NonNull<EcsMaster>` per turn, fire global then entity triggers,
//!    then re-derive `ChildOf` per hop through a fresh read-only view.
//! 3. **The on_despawn fire + the parent-first cascade** — `fire_despawn_hooks`
//!    fires Despawn observers BEFORE drop, and the cascade re-enters the drain.
//! 4. **The sticky-bit raise through the raw archetype ptr** —
//!    `raise_entity_observer_bit` does `(*archetype_ptr).flags.insert(..)`
//!    through a `*mut Archetype` while the entity_master borrow is released.
//! 5. **Re-entrancy** — an observer enqueues a deferred command; the outermost
//!    drain applies it (no cached `NonNull<EcsMaster>` written in a Drop).
//!
//! # File gate
//!
//! `#![cfg(miri)]` — only compiles under Miri. Native runs ignore the file; the
//! `feature2_observers_*` integration suites cover the same semantics
//! end-to-end on the native target. Entity counts are kept tiny (Miri is slow).

#![cfg(miri)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::trigger::{Trigger, TriggerContext};
use boyko_ecs::ecs::core::component::observers::propagate::propagate;
use boyko_ecs::ecs::core::component::observers::traversal::ChildOfTraversal;
use boyko_ecs::ecs::core::component::observers::{ObserverContext, ObserverKind};
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// Target 1 + 4 — entity-observer fire loop (the fn-ptr transmute) + the sticky
//                bit raise through the raw archetype ptr, exercised over the
//                migration path (add/insert) and the in-place replace path.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct MA(u32);
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct MB(u32);
#[derive(Bundle)]
struct MBBundle {
    b: MB,
}

static MT1_ADD: AtomicUsize = AtomicUsize::new(0);
static MT1_INSERT: AtomicUsize = AtomicUsize::new(0);
static MT1_REPLACE: AtomicUsize = AtomicUsize::new(0);
static MT1_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn mt1_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    MT1_ADD.fetch_add(1, SEQ);
}
unsafe fn mt1_insert(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    MT1_INSERT.fetch_add(1, SEQ);
}
unsafe fn mt1_replace(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    MT1_REPLACE.fetch_add(1, SEQ);
}
unsafe fn mt1_remove(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    MT1_REMOVE.fetch_add(1, SEQ);
}

#[test]
fn miri_entity_observer_fire_loop_and_sticky_raise() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[MA::component_id()]);
    let e = ecs.spawn_one(arch, MA(1)).expect("spawn");
    let _ = MB::component_id();

    // raise_entity_observer_bit through the raw *mut Archetype.
    ecs.observe_entity(e, ObserverKind::Add, MB::component_id(), mt1_add);
    ecs.observe_entity(e, ObserverKind::Insert, MB::component_id(), mt1_insert);
    ecs.observe_entity(e, ObserverKind::Replace, MB::component_id(), mt1_replace);
    ecs.observe_entity(e, ObserverKind::Remove, MB::component_id(), mt1_remove);

    // Add via migration -> fires the fn-ptr-transmuted add + insert.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(MBBundle { b: MB(10) });
    });
    assert_eq!(MT1_ADD.load(SEQ), 1, "entity on_add fired through the transmuted fn-ptr");
    assert_eq!(MT1_INSERT.load(SEQ), 1, "entity on_insert fired");

    // In-place replace -> fires replace (old) + insert (new).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(MBBundle { b: MB(20) });
    });
    assert_eq!(MT1_REPLACE.load(SEQ), 1, "entity on_replace fired");

    // Remove via migration -> fires remove.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove::<MB>();
    });
    assert_eq!(MT1_REMOVE.load(SEQ), 1, "entity on_remove fired");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 2 — custom-trigger walk + propagation (NonNull<EcsMaster> per turn,
//            ChildOf re-derived per hop through a fresh read-only view).
// ════════════════════════════════════════════════════════════════════════════

struct MTrigger {
    amount: u32,
}
impl Trigger for MTrigger {
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}

struct MBubble;
impl Trigger for MBubble {
    const AUTO_PROPAGATE: bool = true;
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct MC(u32);
#[derive(Bundle)]
struct MCBundle {
    c: MC,
}

static MT2_GLOBAL: AtomicUsize = AtomicUsize::new(0);
static MT2_ENTITY: AtomicUsize = AtomicUsize::new(0);
static MT2_BUBBLE: AtomicUsize = AtomicUsize::new(0);

unsafe fn mt2_global(_w: DeferredEcsMaster<'_>, _c: TriggerContext, event: *const u8) {
    let ev = unsafe { &*(event as *const MTrigger) };
    MT2_GLOBAL.fetch_add(ev.amount as usize, SEQ);
}
unsafe fn mt2_entity(_w: DeferredEcsMaster<'_>, _c: TriggerContext, event: *const u8) {
    let ev = unsafe { &*(event as *const MTrigger) };
    MT2_ENTITY.fetch_add(ev.amount as usize, SEQ);
}
unsafe fn mt2_bubble(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _e: *const u8) {
    MT2_BUBBLE.fetch_add(1, SEQ);
}

#[test]
fn miri_custom_trigger_walk_and_propagation() {
    let mut ecs = EcsMaster::new();

    // 2a — global + entity-targeted single-target fire (no bubble).
    let arch = ecs.create_archetype(&[MC::component_id()]);
    let e = ecs.spawn_one(arch, MC(1)).expect("spawn");
    ecs.observe::<MTrigger>(mt2_global);
    ecs.observe_entity_event::<MTrigger>(e, mt2_entity);
    ecs.trigger::<MTrigger>(e, MTrigger { amount: 3 });
    assert_eq!(MT2_GLOBAL.load(SEQ), 3, "global trigger fired, read payload");
    assert_eq!(MT2_ENTITY.load(SEQ), 3, "entity trigger fired, read payload");
    ecs.trigger_global::<MTrigger>(MTrigger { amount: 2 });
    assert_eq!(MT2_GLOBAL.load(SEQ), 5, "trigger_global ran the global observer only");
    assert_eq!(MT2_ENTITY.load(SEQ), 3, "trigger_global did not re-fire the entity observer");

    // 2b — bubble walk: child -> parent (the per-hop ChildOf re-derive).
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut s = probe.lock().expect("lock");
        s.push(cmds.spawn(MCBundle { c: MC(0) }).id()); // parent
        s.push(cmds.spawn(MCBundle { c: MC(1) }).id()); // child
    });
    let ents = sink.lock().expect("lock").clone();
    let (parent, child) = (ents[0], ents[1]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });
    ecs.observe_entity_event::<MBubble>(child, mt2_bubble);
    ecs.observe_entity_event::<MBubble>(parent, mt2_bubble);
    ecs.trigger::<MBubble>(child, MBubble);
    assert_eq!(MT2_BUBBLE.load(SEQ), 2, "bubble walked child -> parent (per-hop re-derive)");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 3 — on_despawn fire + parent-first cascade (pre-drop intact read +
//            re-entrant drain).
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct MNode(u32);
#[derive(Bundle)]
struct MNodeBundle {
    n: MNode,
}

static MT3_FIRES: AtomicUsize = AtomicUsize::new(0);
static MT3_SUM: AtomicUsize = AtomicUsize::new(0);

unsafe fn mt3_on_despawn(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    // Read the still-intact value through the view (pre-drop).
    if let Some(node) = w.get_component::<MNode>(ctx.entity) {
        MT3_SUM.fetch_add(node.0 as usize, SEQ);
    }
    MT3_FIRES.fetch_add(1, SEQ);
}

#[test]
fn miri_on_despawn_fire_and_cascade() {
    let mut ecs = EcsMaster::new();
    ecs.register_component_hooks::<MNode>().on_despawn(mt3_on_despawn).finish();

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut s = probe.lock().expect("lock");
        s.push(cmds.spawn(MNodeBundle { n: MNode(1) }).id()); // root
        s.push(cmds.spawn(MNodeBundle { n: MNode(2) }).id()); // mid
        s.push(cmds.spawn(MNodeBundle { n: MNode(4) }).id()); // leaf
    });
    let ents = sink.lock().expect("lock").clone();
    let (root, mid, leaf) = (ents[0], ents[1], ents[2]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(root).add_child(mid);
        cmds.entity(mid).add_child(leaf);
    });

    assert!(ecs.delete_entity(root), "cascade despawn");
    assert_eq!(MT3_FIRES.load(SEQ), 3, "on_despawn fired once per subtree entity");
    assert_eq!(MT3_SUM.load(SEQ), 1 + 2 + 4, "each handler read its intact pre-drop value");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 5 — re-entrancy: an observer enqueues a deferred spawn; the outermost
//            drain applies it exactly once (no Drop-cached NonNull<EcsMaster>).
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct MTrig(u32);
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct MSpawned(u32);
#[derive(Bundle)]
struct MSpawnedBundle {
    s: MSpawned,
}

static MT5_SPAWNED_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn mt5_on_remove_spawns(mut w: DeferredEcsMaster<'_>, _c: HookContext) {
    w.commands().spawn(MSpawnedBundle { s: MSpawned(7) });
}
unsafe fn mt5_spawned_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    MT5_SPAWNED_ADD.fetch_add(1, SEQ);
}

#[test]
fn miri_reentrant_deferred_spawn_from_observer() {
    let mut ecs = EcsMaster::new();
    ecs.register_component_hooks::<MTrig>().on_remove(mt5_on_remove_spawns).finish();
    ecs.observe_on_add::<MSpawned>(mt5_spawned_add);
    let _ = MSpawned::component_id();

    let arch = ecs.create_archetype(&[MTrig::component_id()]);
    let e = ecs.spawn_one(arch, MTrig(1)).expect("spawn");
    assert!(ecs.delete_entity(e), "despawn re-entrantly enqueues a spawn");
    assert_eq!(MT5_SPAWNED_ADD.load(SEQ), 1, "the deferred spawn applied once at the outermost drain");

    // Keep `propagate` linked (used by the bubble target above; this is a
    // no-op touch so the import is not flagged unused if the bubble test is
    // ever feature-gated out).
    propagate(false);
}
