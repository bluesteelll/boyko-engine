//! Phase 22 Wave 1A — EMPTY archetype integration tests (plan D5).
//!
//! Pins the four contract slices of the empty-archetype work:
//!
//! 1. `spawn_empty` creates a live zero-component entity in the lazily
//!    created EMPTY archetype, and despawning it works (D5(4) + D5(6)).
//! 2. **Two-empty-entities despawn-identity regression** — the exact
//!    audited corruption: before the D5(3) row fix, every empty entity
//!    received the vacuous row 0 from `push_entity_components`, so
//!    despawning one of two empty entities swap-removed the WRONG row and
//!    evicted the survivor's id from the archetype's entity list.
//! 3. Remove-last → EMPTY → re-insert round trip: removing an entity's
//!    only component routes it to the EMPTY archetype through the normal
//!    migration funnel (D5(2)), hooks still fire (it is an ordinary
//!    migration), and a subsequent insert attaches FROM the empty
//!    archetype (zero retained columns).
//! 4. The EMPTY archetype NEVER matches a component query (D5(5) — the
//!    flecs invariant falls out of subset matching; pinned with a typed
//!    query).
//!
//! Component ids are minted by `#[derive(Component)]` (process-global,
//! collision-free); the hook test uses a dedicated type so the Phase-21 H1
//! staleness gate never sees it in an archetype before registration.

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, InlandPoolId};
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

/// Reads the EMPTY archetype's dense row list back ARCHETYPE-side (the
/// fast-store `iter_entities` cannot discriminate the audited corruption:
/// the bug left the survivor's inland record intact while evicting its id
/// from `Archetype::entity_ids`). `query_entities(&[])` is unusable here —
/// the few-components fast path early-returns on an empty include set.
fn empty_archetype_rows(ecs: &EcsMaster, empty_arch: ArchetypeId) -> Vec<usize> {
    let arch = ecs
        .archetype_master()
        .get_archetype(empty_arch)
        .expect("EMPTY archetype exists");
    (0..arch.entity_count())
        .map(|row| {
            arch.get_entity_id_at(InlandPoolId(row))
                .expect("row < entity_count")
                .0
        })
        .collect()
}

// ── Component / bundle fixtures ──────────────────────────────────────────────

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct P22Pos {
    x: f32,
    y: f32,
}

#[derive(Bundle)]
struct P22PosBundle {
    p: P22Pos,
}

// ════════════════════════════════════════════════════════════════════════════
// 1 — spawn_empty → live entity in the lazy EMPTY archetype → despawn
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn spawn_empty_creates_live_zero_component_entity_and_despawns() {
    let mut ecs = EcsMaster::new();

    let e = ecs.spawn_empty();
    assert!(ecs.has_entity(e), "spawn_empty must register a live entity");
    assert_eq!(ecs.entity_count(), 1);
    assert!(
        !ecs.has_component(e, P22Pos::component_id()),
        "an empty entity hosts no components"
    );

    // The entity lives in THE empty archetype (the same id the funnel
    // resolves for the empty component set).
    let empty_arch = ecs.get_or_create_archetype(&[]);
    assert_eq!(ecs.get_entity_archetype_id(e), Some(empty_arch));

    // Despawn works over zero pools (D5(6)).
    assert!(ecs.delete_entity(e), "despawning an empty entity must succeed");
    assert!(!ecs.has_entity(e));
    assert_eq!(ecs.entity_count(), 0);
}

/// The EMPTY archetype is created lazily ONCE and shared by every empty
/// entity (D5(1) — `get_or_create_archetype(&[])` + exact-mask caching).
#[test]
fn spawn_empty_entities_share_one_lazily_created_empty_archetype() {
    let mut ecs = EcsMaster::new();

    let before = ecs.archetype_count();
    let e1 = ecs.spawn_empty();
    let after_first = ecs.archetype_count();
    assert_eq!(
        after_first,
        before + 1,
        "first spawn_empty lazily creates the EMPTY archetype"
    );

    let e2 = ecs.spawn_empty();
    assert_eq!(
        ecs.archetype_count(),
        after_first,
        "second spawn_empty reuses the cached EMPTY archetype"
    );
    assert_eq!(ecs.get_entity_archetype_id(e1), ecs.get_entity_archetype_id(e2));
}

