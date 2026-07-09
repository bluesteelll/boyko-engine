//! Phase 12.5 Track A — `spawn_batch` integration tests (plan §5.11).
//!
//! Covers the smoke contract for `Commands::spawn_batch` and
//! `EcsMaster::spawn_batch` end-to-end across:
//!
//! * one-component and three-component bundles (`apply` row-write loop +
//!   per-pool capacity reserve).
//! * empty-iter no-op (`n == 0` early exit).
//! * `SpawnBatchIter` drop-without-consume preserves the spawn (SBO8b — I-N2).
//! * cap-overrun (`MAX_BATCH_HINT + 1`) returns `Err` without advancing
//!   the counter (SBO17).
//! * world-capacity overshoot via the direct path returns `Err`
//!   (W2 + I-N1).
//! * `EcsMaster::spawn_batch` routes through `reserve_batch`, NOT through
//!   a direct `fetch_add` on `next_entity_id` (C-N2 grep).
//! * `BundleColumnCache::pool_ids` is canonical-sorted (W4 install-time
//!   debug assertion).
//!
//! # Component-slot range
//!
//! 360..=379 — disjoint from every prior phase's allocations (Phase 11:
//! 411-413; Phase 8.5 derive_bundle: 290-309) and below `MAX_COMPONENTS = 512`.
//! (The original Phase 12.5 commit used 700+ which exceeded MAX_COMPONENTS.)

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::error::EcsError;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

// ── Component slots ──────────────────────────────────────────────────────────

