//! KE2 (Aether v2 rung R1) — the `Entities<'w>` read-only `SystemParam`.
//!
//! # What the row asks for
//!
//! `docs/aether-v2/KERNEL-BACKLOG.md` KE2: a read-only param exposing
//! `get(EntityId) -> Option<Entity>`, whose carrier pointer is
//! `*const InlandStore` — **not** `*const EntityMaster`. The narrowness is
//! type-enforced, following the `EntityCounter` precedent (`*const AtomicUsize`,
//! so no compile-time path reaches a sibling `EntityMaster` field).
//!
//! # Why it exists
//!
//! Machines need `me: Entity`. Query iteration yields `EntityId` (no
//! generation); events carry `Entity` (with generation). Before this param the
//! only resolver — `EntityMaster::get_entity` — sat behind `&EcsMaster`, i.e.
//! behind an **exclusive** system. A per-entity machine cannot be exclusive.
//!
//! # The generation is the whole point
//!
//! `EntityId` alone is ambiguous across a despawn/respawn cycle: the id is
//! recycled and the generation bumped. `resolve_id_after_recycle_carries_the_
//! bumped_generation` is the test that would let a naive
//! `Entity::new(id, 0)` implementation through if it were absent.
//!
//! # Access declaration
//!
//! `Entities` declares NO component/resource access — it reads the entity fast
//! store, an axis the `FilteredAccessSet` does not model at all (same footing as
//! `Commands`, whose EM6 counter projection declares nothing either). The
//! `entities_param_declares_no_conflicting_access` test pins that: a system
//! taking `Entities` alongside a `ResMut` and a mutable `Query` must build and
//! run, not trip the B0002 intra-system conflict panic.
//!
//! The in-crate unit tests (`system/params/entities.rs`) carry the size/`Send`/
//! `Sync` contract and the SCH7 debug-assert that mirrors the writer guard.

// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::{Commands, Entities, ResMut};
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::{Bundle, Component, Resource};

// ── Fixtures ────────────────────────────────────────────────────────────────

/// Payload component — its `p` is the identity the oracles are written in.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct KE2Payload {
    p: u32,
}

/// Second component, so a mutable query can be declared alongside `Entities`
/// in the access-declaration test.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct KE2Other {
    v: u32,
}

#[derive(Bundle)]
struct KE2Bundle {
    p: KE2Payload,
    o: KE2Other,
}

/// Carries the entity handle under test into the reader system.
#[derive(Resource)]
struct KE2Target(Entity);

/// Collects the reader system's answer.
#[derive(Resource)]
struct KE2Answer(Option<Entity>);

// ── Tests ───────────────────────────────────────────────────────────────────

/// A live entity resolves to a handle equal to the one `spawn` handed out —
/// same id AND same generation.
#[test]
fn resolve_live_entity_round_trips_the_spawn_handle() {
    let mut world = EcsMaster::new();

    let spawned = world.run_system(|mut cmds: Commands| {
        cmds.spawn(KE2Bundle { p: KE2Payload { p: 1 }, o: KE2Other { v: 1 } })
            .id()
    });

    world.insert_resource(KE2Target(spawned));
    world.insert_resource(KE2Answer(None));
    world.run_system(|ents: Entities, target: ResMut<KE2Target>, mut out: ResMut<KE2Answer>| {
        out.0 = ents.get(target.0.id());
    });

    assert_eq!(
        world.resource::<KE2Answer>().0,
        Some(spawned),
        "Entities::get must resolve a live id to the handle spawn returned \
         (id AND generation)"
    );
}

/// An id that was never allocated resolves to `None` — not to a zero-generation
/// handle, and not to a panic.
#[test]
fn resolve_never_allocated_id_is_none() {
    let mut world = EcsMaster::new();
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(KE2Bundle { p: KE2Payload { p: 1 }, o: KE2Other { v: 1 } });
    });

    world.insert_resource(KE2Answer(None));
    world.run_system(|ents: Entities, mut out: ResMut<KE2Answer>| {
        // Far past the fast store's live range.
        out.0 = ents.get(EntityId(9_999));
    });

    assert_eq!(
        world.resource::<KE2Answer>().0,
        None,
        "an id past the fast store's live range must resolve to None"
    );
}