// ════════════════════════════════════════════════════════════════════════════
// 2 — the audited despawn-identity corruption (two empty entities)
// ════════════════════════════════════════════════════════════════════════════

/// Despawn the SECOND of two empty entities. Before the D5(3) fix both
/// entities claimed row 0 (the vacuous `push_entity_components` return), so
/// this despawn took the swap path against the wrong row and evicted the
/// FIRST entity's id from the archetype's entity list.
#[test]
fn two_empty_entities_despawn_second_preserves_first_identity() {
    let mut ecs = EcsMaster::new();

    let e1 = ecs.spawn_empty();
    let e2 = ecs.spawn_empty();
    assert_eq!(ecs.entity_count(), 2);
    let empty_arch = ecs.get_or_create_archetype(&[]);

    // D5(3) pin: the two entities occupy DISTINCT dense rows, in spawn order.
    assert_eq!(
        empty_archetype_rows(&ecs, empty_arch),
        vec![e1.id().0, e2.id().0],
        "two empty entities occupy rows 0 and 1"
    );

    assert!(ecs.delete_entity(e2));
    assert!(!ecs.has_entity(e2));
    assert!(ecs.has_entity(e1), "survivor's fast-store identity intact");
    assert_eq!(ecs.entity_count(), 1);

    // Archetype-side identity: the EMPTY archetype's row list must contain
    // exactly e1. The pre-fix corruption left the dead e2's id in the list
    // and dropped e1's.
    assert_eq!(
        empty_archetype_rows(&ecs, empty_arch),
        vec![e1.id().0],
        "the surviving row is e1"
    );

    // The survivor remains fully operable.
    assert!(ecs.delete_entity(e1));
    assert_eq!(ecs.entity_count(), 0);
    assert!(empty_archetype_rows(&ecs, empty_arch).is_empty());
}

/// Mirror order: despawn the FIRST of two empty entities (exercises the
/// swap-remove `Swapped` outcome over zero pools; the moved survivor's
/// `unit_index` fixup must keep it addressable).
#[test]
fn two_empty_entities_despawn_first_swaps_second_identity_intact() {
    let mut ecs = EcsMaster::new();

    let e1 = ecs.spawn_empty();
    let e2 = ecs.spawn_empty();
    let empty_arch = ecs.get_or_create_archetype(&[]);

    assert!(ecs.delete_entity(e1));
    assert!(!ecs.has_entity(e1));
    assert!(ecs.has_entity(e2), "swapped survivor's identity intact");
    assert_eq!(ecs.entity_count(), 1);

    assert_eq!(
        empty_archetype_rows(&ecs, empty_arch),
        vec![e2.id().0],
        "e2 swap-moved into row 0"
    );

    assert!(ecs.delete_entity(e2), "survivor despawnable after the swap");
    assert_eq!(ecs.entity_count(), 0);
    assert!(empty_archetype_rows(&ecs, empty_arch).is_empty());
}

