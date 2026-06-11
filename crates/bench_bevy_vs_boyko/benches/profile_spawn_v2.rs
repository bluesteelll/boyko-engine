//! Phase 12.5 — Spawn regression-diagnosis profile bench (v2).
//!
//! This file is a **measurement-only instrument** for diagnosing why Track A
//! REGRESSED the spawn benchmarks instead of optimising them:
//!
//! * `Commands::spawn 10k`: pre-12.5 ~1.044 ms → post-12.5 **2.20 ms**
//!   (REGRESSION 2.1×).
//! * `Commands::spawn_batch 10k` (new API, intended win):
//!   **3.10 ms** vs Bevy 270 µs (**11× slower than Bevy**).
//!
//! # Hypotheses tested
//!
//! 1. **H2: `EcsMaster::new` pre-extends fast-stores to 72 192 slots** —
//!    cost of `iter_with_setup` per-iter world rebuild.
//! 2. **H3: `SpawnBatchCommand::apply` per-entity cost** — bulk-apply path
//!    is supposed to be the headline win but is ~310 ns/entity.
//! 3. **H1: `BundleColumnCache` warm hit cost** — supposed to be ~3 ns.
//! 4. **H6: `create_entity_at_with_pool_ids` path overhead** — Opt-A3 wired
//!    here, should be faster than legacy `create_entity_at`.
//! 5. **H4: `for_each_component_bytes` callback dispatch** — the bundle
//!    callback may not be getting inlined under Track A wiring.
//! 6. **H5: `CommandQueue::push` enqueue cost** — sanity check that the
//!    push side isn't degraded.
//!
//! # Method
//!
//! Every bench is structured as an **isolated micro-measurement** of one
//! stage; numbers from independent micros are summed externally to attribute
//! costs against the head-to-head bench number. The world is rebuilt per
//! iter only where the bench specifically calls out the setup cost (H2).
//!
//! Run via `cargo bench -p bench-bevy-vs-boyko --bench profile_spawn_v2`.

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

// ── Workload constants ─────────────────────────────────────────────────────

const N_ENTITIES: usize = 10_000;
const BATCH_CHUNK: usize = 5_000;

// Slot 360 is outside every other reserved range in the boyko_ecs codebase.
// comparison.rs uses 350; profile_spawn uses 351. We pick a non-conflicting
// slot to avoid the cross-bench registry collision.
const V2_POS_ID: ComponentId = ComponentId(360);
const V2_VEL_ID: ComponentId = ComponentId(361);
const V2_TAG_ID: ComponentId = ComponentId(362);

#[repr(C)]
#[derive(Clone, Copy)]
struct V2Position {
    x: f32,
    y: f32,
    z: f32,
}