const SLOT_POS: ComponentId = ComponentId(360);
const SLOT_VEL: ComponentId = ComponentId(361);
const SLOT_HEALTH: ComponentId = ComponentId(362);

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Velocity {
    x: f32,
    y: f32,
    z: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Health(i32);

impl Component for Position {
    fn component_id() -> ComponentId {
        SLOT_POS
    }
}
impl Component for Velocity {
    fn component_id() -> ComponentId {
        SLOT_VEL
    }
}
impl Component for Health {
    fn component_id() -> ComponentId {
        SLOT_HEALTH
    }
}

fn register_components() {
    register_layout::<Position>(SLOT_POS.0);
    register_layout::<Velocity>(SLOT_VEL.0);
    register_layout::<Health>(SLOT_HEALTH.0);
}

#[derive(Bundle)]
struct PosBundle {
    pos: Position,
}

#[derive(Bundle)]
struct PosVelHealth {
    pos: Position,
    vel: Velocity,
    health: Health,
}

// ── Smoke: one-component batch ──────────────────────────────────────────────

#[test]
fn spawn_batch_one_thousand_one_component() {
    register_components();
    let mut ecs = EcsMaster::new();
    ecs.run_system(|mut cmds: Commands| {
        let _ = cmds
            .spawn_batch((0..1_000).map(|i| PosBundle {
                pos: Position {
                    x: i as f32,
                    y: 0.0,
                    z: 0.0,
                },
            }))
            .expect("1000 ≤ MAX_BATCH_HINT");
    });
    assert_eq!(
        ecs.entity_count(),
        1_000,
        "spawn_batch should land 1000 entities after apply"
    );
}

// ── Smoke: three-component batch ────────────────────────────────────────────

#[test]
fn spawn_batch_three_component_bundle() {
    register_components();
    let mut ecs = EcsMaster::new();
    ecs.run_system(|mut cmds: Commands| {
        let _ = cmds
            .spawn_batch((0..1_000).map(|i| PosVelHealth {
                pos: Position {
                    x: i as f32,
                    y: 0.0,
                    z: 0.0,
                },
                vel: Velocity {
                    x: 0.0,
                    y: i as f32,
                    z: 0.0,
                },
                health: Health(i),
            }))
            .expect("1000 ≤ MAX_BATCH_HINT");
    });
    assert_eq!(ecs.entity_count(), 1_000);
}

// ── Empty-iter is a no-op (n == 0 early exit) ───────────────────────────────

#[test]
fn spawn_batch_empty_iter_is_noop() {
    register_components();
    let mut ecs = EcsMaster::new();
    let before_counter = ecs.entity_master().next_entity_id();
    ecs.run_system(|mut cmds: Commands| {
        let _ = cmds
            .spawn_batch(std::iter::empty::<PosBundle>())
            .expect("empty iter must succeed");
    });
    assert_eq!(ecs.entity_count(), 0);
    let after_counter = ecs.entity_master().next_entity_id();
    assert_eq!(
        before_counter.0, after_counter.0,
        "empty batch must not advance the entity counter"
    );
}

// ── Drop-without-consume still spawns (SBO8b / I-N2) ────────────────────────

#[test]
fn spawn_batch_iter_drop_without_consume_still_spawns() {
    register_components();
    let mut ecs = EcsMaster::new();
    ecs.run_system(|mut cmds: Commands| {
        // Drop the returned iter immediately — the spawn must still
        // happen at apply time.
        let _ = cmds
            .spawn_batch((0..100).map(|i| PosBundle {
                pos: Position {
                    x: i as f32,
                    y: 0.0,
                    z: 0.0,
                },
            }))
            .expect("100 ≤ MAX_BATCH_HINT");
        // Iter dropped here; SpawnBatchCommand remains enqueued.
    });
    assert_eq!(
        ecs.entity_count(),
        100,
        "drop-without-consume must not cancel the spawn"
    );
}

// ── Cap overrun returns Err without advancing counter (SBO17) ───────────────

#[test]
fn spawn_batch_exceeds_max_batch_hint_returns_err() {
    register_components();
    let mut ecs = EcsMaster::new();
    let before = ecs.entity_master().next_entity_id();
    ecs.run_system(|mut cmds: Commands| {
        let result = cmds.spawn_batch((0..8_193).map(|i| PosBundle {
            pos: Position {
                x: i as f32,
                y: 0.0,
                z: 0.0,
            },
        }));
        assert!(matches!(
            result,
            Err(EcsError::SpawnBatchExceedsCapacity { requested: 8_193, max: 8_192 })
        ));
    });
    let after = ecs.entity_master().next_entity_id();
    assert_eq!(
        before.0, after.0,
        "SBO17: counter must not advance on SpawnBatchExceedsCapacity"
    );
}

// ── At-cap batch succeeds (boundary check) ──────────────────────────────────

#[test]
fn spawn_batch_at_max_batch_hint_succeeds() {
    register_components();
    let mut ecs = EcsMaster::new();
    ecs.run_system(|mut cmds: Commands| {
        let _ = cmds
            .spawn_batch((0..8_192).map(|i| PosBundle {
                pos: Position {
                    x: i as f32,
                    y: 0.0,
                    z: 0.0,
                },
            }))
            .expect("8192 == MAX_BATCH_HINT (boundary, inclusive)");
    });
    assert_eq!(ecs.entity_count(), 8_192);
}

// ── Direct path returns Vec<Entity> (W3 ergonomic) ──────────────────────────

#[test]
fn direct_spawn_batch_eager_path() {
    register_components();
    let mut ecs = EcsMaster::new();
    let entities = ecs
        .spawn_batch((0..64).map(|i| PosBundle {
            pos: Position {
                x: i as f32,
                y: 0.0,
                z: 0.0,
            },
        }))
        .expect("64 ≤ MAX_BATCH_HINT");
    assert_eq!(entities.len(), 64);
    assert_eq!(ecs.entity_count(), 64);
    // Every returned entity must be live in the world.
    for entity in entities {
        assert!(ecs.has_entity(entity));
    }
}

// ── EcsMaster::spawn_batch routes through reserve_batch (C-N2 grep) ─────────
//
// Source-level grep regression: `EcsMaster::spawn_batch` MUST NOT poke
// `next_entity_id` directly via `fetch_add`. It MUST route through
// `self.entity_master.reserve_batch(n)` so the EM6 surface stays intact.
//
// We assert this by reading the source file as a string and checking for
// the forbidden pattern. The test is fragile to refactoring; that's by
// design — touching the spawn_batch body should trip this assertion and
// force a re-validation of the C-N2 lock-down.
#[test]
fn ecs_master_spawn_batch_uses_reserve_batch_no_direct_atomic() {
    // The actual source file path is relative to the crate root. Cargo
    // sets `CARGO_MANIFEST_DIR` at test compile time.
    let source = include_str!("../src/ecs/core/ecs_master/ecs_master.rs");
    // Locate the `pub fn spawn_batch` method body.
    let start = source.find("pub fn spawn_batch<B, I>").expect(
        "EcsMaster::spawn_batch signature not found — has it been renamed?",
    );
    // Heuristic: grab the next ~5000 chars (the body fits well within).
    let tail = &source[start..source.len().min(start + 5_000)];
    assert!(
        tail.contains("self.entity_master.reserve_batch"),
        "EcsMaster::spawn_batch must route through entity_master.reserve_batch (C-N2)"
    );
    assert!(
        !tail.contains("next_entity_id.fetch_add"),
        "EcsMaster::spawn_batch must NOT poke next_entity_id.fetch_add directly (C-N2)"
    );
}

// ── Direct-path lazy growth across legacy SBO17 capacity boundary ───────────
//
// Phase 12.6 — `EcsMaster::new` no longer pre-extends `entities_inland`
// to `MAX_ENTITIES_HINT + MAX_BATCH_HINT`. The capacity is grown lazily
// on the dispatcher path by `SpawnBatchCommand::apply`'s
// `EntityMaster::ensure_capacity` call (which holds `&mut EcsMaster` and
// therefore cannot race a worker `&self` read). The Phase 12.5
// `WorldEntityCapacityExceeded` Err return is no longer reachable on
// this path — the apply grows instead of panicking.
//
// This test pins the new contract: spawning past the legacy 72_192-slot
// cap **succeeds** and the entity count matches the cumulative request.
#[test]
fn direct_path_grows_past_legacy_capacity() {
    register_components();
    let mut ecs = EcsMaster::new();
    // Phase 12.6 — fast-store starts at zero capacity.
    assert_eq!(ecs.entity_master().capacity(), 0);

    // Spawn past the legacy 72_192-slot ceiling in MAX_BATCH_HINT-sized
    // chunks. The total ID range crosses the old hard limit.
    let total = 75_000;
    let chunk = 8_000;
    let mut spawned = 0;
    while spawned < total {
        let this = (total - spawned).min(chunk);
        let _ = ecs
            .spawn_batch((0..this).map(|i| PosBundle {
                pos: Position {
                    x: i as f32,
                    y: 0.0,
                    z: 0.0,
                },
            }))
            .expect("Phase 12.6: lazy growth must allow batches past 72_192");
        spawned += this;
    }
    assert_eq!(spawned, total);
    assert!(ecs.entity_master().capacity() >= total);
    // The MAX_BATCH_HINT cap still applies (separate error path).
    let oversize = ecs.spawn_batch((0..8_193).map(|i| PosBundle {
        pos: Position {
            x: i as f32,
            y: 0.0,
            z: 0.0,
        },
    }));
    assert!(matches!(
        oversize,
        Err(EcsError::SpawnBatchExceedsCapacity { .. })
    ));
}

// ── SpawnBatchIter user-visible type signature (W5 compile test) ────────────
//
// W5: the iter must not leak the bundle iterator's unnameable closure
// type. The signature is `SpawnBatchIter<'_, '_, PosBundle>` — just one
// type param, not two.
#[test]
fn spawn_batch_iter_type_signature_no_bundle_iter_leak() {
    register_components();
    let mut ecs = EcsMaster::new();
    ecs.run_system(|mut cmds: Commands| {
        // The type annotation below pins the W5 signature shape. If a
        // future change re-introduces an `I` parameter, this line fails
        // to compile (type signature mismatch).
        let iter: boyko_ecs::ecs::core::commands::SpawnBatchIter<'_, '_, PosBundle> = cmds
            .spawn_batch((0..1).map(|i| PosBundle {
                pos: Position {
                    x: i as f32,
                    y: 0.0,
                    z: 0.0,
                },
            }))
            .unwrap();
        assert_eq!(iter.len(), 1);
    });
}

// ── BundleColumnCache: resolved pool_ids must be canonical-sorted ───────────
//
// Indirect assertion: after spawning one entity through the cache, the
// resolve_and_cache path leaks a `&'static [InlandPoolId]` whose order
// matches `B::component_ids()`. The W4 invariant is checked by the
// debug_assert inside SpawnBatchCommand::apply — this test merely
// exercises the path so the assert would fire on a violation in debug
// builds. The release build is unaffected.
#[test]
fn resolve_and_cache_pool_ids_is_sorted() {
    register_components();
    let mut ecs = EcsMaster::new();
    // PosVelHealth declared in (pos, vel, health) order. Their ids are
    // 700, 701, 702 → already sorted. The derive(Bundle) macro emits a
    // canonical sort regardless, so the assertion holds for any field
    // ordering.
    let _ = ecs
        .spawn_batch((0..16).map(|i| PosVelHealth {
            pos: Position {
                x: i as f32,
                y: 0.0,
                z: 0.0,
            },
            vel: Velocity {
                x: 0.0,
                y: i as f32,
                z: 0.0,
            },
            health: Health(i),
        }))
        .expect("16 ≤ MAX_BATCH_HINT");
    assert_eq!(ecs.entity_count(), 16);
    // The component-ids slice on the bundle is the canonical-sorted
    // source for the cache's `pool_ids` ordering. Verify here.
    let ids = <PosVelHealth as Bundle>::component_ids();
    assert!(
        ids.windows(2).all(|w| w[0].0 <= w[1].0),
        "B1: PosVelHealth::component_ids() must be canonical-sorted"
    );
}

// ── pin-test bundle compiles (W1 — assert_impl_all! gate) ───────────────────
//
// W1: the production `assert_impl_all!` in
// `commands/spawn_batch_command.rs` compiles in this build. Failure mode
// is build-break, so reaching this test body is itself evidence of pass.
#[test]
fn pin_test_bundle_assert_impl_all_compiles() {
    // Empty body — the assertion lives at module scope inside the
    // production crate. Build-breaks at compile time on regression.
}
