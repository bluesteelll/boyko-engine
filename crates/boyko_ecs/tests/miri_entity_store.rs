//! Phase X.G — Miri (Tree Borrows) coverage for the `InlandStore` fallback
//! arm (the arm Miri actually compiles: eager `alloc_zeroed` reserve, commit
//! = watermark bump).
//!
//! M1 lives as in-crate unit tests (`inland_store.rs` — the store type is
//! `pub(crate)`); THIS file drives the store through the public
//! `EcsMaster` surface so Miri validates the whole wiring: ensure-growth
//! during spawn, reads of never-program-written slots (the
//! initialized-zero position), clear + regrow, recycle/generation churn.
//!
//! Run:
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-ecs --test miri_entity_store
//! ```
//! `-Zmiri-ignore-leaks` is required: `spawn_batch` routes through the
//! command machinery whose bounded `Box::leak` (#53, triaged NOT-A-BUG in
//! the post-14b backlog cleanup) trips the exit leak-checker — the same
//! known class as the miri_phase8a/8cd/8_5/14a suites (PHASE-XF-RESULTS.md).
#![cfg(miri)]

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_macros::{Bundle, Component};

#[derive(Component)]
#[repr(C)]
struct MPayload {
    value: u32,
}

#[derive(Bundle)]
struct MBundle {
    payload: MPayload,
}

/// M2 — EntityMaster churn through the public API on the fallback arm:
/// spawn (ensure-growth) → read → despawn (recycle) → respawn (generation
/// bump) → clear → regrow. Small counts (Miri is ~100× slower).
#[test]
fn m2_entity_churn_on_fallback_arm() {
    let mut world = EcsMaster::new();

    // Spawn growth: the store starts len 0 and ensures lazily.
    let first = world
        .spawn_batch((0..50u32).map(|i| MBundle { payload: MPayload { value: i } }))
        .expect("spawn batch");
    assert_eq!(first.len(), 50);
    for (i, &e) in first.iter().enumerate() {
        let v = world.get_component::<MPayload>(e).expect("component readable");
        assert_eq!(v.value, i as u32);
    }

    // Despawn a few → recycle ids with bumped generations.
    for &e in &first[..10] {
        assert!(world.delete_entity(e), "despawn must succeed");
    }
    for &e in &first[..10] {
        assert!(world.get_component::<MPayload>(e).is_none(), "stale read after despawn");
    }

    // Respawn into recycled slots: old handles must stay dead (generation).
    let recycled = world
        .spawn_batch((0..10u32).map(|i| MBundle { payload: MPayload { value: 100 + i } }))
        .expect("respawn");
    for &e in &recycled {
        let v = world.get_component::<MPayload>(e).expect("recycled entity readable");
        assert!(v.value >= 100);
    }
    for &e in &first[..10] {
        assert!(
            world.get_component::<MPayload>(e).is_none(),
            "old generation resolved after recycle"
        );
    }

    // Clear + regrow (the D5 memset under Miri) — fresh world semantics.
    // Reuses the SAME bundle type as before the clear: the stale
    // bundle-archetype cache across `clear()` is fixed (clear() resets the
    // per-world bundle caches), so the former MFreshBundle workaround is
    // gone. Dedicated regression suite: tests/clear_respawn.rs.
    world.clear();
    let fresh = world
        .spawn_batch((0..60u32).map(|i| MBundle { payload: MPayload { value: 1000 + i } }))
        .expect("regrow after clear");
    for (i, &e) in fresh.iter().enumerate() {
        let v = world.get_component::<MPayload>(e).expect("fresh entity readable");
        assert_eq!(v.value, 1000 + i as u32);
    }
}