impl BoykoComponent for V2Position {
    fn component_id() -> ComponentId {
        V2_POS_ID
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct V2Velocity {
    x: f32,
    y: f32,
    z: f32,
}

impl BoykoComponent for V2Velocity {
    fn component_id() -> ComponentId {
        V2_VEL_ID
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct V2Tag {
    flags: u32,
}

impl BoykoComponent for V2Tag {
    fn component_id() -> ComponentId {
        V2_TAG_ID
    }
}

#[derive(Bundle)]
struct V2PosBundle {
    pos: V2Position,
}

#[derive(Bundle)]
struct V2Bundle3 {
    pos: V2Position,
    vel: V2Velocity,
    tag: V2Tag,
}

fn register_v2_components() {
    register_layout::<V2Position>(V2_POS_ID.0);
    register_layout::<V2Velocity>(V2_VEL_ID.0);
    register_layout::<V2Tag>(V2_TAG_ID.0);
}

// ── bevy mirror types ──────────────────────────────────────────────────────

#[derive(BevyComponentDerive, Clone, Copy)]
#[allow(dead_code)]
struct BevyV2Pos {
    x: f32,
    y: f32,
    z: f32,
}

// ===========================================================================
// H2 — EcsMaster::new() pre-extends fast-stores to 72 192 slots
// ===========================================================================
//
// HISTORICAL (Phase 12.5 wording): `entities_inland` and `sparse_to_active`
// pre-extended to `MAX_ENTITIES_HINT + MAX_BATCH_HINT = 64_000 + 8_192 =
// 72_192`. Phase 12.6 made growth lazy; Phase X.D deleted
// `sparse_to_active`; Phase X.G replaced the Vec with an address-stable
// reserve/commit `InlandStore` (growth = frontier commit, no realloc).
//
// Memory layout:
//   - EntityInland = 16 B (ptr + u32 + u32) × 72 192 = 1.155 MB
//   - sparse_to_active = u32 × 72 192 = 288 KB
//   - bundle_archetype_cache = Box<[OnceLock<ArchetypeId>; 1024]>
//   - bundle_column_cache = Box<[OnceLock<BundleColumnRecord>; 1024]>
//   - query_state_cache = Box<[OnceLock<QueryCacheSlot>; 1024]>
//   - arena = Box<Arena> (separate allocation)
//
// Hypothesis: rebuilding the world per criterion iter pays a non-trivial
// setup cost (allocation + zero-init of ~1.5 MB of fast-store slots).
//
// Expected: ≥ 50 µs / world if H2 is the regression driver.

fn h2_ecs_master_new(c: &mut Criterion) {
    register_v2_components();
    c.bench_function("h2_ecs_master_new", |b| {
        b.iter(|| {
            let world = EcsMaster::new();
            black_box(world);
        });
    });
}

fn h2_ecs_master_new_with_capacity_default(c: &mut Criterion) {
    register_v2_components();
    c.bench_function("h2_ecs_master_with_capacity_64k", |b| {
        b.iter(|| {
            let world = EcsMaster::with_capacity(64_000, 256);
            black_box(world);
        });
    });
}

fn h2_bevy_world_new(c: &mut Criterion) {
    c.bench_function("h2_bevy_world_new", |b| {
        b.iter(|| {
            let world = World::new();
            black_box(world);
        });
    });
}

// ===========================================================================
// H5 — Pure enqueue cost: Commands::spawn × 10k WITHOUT apply
// ===========================================================================
//
// We need a way to measure enqueue without the apply pass that comes
// implicitly with `run_system`. The cleanest is to expose the queue's bytes
// length after the body returns — but `__test_bytes_len` is doc-hidden and
// only `pub`. We use it here from the bench (the lib exposes it as
// `#[doc(hidden)] pub fn __test_apply` and friends).
//
// To avoid the apply, we drop the world before SystemParam::apply can flush.
// That is hard without re-entering the system; instead we use a closure-based
// "manual" `CommandQueue::__test_apply` to verify the cost shape.
//
// Approach: keep a single fixed world. Set up a system body that spawns one
// entity per call (NOT 10k). Run that 10k times in the bench loop. Subtract
// the apply cost of one SpawnAtCommand from the whole 10k-per-iter run.
//
// Simpler: instrument inside the closure. We bracket the enqueue body alone
// with Instant::now (we already know each pair costs ~60 ns on Windows; for
// 10k entities the floor is 60 ns / 10k = 0.006 ns/entity — negligible if
// we bracket the *whole* 10k-entity body, not each entity).

static H5_ENQUEUE_NS: AtomicU64 = AtomicU64::new(0);
static H5_ITERS: AtomicUsize = AtomicUsize::new(0);

fn h5_commands_spawn_enqueue_only_10k(c: &mut Criterion) {
    register_v2_components();
    H5_ENQUEUE_NS.store(0, Ordering::Relaxed);
    H5_ITERS.store(0, Ordering::Relaxed);

    c.bench_function("h5_commands_spawn_enqueue_only_10k", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            world.run_system(|mut cmds: BoykoCommands| {
                let t0 = Instant::now();
                for i in 0..N_ENTITIES {
                    cmds.spawn(V2PosBundle {
                        pos: V2Position {
                            x: i as f32,
                            y: 0.0,
                            z: 0.0,
                        },
                    });
                }
                let t1 = Instant::now();
                H5_ENQUEUE_NS
                    .fetch_add(t1.duration_since(t0).as_nanos() as u64, Ordering::Relaxed);
                H5_ITERS.fetch_add(1, Ordering::Relaxed);
            });
            // The apply runs here (when run_system returns) BUT we've
            // captured the enqueue-only time in H5_ENQUEUE_NS above.
            black_box(&world);
        });
    });

    let iters = H5_ITERS.load(Ordering::Relaxed).max(1);
    let ns = H5_ENQUEUE_NS.load(Ordering::Relaxed);
    eprintln!();
    eprintln!(
        "h5 enqueue-only: {:.2} ns/entity ({} iters × {} entities)",
        ns as f64 / (iters * N_ENTITIES) as f64,
        iters,
        N_ENTITIES
    );
    eprintln!();
}

// ===========================================================================
// H5b — Same shape but isolating CommandQueue::apply on the spawn_at path
// ===========================================================================
//
// We want to know how much time the APPLY pass takes vs ENQUEUE. The
// `run_system` body bench above (h5) excludes the apply by reading the inner
// Instant pair, so the difference between H5 wall time per iter and h5
// enqueue is the apply time. We bracket the FULL `run_system` call instead.

static H5B_TOTAL_NS: AtomicU64 = AtomicU64::new(0);
static H5B_ITERS: AtomicUsize = AtomicUsize::new(0);

fn h5b_commands_spawn_total_10k(c: &mut Criterion) {
    register_v2_components();
    H5B_TOTAL_NS.store(0, Ordering::Relaxed);
    H5B_ITERS.store(0, Ordering::Relaxed);

    c.bench_function("h5b_commands_spawn_total_10k", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            let t0 = Instant::now();
            world.run_system(|mut cmds: BoykoCommands| {
                for i in 0..N_ENTITIES {
                    cmds.spawn(V2PosBundle {
                        pos: V2Position {
                            x: i as f32,
                            y: 0.0,
                            z: 0.0,
                        },
                    });
                }
            });
            let t1 = Instant::now();
            H5B_TOTAL_NS
                .fetch_add(t1.duration_since(t0).as_nanos() as u64, Ordering::Relaxed);
            H5B_ITERS.fetch_add(1, Ordering::Relaxed);
            black_box(&world);
        });
    });

    let iters = H5B_ITERS.load(Ordering::Relaxed).max(1);
    let ns = H5B_TOTAL_NS.load(Ordering::Relaxed);
    eprintln!();
    eprintln!(
        "h5b total per run: {:.2} ns/entity ({} iters × {} entities)",
        ns as f64 / (iters * N_ENTITIES) as f64,
        iters,
        N_ENTITIES
    );
    eprintln!();
}

