//! Phase 12.6 — Profile `Commands::spawn` (single API) × 10 000.
//!
//! Post Phase-12.5 head-to-head:
//!   - boyko `Commands::spawn` × 10k single = 2.24 ms (224 ns/entity)
//!   - bevy  `Commands::spawn` × 10k single = 762 µs (76 ns/entity)
//!   - Gap = 148 ns/entity, 3× slower.
//!
//! Phase 12.5 Track A delivered the batch path (Commands::spawn_batch is
//! 35 ns/e warm, close to Bevy's 26 ns/e). The single-spawn path was NOT a
//! primary target but Bevy still beats boyko by 3×. This bench profiles
//! per-stage to localise where the 148 ns/e gap lives.
//!
//! # Workload
//!
//! All boyko micro-benches use a 1-component bundle (`V6PosBundle`, 12 B),
//! 10 000 entities, fresh world per iter via `iter_with_setup`. The
//! per-iter `EcsMaster::new` cost is excluded from criterion's measurement
//! window per `iter_with_setup` semantics (verified earlier in v2 profile).
//!
//! # Stages instrumented
//!
//! 1. `Commands::spawn` enqueue (the loop body only) — pure push cost.
//! 2. `Commands::spawn` total wall time (enqueue + apply).
//! 3. `CommandQueue::apply` standalone (push 10k commands into a fresh
//!    queue, then time the apply walk).
//! 4. Pure `CommandQueue::push` × 10k (using `__test_push`).
//! 5. Per-row inside `SpawnAtCommand::apply` (sub-stages a-c):
//!    a. BundleColumnCache warm lookup (per row).
//!    b. Direct `EcsMaster::create_entity_at_with_pool_ids` × 10k —
//!    isolates the apply work from the queue dispatch.
//!    c. Direct `EcsMaster::create_entity` × 10k (legacy 4× SparseMap
//!    path) — comparable baseline for the structural gap.
//! 6. Bevy mirrors: `Commands::spawn` total, `World::spawn_at` direct.
//! 7. Bevy `CommandQueue::apply` standalone (same shape as #3).
//!
//! # How to read
//!
//! - The "total" benches report the wall time the head-to-head sees.
//! - The decomposed benches add up to the total minus dispatch noise. Where
//!   per-stage instants are used, the timing-floor (~60 ns/pair on Windows
//!   QPC) is reported separately in `p0` and must be subtracted manually
//!   when interpreting inner instants.
//! - All boyko/Bevy comparisons keep the same workload shape and per-iter
//!   setup so the ratio is meaningful.
//!
//! Run via `cargo bench -p bench-bevy-vs-boyko --bench profile_spawn_single`.

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc, which
// is far more deterministic and exposes structural signals the system heap
// masks (the documented ±20-30% variance source). See docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

// ── boyko imports ──────────────────────────────────────────────────────────
use boyko_ecs::ecs::core::bundle::Bundle as BoykoBundle;
use boyko_ecs::ecs::core::commands::CommandQueue as BoykoCommandQueue;
use boyko_ecs::ecs::core::component::component::Component as BoykoComponent;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands as BoykoCommands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

// ── bevy imports ───────────────────────────────────────────────────────────
use bevy_ecs::prelude::Commands as BevyCommands;
use bevy_ecs::prelude::Component as BevyComponentDerive;
use bevy_ecs::prelude::*;
use bevy_ecs::system::RunSystemOnce;
use bevy_ecs::world::CommandQueue as BevyCommandQueue;

// ── Workload constants ─────────────────────────────────────────────────────

const N_ENTITIES: usize = 10_000;

// Slot 370 is outside every other reserved range in the codebase
// (comparison.rs uses 350; profile_spawn.rs uses 351; profile_spawn_v2.rs
// uses 360-362). MAX_COMPONENTS = 512 caps the legal range.
const V6_POS_ID: ComponentId = ComponentId(370);

#[repr(C)]
#[derive(Clone, Copy)]
struct V6Position {
    x: f32,
    y: f32,
    z: f32,
}

impl BoykoComponent for V6Position {
    fn component_id() -> ComponentId {
        V6_POS_ID
    }
}

#[derive(Bundle)]
struct V6PosBundle {
    pos: V6Position,
}

