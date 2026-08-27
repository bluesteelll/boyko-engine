//! Kernel gate — **every walk over an archetype's RETAINED `component_ids()`
//! must route through `is_signature_storage` before it touches a per-archetype
//! `ComponentPool`.**
//!
//! # The false premise these tests pin
//!
//! `Archetype::component_ids()` is NOT the archetype signature. `create_by_ids`
//! stores the id list **verbatim** (`component_ids.to_vec()`) while filtering
//! only the *mask* (`filtered_signature_mask`). So a `StorageKind::Bitset` or
//! `StorageKind::Dense` id stays in the retained list forever, even though it
//! owns **no per-archetype pool by construction** — its data lives in the global
//! `DenseStore` (dense) or the `EnableStore` (bitset), or nowhere at all.
//!
//! Several migration walks assumed the opposite and fed the retained list
//! straight into `get_pool(...).expect(...)`. The `.expect` is an `Option`
//! unwrap, so it fires in **release** exactly as in debug — this file's tests
//! are therefore plain positive tests that exercise the same code in both
//! profiles, not `#[cfg(debug_assertions)]` assertion probes. (A
//! `#[should_panic]` shape would have been wrong twice over: it would go green
//! on the defect and red on the fix, and it would prove nothing about the data
//! that has to survive the migration.)
//!
//! # Why a table-only entity is a victim
//!
//! Archetype dedup keys on the **filtered** mask, but the retained id list
//! belongs to **whichever spawn minted that archetype first**. An entity that
//! never touched a dense or bitset component therefore lands in an archetype
//! whose retained list carries one — and its next structural op walks that list.
//! `attach_ids_walk_skips_a_retained_dense_id_for_a_table_only_entity` and
//! `remove_migration_skips_a_retained_dense_id_on_a_pure_table_entity` are that
//! case; in the second one the entity, its source archetype and the removed
//! component are **all** pure table.
//!
//! # The in-tree template
//!
//! `clone/materialize.rs` walks the same list and skips pool-less ids through
//! `component_registry::is_signature_storage(storage_kind(id.0))`, whose own doc
//! calls it "the single shared predicate every signature-exclude / pool-skip
//! site routes through". These tests hold the migration layer to it.
//!
//! # The resolver half
//!
//! Two `_dyn` archetype resolvers seeded their union / difference from the
//! **unfiltered** retained list, so every archetype newly minted through
//! `add_tag` / `remove_tag` inherited the non-signature id and became a fresh
//! trap for the walks above. The last two tests observe the minted archetype's
//! retained list directly — they are the only gate on that half, because a
//! filtered *walk* hides an unfiltered *resolver* by construction.

// Test oracle model: the `Arc<Mutex<_>>` below is the cross-thread observation
// channel used to lift a `Commands::spawn` handle out of a system closure - the
// exact pattern the sibling dense/tag suites use. Never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::bundle::Bundle as BundleTrait;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::EcsMaster;
use boyko_macros::{Bundle, Component};

// ════════════════════════════════════════════════════════════════════════════
// Shared helpers
// ════════════════════════════════════════════════════════════════════════════

/// Spawns one bundle through `Commands::spawn` and returns the live handle.
fn spawn_one<B: BundleTrait + Send + Sync>(
    ecs: &mut EcsMaster,
    make: impl Fn() -> B + Send + Sync + 'static,
) -> Entity {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        probe.lock().expect("lock").push(cmds.spawn(make()).id());
    });
    sink.lock().expect("lock")[0]
}

/// Canonical-sorts an id array. `ComponentId`s are minted in process-global
/// registration order, which test parallelism makes unpredictable, so the sort
/// must happen at RUNTIME — the archetype funnels debug-assert canonical order.
fn sorted<const N: usize>(mut ids: [ComponentId; N]) -> [ComponentId; N] {
    ids.sort_unstable_by_key(|c| c.0);
    ids
}

/// Borrows a `#[repr(C)]` POD value's own bytes for `create_entity`.
///
/// # Safety
///
/// `T` must be `#[repr(C)]` POD whose byte span is a valid representation of the
/// id it is paired with (every caller here passes the matching registered type).
unsafe fn pod_bytes<T: Copy>(v: &T) -> &[u8] {
    // SAFETY: `T` is `#[repr(C)]` POD (caller contract); the slice borrows `v`
    // and is consumed before `v` goes out of scope.
    unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), std::mem::size_of::<T>()) }
}

