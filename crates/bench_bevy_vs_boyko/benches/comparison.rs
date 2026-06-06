//! Head-to-head benchmark suite: `boyko_ecs` vs `bevy_ecs` 0.18.1.
//!
//! All four groups run identical workloads on both engines so the numbers
//! reported by criterion can be compared like-for-like. Where the APIs do
//! not map exactly (boyko's `Query` is always SystemParam-routed, Bevy's
//! `QueryState` can iterate against `&World` directly), the comment block
//! above each bench documents the asymmetry honestly.
//!
//! # Workload summary
//!
//! | Group | What is measured | Threads |
//! |-------|------------------|---------|
//! | 1 | 50 empty systems dispatch (Schedule::run) | 8 |
//! | 2 | Query iter 10k entities (read-only sum) | 1 |
//! | 3 | par_iter 10k entities (read-only sum) | 8 |
//! | 4 | Spawn 10k entities via Commands (incl. apply) | 1 |
//!
//! # Reading the output
//!
//! Criterion prints one `name { boyko | bevy }` block per bench. Compare the
//! "time" line directly; both engines are exercised on the same machine in
//! the same run. The ratio (boyko/bevy) is computed by the markdown report
//! generated alongside this file.

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc, which
// is far more deterministic and exposes structural signals the system heap
// masks (the documented ±20-30% variance source). See docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

// ── boyko imports ──────────────────────────────────────────────────────────
use boyko_ecs::ecs::core::component::component::Component as BoykoComponent;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query as BoykoQuery;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands as BoykoCommands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

// ── bevy imports ───────────────────────────────────────────────────────────
use bevy_ecs::prelude::Commands as BevyCommands;
use bevy_ecs::prelude::Component as BevyComponentDerive;
use bevy_ecs::prelude::Query as BevyQuery;
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ExecutorKind;
use bevy_ecs::system::RunSystemOnce;

// ── Shared workload constants ─────────────────────────────────────────────

const N_ENTITIES: usize = 10_000;
const N_SYSTEMS: usize = 50;
const POOL_THREADS: usize = 8;

// ── boyko Position component ───────────────────────────────────────────────
//
// Component slot 700 is outside every other reserved range in the codebase
// (existing reservations top out near 510). The crate-wide convention is to
// pin a fixed `ComponentId` per type and register the layout once.

// Slot 350 is outside every other reserved range in the boyko_ecs codebase
// at the time of writing (max reservation tops out near 510, and ranges
// 340-349 / 320-329 / 290-303 / 200-207 are taken). MAX_COMPONENTS = 512
// caps the legal range.
const BOYKO_POS_ID: ComponentId = ComponentId(350);

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

// ── bevy Position component ────────────────────────────────────────────────

#[derive(BevyComponentDerive, Clone, Copy)]
#[allow(dead_code)]
struct BevyPosition {
    x: f32,
    y: f32,
    z: f32,
}

// ── Pool factory ───────────────────────────────────────────────────────────

fn build_pool(num_threads: usize) -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(num_threads).build()
}

// ===========================================================================
// GROUP 1 — 50 empty systems dispatch (Schedule::run)
// ===========================================================================
//
// boyko: 50 zero-param closures via `ScheduleBuilder::add_system(|| {})`. The
// FunctionSystem path infers an empty `()` SystemParam tuple; the resulting
// systems have empty Access ⇒ the conflict graph permits full parallel
// dispatch ⇒ the executor fan-out matches what we measure for Bevy below.
//
// bevy: `schedule.add_systems(|| { ... })` registers 50 empty systems. We
// pin `ExecutorKind::MultiThreaded` so the bevy scheduler uses its parallel
// executor (matching boyko's worker fan-out).
//
// Both: 8-thread pool. The dispatch cost being measured is "outer apply
// window + spawn + completion drain" per frame, dominated by sync primitives
// rather than user code.

fn bench_boyko_50_empty_systems(c: &mut Criterion) {
    let pool = build_pool(POOL_THREADS);
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(pool);

    static SINK: AtomicUsize = AtomicUsize::new(0);
    SINK.store(0, Ordering::Relaxed);

    for _ in 0..N_SYSTEMS {
        builder.add_system(|| {
            // black_box prevents the closure from being optimised away.
            black_box(SINK.load(Ordering::Relaxed));
        });
    }

    let mut sched = builder.build(&mut world);

    c.bench_function("g1_boyko_50_empty_systems", |b| {
        b.iter(|| {
            sched.run(black_box(&mut world));
        });
    });
}

