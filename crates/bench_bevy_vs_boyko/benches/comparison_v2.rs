//! Phase 12.5 final scoreboard — extends `comparison.rs` with the new
//! Track A/B APIs:
//!
//! * `g2b_boyko_query_iter_10k_direct` — uses the new `EcsMaster::query::<&T>()`
//!   direct API (Opt-B1) instead of the system-wrapper from `g2`.
//! * `g5_boyko_commands_spawn_batch_10k` — uses the new `Commands::spawn_batch`
//!   (Opt-A2) for batched spawn vs `g4`'s per-entity loop.
//! * `g5_bevy_commands_spawn_batch_10k` — Bevy's `Commands::spawn_batch`
//!   for parity comparison.
//!
//! Run via `cargo bench -p bench-bevy-vs-boyko --bench comparison_v2`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

// boyko imports
use boyko_ecs::ecs::core::component::component::Component as BoykoComponent;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::filter::Or;
use boyko_ecs::ecs::core::system::Commands as BoykoCommands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

// bevy imports
use bevy_ecs::prelude::Commands as BevyCommands;
use bevy_ecs::prelude::Component as BevyComponentDerive;
use bevy_ecs::prelude::*;
use bevy_ecs::system::RunSystemOnce;

const N_ENTITIES: usize = 10_000;

// Slot 351 is outside every other reserved range in the codebase at the time
// of writing. The canonical `comparison.rs` uses 350; we use 351 to avoid
// the cross-bench shared-state hazard (registering the same ID with two
// different Layouts panics).
const BOYKO_POS_ID: ComponentId = ComponentId(351);

#[repr(C)]
#[derive(Clone, Copy)]
struct BoykoPosition {
    x: f32,
    y: f32,
    z: f32,
}

impl BoykoComponent for BoykoPosition {
    fn component_id() -> ComponentId {
        BOYKO_POS_ID
    }
}

#[derive(Bundle)]
struct BoykoPosBundle {
    pos: BoykoPosition,
}

fn register_boyko_position() {
    register_layout::<BoykoPosition>(BOYKO_POS_ID.0);
}

#[derive(BevyComponentDerive, Clone, Copy)]
#[allow(dead_code)]
struct BevyPosition {
    x: f32,
    y: f32,
    z: f32,
}

fn _build_pool(num_threads: usize) -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(num_threads).build()
}

// ===========================================================================
// GROUP 2b — Query iter via direct API (Phase 12.5 Opt-B1)
// ===========================================================================
//
// Uses the new `EcsMaster::query::<&T, ()>()` direct API instead of the
// system-wrapper path that `g2_boyko_query_iter_10k` exercises. The plan
// claim is that this closes the 0.88x loss to Bevy (~1 us delta) by skipping
// the FunctionSystem rebuild + per-call QueryDataState::new + the apply
// no-op pass.
//
// The bevy reference number is identical to g2 (Bevy's `state.iter(&world)`
// is already a direct path).