/// A despawned entity's slot is dead: the id resolves to `None` while it sits
/// on the free list unregistered.
#[test]
fn resolve_despawned_entity_is_none() {
    let mut world = EcsMaster::new();

    let spawned = world.run_system(|mut cmds: Commands| {
        cmds.spawn(KE2Bundle { p: KE2Payload { p: 2 }, o: KE2Other { v: 2 } })
            .id()
    });
    world.run_system(move |mut cmds: Commands| {
        cmds.despawn(spawned);
    });

    world.insert_resource(KE2Target(spawned));
    world.insert_resource(KE2Answer(None));
    world.run_system(|ents: Entities, target: ResMut<KE2Target>, mut out: ResMut<KE2Answer>| {
        out.0 = ents.get(target.0.id());
    });

    assert_eq!(
        world.resource::<KE2Answer>().0,
        None,
        "a despawned id must resolve to None — its inland slot is null"
    );
}

/// The load-bearing case. After despawn + respawn the id is RECYCLED with a
/// bumped generation. `get` must report the CURRENT generation, so a caller
/// holding the pre-despawn handle can tell the two apart.
///
/// An implementation that returned `Entity::new(id, 0)` — or that echoed a
/// caller-supplied generation — passes every other test in this file and fails
/// only here.
///
/// # Why the direct `EcsMaster` path and not `Commands`
///
/// `Commands::spawn` mints its id through `EntityCounter::reserve_entity`,
/// which by EM2 **never pops the free list** — recycling is dispatcher-only. A
/// `Commands`-driven despawn/respawn therefore hands out a FRESH id and this
/// test would be vacuous. (Measured: the fixture precondition below caught
/// exactly that on the first draft.) `spawn_one` / `delete_entity` go through
/// `EntityMaster::allocate_entity`, which is the recycling path.
#[test]
fn resolve_id_after_recycle_carries_the_bumped_generation() {
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[<KE2Payload as boyko_ecs::ecs::core::component::component::Component>::component_id()]);

    let first = world
        .spawn_one(arch, KE2Payload { p: 3 })
        .expect("spawn_one must succeed");
    assert!(world.delete_entity(first), "delete must succeed");
    let second = world
        .spawn_one(arch, KE2Payload { p: 4 })
        .expect("respawn must succeed");

    assert_eq!(
        second.id(),
        first.id(),
        "fixture precondition: the despawned id must have been recycled \
         (LIFO free list) — otherwise this test is vacuous"
    );
    assert_ne!(
        second.generation(),
        first.generation(),
        "fixture precondition: recycling must bump the generation"
    );

    world.insert_resource(KE2Target(first));
    world.insert_resource(KE2Answer(None));
    world.run_system(|ents: Entities, target: ResMut<KE2Target>, mut out: ResMut<KE2Answer>| {
        // Ask with the STALE handle's id.
        out.0 = ents.get(target.0.id());
    });

    let resolved = world.resource::<KE2Answer>().0;
    assert_eq!(
        resolved,
        Some(second),
        "Entities::get must resolve the recycled id to the CURRENT occupant, \
         generation and all"
    );
    assert_ne!(
        resolved,
        Some(first),
        "the stale pre-despawn handle must NOT round-trip — that is the whole \
         reason `me` carries a generation"
    );
}

/// `Entities` declares no component/resource access, so it composes with an
/// arbitrary mutable query and a `ResMut` in the same system without tripping
/// the B0002 intra-system conflict panic.
///
/// A param that over-declared (e.g. a blanket component read) would panic here
/// at `init_access` time, not return a wrong answer — but it would still be a
/// scheduler-visible defect, so it is pinned.
#[test]
fn entities_param_declares_no_conflicting_access() {
    let mut world = EcsMaster::new();

    let spawned = world.run_system(|mut cmds: Commands| {
        cmds.spawn(KE2Bundle { p: KE2Payload { p: 5 }, o: KE2Other { v: 5 } })
            .id()
    });

    world.insert_resource(KE2Target(spawned));
    world.insert_resource(KE2Answer(None));
    let seen = world.run_system(
        |ents: Entities,
         target: ResMut<KE2Target>,
         mut out: ResMut<KE2Answer>,
         mut q: Query<&mut KE2Other>| {
            for o in &mut q {
                o.v += 1;
            }
            out.0 = ents.get(target.0.id());
            out.0
        },
    );

    assert_eq!(
        seen,
        Some(spawned),
        "Entities alongside ResMut + Query<&mut _> must build, run, and resolve"
    );
}