// ===========================================================================
// H3 — SpawnBatchCommand::apply per-entity cost (via the direct path)
// ===========================================================================
//
// The plan claims SpawnBatchCommand::apply costs ~50 ns/entity headline.
// We measure the full direct-path `EcsMaster::spawn_batch` to get the apply
// cost without the Commands routing overhead. The route is:
//
//   spawn_batch -> reserve_batch (one atomic) -> SpawnBatchCommand::apply
//
// The apply does:
//   1. cached_archetype_id resolve (one OnceLock::get)
//   2. archetype_ptr_for (slab lookup)
//   3. bundle_column_cache get_resolved (one OnceLock::get) OR resolve_and_cache
//   4. SBO17b runtime guard
//   5. archetype.reserve_capacity (per-pool can_reserve)
//   6. per-row: iter.next() + for_each_component_bytes + per-component pool write
//   7. commit_units_batch + fill_ticks_batch
//   8. archetype.entity_ids push loop
//   9. EntityMaster::register_batch
//   10. Vec<Entity> materialisation (W3 ergonomic alloc)
//
// We bench BOTH the warm path (cache populated) AND the cold path (first
// call per world) to separate the 250 ns once-per-world cost from the
// per-entity cost.

fn h3_spawn_batch_direct_10k_cold(c: &mut Criterion) {
    register_v2_components();
    c.bench_function("h3_spawn_batch_direct_10k_cold", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            // First call per world: cold path (resolve_and_cache fires).
            for chunk in 0..(N_ENTITIES / BATCH_CHUNK) {
                let base = chunk * BATCH_CHUNK;
                let iter = (0..BATCH_CHUNK).map(move |i| V2PosBundle {
                    pos: V2Position {
                        x: (base + i) as f32,
                        y: 0.0,
                        z: 0.0,
                    },
                });
                let _ = world.spawn_batch(iter).expect("spawn_batch");
            }
            black_box(&world);
        });
    });
}