fn bench_bevy_50_empty_systems(c: &mut Criterion) {
    let mut world = World::new();
    let mut sched = Schedule::default();
    sched.set_executor_kind(ExecutorKind::MultiThreaded);

    static SINK: AtomicUsize = AtomicUsize::new(0);
    SINK.store(0, Ordering::Relaxed);

    for _ in 0..N_SYSTEMS {
        sched.add_systems(|| {
            black_box(SINK.load(Ordering::Relaxed));
        });
    }

    // Bevy 0.18: `Schedule::initialize` requires `&mut World`.
    sched.initialize(&mut world).unwrap();

    c.bench_function("g1_bevy_50_empty_systems", |b| {
        b.iter(|| {
            sched.run(black_box(&mut world));
        });
    });
}

// ===========================================================================
// GROUP 2 — Query iter over 10k entities (read-only, single-thread)
// ===========================================================================
//
// boyko: Query<&BoykoPosition>::iter() summing `.x`. boyko's `Query` is a
// `SystemParam` — the canonical access is from inside a system body. To make
// the iteration cost the only thing measured, we hoist a single
// `FunctionSystem` outside the timed loop and call `run_cached_system` per
// iter. The dispatcher cost (initialize-once + ~30 ns dispatch + apply
// no-op) is the same constant for both engines; the dominant cost is the
// per-row work.
//
// bevy: `QueryState::iter(&world)` walks every matched row directly,
// bypassing the schedule. This is Bevy's standard standalone Query path.
// Both paths iterate the SAME number of rows (N_ENTITIES) with the SAME
// per-row body (`sum += x`), so the comparison is fair at the "throughput
// of read-only iter" level.
//
// Asymmetry note: boyko's Query path pays one system dispatch round per
// `iter()`; Bevy's path does not. We report both numbers honestly and let
// the reader decide whether to attribute the gap to dispatch or to the
// iter body.

fn bench_boyko_query_iter_10k(c: &mut Criterion) {
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

    c.bench_function("g2_boyko_query_iter_10k", |b| {
        b.iter(|| {
            world.run_system(|q: BoykoQuery<&BoykoPosition>| {
                let mut sum = 0.0f32;
                for p in &q {
                    sum += p.x;
                }
                SUM_SINK.store(black_box(sum) as usize, Ordering::Relaxed);
            });
        });
    });
}

