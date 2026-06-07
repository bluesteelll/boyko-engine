//! Bug #55 regression — insert-migration overlap must drop the DISPLACED value
//! exactly once (no leak, no double-free).
//!
//! # The bug
//!
//! `migrate_entity_insert` (`commands/migration_helpers.rs`) copies the retained
//! source components into the freshly-reserved target row via a bitwise memcpy
//! (Step 1). When the inserted bundle ALSO carries a component already present in
//! the source (an "overlap"), the closure that writes the bundle bytes must drop
//! the displaced (source-copy) value BEFORE overwriting the slot — otherwise that
//! value's destructor never runs (a leak; for a heap-owning component, the
//! `Box`/`Vec`/`Arc` it owns leaks). The fix mirrors
//! `InsertCommand::apply_replace_in_place`: in the `has_row(row)` overlap branch,
//! `dst_pool.drop_at(row)` runs first, THEN `dst_pool.write_at(row, bundle_bytes)`
//! (migration_helpers.rs ~357-362).
//!
//! # What this file pins
//!
//! A `#[derive(Component)]` `Tracked` whose `Drop` increments a process-global
//! `AtomicUsize`. An entity holds `{Tracked(v=1), Keep}`; inserting
//! `(Tracked(v=2), New)` forces an archetype change (the genuinely-new `New`
//! makes the union differ from the source), so the deferred apply takes the
//! MIGRATION path (NOT the in-place replace fast path) and runs
//! `migrate_entity_insert`. The displaced `Tracked(v=1)` must be dropped exactly
//! once at apply time (count == 1), the live value must read back as `v == 2`,
//! and despawning the entity must drop the surviving `Tracked(v=2)` (count == 2).
//!
//! The load-bearing assertion is **count == 1 after insert**: pre-fix the
//! displaced source copy was released byte-wise by `move_out_entity` WITHOUT a
//! drop (W-N2), so its destructor never ran → the count would have been 0 there.
//! count == 1 is therefore the exact discriminator between the buggy and fixed
//! code.
//!
//! # Drop-count harness
//!
//! `Tracked` is NOT `Copy` (it has a `Drop` impl) — `#[derive(Component)]` does
//! not require `Copy`, so the derive coexists with the destructor and the
//! registry records a `drop_fn` (set whenever `mem::needs_drop::<T>()` is true).
//! The counters are `static AtomicUsize` guarded by a process-wide `TEST_MUTEX`,
//! the same shape as `phase14b_insert_migration_correctness.rs` /
//! `phase10_change_detection.rs`. Component ids are minted lazily from the global
//! atomic counter, so they are disjoint from every other test in the binary.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

const REL: Ordering = Ordering::Relaxed;

/// Serializes the two tests in this file — they share the process-global
/// `TRACKED_DROPS` counter (the static is binary-wide).
static TEST_MUTEX: Mutex<()> = Mutex::new(());

/// Bumped once per `Tracked` destructor. Reset under the test mutex at the start
/// of each test.
static TRACKED_DROPS: AtomicUsize = AtomicUsize::new(0);

/// A component carrying a payload value AND a `Drop` that increments the global
/// drop counter. NOT `Copy` (cannot be — it has `Drop`); `#[derive(Component)]`
/// does not require `Copy`, so the registry still gets a `drop_fn`.
#[derive(Component)]
#[repr(C)]
struct Tracked {
    v: u64,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        TRACKED_DROPS.fetch_add(1, REL);
    }
}

/// A plain retained companion so the source archetype is `{Tracked, Keep}` and
/// the migration has a genuine non-overlapping retained column to copy through.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct Keep {
    other: u64,
}

/// A genuinely-new component. Inserting it (alongside the overlapping `Tracked`)
/// makes the union `{Tracked, Keep, New}` differ from the source `{Tracked,
/// Keep}` → the apply takes the MIGRATION path, not the in-place replace.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct New {
    z: u64,
}

/// The inserted bundle: the ALREADY-PRESENT `Tracked` (overlap) plus the
/// genuinely-new `New`. The overlap is what exercises the bug-fix branch.
#[derive(Bundle)]
struct OverlapInsertBundle {
    t: Tracked,
    n: New,
}

