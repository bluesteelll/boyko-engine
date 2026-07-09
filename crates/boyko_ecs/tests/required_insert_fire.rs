//! Feature 1 (required components) — the C1 HEADLINE fire-site tests for the
//! INSERT path, distinct from the spawn path.
//!
//! Spec `docs/REQUIRED-COMPONENTS-PLAN.md` FIX C1/X1: `migrate_entity_insert`
//! historically fired only over the bundle id set, so a required component
//! auto-inserted by the constructor pass would fire NEITHER on_add NOR on_insert
//! on the insert path (the Phase-14b "undercounting fire sites" class —
//! `#[require(B)]` where B has on_add works on spawn but silently breaks on
//! insert). The C1 fix pushes each constructed (absent-in-source) required id
//! into the insert fire-iteration set with its `added` flag true.
//!
//! Three insert-path cases (the spawn-path test lives in `required_components.rs`):
//! * `insert_a_requires_b_fires_b_on_add` — entity exists; `insert(A)` where A
//!   `#[require(B)]`, B has on_add hook + observer → B's on_add fires once.
//! * `insert_a_requires_b_present_does_not_fire` — B already present → no
//!   construct, no fire (the present⇒skip semantic).
//! * `insert_via_commands_deferred_fires` — the deferred `Commands::insert`
//!   apply path also fires (same `migrate_entity_insert` funnel).
//!
//! All inserts route through `cmds.entity(e).insert(bundle)` driven by
//! `ecs.run_system(...)` — the deferred `InsertCommand::apply` →
//! `migrate_entity_insert` path. `EcsMaster` has no in-place direct-insert API
//! that bypasses the funnel.
//!
//! BUG-REQ-SNAKE-1 is FIXED at the macro layer (the derive emits its own
//! `#[allow(non_snake_case)]` on the generated ctor fns), so no crate-level mask
//! is needed here.

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::{ObserverContext, ObserverKind};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// C1.1 — insert(A) where A requires B (absent) ⇒ B's on_add hook + observer fire
// ════════════════════════════════════════════════════════════════════════════

static C1_B_ADD: AtomicUsize = AtomicUsize::new(0);
static C1_B_INSERT: AtomicUsize = AtomicUsize::new(0);
static C1_B_OBS_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn c1_b_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    C1_B_ADD.fetch_add(1, SEQ);
}
unsafe fn c1_b_insert(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    C1_B_INSERT.fetch_add(1, SEQ);
}
unsafe fn c1_b_obs_add(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Add, "observer kind is Add");
    C1_B_OBS_ADD.fetch_add(1, SEQ);
}

#[derive(Component, Default)]
#[component(on_add = c1_b_add, on_insert = c1_b_insert)]
#[repr(C)]
struct C1B(u32);

/// The "anchor" component the entity already has, so the insert is a real
/// MIGRATION ({Anchor} → {Anchor, A, B}) and not an in-place replace.
#[derive(Component, Clone, Copy, Default)]
#[repr(C)]
struct C1Anchor(u32);

#[derive(Component)]
#[require(C1B)]
#[repr(C)]
struct C1A(u32);

#[derive(Bundle)]
struct C1ABundle {
    a: C1A,
}

#[test]
fn insert_a_requires_b_fires_b_on_add() {
    let mut ecs = EcsMaster::new();
    let _ = C1A::component_id();
    let _ = C1B::component_id();
    ecs.observe_on_add::<C1B>(c1_b_obs_add);

    // Existing entity with ONLY the anchor — A and B are both absent.
    let arch = ecs.create_archetype(&[C1Anchor::component_id()]);
    let e = ecs.spawn_one(arch, C1Anchor(1)).expect("spawn anchor");

    C1_B_ADD.store(0, SEQ);
    C1_B_INSERT.store(0, SEQ);
    C1_B_OBS_ADD.store(0, SEQ);

    // INSERT A ⇒ migration to {Anchor, A, B}; B is required + absent ⇒ constructed.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(C1ABundle { a: C1A(2) });
    });

    assert!(
        ecs.has_component(e, C1B::component_id()),
        "C1: required B is auto-inserted on the INSERT path"
    );
    assert_eq!(
        C1_B_ADD.load(SEQ),
        1,
        "C1 HEADLINE: required B's on_add HOOK fires exactly once on the insert path \
         (the fire-set augmentation fix — silently broken before C1)"
    );
    assert_eq!(
        C1_B_INSERT.load(SEQ),
        1,
        "C1: required B's on_insert HOOK fires exactly once on the insert path"
    );
    assert_eq!(
        C1_B_OBS_ADD.load(SEQ),
        1,
        "C1: required B's on_add OBSERVER fires exactly once on the insert path"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// C1.2 — insert(A) where required B is ALREADY PRESENT ⇒ no construct, no fire
// ════════════════════════════════════════════════════════════════════════════

static C1P_B_ADD: AtomicUsize = AtomicUsize::new(0);
static C1P_B_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn c1p_b_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    C1P_B_ADD.fetch_add(1, SEQ);
}
unsafe fn c1p_b_insert(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    C1P_B_INSERT.fetch_add(1, SEQ);
}

