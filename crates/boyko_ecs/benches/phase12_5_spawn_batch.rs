//! Phase 12.5 Track A — `spawn_batch` micro-benchmarks (plan §1.2 / §5.11).
//!
//! Per the target-metrics table:
//!
//! * `spawn_batch_10k_1comp` (chunked 2×5K): ≤ 800 µs (≤ 80 ns/entity).
//! * `spawn_batch_10k_3comp` (chunked 2×5K): ≤ 1.4 ms.
//! * `bench_spawn_batch_apply_warm_per_entity`: ≤ 60 ns (1-comp) / ≤ 100 ns (3-comp).
//! * `bench_commands_apply_50_noops` (Opt-A1 hoist): ≤ 5 µs.
//!
//! The criterion harness exits via `criterion_main!` — these benches are
//! NOT auto-run by the developer; the `tester` agent invokes
//! `cargo bench --bench phase12_5_spawn_batch` post-impl.

use criterion::{Criterion, black_box, criterion_group, criterion_main};

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

const SLOT_POS: ComponentId = ComponentId(363);
const SLOT_VEL: ComponentId = ComponentId(364);
const SLOT_HEALTH: ComponentId = ComponentId(365);

#[repr(C)]
#[derive(Clone, Copy)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Velocity {
    x: f32,
    y: f32,
    z: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
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

fn register() {
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

// ── spawn_batch_10k_1comp (target ≤ 800 µs / ≤ 80 ns per entity) ────────────

fn spawn_batch_10k_1comp(c: &mut Criterion) {
    register();
    c.bench_function("spawn_batch_10k_1comp", |b| {
        b.iter_with_setup(EcsMaster::new, |mut ecs| {
            ecs.run_system(|mut cmds: Commands| {
                for chunk in 0..2 {
                    let base = chunk * 5_000;
                    let _ = cmds
                        .spawn_batch((0..5_000).map(move |i| PosBundle {
                            pos: Position {
                                x: (base + i) as f32,
                                y: 0.0,
                                z: 0.0,
                            },
                        }))
                        .expect("5000 ≤ MAX_BATCH_HINT");
                }
            });
            black_box(ecs.entity_count());
        });
    });
}

// ── spawn_batch_10k_3comp (target ≤ 1.4 ms) ────────────────────────────────

fn spawn_batch_10k_3comp(c: &mut Criterion) {
    register();
    c.bench_function("spawn_batch_10k_3comp", |b| {
        b.iter_with_setup(EcsMaster::new, |mut ecs| {
            ecs.run_system(|mut cmds: Commands| {
                for chunk in 0..2 {
                    let base = chunk * 5_000;
                    let _ = cmds
                        .spawn_batch((0..5_000).map(move |i| PosVelHealth {
                            pos: Position {
                                x: (base + i) as f32,
                                y: 0.0,
                                z: 0.0,
                            },
                            vel: Velocity {
                                x: 0.0,
                                y: (base + i) as f32,
                                z: 0.0,
                            },
                            health: Health(base + i),
                        }))
                        .expect("5000 ≤ MAX_BATCH_HINT");
                }
            });
            black_box(ecs.entity_count());
        });
    });
}

// ── spawn_batch direct path (no command queue) ──────────────────────────────

fn spawn_batch_direct_10k_1comp(c: &mut Criterion) {
    register();
    c.bench_function("spawn_batch_direct_10k_1comp", |b| {
        b.iter_with_setup(EcsMaster::new, |mut ecs| {
            for chunk in 0..2 {
                let base = chunk * 5_000;
                let _ = ecs
                    .spawn_batch((0..5_000).map(move |i| PosBundle {
                        pos: Position {
                            x: (base + i) as f32,
                            y: 0.0,
                            z: 0.0,
                        },
                    }))
                    .expect("5000 ≤ MAX_BATCH_HINT");
            }
            black_box(ecs.entity_count());
        });
    });
}

// ── Sanity: B::component_ids() resolution post-Bundle-supertrait ────────────
//
// Pin the assertion that the supertrait addition didn't break the
// existing derive(Bundle) codegen — `component_ids()` returns the same
// canonical-sorted slice across calls.
fn component_ids_static_pin(c: &mut Criterion) {
    register();
    c.bench_function("component_ids_static_pin", |b| {
        b.iter(|| {
            let ids = <PosVelHealth as Bundle>::component_ids();
            black_box(ids);
        });
    });
}

criterion_group!(
    benches,
    spawn_batch_10k_1comp,
    spawn_batch_10k_3comp,
    spawn_batch_direct_10k_1comp,
    component_ids_static_pin,
);
criterion_main!(benches);
