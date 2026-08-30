//! KE6 — the `ArchAdded` structural stamp (ruling **D2**).
//!
//! Kernel backlog `KE6`. The oracle the row prescribes is **stamp unit tests +
//! the check-ticks clamp**; the clamp half lives inside the crate (the scan is
//! `pub(crate)`) as `check_ticks::tests::check_ticks_clamps_the_arch_added_stamp`,
//! and this file is the stamp half.
//!
//! # What "the stamp is correct" means, and why coverage is the whole game
//!
//! `Archetype::arch_added` claims to be `>=` (in the `Tick::is_newer_than`
//! sense) every per-row `added_tick` in that archetype's table columns. The
//! claim is maintained by writing the stamp at every **row add**, and the fatal
//! direction is asymmetric:
//!
//! * **over-stamping** (a stamp newer than any row) costs a consumer one
//!   non-skip — a lost optimisation, never a wrong answer;
//! * **under-stamping** (a missed write site) lets a consumer skip an archetype
//!   that does hold a fresh row — a silent wrong answer, which is the failure
//!   class this campaign exists to kill.
//!
//! So these tests are organised by **write site**, not by feature: one test per
//! reachable row-add path, each driving it through a public API. The census
//! (nine sites) and its producing grep are recorded on
//! `Archetype::stamp_arch_added`.
//!
//! # Site 3 has no production caller
//!
//! `Archetype::create_entity_with_pool_ids` is stamped like the rest, but
//! nothing in `crates/*/src` calls its only wrapper
//! (`EcsMaster::create_entity_at_with_pool_ids`) since the Phase 12.6
//! `SpawnAtCommand` refit collapsed that chain. It is therefore untestable
//! through a public API today, and this file says so rather than leaving a
//! reader to assume the eight tests below are nine.
//!
//! # Non-vacuity
//!
//! A fresh `EcsMaster` starts at `change_tick == 0`, and an unstamped archetype
//! also reads `Tick::ZERO` — so `assert_eq!(stamp, current_tick())` on a fresh
//! world would pass against an implementation that never stamps at all. Every
//! test here therefore advances the world tick first (via real schedule frames,
//! the only public bump) and asserts the observed tick is non-zero before
//! comparing. `stamp_is_zero_before_any_row_is_added` pins the sentinel from the
//! other side.

use std::sync::Arc;

use boyko_ecs::ecs::core::archetype::archetype::Archetype;
use boyko_ecs::ecs::core::change_detection::Tick;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

// ── Harness ──────────────────────────────────────────────────────────────────

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// Advances the world's change tick off zero using the only public bump there
/// is — a real schedule frame. Without this every `assert_eq!(stamp, tick)`
/// below would be satisfied by `Tick::ZERO == Tick::ZERO`.
fn advance_tick(world: &mut EcsMaster, frames: usize) {
    let mut builder = ScheduleBuilder::new(serial_pool());
    builder.add_system(|| {});
    let mut schedule = builder.build(world);
    for _ in 0..frames {
        schedule.run(world);
    }
}

/// The `ArchAdded` stamp of the archetype currently hosting `entity`.
fn stamp_of(world: &EcsMaster, entity: Entity) -> Tick {
    let arch_id = world
        .entity_archetype_id(entity)
        .expect("entity must be live to have an archetype");
    world
        .archetype_master()
        .get_archetype(arch_id)
        .expect("the entity's archetype id must resolve")
        .arch_added()
}

/// Asserts `stamp_of(entity) == world.current_tick()`, plus the non-vacuity
/// guard that the tick is not the `Tick::ZERO` sentinel.
fn assert_stamped_now(world: &EcsMaster, entity: Entity, site: &str) {
    let now = world.current_tick();
    assert_ne!(
        now,
        Tick::ZERO,
        "{site}: the world tick must be off zero, or this assertion cannot \
         distinguish a stamp from the never-stamped sentinel",
    );
    assert_eq!(
        stamp_of(world, entity),
        now,
        "{site}: the row add must have stamped ArchAdded with the current tick",
    );
}

// ── Component / bundle types ─────────────────────────────────────────────────