// ════════════════════════════════════════════════════════════════════════════
// 3 — remove-last → EMPTY → re-insert round trip (D5(2) + attach-from-empty)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn remove_last_component_routes_entity_to_empty_archetype_and_back() {
    let mut ecs = EcsMaster::new();

    // Spawn with exactly one component through the deferred funnel.
    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(P22PosBundle { p: P22Pos { x: 1.0, y: 2.0 } });
    });
    assert_eq!(ecs.entity_count(), 1);
    let e = ecs.iter_entities().next().expect("one entity exists");
    let pos_arch = ecs.get_entity_archetype_id(e).expect("Pos archetype");

    // Remove the ONLY component: Phase 11 silently no-op'd this; Phase 22
    // D5(2) routes the entity to the EMPTY archetype as an ordinary
    // migration.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove::<P22Pos>();
    });

    assert!(ecs.has_entity(e), "entity survives losing its last component");
    assert!(!ecs.has_component(e, P22Pos::component_id()));
    assert!(ecs.get_component::<P22Pos>(e).is_none());

    let empty_arch = ecs.get_or_create_archetype(&[]);
    let now_arch = ecs.get_entity_archetype_id(e).expect("entity still placed");
    assert_ne!(now_arch, pos_arch, "archetype migrated on remove-last");
    assert_eq!(now_arch, empty_arch, "remove-last lands in THE empty archetype");

    // While empty, the entity is invisible to the component query (D5(5)).
    {
        let view = ecs.query::<&P22Pos, ()>();
        assert_eq!(
            view.iter().count(),
            0,
            "an entity in the EMPTY archetype matches no component query"
        );
    }

    // Re-insert: attach FROM the empty archetype (zero retained columns).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(P22PosBundle { p: P22Pos { x: 3.0, y: 4.0 } });
    });

    assert_eq!(
        ecs.get_entity_archetype_id(e),
        Some(pos_arch),
        "re-insert returns the entity to the original Pos archetype"
    );
    let p = ecs.get_component::<P22Pos>(e).expect("Pos re-attached");
    assert_eq!(p.x, 3.0);
    assert_eq!(p.y, 4.0);
}

// ════════════════════════════════════════════════════════════════════════════
// 3b — hooks still fire on the remove-last path (now an ordinary migration)
// ════════════════════════════════════════════════════════════════════════════

static P22_REMOVE_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn p22_on_remove(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    P22_REMOVE_FIRES.fetch_add(1, SEQ);
}

/// Dedicated type: used ONLY in the hook test below, so the Phase-21 H1
/// staleness gate never sees it archetyped before hook registration.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct P22Hooked(u32);

#[derive(Bundle)]
struct P22HookedBundle {
    h: P22Hooked,
}

#[test]
fn remove_last_component_still_fires_remove_hooks() {
    let mut ecs = EcsMaster::new();

    // Register BEFORE the component ever appears in an archetype (H1 gate).
    ecs.register_component_hooks::<P22Hooked>()
        .on_remove(p22_on_remove)
        .finish();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(P22HookedBundle { h: P22Hooked(1) });
    });
    let e = ecs.iter_entities().next().expect("one entity exists");
    assert_eq!(P22_REMOVE_FIRES.load(SEQ), 0);

    // Remove-last is a NORMAL migration into the EMPTY archetype — the
    // on_remove hook must fire exactly once on the dying source row.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove::<P22Hooked>();
    });

    assert_eq!(
        P22_REMOVE_FIRES.load(SEQ),
        1,
        "on_remove fires exactly once on the remove-last migration"
    );
    assert!(ecs.has_entity(e), "entity alive in the EMPTY archetype");
    let empty_arch = ecs.get_or_create_archetype(&[]);
    assert_eq!(ecs.get_entity_archetype_id(e), Some(empty_arch));
}

// ════════════════════════════════════════════════════════════════════════════
// 4 — the EMPTY archetype never matches a typed component query (D5(5))
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn empty_archetype_never_matches_component_queries() {
    let mut ecs = EcsMaster::new();

    let _empty_entity = ecs.spawn_empty();
    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(P22PosBundle { p: P22Pos { x: 7.0, y: 8.0 } });
    });
    assert_eq!(ecs.entity_count(), 2);

    // A typed query with one required component must see ONLY the Pos
    // entity — the EMPTY archetype's signature is the empty mask and can
    // never be a superset of a non-empty include mask.
    let view = ecs.query::<&P22Pos, ()>();
    let xs: Vec<f32> = view.iter().map(|p: &P22Pos| p.x).collect();
    assert_eq!(xs, vec![7.0], "typed query yields exactly the Pos entity");
}