/// Bug #55 core: the displaced `Tracked(v=1)` is dropped EXACTLY once when the
/// insert-migration overwrites the overlapping slot with `Tracked(v=2)`; the
/// live value is then `v=2`; despawning drops the survivor (total == 2).
#[test]
fn insert_migration_overlap_drops_displaced_value_exactly_once() {
    let _guard = TEST_MUTEX.lock().expect("test mutex");
    TRACKED_DROPS.store(0, REL);

    let mut world = EcsMaster::new();

    // Source archetype = {Tracked, Keep}; spawn directly (no schedule needed —
    // this is a drop-accounting test, not a change-detection one).
    let src_arch =
        world.create_archetype(&[Tracked::component_id(), Keep::component_id()]);
    let entity = world
        .spawn_two(src_arch, Tracked { v: 1 }, Keep { other: 0xDEAD_BEEF })
        .expect("spawn {Tracked(1), Keep}");
    let src_arch_id = world.get_entity_archetype_id(entity).expect("source arch id");

    // Pre-register `New` so the migration target archetype is resolvable.
    let _ = New::component_id();

    // No drops yet — Tracked(v=1) is live in the source row.
    assert_eq!(
        TRACKED_DROPS.load(REL),
        0,
        "no Tracked has been dropped before the migration"
    );

    // Insert {Tracked(v=2) (OVERLAP), New} via a deferred command driven through
    // the single-system runner. The apply runs `migrate_entity_insert` because
    // the union differs from the source (genuinely-new `New`).
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).insert(OverlapInsertBundle {
            t: Tracked { v: 2 },
            n: New { z: 7 },
        });
    });

    // The op MUST have taken the migration path (target archetype != source).
    let new_arch_id = world
        .get_entity_archetype_id(entity)
        .expect("post-insert arch id");
    assert_ne!(
        src_arch_id, new_arch_id,
        "the genuinely-new `New` forces a true migration (not the in-place replace fast path) \
         — `migrate_entity_insert` must run"
    );

    // THE load-bearing assertion (Bug #55): the displaced Tracked(v=1) was
    // dropped EXACTLY once during the overlap overwrite. Pre-fix this was 0
    // (the source copy was byte-released without a drop → leak).
    assert_eq!(
        TRACKED_DROPS.load(REL),
        1,
        "Bug#55: the displaced Tracked(v=1) must be dropped exactly once at insert-migration \
         (drop_at before write_at on the overlapping slot) — 0 here would mean the pre-fix leak"
    );

    // The live value is now the BUNDLE's Tracked(v=2) (bundle wins on overlap).
    assert_eq!(
        world.get_component::<Tracked>(entity).expect("Tracked present after migration").v,
        2,
        "bundle wins on overlap: the migrated Tracked holds v=2, not the retained v=1"
    );
    // The genuinely-new component is present with its bundle value.
    assert_eq!(
        world.get_component::<New>(entity).expect("New inserted").z,
        7,
        "the newly-added New holds its bundle value"
    );
    // The non-overlapping retained component survived byte-exact.
    assert_eq!(
        world.get_component::<Keep>(entity).expect("Keep retained").other,
        0xDEAD_BEEF,
        "the non-overlapping retained Keep value is copied through migration byte-exact"
    );

    // Despawn: the surviving Tracked(v=2) must now be dropped (total == 2). This
    // proves the overlap overwrite did NOT corrupt the slot's ownership (no
    // double-free of the displaced value, no lost ownership of the new one).
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).despawn();
    });
    assert!(
        !world.has_entity(entity),
        "the entity is gone after despawn"
    );
    assert_eq!(
        TRACKED_DROPS.load(REL),
        2,
        "after despawn the surviving Tracked(v=2) is dropped exactly once more (total == 2): \
         no leak of the displaced value, no double-free of the survivor"
    );
}

/// Control / contrast: a NON-overlapping insert-migration. The bundle carries
/// ONLY the genuinely-new `New` (no `Tracked`), so the migration retains
/// `Tracked` untouched — NO displaced value, NO drop at migration. This isolates
/// the overlap drop in the test above (it proves the +1 there comes from the
/// overlap branch specifically, not from migration in general).
#[derive(Bundle)]
struct NonOverlapInsertBundle {
    n: New,
}

#[test]
fn insert_migration_without_overlap_does_not_drop_retained_tracked() {
    let _guard = TEST_MUTEX.lock().expect("test mutex");
    TRACKED_DROPS.store(0, REL);

    let mut world = EcsMaster::new();

    let src_arch =
        world.create_archetype(&[Tracked::component_id(), Keep::component_id()]);
    let entity = world
        .spawn_two(src_arch, Tracked { v: 1 }, Keep { other: 5 })
        .expect("spawn {Tracked(1), Keep}");
    let src_arch_id = world.get_entity_archetype_id(entity).expect("source arch id");
    let _ = New::component_id();

    // Insert ONLY New — Tracked is retained (NOT in the bundle), so the migration
    // memcpy's Tracked's bytes into the target row and releases the source row
    // without a drop. No displaced value exists.
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).insert(NonOverlapInsertBundle { n: New { z: 1 } });
    });

    let new_arch_id = world
        .get_entity_archetype_id(entity)
        .expect("post-insert arch id");
    assert_ne!(src_arch_id, new_arch_id, "New forces a migration");

    assert_eq!(
        TRACKED_DROPS.load(REL),
        0,
        "a NON-overlapping migration must NOT drop the retained Tracked (it is moved, not displaced)"
    );
    assert_eq!(
        world.get_component::<Tracked>(entity).expect("Tracked retained").v,
        1,
        "the retained Tracked keeps its original value v=1 across the non-overlapping migration"
    );

    // Despawn drops the single live Tracked exactly once.
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).despawn();
    });
    assert_eq!(
        TRACKED_DROPS.load(REL),
        1,
        "despawn drops the one live Tracked exactly once (total == 1) — confirms no leak/no double-free \
         on the non-overlap path"
    );
}
