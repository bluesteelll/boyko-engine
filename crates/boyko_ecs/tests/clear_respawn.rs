//! Regression suite — stale bundle-archetype cache across `EcsMaster::clear()`.
//!
//! Discovered during Phase X.G testing (the FreshBundle / MFreshBundle
//! workarounds, now removed): `clear()` reset the archetype registry
//! (`next_archetype_id` rolls back to `ArchetypeId(1)`) but left the
//! per-world bundle caches (`bundle_archetype_cache` +
//! `bundle_column_cache`) holding pre-clear `ArchetypeId`s /
//! `InlandPoolId`s. Respawning a bundle type used before the clear then
//! panicked ("cached_archetype_id returned an unregistered id" class) or
//! aliased a wrong archetype. The fix resets both caches inside `clear()`.
//!
//! Multi-world half: the caches are world-owned fields initialised fresh by
//! `EcsMaster::new()`, so a second world never observed another world's
//! cached ids even before the fix — `second_world_same_bundle_type` pins
//! that this stays true (it is the opening invariant of the multi-world
//! direction).

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

// ── spawn_batch path (SpawnBatchCommand::apply → bundle_column_cache) ───────

#[derive(Component)]
#[repr(C)]
struct BatchPayload {
    value: u64,
}

#[derive(Bundle)]
struct BatchBundle {
    payload: BatchPayload,
}

/// THE regression: spawn bundle B via `spawn_batch`, `clear()`, respawn the
/// SAME bundle type. Pre-fix this panicked because the per-world bundle
/// caches survived the clear while the archetype registry did not. Two
/// clear/respawn cycles verify the reset is repeatable.
#[test]
fn respawn_same_bundle_after_clear_spawn_batch() {
    let mut world = EcsMaster::new();

    for cycle in 0..2u64 {
        let base = cycle * 100_000;
        let spawned = world
            .spawn_batch((0..100u32).map(move |i| BatchBundle {
                payload: BatchPayload { value: base + i as u64 },
            }))
            .expect("pre-clear spawn");
        assert_eq!(spawned.len(), 100);
        for (i, &e) in spawned.iter().enumerate() {
            let v = world
                .get_component::<BatchPayload>(e)
                .expect("pre-clear entity readable");
            assert_eq!(v.value, base + i as u64);
        }

        world.clear();

        // Old handles must be dead (checked BEFORE respawn — post-clear
        // entity ids restart, so a fresh entity may numerically collide
        // with a stale handle by design).
        for &e in &[spawned[0], spawned[99]] {
            assert!(
                world.get_component::<BatchPayload>(e).is_none(),
                "stale entity survived clear()"
            );
        }
    }

    // Final respawn after the last clear: must not panic, must land in a
    // valid (freshly registered) archetype, components readable.
    let fresh = world
        .spawn_batch((0..100u32).map(|i| BatchBundle {
            payload: BatchPayload { value: 1_000_000 + i as u64 },
        }))
        .expect("post-clear respawn of the pre-clear bundle type");
    for (i, &e) in fresh.iter().enumerate() {
        let v = world
            .get_component::<BatchPayload>(e)
            .expect("post-clear entity readable");
        assert_eq!(v.value, 1_000_000 + i as u64);
    }
}

// ── Commands path (SpawnAtCommand::apply → bundle_archetype_cache) ──────────

#[derive(Component)]
#[repr(C)]
struct CmdPayload {
    value: u64,
}

#[derive(Bundle)]
struct CmdBundle {
    payload: CmdPayload,
}

/// Same regression through the single-spawn deferred-command path
/// (`Commands::spawn` → `SpawnAtCommand::apply` →
/// `B::cached_archetype_id(world)` → `bundle_archetype_cache`).
#[test]
fn respawn_same_bundle_after_clear_commands_spawn() {
    let mut world = EcsMaster::new();

    let pre = world.run_system(|mut cmds: Commands| {
        cmds.spawn(CmdBundle { payload: CmdPayload { value: 7 } }).id()
    });
    assert_eq!(
        world
            .get_component::<CmdPayload>(pre)
            .expect("pre-clear entity readable")
            .value,
        7
    );

    world.clear();
    assert!(
        world.get_component::<CmdPayload>(pre).is_none(),
        "stale entity survived clear()"
    );

    let post = world.run_system(|mut cmds: Commands| {
        cmds.spawn(CmdBundle { payload: CmdPayload { value: 9 } }).id()
    });
    let v = world
        .get_component::<CmdPayload>(post)
        .expect("post-clear respawn of the pre-clear bundle type readable");
    assert_eq!(v.value, 9);
}

// ── Multi-world pin ──────────────────────────────────────────────────────────

#[derive(Component)]
#[repr(C)]
struct MwPayload {
    value: u64,
}

#[derive(Bundle)]
struct MwBundle {
    payload: MwPayload,
}

/// Multi-world pin: the bundle caches are world-owned (`EcsMaster::new()`
/// initialises them fresh), so a second world resolving the SAME bundle type
/// must land in its own archetype registry — never in the first world's.
/// This case was already correct before the clear() fix; the test prevents a
/// future regression to a process-global cache shape.
#[test]
fn second_world_same_bundle_type() {
    // World A warms the per-type process statics (BundleTypeId,
    // component_ids) AND its own per-world caches.
    let mut world_a = EcsMaster::new();
    let a = world_a
        .spawn_batch((0..50u32).map(|i| MwBundle { payload: MwPayload { value: i as u64 } }))
        .expect("world A spawn");

    // World B starts cold per-world; same bundle type must cold-resolve
    // against B's OWN registry, not reuse A's cached ArchetypeId.
    let mut world_b = EcsMaster::new();
    let b = world_b
        .spawn_batch(
            (0..50u32).map(|i| MwBundle { payload: MwPayload { value: 1_000 + i as u64 } }),
        )
        .expect("world B spawn of a bundle type already cached in world A");

    for (i, &e) in b.iter().enumerate() {
        let v = world_b
            .get_component::<MwPayload>(e)
            .expect("world B entity readable");
        assert_eq!(v.value, 1_000 + i as u64);
    }
    // World A is untouched by B's resolution.
    for (i, &e) in a.iter().enumerate() {
        let v = world_a
            .get_component::<MwPayload>(e)
            .expect("world A entity readable after B used the same bundle type");
        assert_eq!(v.value, i as u64);
    }

    // Clearing A must not disturb B (cache reset is strictly per-world).
    world_a.clear();
    let a2 = world_a
        .spawn_batch((0..10u32).map(|i| MwBundle { payload: MwPayload { value: 2_000 + i as u64 } }))
        .expect("world A respawn after clear");
    for (i, &e) in a2.iter().enumerate() {
        assert_eq!(
            world_a
                .get_component::<MwPayload>(e)
                .expect("world A post-clear entity readable")
                .value,
            2_000 + i as u64
        );
    }
    for (i, &e) in b.iter().enumerate() {
        assert_eq!(
            world_b
                .get_component::<MwPayload>(e)
                .expect("world B entity readable after world A cleared")
                .value,
            1_000 + i as u64
        );
    }
}