/// Reads a table column back as `T`.
///
/// # Safety
///
/// `component_id` must be `T`'s registered id and the entity must host it.
unsafe fn read_table<T: Copy>(ecs: &EcsMaster, e: Entity, component_id: ComponentId) -> T {
    let raw = ecs
        .get_component_raw(e, component_id)
        .expect("invariant: the table column must survive the migration");
    // SAFETY: `raw` addresses the live, initialized `T` slot for this read.
    unsafe { *(raw as *const T) }
}

/// Returns the retained id list of the archetype the entity currently lives in.
fn retained_ids_of(ecs: &EcsMaster, e: Entity) -> Vec<ComponentId> {
    let arch_id = ecs.entity_archetype_id(e).expect("entity is live");
    ecs.archetype_master()
        .get_archetype(arch_id)
        .expect("archetype is live")
        .component_ids()
        .to_vec()
}

// ════════════════════════════════════════════════════════════════════════════
// S1 — `migrate_entity_attach_ids` Step 1 walks `source.component_ids()`
//      (`migration_helpers.rs`, the `get_pool(retained_cid).expect(
//       "invariant: source hosts its own component id")`)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct A1Pos(u32);

#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct A1Dense(u64);

#[derive(Bundle)]
struct A1Mixed {
    p: A1Pos,
    d: A1Dense,
}

/// The entity really does carry a dense column, and `add_tag` migrates it.
#[test]
fn attach_ids_walk_skips_a_retained_dense_id() {
    let mut ecs = EcsMaster::new();
    let e = spawn_one(&mut ecs, || A1Mixed {
        p: A1Pos(7),
        d: A1Dense(0xDEAD_BEEF),
    });
    assert!(
        ecs.dense_contains(e, A1Dense::component_id()),
        "precondition: the mixed spawn recorded the dense membership"
    );
    let tag = ecs.register_tag("retained_walk_s1_mixed");

    // UNFIXED: panics in `migrate_entity_attach_ids` — the retained walk hits
    // the dense id and `source.get_pool(dense)` is `None`.
    ecs.add_tag(e, tag);

    assert!(ecs.has_tag(e, tag), "the tag attached");
    assert_eq!(
        // SAFETY: `A1Pos` is the registered type for its id; the entity hosts it.
        unsafe { read_table::<A1Pos>(&ecs, e, A1Pos::component_id()) },
        A1Pos(7),
        "the retained TABLE column keeps its value across the attach migration"
    );
    assert!(
        ecs.dense_contains(e, A1Dense::component_id()),
        "a TABLE migration must not disturb the global dense membership"
    );
}

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct A2Pos(u32);

#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct A2Dense(u64);

#[derive(Bundle)]
struct A2Mixed {
    p: A2Pos,
    d: A2Dense,
}

#[derive(Bundle)]
struct A2TableOnly {
    p: A2Pos,
}