fn h3_spawn_batch_direct_warm_5k(c: &mut Criterion) {
    register_v2_components();
    c.bench_function("h3_spawn_batch_direct_warm_5k", |b| {
        b.iter_with_setup(
            || {
                // Pre-warm the cache by doing one tiny spawn_batch.
                // We need to keep the world for the bench body without the
                // tiny spawn skewing measurements; only the cache state
                // (which is cold otherwise) matters.
                let mut world = EcsMaster::new();
                let iter = (0..1).map(|i| V2PosBundle {
                    pos: V2Position {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    },
                });
                let _ = world.spawn_batch(iter).expect("warm-up");
                world
            },
            |mut world| {
                // Now measure the warm path with 5k entities.
                let iter = (0..BATCH_CHUNK).map(|i| V2PosBundle {
                    pos: V2Position {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    },
                });
                let _ = world.spawn_batch(iter).expect("spawn_batch");
                black_box(&world);
            },
        );
    });
}

// ===========================================================================
// H6 — Direct create_entity_at_with_pool_ids vs legacy create_entity
// ===========================================================================
//
// Opt-A3 wired SpawnAtCommand to use the cached pool-ids variant. The plan
// claims this saves ~10-20 ns/entity by skipping the 4× SparseMap lookup.
// We bench both paths to confirm or refute.

fn h6_direct_create_entity_legacy_10k(c: &mut Criterion) {
    register_v2_components();
    c.bench_function("h6_direct_create_entity_legacy_10k", |b| {
        b.iter_with_setup(
            || {
                let mut world = EcsMaster::new();
                let arch = world.create_archetype(&[V2_POS_ID]);
                (world, arch)
            },
            |(mut world, arch)| {
                for i in 0..N_ENTITIES {
                    let pos = V2Position {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    };
                    let bytes: &[u8] = unsafe {
                        std::slice::from_raw_parts(
                            std::ptr::addr_of!(pos) as *const u8,
                            std::mem::size_of::<V2Position>(),
                        )
                    };
                    world
                        .create_entity(arch, &[(V2_POS_ID, bytes)])
                        .expect("create_entity");
                }
                black_box(&world);
            },
        );
    });
}