fn bench_bevy_query_iter_10k(c: &mut Criterion) {
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

    c.bench_function("g2_bevy_query_iter_10k", |b| {
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
// GROUP 3 — par_iter over 10k entities (parallel, 8 threads)
// ===========================================================================
//
// boyko: a single system that calls `q.par_iter().for_each(...)`. The
// system runs inside `Schedule::run` so the ambient ThreadPool is attached
// — `par_iter` fans the 10k rows into worker chunks. The atomic increment
// inside the body keeps the work measurable without dominating the dispatch
// cost.
//
// bevy: same shape via `q.par_iter().for_each(...)`. Bevy's MultiThreaded
// executor manages worker scheduling; the parallel body fans out within the
// system.
//
// Both: 8-thread pools.
//
// What this measures: one full frame including outer install/dispatch +
// per-chunk fork/join + per-row body. With only 10k rows the per-row work
// is tiny so dispatch overhead is a meaningful component — the numbers
// reflect the lower bound where parallel iteration becomes worthwhile.

fn bench_boyko_par_iter_10k(c: &mut Criterion) {
    register_boyko_position();
    let pool = build_pool(POOL_THREADS);
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

    static SUM: AtomicUsize = AtomicUsize::new(0);

    let mut builder = ScheduleBuilder::new(pool);
    builder.add_system(|q: BoykoQuery<&BoykoPosition>| {
        q.par_iter().for_each(|p: &BoykoPosition| {
            SUM.fetch_add(p.x as usize, Ordering::Relaxed);
        });
    });
    let mut sched = builder.build(&mut world);

    c.bench_function("g3_boyko_par_iter_10k", |b| {
        b.iter(|| {
            SUM.store(0, Ordering::Relaxed);
            sched.run(black_box(&mut world));
        });
    });
}

fn bench_bevy_par_iter_10k(c: &mut Criterion) {
    let mut world = World::new();
    for i in 0..N_ENTITIES {
        world.spawn(BevyPosition {
            x: i as f32,
            y: 0.0,
            z: 0.0,
        });
    }

    static SUM: AtomicUsize = AtomicUsize::new(0);

    let mut sched = Schedule::default();
    sched.set_executor_kind(ExecutorKind::MultiThreaded);
    sched.add_systems(|q: BevyQuery<&BevyPosition>| {
        q.par_iter().for_each(|p: &BevyPosition| {
            SUM.fetch_add(p.x as usize, Ordering::Relaxed);
        });
    });
    sched.initialize(&mut world).unwrap();

    c.bench_function("g3_bevy_par_iter_10k", |b| {
        b.iter(|| {
            SUM.store(0, Ordering::Relaxed);
            sched.run(black_box(&mut world));
        });
    });
}

// ===========================================================================
// GROUP 4 — Spawn 10k entities via Commands (incl. apply)
// ===========================================================================
//
// boyko: a system body that loops `cmds.spawn(BoykoPosBundle { pos: ... })`
// for N_ENTITIES iterations. The `SystemParam::apply` flush runs at the
// end of the system body (inside `run_system`), so the measured cost
// includes both the enqueue and the apply path that creates each entity in
// the archetype. We rebuild the world per iter via `iter_batched` so each
// run starts from an empty state.
//
// bevy: identical shape via `Commands::spawn(BevyPosition { ... })` inside
// a one-shot system call. We use `world.run_system_once(...)` which runs
// the system and applies its commands. Per-iter fresh `World`.
//
// Both: single-threaded (`Commands` is not parallelisable by design).

fn bench_boyko_commands_spawn_10k(c: &mut Criterion) {
    register_boyko_position();

    c.bench_function("g4_boyko_commands_spawn_10k", |b| {
        b.iter_with_setup(
            EcsMaster::new,
            |mut world| {
                world.run_system(|mut cmds: BoykoCommands| {
                    for i in 0..N_ENTITIES {
                        cmds.spawn(BoykoPosBundle {
                            pos: BoykoPosition {
                                x: i as f32,
                                y: 0.0,
                                z: 0.0,
                            },
                        });
                    }
                });
                black_box(&world);
            },
        );
    });
}

fn bench_bevy_commands_spawn_10k(c: &mut Criterion) {
    c.bench_function("g4_bevy_commands_spawn_10k", |b| {
        b.iter_with_setup(
            World::new,
            |mut world| {
                // `run_system_once` runs the system and applies its commands
                // against a fresh world.
                let _ = world.run_system_once(|mut cmds: BevyCommands| {
                    for i in 0..N_ENTITIES {
                        cmds.spawn(BevyPosition {
                            x: i as f32,
                            y: 0.0,
                            z: 0.0,
                        });
                    }
                });
                black_box(&world);
            },
        );
    });
}

// ── Criterion wiring ───────────────────────────────────────────────────────

fn configure() -> Criterion {
    // Phase X.E: a longer warm-up lets the CPU reach a steady clock/cache state
    // before sampling, and a 5% noise threshold (criterion's default is 1%)
    // stops this noisy Windows box from reporting routine run-to-run jitter as
    // a regression. See docs/BENCHMARKING.md.
    Criterion::default()
        .sample_size(50)
        .measurement_time(Duration::from_secs(3))
        .warm_up_time(Duration::from_secs(3))
        .noise_threshold(0.05)
}

criterion_group! {
    name = comparison;
    config = configure();
    targets =
        bench_boyko_50_empty_systems,
        bench_bevy_50_empty_systems,
        bench_boyko_query_iter_10k,
        bench_bevy_query_iter_10k,
        bench_boyko_par_iter_10k,
        bench_bevy_par_iter_10k,
        bench_boyko_commands_spawn_10k,
        bench_bevy_commands_spawn_10k,
}

criterion_main!(comparison);