/// THE WIDER VICTIM CLASS: the entity carries no dense column at all. It is
/// caught only because archetype dedup keys on the FILTERED mask while the
/// retained list belongs to the spawn that minted the archetype first.
#[test]
fn attach_ids_walk_skips_a_retained_dense_id_for_a_table_only_entity() {
    let mut ecs = EcsMaster::new();

    // Mint the archetype from the MIXED spawn first — it retains the dense id.
    let mixed = spawn_one(&mut ecs, || A2Mixed {
        p: A2Pos(1),
        d: A2Dense(1),
    });
    // A pure-table spawn with the same FILTERED mask dedups into it.
    let table_only = spawn_one(&mut ecs, || A2TableOnly { p: A2Pos(9) });

    assert_eq!(
        ecs.entity_archetype_id(mixed),
        ecs.entity_archetype_id(table_only),
        "precondition: dedup keys on the filtered mask, so both entities share \
         ONE archetype"
    );
    assert!(
        !ecs.dense_contains(table_only, A2Dense::component_id()),
        "precondition: the victim carries NO dense column — it is an innocent \
         bystander of the archetype it was deduped into"
    );

    let tag = ecs.register_tag("retained_walk_s1_table_only");

    // UNFIXED: panics at the same site as the mixed case above.
    ecs.add_tag(table_only, tag);

    assert!(ecs.has_tag(table_only, tag), "the tag attached");
    assert_eq!(
        // SAFETY: `A2Pos` is the registered type for its id; the entity hosts it.
        unsafe { read_table::<A2Pos>(&ecs, table_only, A2Pos::component_id()) },
        A2Pos(9),
        "the retained TABLE column keeps its value"
    );
    assert!(
        !ecs.dense_contains(table_only, A2Dense::component_id()),
        "the migration must not invent a dense membership"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// S2 — `migrate_entity_detach_ids` Step 1 walks `source.component_ids()`
//      (the twin `.expect("invariant: source hosts its own component id")`)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct A3Pos(u32);

#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct A3Dense(u64);

#[derive(Bundle)]
struct A3TableOnly {
    p: A3Pos,
}

/// `remove_tag` off a dense-RETAINING archetype. The entity is pure table
/// throughout: it is routed into the retaining archetype by `add_tag`'s dedup.
#[test]
fn detach_ids_walk_skips_a_retained_dense_id() {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_tag("retained_walk_s2");

    // Mint the RETAINING archetype first: filtered mask {A3Pos, tag}, retained
    // list additionally carrying the dense id.
    let retaining = ecs.create_archetype(&sorted([
        A3Pos::component_id(),
        tag.component_id(),
        A3Dense::component_id(),
    ]));

    let e = spawn_one(&mut ecs, || A3TableOnly { p: A3Pos(4) });
    ecs.add_tag(e, tag);

    assert_eq!(
        ecs.entity_archetype_id(e),
        Some(retaining),
        "precondition: the attach target dedups into the retaining archetype"
    );
    assert!(
        !ecs.dense_contains(e, A3Dense::component_id()),
        "precondition: the entity carries NO dense column"
    );

    // UNFIXED: panics in `migrate_entity_detach_ids` — the retained walk hits
    // the dense id the SOURCE archetype merely retains.
    ecs.remove_tag(e, tag);

    assert!(!ecs.has_tag(e, tag), "the tag detached");
    assert_eq!(
        // SAFETY: `A3Pos` is the registered type for its id; the entity hosts it.
        unsafe { read_table::<A3Pos>(&ecs, e, A3Pos::component_id()) },
        A3Pos(4),
        "the retained TABLE column keeps its value across the detach migration"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// S3 — `migrate_entity_remove` Step 1 walks `target.component_ids()`
//      (`.expect("invariant: target ⊂ source")`)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct A4Pos(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct A4Vel(u32);

#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct A4Dense(u64);

#[derive(Bundle)]
struct A4Bundle {
    p: A4Pos,
    v: A4Vel,
}

/// THE WIDEST VICTIM CLASS: the entity, its source archetype AND the removed
/// component are all pure table. Only the REMOVE TARGET retains a dense id, and
/// the walk over the target's list is unconditional.
#[test]
fn remove_migration_skips_a_retained_dense_id_on_a_pure_table_entity() {
    let mut ecs = EcsMaster::new();

    // Mint the {A4Pos}-masked archetype so that it RETAINS a dense id.
    let retaining =
        ecs.create_archetype(&sorted([A4Pos::component_id(), A4Dense::component_id()]));

    let e = spawn_one(&mut ecs, || A4Bundle {
        p: A4Pos(3),
        v: A4Vel(5),
    });
    assert!(
        !ecs.dense_contains(e, A4Dense::component_id()),
        "precondition: the entity carries NO dense column"
    );
    assert_ne!(
        ecs.entity_archetype_id(e),
        Some(retaining),
        "precondition: the entity starts in the {{A4Pos, A4Vel}} archetype"
    );

    // UNFIXED: panics in `migrate_entity_remove` — the remove target dedups
    // into `retaining`, whose retained list carries the dense id, and the walk
    // looks that id up in the pure-table SOURCE.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove::<A4Vel>();
    });

    assert_eq!(
        ecs.entity_archetype_id(e),
        Some(retaining),
        "the remove target IS the retaining archetype (filtered-mask dedup)"
    );
    assert!(
        ecs.get_component_raw(e, A4Vel::component_id()).is_none(),
        "the removed column is gone"
    );
    assert_eq!(
        // SAFETY: `A4Pos` is the registered type for its id; the entity hosts it.
        unsafe { read_table::<A4Pos>(&ecs, e, A4Pos::component_id()) },
        A4Pos(3),
        "the retained TABLE column keeps its value across the remove migration"
    );
    assert!(
        !ecs.dense_contains(e, A4Dense::component_id()),
        "the migration must not invent a dense membership"
    );
}

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct A6Pos(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct A6Vel(u32);

/// A bitset enable tag: no pool, no dense store, filtered from every signature.
/// It cannot be a `Bundle` field (the derive suppresses that for
/// `storage = "bitset"`), so it enters only through `create_archetype`.
#[derive(Component)]
#[component(storage = "bitset")]
struct A6Bit;

#[derive(Bundle)]
struct A6Bundle {
    p: A6Pos,
    v: A6Vel,
}

/// The same walk, the same panic, with a BITSET id instead of a dense one —
/// the defect is `non-signature-storage`, not `dense`, which is why the fix is
/// the shared `is_signature_storage` predicate and not a `matches!(.., Dense)`.
#[test]
fn remove_migration_skips_a_retained_bitset_id_on_a_pure_table_entity() {
    let mut ecs = EcsMaster::new();

    let retaining = ecs.create_archetype(&sorted([A6Pos::component_id(), A6Bit::component_id()]));

    let e = spawn_one(&mut ecs, || A6Bundle {
        p: A6Pos(31),
        v: A6Vel(41),
    });

    // UNFIXED: panics in `migrate_entity_remove` at the identical site.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove::<A6Vel>();
    });

    assert_eq!(
        ecs.entity_archetype_id(e),
        Some(retaining),
        "the remove target IS the bitset-retaining archetype"
    );
    assert_eq!(
        // SAFETY: `A6Pos` is the registered type for its id; the entity hosts it.
        unsafe { read_table::<A6Pos>(&ecs, e, A6Pos::component_id()) },
        A6Pos(31),
        "the retained TABLE column keeps its value"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// S4 — `migrate_entity_insert` Step 1 walks `target.component_ids()`
//      (`.expect("invariant: retained component must exist in source")`,
//       guarded by `src.component_ids().contains(&cid)` — so SOURCE and TARGET
//       must both retain)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct A5Pos(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct A5Vel(u32);

#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct A5Dense(u64);

#[derive(Bundle)]
struct A5VelBundle {
    v: A5Vel,
}

#[test]
fn insert_migration_skips_a_retained_dense_id() {
    let mut ecs = EcsMaster::new();

    // Both the source and the target archetype retain the dense id — the
    // `contains(&cid)` guard on the source means a clean source would skip.
    let src = ecs.create_archetype(&sorted([A5Pos::component_id(), A5Dense::component_id()]));
    let tgt = ecs.create_archetype(&sorted([
        A5Pos::component_id(),
        A5Vel::component_id(),
        A5Dense::component_id(),
    ]));

    let pos = A5Pos(11);
    // SAFETY: `A5Pos` is `#[repr(C)]` POD paired with its own registered id.
    let pos_bytes = unsafe { pod_bytes(&pos) };
    let e = ecs
        .create_entity(src, &[(A5Pos::component_id(), pos_bytes)])
        .expect("create_entity into the source archetype");
    assert!(
        !ecs.dense_contains(e, A5Dense::component_id()),
        "precondition: the entity carries NO dense column"
    );

    // UNFIXED: panics in `migrate_entity_insert` — the target's retained list
    // carries the dense id, the source retains it too, so the guard passes and
    // `src.get_pool(dense)` is `None`.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(A5VelBundle { v: A5Vel(22) });
    });

    assert_eq!(
        ecs.entity_archetype_id(e),
        Some(tgt),
        "the insert target IS the retaining archetype"
    );
    assert_eq!(
        // SAFETY: `A5Pos` is the registered type for its id; the entity hosts it.
        unsafe { read_table::<A5Pos>(&ecs, e, A5Pos::component_id()) },
        A5Pos(11),
        "the retained TABLE column keeps its value across the insert migration"
    );
    assert_eq!(
        // SAFETY: `A5Vel` is the registered type for its id; the entity hosts it.
        unsafe { read_table::<A5Vel>(&ecs, e, A5Vel::component_id()) },
        A5Vel(22),
        "the inserted column landed"
    );
    assert!(
        !ecs.dense_contains(e, A5Dense::component_id()),
        "the migration must not invent a dense membership"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// The RESOLVER half — the two `_dyn` archetype resolvers must not SEED a
// non-signature id into a newly-minted archetype (their generic twins
// `merged_archetype_id` / `without_component_archetype_id` already filter).
//
// A filtered WALK hides an unfiltered RESOLVER: the migration then succeeds
// while the kernel quietly manufactures the next trap. These two tests observe
// the minted archetype's retained list directly, so they stay red on exactly
// that half.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A7Pos(u32);

#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct A7Dense(u64);

#[derive(Bundle)]
struct A7Mixed {
    p: A7Pos,
    d: A7Dense,
}

#[test]
fn attach_resolver_does_not_seed_a_non_signature_id_into_a_new_archetype() {
    let mut ecs = EcsMaster::new();
    let e = spawn_one(&mut ecs, || A7Mixed {
        p: A7Pos(1),
        d: A7Dense(2),
    });
    let tag = ecs.register_tag("retained_walk_attach_resolver");

    // No {A7Pos, tag} archetype exists yet, so the funnel MINTS one here — from
    // whatever the resolver seeded.
    ecs.add_tag(e, tag);

    let ids = retained_ids_of(&ecs, e);
    assert!(
        !ids.contains(&A7Dense::component_id()),
        "the attach resolver seeded a non-signature id into the newly-minted \
         archetype — every entity that later dedups into it becomes a victim of \
         the retained-walk panics above; retained list was {ids:?}"
    );
    assert!(
        ids.contains(&A7Pos::component_id()) && ids.contains(&tag.component_id()),
        "the signature ids must still all be there; retained list was {ids:?}"
    );
    assert!(
        ecs.dense_contains(e, A7Dense::component_id()),
        "dropping the id from the SIGNATURE must not drop the dense membership"
    );
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A8Pos(u32);

#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct A8Dense(u64);

#[derive(Bundle)]
struct A8TableOnly {
    p: A8Pos,
}

#[test]
fn detach_resolver_does_not_seed_a_non_signature_id_into_a_new_archetype() {
    let mut ecs = EcsMaster::new();
    let tag1 = ecs.register_tag("retained_walk_detach_resolver_1");
    let tag2 = ecs.register_tag("retained_walk_detach_resolver_2");

    // A dense-RETAINING {A8Pos, tag1, tag2} archetype, reached by two attaches.
    let retaining = ecs.create_archetype(&sorted([
        A8Pos::component_id(),
        tag1.component_id(),
        tag2.component_id(),
        A8Dense::component_id(),
    ]));

    let e = spawn_one(&mut ecs, || A8TableOnly { p: A8Pos(6) });
    ecs.add_tag(e, tag1);
    ecs.add_tag(e, tag2);
    assert_eq!(
        ecs.entity_archetype_id(e),
        Some(retaining),
        "precondition: the entity sits in the retaining archetype"
    );

    // Detaching tag1 leaves {A8Pos, tag2} — a mask nothing has minted yet, so
    // the funnel MINTS from whatever the resolver seeded.
    ecs.remove_tag(e, tag1);

    let ids = retained_ids_of(&ecs, e);
    assert!(
        !ids.contains(&A8Dense::component_id()),
        "the detach resolver carried a non-signature id from the source's \
         retained list into the newly-minted archetype; retained list was {ids:?}"
    );
    assert!(
        ids.contains(&A8Pos::component_id()) && ids.contains(&tag2.component_id()),
        "the surviving signature ids must all be there; retained list was {ids:?}"
    );
    assert!(!ecs.has_tag(e, tag1), "tag1 detached");
    assert!(ecs.has_tag(e, tag2), "tag2 survived");
}