fn register_v6_components() {
    register_layout::<V6Position>(V6_POS_ID.0);
}

// ── bevy mirror types ──────────────────────────────────────────────────────

#[derive(BevyComponentDerive, Clone, Copy)]
#[allow(dead_code)]
struct BevyV6Pos {
    x: f32,
    y: f32,
    z: f32,
}

// ===========================================================================
// p0 — Instant::now() pair floor (Windows QPC)
// ===========================================================================
//
// Establishes the per-pair floor we have to subtract when reading
// instrumented benches. Reported in the diagnosis as ~60 ns on Windows.

fn p0_instant_now_pair(c: &mut Criterion) {
    c.bench_function("p0_instant_now_pair", |b| {
        b.iter(|| {
            let t0 = Instant::now();
            let t1 = Instant::now();
            black_box(t1.duration_since(t0));
        });
    });
}

// ===========================================================================
// p1 — boyko Commands::spawn × 10k total (mirror of comparison g4)
// ===========================================================================

fn p1_boyko_commands_spawn_total(c: &mut Criterion) {
    register_v6_components();
    c.bench_function("p1_boyko_commands_spawn_total", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            world.run_system(|mut cmds: BoykoCommands| {
                for i in 0..N_ENTITIES {
                    cmds.spawn(V6PosBundle {
                        pos: V6Position {
                            x: i as f32,
                            y: 0.0,
                            z: 0.0,
                        },
                    });
                }
            });
            black_box(&world);
        });
    });
}

// ===========================================================================
// p2 — bevy Commands::spawn × 10k total (mirror of comparison g4)
// ===========================================================================

fn p2_bevy_commands_spawn_total(c: &mut Criterion) {
    c.bench_function("p2_bevy_commands_spawn_total", |b| {
        b.iter_with_setup(World::new, |mut world| {
            let _ = world.run_system_once(|mut cmds: BevyCommands| {
                for i in 0..N_ENTITIES {
                    cmds.spawn(BevyV6Pos {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    });
                }
            });
            black_box(&world);
        });
    });
}

// ===========================================================================
// p3 — Per-stage split via Instant brackets (boyko enqueue vs apply)
// ===========================================================================
//
// Bracket the body before SystemParam::apply with an Instant pair to
// isolate enqueue cost. The wall time minus the enqueue is the apply
// cost. Per-iter floor: 1 × ~60 ns / 10k = 0.006 ns/entity — negligible
// because we bracket the whole 10k-entity loop, not each entity.

static P3_ENQUEUE_NS: AtomicU64 = AtomicU64::new(0);
static P3_TOTAL_NS: AtomicU64 = AtomicU64::new(0);
static P3_ITERS: AtomicUsize = AtomicUsize::new(0);

fn p3_boyko_enqueue_vs_apply(c: &mut Criterion) {
    register_v6_components();
    P3_ENQUEUE_NS.store(0, Ordering::Relaxed);
    P3_TOTAL_NS.store(0, Ordering::Relaxed);
    P3_ITERS.store(0, Ordering::Relaxed);

    c.bench_function("p3_boyko_enqueue_vs_apply", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            let t_start = Instant::now();
            world.run_system(|mut cmds: BoykoCommands| {
                let t_enq_start = Instant::now();
                for i in 0..N_ENTITIES {
                    cmds.spawn(V6PosBundle {
                        pos: V6Position {
                            x: i as f32,
                            y: 0.0,
                            z: 0.0,
                        },
                    });
                }
                let t_enq_end = Instant::now();
                P3_ENQUEUE_NS.fetch_add(
                    t_enq_end.duration_since(t_enq_start).as_nanos() as u64,
                    Ordering::Relaxed,
                );
            });
            let t_end = Instant::now();
            P3_TOTAL_NS.fetch_add(
                t_end.duration_since(t_start).as_nanos() as u64,
                Ordering::Relaxed,
            );
            P3_ITERS.fetch_add(1, Ordering::Relaxed);
            black_box(&world);
        });
    });

    let iters = P3_ITERS.load(Ordering::Relaxed).max(1);
    let enq_ns = P3_ENQUEUE_NS.load(Ordering::Relaxed);
    let tot_ns = P3_TOTAL_NS.load(Ordering::Relaxed);
    let apply_ns = tot_ns.saturating_sub(enq_ns);
    eprintln!();
    eprintln!(
        "p3 boyko per-stage ({} iters × {} entities):",
        iters, N_ENTITIES
    );
    eprintln!(
        "    enqueue: {:.2} ns/entity",
        enq_ns as f64 / (iters * N_ENTITIES) as f64
    );
    eprintln!(
        "    apply:   {:.2} ns/entity",
        apply_ns as f64 / (iters * N_ENTITIES) as f64
    );
    eprintln!(
        "    total:   {:.2} ns/entity",
        tot_ns as f64 / (iters * N_ENTITIES) as f64
    );
    eprintln!();
}