// `Clone` is what makes a component cloneable: the derive registers a
// `Cloneability::Clone` handler for a `Component` that also derives `Clone`,
// and a component without one is skipped by `clone_and_spawn` / `instantiate`
// (the clone then lands in a SMALLER archetype, which would make sites 4 and 5
// pin the wrong archetype's stamp — measured, it panics in `materialize`).
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Ke6Hp {
    hp: u32,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Ke6Pos {
    x: f32,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Ke6Vel {
    dx: f32,
}

#[derive(Bundle)]
struct Ke6Body {
    hp: Ke6Hp,
    pos: Ke6Pos,
}

#[derive(Bundle)]
struct Ke6VelOnly {
    vel: Ke6Vel,
}

// =============================================================================
// The sentinel, from both sides
// =============================================================================

/// An archetype that has never taken a row carries `Tick::ZERO`, and
/// `has_structural_add_since` answers `false` for it regardless of the window —
/// the `entity_count() != 0` conjunct is what keeps the sentinel from ever
/// being fed to `is_newer_than`, whose wrapping arithmetic has no meaningful
/// answer for it.
#[test]
fn stamp_is_zero_before_any_row_is_added() {
    let mut world = EcsMaster::new();
    advance_tick(&mut world, 3);

    let arch_id = world.create_archetype(&[Ke6Hp::component_id()]);
    let arch = world
        .archetype_master()
        .get_archetype(arch_id)
        .expect("freshly created archetype resolves");

    assert_eq!(arch.entity_count(), 0, "the archetype is empty");
    assert_eq!(
        arch.arch_added(),
        Tick::ZERO,
        "never stamped ⇒ the sentinel",
    );

    let now = world.current_tick();
    assert!(
        !arch.has_structural_add_since(Tick::ZERO, now),
        "an empty archetype reports no structural add, whatever the window",
    );
    assert!(
        !arch.has_structural_add_since(
            Tick::new(now.get().wrapping_sub(1)),
            now
        ),
        "…including a window that would otherwise admit the ZERO sentinel",
    );
}

// =============================================================================
// Write site 1/9 — `Archetype::create_entity`, via `EcsMaster::spawn_one`
// =============================================================================

#[test]
fn site_1_immediate_spawn_stamps_the_archetype() {
    let mut world = EcsMaster::new();
    advance_tick(&mut world, 2);

    let arch = world.create_archetype(&[Ke6Hp::component_id()]);
    let e = world
        .spawn_one(arch, Ke6Hp { hp: 7 })
        .expect("immediate spawn");

    assert_stamped_now(&world, e, "site 1 (create_entity)");
}

/// The stamp tracks the *latest* add, not the first: a second spawn on a later
/// tick moves it forward. Without this, a stamp written once at archetype
/// creation would pass `site_1_...` and still be useless.
#[test]
fn site_1_a_later_spawn_moves_the_stamp_forward() {
    let mut world = EcsMaster::new();
    advance_tick(&mut world, 2);

    let arch = world.create_archetype(&[Ke6Hp::component_id()]);
    let first = world.spawn_one(arch, Ke6Hp { hp: 1 }).expect("first spawn");
    let stamp_after_first = stamp_of(&world, first);

    advance_tick(&mut world, 2);
    let second = world.spawn_one(arch, Ke6Hp { hp: 2 }).expect("second spawn");

    let stamp_after_second = stamp_of(&world, second);
    assert_ne!(
        stamp_after_second, stamp_after_first,
        "a spawn on a later tick must move the stamp",
    );
    assert_stamped_now(&world, second, "site 1 (second spawn)");
}

// =============================================================================
// Write site 2/9 — `create_entity_with_ticks`, via a detach migration
// =============================================================================

/// `EcsMaster::remove_tag` detaches an id and migrates the entity into the
/// smaller archetype through `migrate_entity_detach_ids` →
/// `Archetype::create_entity_with_ticks`. The destination archetype takes a
/// **new row**, so it must be stamped even though the row's per-component
/// `added_tick`s are carried forward unchanged from the source.
///
/// That divergence is the point: the stamp records when the row entered THIS
/// archetype, and is deliberately newer than the ticks it summarises.
#[test]
fn site_2_detach_migration_stamps_the_destination_archetype() {
    let mut world = EcsMaster::new();
    advance_tick(&mut world, 2);

    let tag = world.register_tag("ke6_detach_tag");
    let arch = world.create_archetype(&[Ke6Hp::component_id()]);
    let e = world.spawn_one(arch, Ke6Hp { hp: 3 }).expect("spawn");
    world.add_tag(e, tag);

    advance_tick(&mut world, 2);
    world.remove_tag(e, tag);

    assert_stamped_now(&world, e, "site 2 (create_entity_with_ticks)");
}

// =============================================================================
// Write site 4/9 — `materialize_clone_into`, via `clone_and_spawn`
// =============================================================================

#[test]
fn site_4_clone_and_spawn_stamps_the_target_archetype() {
    let mut world = EcsMaster::new();
    advance_tick(&mut world, 2);

    let arch = world.create_archetype(&[Ke6Hp::component_id()]);
    let src = world.spawn_one(arch, Ke6Hp { hp: 11 }).expect("spawn source");

    advance_tick(&mut world, 2);
    let clone = world.clone_and_spawn(src);

    assert_stamped_now(&world, clone, "site 4 (materialize_clone_into)");
}

// =============================================================================
// Write site 5/9 — `Prefab::instantiate`
// =============================================================================

#[test]
fn site_5_prefab_instantiate_stamps_the_target_archetype() {
    let mut world = EcsMaster::new();
    advance_tick(&mut world, 2);

    let arch = world.create_archetype(&[Ke6Hp::component_id()]);
    let src = world.spawn_one(arch, Ke6Hp { hp: 21 }).expect("spawn source");
    let prefab = world.capture_prefab(src);

    advance_tick(&mut world, 2);
    let instance = world.instantiate(&prefab);

    assert_stamped_now(&world, instance, "site 5 (Prefab::instantiate)");
}

// =============================================================================
// Write site 6/9 — `migrate_entity_insert`, via `EntityCommands::insert`
// =============================================================================

/// The raw-pointer write site. `migrate_entity_insert` advances
/// `current_index` through `addr_of_mut!` rather than a place-assign so the
/// interior-mutable slab cell is not narrowed to a persistent `Unique` tag; the
/// stamp beside it uses `Archetype::stamp_arch_added_raw` for the same reason.
/// If that raw twin wrote to the wrong offset — the failure mode a `&mut`-based
/// store cannot have — this test is what catches it.
#[test]
fn site_6_deferred_insert_stamps_the_target_archetype() {
    let mut world = EcsMaster::new();
    advance_tick(&mut world, 2);

    let arch = world.create_archetype(&[Ke6Hp::component_id()]);
    let e = world.spawn_one(arch, Ke6Hp { hp: 5 }).expect("spawn");
    let source_arch = world
        .entity_archetype_id(e)
        .expect("live entity has an archetype");

    let mut builder = ScheduleBuilder::new(serial_pool());
    builder.add_system(move |mut commands: Commands| {
        commands.entity(e).insert(Ke6VelOnly {
            vel: Ke6Vel { dx: 1.0 },
        });
    });
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    let target_arch = world
        .entity_archetype_id(e)
        .expect("entity survived the migration");
    assert_ne!(
        target_arch, source_arch,
        "the insert must have migrated the entity to a wider archetype — \
         otherwise this test pins the source archetype's stamp by accident",
    );

    // The apply window bumps the tick once past the frame's `this_run`, so the
    // stamp is the tick the command applied at, which is the world's tick now.
    assert_stamped_now(&world, e, "site 6 (migrate_entity_insert, raw twin)");
}

// =============================================================================
// Write site 7/9 — `migrate_entity_attach_ids`, via `add_tag`
// =============================================================================

#[test]
fn site_7_add_tag_stamps_the_destination_archetype() {
    let mut world = EcsMaster::new();
    advance_tick(&mut world, 2);

    let tag = world.register_tag("ke6_attach_tag");
    let arch = world.create_archetype(&[Ke6Hp::component_id()]);
    let e = world.spawn_one(arch, Ke6Hp { hp: 4 }).expect("spawn");
    let source_arch = world.entity_archetype_id(e).expect("live");

    advance_tick(&mut world, 2);
    world.add_tag(e, tag);

    let target_arch = world.entity_archetype_id(e).expect("live after attach");
    assert_ne!(target_arch, source_arch, "the tag attach migrated the entity");
    assert_stamped_now(&world, e, "site 7 (migrate_entity_attach_ids)");
}

// =============================================================================
// Write site 8/9 — `SpawnAtCommand::apply`, via `Commands::spawn`
// =============================================================================

#[test]
fn site_8_deferred_spawn_stamps_the_archetype() {
    let mut world = EcsMaster::new();
    advance_tick(&mut world, 2);

    let mut builder = ScheduleBuilder::new(serial_pool());
    builder.add_system(|mut commands: Commands| {
        commands.spawn(Ke6Body {
            hp: Ke6Hp { hp: 9 },
            pos: Ke6Pos { x: 1.0 },
        });
    });
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    // Find the spawned entity through the query API (its id is not returned
    // across the schedule boundary here).
    let entities = world.query_entities(&[Ke6Hp::component_id(), Ke6Pos::component_id()]);
    assert_eq!(entities.len(), 1, "exactly one deferred spawn landed");

    assert_stamped_now(&world, entities[0], "site 8 (SpawnAtCommand::apply)");
}

// =============================================================================
// Write site 9/9 — `SpawnBatchCommand::apply`, via `EcsMaster::spawn_batch`
// =============================================================================

/// The spawn-burst case the item names as its motivation: one stamp store for
/// the whole batch, because every row in it shares one `current_tick`.
#[test]
fn site_9_spawn_batch_stamps_the_archetype_once_for_the_whole_batch() {
    let mut world = EcsMaster::new();
    advance_tick(&mut world, 2);

    let spawned = world
        .spawn_batch((0..64u32).map(|i| Ke6Body {
            hp: Ke6Hp { hp: i },
            pos: Ke6Pos { x: i as f32 },
        }))
        .expect("batch spawn");
    assert_eq!(spawned.len(), 64);

    let stamp = stamp_of(&world, spawned[0]);
    assert_stamped_now(&world, spawned[0], "site 9 (SpawnBatchCommand::apply)");
    assert_eq!(
        stamp_of(&world, spawned[63]),
        stamp,
        "one archetype, one stamp — the last row of the batch reads the same \
         value as the first",
    );
}

// =============================================================================
// Negative control — a removal must NOT advance the stamp
// =============================================================================

/// Over-stamping is cheap but not free, and a stamp bumped by *removals* would
/// be permanently dirty in any world with churn — which is the same as having
/// no summary at all. Row removals write no `added_tick`, so they must write no
/// stamp either.
///
/// Without this control, "stamp on every archetype mutation" would pass all
/// eight site tests above.
#[test]
fn a_despawn_does_not_advance_the_stamp() {
    let mut world = EcsMaster::new();
    advance_tick(&mut world, 2);

    let arch = world.create_archetype(&[Ke6Hp::component_id()]);
    let keep = world.spawn_one(arch, Ke6Hp { hp: 1 }).expect("spawn keep");
    let doomed = world.spawn_one(arch, Ke6Hp { hp: 2 }).expect("spawn doomed");
    let stamp_before = stamp_of(&world, keep);

    advance_tick(&mut world, 3);
    assert_ne!(
        world.current_tick(),
        stamp_before,
        "the tick moved, so a spurious re-stamp would be visible",
    );

    assert!(world.delete_entity(doomed), "despawn succeeds");

    assert_eq!(
        stamp_of(&world, keep),
        stamp_before,
        "a removal writes no added_tick and must write no ArchAdded stamp",
    );
}

/// `has_structural_add_since` is the consumer predicate: `false` is the strong
/// answer ("no table row here was added in this window"), `true` is "maybe".
/// Pinned on a real archetype across a window that does and does not contain
/// the add.
#[test]
fn has_structural_add_since_answers_the_window() {
    let mut world = EcsMaster::new();
    advance_tick(&mut world, 2);

    let arch_id = world.create_archetype(&[Ke6Hp::component_id()]);
    let e = world.spawn_one(arch_id, Ke6Hp { hp: 1 }).expect("spawn");
    let spawn_tick = world.current_tick();
    let arch: &Archetype = world
        .archetype_master()
        .get_archetype(world.entity_archetype_id(e).expect("live"))
        .expect("resolves");

    // A window that contains the spawn tick.
    assert!(
        arch.has_structural_add_since(
            Tick::new(spawn_tick.get().wrapping_sub(1)),
            spawn_tick
        ),
        "the spawn tick is inside (spawn - 1, spawn]",
    );

    // A window that opens AT the spawn tick — the bound is exclusive below, so
    // the add is outside it.
    assert!(
        !arch.has_structural_add_since(spawn_tick, Tick::new(spawn_tick.get() + 5)),
        "the window (spawn, spawn + 5] excludes the add itself",
    );
}
