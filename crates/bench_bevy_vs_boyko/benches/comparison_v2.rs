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

// boyko imports
use boyko_ecs::ecs::core::component::component::Component as BoykoComponent;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity as BoykoEntity;
use boyko_ecs::ecs::core::iters::query::Query as BoykoQuery;
use boyko_ecs::ecs::core::iters::query::filter::Or;
use boyko_ecs::ecs::core::system::Commands as BoykoCommands;
use boyko_ecs::ecs::core::system::into_system::IntoSystem as BoykoIntoSystem;
use boyko_ecs::ecs::core::system::system::System as BoykoSystem;
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
// GROUP 2c — Honest SystemParam steady-state (Wave-0 Decision 1)
// ===========================================================================
//
// The existing `g2_boyko_query_iter_10k` (comparison.rs) calls
// `world.run_system(closure)` INSIDE `b.iter`. `run_system` rebuilds the
// `FunctionSystem` via `IntoSystem::into_system` EVERY iteration and then
// `run_cached_system` calls `system.initialize(self)` — which on the FIRST
// call of a freshly-built system pays the cold `FilteredAccessSet::new`
// 24 KB Box alloc/zero/free (filtered_access_set.rs:139). A real `Schedule`
// pays this exactly ONCE at build; charging it per-iter is a harness
// artifact, not a steady-state cost. This is the Phase-22.1 wrong-baseline
// trap the plan calls out (Decision 1 / Open Q1).
//
// `g2c` builds the `FunctionSystem` ONCE outside `b.iter`, runs it once to
// warm `initialize` (FS1 short-circuits `state.is_some()` on every later
// call — function_system.rs:188), then the timed loop calls
// `EcsMaster::run_cached_system(&mut sys)` on the cached system. This
// mirrors Bevy's pre-built `QueryState` (g2b_bevy) and is the honest
// SystemParam steady-state envelope: get_param + body + the (no-op for a
// read-only Query) apply + drain tail.
//
// Reported against g2 (per-iter rebuild), g2b (direct API), and the Bevy
// reference (g2b_bevy / g2_bevy — the same `state.iter(&world)` number).
// GOAL: attribute how much of g2's ~1.15x gap is the per-iter rebuild
// (artifact) vs the real steady-state apply/drain envelope (Decision 2).

/// Build a `FunctionSystem` once from a closure, mirroring `run_system`'s
/// bounds. The closure's `SystemParam` marker `M` is inferred at the call
/// site; this generic forwarder is the only way to name `into_system` on a
/// closure whose marker is opaque.
fn build_boyko_system<F, M>(f: F) -> F::System
where
    F: BoykoIntoSystem<(), (), M>,
    F::System: BoykoSystem<Out = ()>,
{
    F::into_system(f)
}

