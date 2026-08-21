//! Phase X.G integration suite â€” `entities_inland` address-stable growth.
//!
//! I1: spawning across the 9192-anchored growth thresholds (the old Vec
//!     doubling chain) keeps every handle valid and every component readable.
//! I2: XG-B6 â€” the no-memcpy growth witness: a slot's ADDRESS is stable
//!     across multi-slab growth (impossible with `Vec`).
//! I3: world `clear()` + respawn exposes no stale liveness (the D5 memset).

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_macros::{Bundle, Component};

#[derive(Component)]
#[repr(C)]
struct GrowthPayload {
    value: u64,
}

#[derive(Bundle)]
struct GrowthBundle {
    payload: GrowthPayload,
}

/// I1 â€” 100k entities via `spawn_batch` in 1,000-entity sub-batches: crosses
/// several former Vec-doubling thresholds (9192-anchored chain: 9192, 18384,
/// 36768, 73536) with the X.G store. Every spawned handle must stay valid and
/// its component readable with the right value.
#[test]
#[cfg_attr(miri, ignore = "100k spawns - minutes under Miri; M1/M2 cover the store there")]
fn i1_spawn_across_growth_thresholds() {
    let mut world = EcsMaster::new();
    let mut all = Vec::with_capacity(100_000);

    for batch in 0..100u64 {
        let base = batch * 1_000;
        let entities = world
            .spawn_batch((0..1_000u32).map(move |i| GrowthBundle {
                payload: GrowthPayload { value: base + i as u64 },
            }))
            .expect("spawn_batch");
        all.extend(entities);
    }

    assert_eq!(all.len(), 100_000);
    // Spot-check around every former doubling threshold + dense head/tail.
    for &idx in &[0usize, 1, 9_191, 9_192, 18_383, 18_384, 36_767, 36_768, 73_535, 73_536, 99_999]
    {
        let e = all[idx];
        let v = world
            .get_component::<GrowthPayload>(e)
            .unwrap_or_else(|| panic!("entity #{idx} lost its component after growth"));
        assert_eq!(v.value, idx as u64, "wrong payload at #{idx}");
    }
}

/// I2 (XG-B6, public-API half) â€” growth is commit-frontier, not realloc:
/// `committed_slots` advances monotonically across slab boundaries while an
/// early entity stays readable with its exact payload. The literal
/// slot-ADDRESS witness (impossible to express with `Vec`) lives in-crate:
/// `inland_store::tests::addresses_stable_across_multi_slab_growth` (U-S2)
/// and `entity_master::tests::xg_b6_slot_address_stable_across_growth`.
#[test]
#[cfg_attr(miri, ignore = "70k spawns; the Miri half is miri_entity_store.rs")]
fn i2_commit_frontier_growth_with_early_entity_intact() {
    let mut world = EcsMaster::new();

    let first = world
        .spawn_batch(std::iter::once(GrowthBundle { payload: GrowthPayload { value: 7 } }))
        .expect("spawn first")[0];
    let committed_at_start = world.entity_master().committed_slots();

    // Grow past several slab boundaries (>= 64k slots > 256KiB-slab capacity).
    for batch in 0..70u64 {
        world
            .spawn_batch((0..1_000u32).map(move |i| GrowthBundle {
                payload: GrowthPayload { value: batch * 1_000 + i as u64 },
            }))
            .expect("growth batch");
    }

    let committed_at_end = world.entity_master().committed_slots();
    assert!(
        committed_at_end > committed_at_start && committed_at_end >= 70_000,
        "commit frontier did not advance across growth ({committed_at_start} -> {committed_at_end})"
    );
    // The early record still resolves with its exact payload.
    let v = world.get_component::<GrowthPayload>(first).expect("first entity still readable");
    assert_eq!(v.value, 7);
}

/// I3 â€” `clear()` + respawn: old handles invalid, fresh handles (generation
/// 0, recycled id space re-zeroed by the D5 memset) fully functional. Guards
/// the stale-bytes hazard end-to-end.
#[test]
#[cfg_attr(miri, ignore = "50k spawns; the Miri clear/regrow half is miri_entity_store.rs")]
fn i3_clear_respawn_no_stale_liveness() {
    let mut world = EcsMaster::new();

    let mut old = Vec::with_capacity(20_000);
    for batch in 0..20u64 {
        let base = batch * 1_000;
        old.extend(
            world
                .spawn_batch((0..1_000u32).map(move |i| GrowthBundle {
                    payload: GrowthPayload { value: base + i as u64 },
                }))
                .expect("first population"),
        );
    }
    assert_eq!(old.len(), 20_000);

    world.clear();

    // Every old handle must be dead.
    for &e in &[old[0], old[9_999], old[19_999]] {
        assert!(
            world.get_component::<GrowthPayload>(e).is_none(),
            "stale entity survived clear()"
        );
    }

    // Respawn a smaller, then a larger population (regrow past the old
    // high-water on the SECOND cycle â€” the two-cycle invariant-J case).
    // Reuses the SAME bundle type as the pre-clear population: the stale
    // bundle-archetype cache across `clear()` is fixed (clear() resets the
    // per-world bundle caches), so the former FreshBundle workaround is gone.
    // The dedicated regression suite is tests/clear_respawn.rs.
    let mut fresh = Vec::with_capacity(30_000);
    for batch in 0..30u64 {
        let base = 1_000_000 + batch * 1_000;
        fresh.extend(
            world
                .spawn_batch((0..1_000u32).map(move |i| GrowthBundle {
                    payload: GrowthPayload { value: base + i as u64 },
                }))
                .expect("respawn"),
        );
    }
    for &idx in &[0usize, 19_999, 20_000, 29_999] {
        let v = world
            .get_component::<GrowthPayload>(fresh[idx])
            .unwrap_or_else(|| panic!("fresh entity #{idx} unreadable after clear+respawn"));
        assert_eq!(v.value, 1_000_000 + idx as u64);
    }
}