// ===========================================================================
// p4 — Pure CommandQueue::push × 10k (no apply)
// ===========================================================================
//
// Use the `__test_push` doc-hidden hook to push N SpawnAtCommand-sized
// commands without running their apply. The bound type's apply is a no-op;
// only the push path is exercised. Drop semantics: the queue's Drop walks
// any un-applied bytes through `consume_and_drop_glue` — the NoopCommand
// implements Drop as a no-op so the drop walk does not skew measurement.
//
// This is the cleanest measure of `CommandQueue::push` cost per command.

struct NoopCommand {
    /// Same size class as SpawnAtCommand<V6PosBundle>:
    ///   Entity(16 B) + V6PosBundle(12 B) = 28 B; round up to 32 with this
    /// 32-B placeholder so the byte arena footprint matches.
    _payload: [u8; 28],
}

impl boyko_ecs::ecs::core::commands::Command for NoopCommand {
    fn apply(self, _world: &mut EcsMaster) {
        // Side-effect: increment a sink so the compiler keeps the value live.
        P4_APPLIES.fetch_add(1, Ordering::Relaxed);
    }
}

static P4_APPLIES: AtomicU64 = AtomicU64::new(0);

fn p4_command_queue_push_only(c: &mut Criterion) {
    c.bench_function("p4_command_queue_push_only", |b| {
        b.iter_with_setup(BoykoCommandQueue::__test_new, |mut q| {
            for _ in 0..N_ENTITIES {
                q.__test_push(NoopCommand { _payload: [0; 28] });
            }
            black_box(&q);
        });
    });
}

// ===========================================================================
// p5 — CommandQueue::apply × 10k (NoopCommand)
// ===========================================================================
//
// Setup: push 10k NoopCommands; bench measures only the apply walk.
// Subtracting the per-cmd apply cost (1 atomic fetch_add) gives the
// dispatch overhead of `CommandQueue::apply_or_drop_queued_no_catch`.

fn p5_command_queue_apply_only(c: &mut Criterion) {
    c.bench_function("p5_command_queue_apply_only", |b| {
        b.iter_with_setup(
            || {
                let mut q = BoykoCommandQueue::__test_new();
                for _ in 0..N_ENTITIES {
                    q.__test_push(NoopCommand { _payload: [0; 28] });
                }
                let world = EcsMaster::new();
                (q, world)
            },
            |(mut q, mut world)| {
                q.__test_apply(&mut world);
                black_box(&world);
            },
        );
    });
}

// ===========================================================================
// p6 — Bevy CommandQueue::apply × 10k (mirror of p5)
// ===========================================================================
//
// Same shape: push 10k commands then time the apply. Bevy uses a closure-
// command via `queue` so the equivalent is to push a closure that does a
// single atomic fetch_add.

static P6_APPLIES: AtomicU64 = AtomicU64::new(0);

fn p6_bevy_command_queue_apply_only(c: &mut Criterion) {
    c.bench_function("p6_bevy_command_queue_apply_only", |b| {
        b.iter_with_setup(
            || {
                let mut q = BevyCommandQueue::default();
                // Bevy's Commands::queue stores any FnOnce(&mut World) as a
                // command. We push 10k such closures with a tiny side effect
                // (atomic fetch_add) — equivalent payload shape to p5.
                for _ in 0..N_ENTITIES {
                    q.push(move |_w: &mut World| {
                        P6_APPLIES.fetch_add(1, Ordering::Relaxed);
                    });
                }
                let world = World::new();
                (q, world)
            },
            |(mut q, mut world)| {
                q.apply(&mut world);
                black_box(&world);
            },
        );
    });
}