fn bench_boyko_query_iter_10k_direct(c: &mut Criterion) {
    register_boyko_position();
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[BOYKO_POS_ID]);
    for i in 0..N_ENTITIES {
        world
            .spawn_one(
                arch,
                BoykoPosition {
                    x: i as f32,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .expect("spawn must succeed");
    }

    static SUM_SINK: AtomicUsize = AtomicUsize::new(0);

    c.bench_function("g2b_boyko_query_iter_10k_direct", |b| {
        b.iter(|| {
            let view = world.query::<&BoykoPosition, ()>();
            let mut sum = 0.0f32;
            for p in view.iter() {
                sum += p.x;
            }
            SUM_SINK.store(black_box(sum) as usize, Ordering::Relaxed);
        });
    });
}

fn bench_bevy_query_iter_10k_direct(c: &mut Criterion) {
    let mut world = World::new();
    for i in 0..N_ENTITIES {
        world.spawn(BevyPosition {
            x: i as f32,
            y: 0.0,
            z: 0.0,
        });
    }
    let mut state: QueryState<&BevyPosition> = world.query();

    static SUM_SINK: AtomicUsize = AtomicUsize::new(0);

    c.bench_function("g2b_bevy_query_iter_10k_direct", |b| {
        b.iter(|| {
            let mut sum = 0.0f32;
            for p in state.iter(&world) {
                sum += p.x;
            }
            SUM_SINK.store(black_box(sum) as usize, Ordering::Relaxed);
        });
    });
}

// ===========================================================================
// GROUP 5 — Commands::spawn_batch 10k entities (Phase 12.5 Opt-A2)
// ===========================================================================
//
// boyko: a system body that calls `cmds.spawn_batch(iter)` with 10k bundles.
// Per the plan, MAX_BATCH_HINT = 8192, so the bench has to chunk into
// 2 batches of 5 000 entities each.
//
// bevy: a system body that calls `commands.spawn_batch(iter)` with 10k
// bundles. Bevy has no MAX_BATCH_HINT — single 10k call.
//
// Both: rebuild world per iter via `iter_batched` so each run starts from
// empty state. Single-threaded (Commands is not parallelisable by design).
//
// Both engines exercise their BATCH API, not the per-entity loop. The
// expected ratio is boyko/bevy >= 1.10x per the umbrella plan.

const BATCH_CHUNK: usize = 5_000;

fn bench_boyko_commands_spawn_batch_10k(c: &mut Criterion) {
    register_boyko_position();

    c.bench_function("g5_boyko_commands_spawn_batch_10k", |b| {
        b.iter_with_setup(
            EcsMaster::new,
            |mut world| {
                world.run_system(|mut cmds: BoykoCommands| {
                    // Chunk into two 5k batches to stay under MAX_BATCH_HINT.
                    for chunk in 0..(N_ENTITIES / BATCH_CHUNK) {
                        let base = chunk * BATCH_CHUNK;
                        let iter = (0..BATCH_CHUNK).map(move |i| BoykoPosBundle {
                            pos: BoykoPosition {
                                x: (base + i) as f32,
                                y: 0.0,
                                z: 0.0,
                            },
                        });
                        let result = cmds.spawn_batch(iter);
                        let _ = black_box(result);
                    }
                });
                black_box(&world);
            },
        );
    });
}

fn bench_bevy_commands_spawn_batch_10k(c: &mut Criterion) {
    c.bench_function("g5_bevy_commands_spawn_batch_10k", |b| {
        b.iter_with_setup(
            World::new,
            |mut world| {
                let _ = world.run_system_once(|mut cmds: BevyCommands| {
                    let iter = (0..N_ENTITIES).map(|i| BevyPosition {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    });
                    cmds.spawn_batch(iter);
                });
                black_box(&world);
            },
        );
    });
}

// ===========================================================================
// EcsMaster::spawn_batch direct (Track A2 direct-path bench, for diagnostic)
// ===========================================================================
//
// Diagnostic: the direct EcsMaster::spawn_batch path bypasses Commands
// entirely. Useful to attribute saving from "Commands removed" vs "batch
// reservation". Not part of the headline 4-bench scoreboard but published
// for completeness.

fn bench_boyko_ecs_master_spawn_batch_10k(c: &mut Criterion) {
    register_boyko_position();

    c.bench_function("g5d_boyko_ecs_master_spawn_batch_10k", |b| {
        b.iter_with_setup(
            EcsMaster::new,
            |mut world| {
                for chunk in 0..(N_ENTITIES / BATCH_CHUNK) {
                    let base = chunk * BATCH_CHUNK;
                    let iter = (0..BATCH_CHUNK).map(move |i| BoykoPosBundle {
                        pos: BoykoPosition {
                            x: (base + i) as f32,
                            y: 0.0,
                            z: 0.0,
                        },
                    });
                    let _ids = world
                        .spawn_batch(iter)
                        .expect("spawn_batch within MAX_BATCH_HINT");
                }
                black_box(&world);
            },
        );
    });
}

// `Or` import used implicitly by future bench additions — keep the import
// alive so a downstream change to add `Query<&T, Or<(...)>>` doesn't require
// re-editing imports.
#[allow(dead_code)]
type _OrUsage = Or<()>;

fn configure() -> Criterion {
    Criterion::default()
        .sample_size(50)
        .measurement_time(Duration::from_secs(3))
        .warm_up_time(Duration::from_millis(500))
}

criterion_group! {
    name = comparison_v2;
    config = configure();
    targets =
        bench_boyko_query_iter_10k_direct,
        bench_bevy_query_iter_10k_direct,
        bench_boyko_commands_spawn_batch_10k,
        bench_bevy_commands_spawn_batch_10k,
        bench_boyko_ecs_master_spawn_batch_10k,
}

criterion_main!(comparison_v2);