fn h6_spawn_one_baseline_10k(c: &mut Criterion) {
    // Same as profile_spawn p4 — used as a comparison reference.
    register_v2_components();
    c.bench_function("h6_spawn_one_baseline_10k", |b| {
        b.iter_with_setup(
            || {
                let mut world = EcsMaster::new();
                let arch = world.create_archetype(&[V2_POS_ID]);
                (world, arch)
            },
            |(mut world, arch)| {
                for i in 0..N_ENTITIES {
                    world
                        .spawn_one(
                            arch,
                            V2Position {
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
// H1 — Cached_archetype_id warm hit cost (microbench)
// ===========================================================================
//
// Plan claims ~2-3 ns warm hit via Bundle::cached_archetype_id. The
// BundleColumnCache is `pub(crate)` so we can't directly bench it from
// outside; we bench the public surface that exercises an OnceLock::get
// of similar shape (Bundle::cached_archetype_id).
//
// Once warmed, this should be sub-3-ns. If significantly higher we've
// regressed the warm-path.

fn h1_cached_archetype_id_warm(c: &mut Criterion) {
    register_v2_components();
    // Set up one world; spawn one entity to warm both caches.
    let mut world = EcsMaster::new();
    let _ = world
        .spawn_batch((0..1).map(|i| V2PosBundle {
            pos: V2Position {
                x: i as f32,
                y: 0.0,
                z: 0.0,
            },
        }))
        .expect("warm-up");

    static SINK: AtomicUsize = AtomicUsize::new(0);

    c.bench_function("h1_cached_archetype_id_warm", |b| {
        b.iter(|| {
            // Warm-path bundle archetype id lookup. This hits the
            // `bundle_archetype_cache` (not the bundle_column_cache).
            // Same OnceLock::get shape — informative as a comparable.
            let id = V2PosBundle::cached_archetype_id(&mut world);
            SINK.store(black_box(id.0), Ordering::Relaxed);
        });
    });
}

// ===========================================================================
// H4 — for_each_component_bytes callback dispatch (1-comp and 3-comp)
// ===========================================================================
//
// Repeats profile_spawn p6 to confirm the bundle walk is fast. If it's
// still ~1.5 ns/entity for 1-comp, the regression is NOT here.

fn h4_bundle_walk_1comp(c: &mut Criterion) {
    register_v2_components();
    static SINK: AtomicU64 = AtomicU64::new(0);

    c.bench_function("h4_bundle_walk_1comp", |b| {
        b.iter(|| {
            for i in 0..N_ENTITIES {
                let bundle = black_box(V2PosBundle {
                    pos: V2Position {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    },
                });
                bundle.for_each_component_bytes(|id, bytes| {
                    SINK.fetch_add(
                        bytes.len() as u64 ^ id.0 as u64,
                        Ordering::Relaxed,
                    );
                });
            }
        });
    });
}

fn h4_bundle_walk_3comp(c: &mut Criterion) {
    register_v2_components();
    static SINK: AtomicU64 = AtomicU64::new(0);

    c.bench_function("h4_bundle_walk_3comp", |b| {
        b.iter(|| {
            for i in 0..N_ENTITIES {
                let bundle = black_box(V2Bundle3 {
                    pos: V2Position {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    },
                    vel: V2Velocity {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    tag: V2Tag { flags: i as u32 },
                });
                bundle.for_each_component_bytes(|id, bytes| {
                    SINK.fetch_add(
                        bytes.len() as u64 ^ id.0 as u64,
                        Ordering::Relaxed,
                    );
                });
            }
        });
    });
}

// ===========================================================================
// H7 — Bevy structural comparison: World::new + spawn_batch
// ===========================================================================

fn h7_bevy_world_new_then_spawn_batch_10k(c: &mut Criterion) {
    c.bench_function("h7_bevy_world_new_then_spawn_batch_10k", |b| {
        b.iter_with_setup(World::new, |mut world| {
            let _ = world.run_system_once(|mut cmds: BevyCommands| {
                let iter = (0..N_ENTITIES).map(|i| BevyV2Pos {
                    x: i as f32,
                    y: 0.0,
                    z: 0.0,
                });
                cmds.spawn_batch(iter);
            });
            black_box(&world);
        });
    });
}

// ===========================================================================
// H8 — Boyko spawn_batch via Commands × 10k (chunked 2×5K) — head-to-head
// ===========================================================================
//
// Mirrors comparison_v2 g5_boyko. Re-runs here so the wall time is in the
// same profile-bench run as h2/h3/h5/h6 for direct comparison.

fn h8_boyko_commands_spawn_batch_10k(c: &mut Criterion) {
    register_v2_components();
    c.bench_function("h8_boyko_commands_spawn_batch_10k", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            world.run_system(|mut cmds: BoykoCommands| {
                for chunk in 0..(N_ENTITIES / BATCH_CHUNK) {
                    let base = chunk * BATCH_CHUNK;
                    let iter = (0..BATCH_CHUNK).map(move |i| V2PosBundle {
                        pos: V2Position {
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
        });
    });
}

// ===========================================================================
// H9 — Per-stage instrumentation of EcsMaster::spawn_batch (5k entities)
// ===========================================================================
//
// Inserts Instant checkpoints around the SpawnBatchCommand::apply hot path
// stages by reimplementing the spawn_batch body around the public API. We
// cannot reach inside `spawn_batch`'s private body, so we time the whole
// 5k-entity call vs the equivalent direct-Commands path to isolate the
// per-entity cost of the apply body alone.

static H9_TOTAL_NS: AtomicU64 = AtomicU64::new(0);
static H9_RESERVE_NS: AtomicU64 = AtomicU64::new(0);
static H9_APPLY_NS: AtomicU64 = AtomicU64::new(0);
static H9_ITERS: AtomicUsize = AtomicUsize::new(0);

fn h9_spawn_batch_stages_5k(c: &mut Criterion) {
    register_v2_components();
    H9_TOTAL_NS.store(0, Ordering::Relaxed);
    H9_RESERVE_NS.store(0, Ordering::Relaxed);
    H9_APPLY_NS.store(0, Ordering::Relaxed);
    H9_ITERS.store(0, Ordering::Relaxed);

    c.bench_function("h9_spawn_batch_stages_5k", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            // Pre-warm cache so cold-path costs don't pollute the warm
            // numbers. ONE entity is enough to populate
            // bundle_archetype_cache, bundle_column_cache, and the archetype.
            let _ = world
                .spawn_batch((0..1).map(|i| V2PosBundle {
                    pos: V2Position {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    },
                }))
                .expect("warm-up");

            let t0 = Instant::now();
            let iter = (0..BATCH_CHUNK).map(|i| V2PosBundle {
                pos: V2Position {
                    x: i as f32,
                    y: 0.0,
                    z: 0.0,
                },
            });
            let _ = world.spawn_batch(iter).expect("spawn_batch");
            let t1 = Instant::now();
            H9_TOTAL_NS
                .fetch_add(t1.duration_since(t0).as_nanos() as u64, Ordering::Relaxed);
            H9_ITERS.fetch_add(1, Ordering::Relaxed);
            black_box(&world);
        });
    });

    let iters = H9_ITERS.load(Ordering::Relaxed).max(1);
    let total = H9_TOTAL_NS.load(Ordering::Relaxed);
    eprintln!();
    eprintln!(
        "h9 spawn_batch_direct_warm_5k (1+5000 entities): {:.2} ns/entity ({} iters)",
        total as f64 / (iters * BATCH_CHUNK) as f64,
        iters
    );
    eprintln!();
}

// ===========================================================================
// H10 — Vec<Entity> materialisation overhead in EcsMaster::spawn_batch
// ===========================================================================
//
// The direct path always allocates a `Vec<Entity>` of length n. For a 5k
// batch that is 40 KB of allocation + zero-init + n push() calls. Is that
// material?

fn h10_vec_entity_alloc_5k(c: &mut Criterion) {
    use boyko_ecs::ecs::core::entity::entity::Entity;
    use boyko_ecs::ecs::identifiers::primitives::EntityId;

    c.bench_function("h10_vec_entity_materialisation_5k", |b| {
        b.iter(|| {
            let mut result: Vec<Entity> = Vec::with_capacity(BATCH_CHUNK);
            for i in 0..BATCH_CHUNK {
                result.push(Entity::new(EntityId(i), 0));
            }
            black_box(result);
        });
    });
}

// ===========================================================================
// H11 — Single SpawnAtCommand apply via Commands::spawn (apply cost shape)
// ===========================================================================
//
// Run JUST ONE Commands::spawn followed by run_system's apply. We compare
// (1 entity) vs (10k entities) total times to derive per-entity SpawnAtCommand
// apply cost.

fn h11_one_commands_spawn(c: &mut Criterion) {
    register_v2_components();
    c.bench_function("h11_one_commands_spawn", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            world.run_system(|mut cmds: BoykoCommands| {
                cmds.spawn(V2PosBundle {
                    pos: V2Position {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                });
            });
            black_box(&world);
        });
    });
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
    name = profile_spawn_v2;
    config = configure();
    targets =
        h2_ecs_master_new,
        h2_ecs_master_new_with_capacity_default,
        h2_bevy_world_new,
        h5_commands_spawn_enqueue_only_10k,
        h5b_commands_spawn_total_10k,
        h3_spawn_batch_direct_10k_cold,
        h3_spawn_batch_direct_warm_5k,
        h6_direct_create_entity_legacy_10k,
        h6_spawn_one_baseline_10k,
        h1_cached_archetype_id_warm,
        h4_bundle_walk_1comp,
        h4_bundle_walk_3comp,
        h7_bevy_world_new_then_spawn_batch_10k,
        h8_boyko_commands_spawn_batch_10k,
        h9_spawn_batch_stages_5k,
        h10_vec_entity_alloc_5k,
        h11_one_commands_spawn,
}

criterion_main!(profile_spawn_v2);