// ===========================================================================
// p7 — boyko direct `create_entity_at_with_pool_ids` × 10k (no Commands)
// ===========================================================================
//
// Hits the Opt-A3 path that SpawnAtCommand::apply uses, but without the
// per-cmd CommandQueue dispatch. The numbers from p7 minus p3.apply tell
// us how much overhead the queue+SpawnAtCommand machinery adds beyond the
// raw apply work.
//
// NOTE: `create_entity_at_with_pool_ids` is `pub(crate)`. We cannot reach
// it directly from a bench crate. We use the next-closest path:
// `EcsMaster::spawn_one` (Opt-A3 NOT wired in spawn_one; uses legacy
// `create_entity` with 4× SparseMap), and `spawn_batch` with count=1 per
// call (Opt-A3 wired but iterator overhead).
//
// Instead we approximate the per-entity Opt-A3 apply cost using `spawn_batch`
// of size 1 in a loop (cache pre-warmed). The bookkeeping per call:
//   - reserve_batch(1) atomic
//   - cached_archetype_id (warm OnceLock::get)
//   - bundle_column_cache.get_resolved (warm OnceLock::get)
//   - per-row: pool_at_unchecked_mut + write_at_unchecked + commit_units(1)
//   - per-row: fill_ticks(1)
//   - archetype.entity_ids.push (×1)
//   - register_batch (×1)
//   - Vec<Entity> materialisation of 1 element
//
// This is the closest user-visible analog of SpawnAtCommand::apply minus
// the queue glue.

fn p7_direct_spawn_batch_size_1_loop(c: &mut Criterion) {
    register_v6_components();
    c.bench_function("p7_direct_spawn_batch_size_1_loop", |b| {
        b.iter_with_setup(
            || {
                // Use `with_capacity` so the entity fast-store is pre-sized
                // — single-element `spawn_batch` calls in the body require
                // entity_master capacity (SBO16 invariant). Then warm the
                // archetype/bundle caches with one spawn.
                let mut world = EcsMaster::with_capacity(N_ENTITIES + 64, 256);
                let _ = world
                    .spawn_batch((0..1).map(|i| V6PosBundle {
                        pos: V6Position {
                            x: i as f32,
                            y: 0.0,
                            z: 0.0,
                        },
                    }))
                    .expect("warm-up");
                world
            },
            |mut world| {
                for i in 0..N_ENTITIES {
                    let _ = world
                        .spawn_batch((0..1).map(move |_| V6PosBundle {
                            pos: V6Position {
                                x: i as f32,
                                y: 0.0,
                                z: 0.0,
                            },
                        }))
                        .expect("spawn_batch(1)");
                }
                black_box(&world);
            },
        );
    });
}

// ===========================================================================
// p8 — boyko direct `create_entity` × 10k (legacy 4× SparseMap path)
// ===========================================================================
//
// Comparable baseline. Reproduces v2 h6. Without the Commands dispatch.
// Used to attribute SpawnAtCommand dispatch overhead by subtraction.

fn p8_direct_create_entity_legacy(c: &mut Criterion) {
    register_v6_components();
    c.bench_function("p8_direct_create_entity_legacy", |b| {
        b.iter_with_setup(
            || {
                let mut world = EcsMaster::new();
                let arch = world.create_archetype(&[V6_POS_ID]);
                (world, arch)
            },
            |(mut world, arch)| {
                for i in 0..N_ENTITIES {
                    let pos = V6Position {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    };
                    let bytes: &[u8] = unsafe {
                        std::slice::from_raw_parts(
                            std::ptr::addr_of!(pos) as *const u8,
                            std::mem::size_of::<V6Position>(),
                        )
                    };
                    world
                        .create_entity(arch, &[(V6_POS_ID, bytes)])
                        .expect("create_entity");
                }
                black_box(&world);
            },
        );
    });
}

// ===========================================================================
// p9 — boyko `spawn_one` × 10k
// ===========================================================================
//
// `spawn_one` wraps create_entity with a typed component. Same path
// underneath but slightly different cost shape (mem::forget on Ok). We
// include this as a sanity check that the typed wrapper does not add
// material overhead vs raw create_entity.