#[derive(Component, Clone, Copy, Default, PartialEq, Debug)]
#[component(on_add = c1p_b_add, on_insert = c1p_b_insert)]
#[repr(C)]
struct C1pB(u32);

#[derive(Component)]
#[require(C1pB)]
#[repr(C)]
struct C1pA(u32);

#[derive(Bundle)]
struct C1pABundle {
    a: C1pA,
}

#[test]
fn insert_a_requires_b_present_does_not_fire() {
    let mut ecs = EcsMaster::new();
    let _ = C1pA::component_id();
    let _ = C1pB::component_id();

    // Existing entity that ALREADY has B (explicit, non-default value).
    let arch = ecs.create_archetype(&[C1pB::component_id()]);
    let e = ecs.spawn_one(arch, C1pB(42)).expect("spawn with B present");

    // Clear the spawn's fires — we assert only about the subsequent insert.
    C1P_B_ADD.store(0, SEQ);
    C1P_B_INSERT.store(0, SEQ);

    // INSERT A ⇒ migration to {B, A}; B is required but ALREADY PRESENT ⇒ skip.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(C1pABundle { a: C1pA(2) });
    });

    assert_eq!(
        C1P_B_ADD.load(SEQ),
        0,
        "C1 present⇒skip: B already present ⇒ B's on_add must NOT fire on insert"
    );
    assert_eq!(
        C1P_B_INSERT.load(SEQ),
        0,
        "C1 present⇒skip: B already present ⇒ B's on_insert must NOT fire on insert"
    );
    assert_eq!(
        ecs.get_component::<C1pB>(e).copied(),
        Some(C1pB(42)),
        "C1 present⇒skip: the explicit B(42) is KEPT, not overwritten by B::default()"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// C1.3 — deferred Commands::insert apply path also fires (same migrate funnel).
//
// This case exercises the FULL deferred pipeline: an entity spawned in one
// system, then a SECOND system inserts A (requiring B) via Commands. The
// deferred `InsertCommand::apply` routes through `migrate_entity_insert`, so it
// inherits the C1 fire-set augmentation. (C1.1 already runs through the deferred
// apply; this case makes the two-system deferral explicit and adds the observer.)
// ════════════════════════════════════════════════════════════════════════════

static C1D_B_ADD: AtomicUsize = AtomicUsize::new(0);
static C1D_B_OBS_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn c1d_b_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    C1D_B_ADD.fetch_add(1, SEQ);
}
unsafe fn c1d_b_obs_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    C1D_B_OBS_ADD.fetch_add(1, SEQ);
}

#[derive(Component, Default)]
#[component(on_add = c1d_b_add)]
#[repr(C)]
struct C1dB(u32);

#[derive(Component, Clone, Copy, Default)]
#[repr(C)]
struct C1dAnchor(u32);

#[derive(Component)]
#[require(C1dB)]
#[repr(C)]
struct C1dA(u32);

#[derive(Bundle)]
struct C1dABundle {
    a: C1dA,
}

#[test]
fn insert_via_commands_deferred_fires() {
    let mut ecs = EcsMaster::new();
    let _ = C1dA::component_id();
    let _ = C1dB::component_id();
    ecs.observe_on_add::<C1dB>(c1d_b_obs_add);

    let arch = ecs.create_archetype(&[C1dAnchor::component_id()]);
    let e = ecs.spawn_one(arch, C1dAnchor(1)).expect("spawn anchor");

    C1D_B_ADD.store(0, SEQ);
    C1D_B_OBS_ADD.store(0, SEQ);

    // Deferred insert in a fresh system invocation — the command is queued and
    // applied at the end-of-system apply window, through migrate_entity_insert.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(C1dABundle { a: C1dA(2) });
    });

    assert!(
        ecs.has_component(e, C1dB::component_id()),
        "deferred insert: required B is constructed"
    );
    assert_eq!(
        C1D_B_ADD.load(SEQ),
        1,
        "C1 deferred apply path: required B's on_add HOOK fires once via deferred InsertCommand::apply"
    );
    assert_eq!(
        C1D_B_OBS_ADD.load(SEQ),
        1,
        "C1 deferred apply path: required B's on_add OBSERVER fires once via deferred InsertCommand::apply"
    );
}