// DIAGNOSTIC (Wave-0, temporary): the g2 harness shape (`run_system` rebuild
// per-iter) but on a PERSISTENT world built once in this `comparison_v2`
// binary — same world construction as g2b/g2c. If this probe reads ~g2
// (7.8 µs) the 38 µs g2b/g2c numbers are a `query().iter()` / cached-dispatch
// artifact; if it reads ~38 µs the slowness is the persistent-world memory
// placement in this binary (independent of the dispatch shape).
fn bench_boyko_query_iter_10k_probe_runsystem(c: &mut Criterion) {
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

    c.bench_function("g2probe_boyko_query_iter_10k_runsystem", |b| {
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

fn bench_boyko_query_iter_10k_cached(c: &mut Criterion) {
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

    // Build the system ONCE (the `into_system` + `FunctionSystem::new` that
    // `run_system` repeats per-iter).
    let mut sys = build_boyko_system(|q: BoykoQuery<&BoykoPosition>| {
        let mut sum = 0.0f32;
        for p in &q {
            sum += p.x;
        }
        SUM_SINK.store(black_box(sum) as usize, Ordering::Relaxed);
    });

    // Warm `initialize` once outside the timed loop so the cold 24 KB
    // FilteredAccessSet alloc is NOT charged per-iter (FS1: subsequent
    // `initialize` calls short-circuit on `state.is_some()`).
    world.run_cached_system(&mut sys);

    c.bench_function("g2c_boyko_query_iter_10k_cached", |b| {
        b.iter(|| {
            world.run_cached_system(black_box(&mut sys));
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

// ===========================================================================
// GROUP 5 WARM — spawn_batch 10k past a PRE-COMMITTED frontier (Wave-0 D6)
// ===========================================================================
//
// VM-commit attribution (Decision 6 vs Decision 4 decision-tree).
//
// The cold benches above (`g5d_boyko_ecs_master_spawn_batch_10k`) rebuild a
// fresh `EcsMaster` per iter, so the first `spawn_batch` chunk drives the
// pool's `committed_rows` from 0 → 5k → 10k, and each growth step calls
// `ComponentPool::grow_rows` → `vm.commit(..)` — a VirtualAlloc/mmap-class
// syscall (component_pool.rs:325-372). Phase 22.1 fingered that syscall as
// the spawn FLOOR, not the per-row write loop.
//
// `g5_warm` ISOLATES the row loop from VM-commit. Setup (UNTIMED): build a
// world, spawn 10k via `spawn_one` (this crosses the frontier once →
// `committed_rows == 10k`), then `delete_entity` all 10k. Despawn drops
// `current_index`/`count` back to 0 but NEVER shrinks `committed_rows`
// (move_out_entity only pops/decrements — archetype.rs:811-861). So the
// world handed to the timed routine has `count == 0` AND
// `committed_rows == 10k`.
//
// The timed `spawn_batch(10k)` then calls `reserve_capacity(10k)` →
// `grow_rows(0 + 10k == 10k)`, which hits the `n <= committed_rows`
// idempotent no-op at component_pool.rs:329-330: ZERO syscalls, ZERO state
// change. The measured cost is therefore the per-row write loop + entity
// reservation/registration + commit/fill_ticks, with VM-commit subtracted.
//
// DECISION TREE:
//   * cold (g5d) ≫ warm  → VM-commit dominates the gap → Decision 6 is the
//     lever (pre-commit / amortize); the row-loop fix (Decision 4) is
//     secondary.
//   * cold (g5d) ≈ warm  → the row loop / reserve dominates → Decision 4
//     (typed write_row_typed) / Decision 5 (single-pass reserve) is the
//     lever; VM-commit is NOT the floor.
//
// NOTE: `iter_with_setup` runs the setup OUTSIDE the timed region (criterion
// re-runs setup per sample), so the spawn+despawn warm-up is never charged.

fn warm_committed_world() -> EcsMaster {
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[BOYKO_POS_ID]);
    // Cross the VM-commit frontier once: 10k live rows ⇒ committed_rows == 10k.
    let mut ents: Vec<BoykoEntity> = Vec::with_capacity(N_ENTITIES);
    for i in 0..N_ENTITIES {
        let e = world
            .spawn_one(
                arch,
                BoykoPosition {
                    x: i as f32,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .expect("warm-up spawn must succeed");
        ents.push(e);
    }
    // Drain to count == 0; committed_rows stays at 10k (no shrink on despawn).
    for e in ents {
        let removed = world.delete_entity(e);
        debug_assert!(removed, "warm-up despawn must remove a live entity");
    }
    world
}

fn bench_boyko_spawn_batch_10k_warm(c: &mut Criterion) {
    register_boyko_position();

    c.bench_function("g5_boyko_spawn_batch_10k_warm", |b| {
        b.iter_with_setup(warm_committed_world, |mut world| {
            // Past the pre-committed frontier ⇒ grow_rows is a no-op ⇒ no
            // vm.commit syscall. Same chunking as the cold g5d bench.
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
        });
    });
}

// `Or` import used implicitly by future bench additions — keep the import
// alive so a downstream change to add `Query<&T, Or<(...)>>` doesn't require
// re-editing imports.
#[allow(dead_code)]
type _OrUsage = Or<()>;

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
    name = comparison_v2;
    config = configure();
    targets =
        bench_boyko_query_iter_10k_direct,
        bench_bevy_query_iter_10k_direct,
        bench_boyko_query_iter_10k_probe_runsystem,
        bench_boyko_query_iter_10k_cached,
        bench_boyko_commands_spawn_batch_10k,
        bench_bevy_commands_spawn_batch_10k,
        bench_boyko_ecs_master_spawn_batch_10k,
        bench_boyko_spawn_batch_10k_warm,
}

criterion_main!(comparison_v2);