fn p9_direct_spawn_one(c: &mut Criterion) {
    register_v6_components();
    c.bench_function("p9_direct_spawn_one", |b| {
        b.iter_with_setup(
            || {
                let mut world = EcsMaster::new();
                let arch = world.create_archetype(&[V6_POS_ID]);
                (world, arch)
            },
            |(mut world, arch)| {
                for i in 0..N_ENTITIES {
                    world
                        .spawn_one(
                            arch,
                            V6Position {
                                x: i as f32,
                                y: 0.0,
                                z: 0.0,
                            },
                        )
                        .expect("spawn_one");
                }
                black_box(&world);
            },
        );
    });
}

// ===========================================================================
// p10 — Bevy direct `World::spawn` × 10k (no Commands)
// ===========================================================================
//
// Reference: how fast can Bevy spawn an entity into a known archetype
// without going through Commands? `World::spawn(bundle)` builds a
// BundleSpawner per call (no spawner caching across calls) — that's the
// same cost shape as Bevy's Commands::spawn apply path.

fn p10_bevy_direct_world_spawn(c: &mut Criterion) {
    c.bench_function("p10_bevy_direct_world_spawn", |b| {
        b.iter_with_setup(World::new, |mut world| {
            for i in 0..N_ENTITIES {
                world.spawn(BevyV6Pos {
                    x: i as f32,
                    y: 0.0,
                    z: 0.0,
                });
            }
            black_box(&world);
        });
    });
}

// ===========================================================================
// p11 — boyko: Commands::spawn loop reading EntityCommands::id() (no chain)
// ===========================================================================
//
// EntityCommands::id() should be free (just reads the Entity field). This
// confirms the new Phase 11 EntityCommands handle doesn't add overhead.

fn p11_boyko_commands_spawn_with_id_read(c: &mut Criterion) {
    register_v6_components();
    static SINK: AtomicU64 = AtomicU64::new(0);
    c.bench_function("p11_boyko_commands_spawn_with_id_read", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            world.run_system(|mut cmds: BoykoCommands| {
                let mut last_id: u64 = 0;
                for i in 0..N_ENTITIES {
                    let id = cmds
                        .spawn(V6PosBundle {
                            pos: V6Position {
                                x: i as f32,
                                y: 0.0,
                                z: 0.0,
                            },
                        })
                        .id();
                    last_id = id.id().0 as u64;
                }
                SINK.store(last_id, Ordering::Relaxed);
            });
            black_box(&world);
        });
    });
}

// ===========================================================================
// p12 — EntityCounter::reserve_entity × 10k (the atomic-RMW bottom)
// ===========================================================================
//
// Pulled out of the Commands::spawn path to measure the atomic cost
// alone. We can't invoke EntityCounter directly from outside the crate,
// but `Commands::reserve_entity` is the public wrapper and goes through
// the same fetch_add path.

fn p12_boyko_reserve_entity_only(c: &mut Criterion) {
    register_v6_components();
    static SINK: AtomicU64 = AtomicU64::new(0);
    c.bench_function("p12_boyko_reserve_entity_only", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            world.run_system(|cmds: BoykoCommands| {
                let mut last: u64 = 0;
                for _ in 0..N_ENTITIES {
                    let e = cmds.reserve_entity();
                    last = e.id().0 as u64;
                }
                SINK.store(last, Ordering::Relaxed);
            });
            black_box(&world);
        });
    });
}

// ===========================================================================
// p13 — boyko Bundle::cached_archetype_id × 10k (warm)
// ===========================================================================

fn p13_boyko_cached_archetype_id_warm(c: &mut Criterion) {
    register_v6_components();
    let mut world = EcsMaster::new();
    // Warm
    let _ = world
        .spawn_batch((0..1).map(|i| V6PosBundle {
            pos: V6Position {
                x: i as f32,
                y: 0.0,
                z: 0.0,
            },
        }))
        .expect("warm-up");

    static SINK: AtomicUsize = AtomicUsize::new(0);
    c.bench_function("p13_boyko_cached_archetype_id_warm", |b| {
        b.iter(|| {
            for _ in 0..N_ENTITIES {
                let id = V6PosBundle::cached_archetype_id(&mut world);
                SINK.store(black_box(id.0), Ordering::Relaxed);
            }
        });
    });
}

// ===========================================================================
// p14 — bevy: pure Commands::spawn enqueue (no apply) via run_system_once
// ===========================================================================
//
// Bevy's Commands::spawn allocates the entity ID up front via
// `self.allocator.alloc()` and pushes a closure that calls spawn_at_with_caller
// on apply. To measure pure enqueue without apply, we use the same
// Instant-bracket technique as p3.

static P14_ENQUEUE_NS: AtomicU64 = AtomicU64::new(0);
static P14_TOTAL_NS: AtomicU64 = AtomicU64::new(0);
static P14_ITERS: AtomicUsize = AtomicUsize::new(0);

fn p14_bevy_enqueue_vs_apply(c: &mut Criterion) {
    P14_ENQUEUE_NS.store(0, Ordering::Relaxed);
    P14_TOTAL_NS.store(0, Ordering::Relaxed);
    P14_ITERS.store(0, Ordering::Relaxed);

    c.bench_function("p14_bevy_enqueue_vs_apply", |b| {
        b.iter_with_setup(World::new, |mut world| {
            let t_start = Instant::now();
            let _ = world.run_system_once(|mut cmds: BevyCommands| {
                let t_enq_start = Instant::now();
                for i in 0..N_ENTITIES {
                    cmds.spawn(BevyV6Pos {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    });
                }
                let t_enq_end = Instant::now();
                P14_ENQUEUE_NS.fetch_add(
                    t_enq_end.duration_since(t_enq_start).as_nanos() as u64,
                    Ordering::Relaxed,
                );
            });
            let t_end = Instant::now();
            P14_TOTAL_NS.fetch_add(
                t_end.duration_since(t_start).as_nanos() as u64,
                Ordering::Relaxed,
            );
            P14_ITERS.fetch_add(1, Ordering::Relaxed);
            black_box(&world);
        });
    });

    let iters = P14_ITERS.load(Ordering::Relaxed).max(1);
    let enq_ns = P14_ENQUEUE_NS.load(Ordering::Relaxed);
    let tot_ns = P14_TOTAL_NS.load(Ordering::Relaxed);
    let apply_ns = tot_ns.saturating_sub(enq_ns);
    eprintln!();
    eprintln!(
        "p14 bevy per-stage ({} iters × {} entities):",
        iters, N_ENTITIES
    );
    eprintln!(
        "    enqueue: {:.2} ns/entity",
        enq_ns as f64 / (iters * N_ENTITIES) as f64
    );
    eprintln!(
        "    apply:   {:.2} ns/entity",
        apply_ns as f64 / (iters * N_ENTITIES) as f64
    );
    eprintln!(
        "    total:   {:.2} ns/entity",
        tot_ns as f64 / (iters * N_ENTITIES) as f64
    );
    eprintln!();
}

// ===========================================================================
// Criterion wiring
// ===========================================================================

fn configure() -> Criterion {
    // Phase X.E: a longer warm-up lets the CPU reach a steady clock/cache state
    // before sampling, and a 5% noise threshold (criterion's default is 1%)
    // stops this noisy Windows box from reporting routine run-to-run jitter as
    // a regression. See docs/BENCHMARKING.md.
    Criterion::default()
        .sample_size(30)
        .measurement_time(Duration::from_secs(2))
        .warm_up_time(Duration::from_secs(3))
        .noise_threshold(0.05)
}

criterion_group! {
    name = profile_spawn_single;
    config = configure();
    targets =
        p0_instant_now_pair,
        p1_boyko_commands_spawn_total,
        p2_bevy_commands_spawn_total,
        p3_boyko_enqueue_vs_apply,
        p4_command_queue_push_only,
        p5_command_queue_apply_only,
        p6_bevy_command_queue_apply_only,
        p7_direct_spawn_batch_size_1_loop,
        p8_direct_create_entity_legacy,
        p9_direct_spawn_one,
        p10_bevy_direct_world_spawn,
        p11_boyko_commands_spawn_with_id_read,
        p12_boyko_reserve_entity_only,
        p13_boyko_cached_archetype_id_warm,
        p14_bevy_enqueue_vs_apply,
}

criterion_main!(profile_spawn_single);
