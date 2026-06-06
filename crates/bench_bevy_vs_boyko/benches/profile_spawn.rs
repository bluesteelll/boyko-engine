//! Phase 12.5 — Spawn-path profile bench.
//!
//! This file is a **measurement-only** instrument: it must not be checked
//! into the spawn hot path of `boyko_ecs`. Its job is to attribute the
//! 514 µs delta against Bevy on the 10 000-entity Commands::spawn workload
//! (boyko 1.044 ms vs bevy 530 µs in the head-to-head comparison.rs).
//!
//! # Method
//!
//! We replicate the *exact* workload of `bench_boyko_commands_spawn_10k`
//! from `comparison.rs` — `BoykoPosBundle { pos: BoykoPosition }`, 10 000
//! `Commands::spawn` calls inside a single system, world rebuilt per iter.
//! Then we decompose it into the hypothesised stages from
//! `docs/PHASE-12.5-SURPASS-BEVY-PLAN.md` §P1 by instrumenting the same
//! call sequence with `std::time::Instant` checkpoints.
//!
//! Criterion handles the outer iteration. Inside one iter we materialise
//! the per-stage cumulative timings into static atomic accumulators (so
//! they survive across criterion's measurement of the bench-function
//! body) and then report them at the end.
//!
//! Because the inner `Instant::now` calls themselves cost ~25 ns on
//! Windows, we measure them in a separate calibration bench so we can
//! subtract the floor when interpreting the numbers.
//!
//! # Workload truth
//!
//! The brief mentioned "3-component bundle" but `comparison.rs` actually
//! ships a 1-component bundle (`BoykoPosBundle { pos }` of 12 bytes /
//! `f32 x 3`). We mirror that exactly here so the µs/ns numbers line up.
//! A separate 3-component variant is added so the future spawn_batch
//! design has a benchmark to gate against.

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
use boyko_ecs::ecs::core::component::component::Component as BoykoComponent;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands as BoykoCommands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

// ── bevy imports (for the structural comparison side bench only) ──────────
use bevy_ecs::prelude::Commands as BevyCommands;
use bevy_ecs::prelude::Component as BevyComponentDerive;
use bevy_ecs::prelude::*;
use bevy_ecs::system::RunSystemOnce;

// ── Shared workload constants (must match comparison.rs) ──────────────────

const N_ENTITIES: usize = 10_000;

// Reserve a separate `ComponentId` from the head-to-head bench so the two
// suites do not race each other in the global component registry.
// comparison.rs uses slot 350; we pick 351 for the 1-component path and
// 352/353/354 for the 3-component variant. MAX_COMPONENTS = 512.
const PROFILE_POS_ID: ComponentId = ComponentId(351);
const PROFILE_VEL_ID: ComponentId = ComponentId(352);
const PROFILE_TAG_ID: ComponentId = ComponentId(353);

#[repr(C)]
#[derive(Clone, Copy)]
struct ProfilePosition {
    x: f32,
    y: f32,
    z: f32,
}

impl BoykoComponent for ProfilePosition {
    fn component_id() -> ComponentId {
        PROFILE_POS_ID
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProfileVelocity {
    x: f32,
    y: f32,
    z: f32,
}

impl BoykoComponent for ProfileVelocity {
    fn component_id() -> ComponentId {
        PROFILE_VEL_ID
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProfileTag {
    flags: u32,
}

impl BoykoComponent for ProfileTag {
    fn component_id() -> ComponentId {
        PROFILE_TAG_ID
    }
}

#[derive(Bundle)]
struct ProfilePosBundle {
    pos: ProfilePosition,
}

#[derive(Bundle)]
struct Profile3Bundle {
    pos: ProfilePosition,
    vel: ProfileVelocity,
    tag: ProfileTag,
}

fn register_profile_components() {
    register_layout::<ProfilePosition>(PROFILE_POS_ID.0);
    register_layout::<ProfileVelocity>(PROFILE_VEL_ID.0);
    register_layout::<ProfileTag>(PROFILE_TAG_ID.0);
}

// ── bevy mirror types (for structural comparison only) ─────────────────────

#[derive(BevyComponentDerive, Clone, Copy)]
#[allow(dead_code)]
struct BevyPositionProf {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(BevyComponentDerive, Clone, Copy)]
#[allow(dead_code)]
struct BevyVelocityProf {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(BevyComponentDerive, Clone, Copy)]
#[allow(dead_code)]
struct BevyTagProf {
    flags: u32,
}

// ===========================================================================
// Stage accumulators
// ===========================================================================
//
// Criterion measures *outer* time, but we also need the per-stage breakdown
// summed across all 10 000 inner iterations. These atomics accumulate
// nanoseconds across one criterion bench's iters and are then divided by
// the total per-entity invocation count at report time.
//
// One AtomicU64 per stage. `fetch_add` with Relaxed is ~1 ns on the dispatcher
// (single thread) — small enough to ignore against the 100+ ns workload.

static STAGE_ITER_COUNT: AtomicUsize = AtomicUsize::new(0);
static STAGE_TOTAL_NS: AtomicU64 = AtomicU64::new(0);
static STAGE_CMDS_ENQUEUE_NS: AtomicU64 = AtomicU64::new(0);
static STAGE_APPLY_TOTAL_NS: AtomicU64 = AtomicU64::new(0);

// Per-entity breakdown inside SpawnAtCommand::apply (measured via the
// direct `EcsMaster::create_entity` path, which is the same code from
// step 2 onwards). Because we cannot instrument the production code, we
// reach into the public API and time the equivalent sequence: lookup
// cached_archetype_id, mint EntityCounter, do the actual create.
static STAGE_ARCH_LOOKUP_NS: AtomicU64 = AtomicU64::new(0);
static STAGE_ENT_RESERVE_NS: AtomicU64 = AtomicU64::new(0);
static STAGE_CREATE_ENTITY_NS: AtomicU64 = AtomicU64::new(0);
static STAGE_BUNDLE_WALK_NS: AtomicU64 = AtomicU64::new(0);

fn reset_stages() {
    STAGE_ITER_COUNT.store(0, Ordering::Relaxed);
    STAGE_TOTAL_NS.store(0, Ordering::Relaxed);
    STAGE_CMDS_ENQUEUE_NS.store(0, Ordering::Relaxed);
    STAGE_APPLY_TOTAL_NS.store(0, Ordering::Relaxed);
    STAGE_ARCH_LOOKUP_NS.store(0, Ordering::Relaxed);
    STAGE_ENT_RESERVE_NS.store(0, Ordering::Relaxed);
    STAGE_CREATE_ENTITY_NS.store(0, Ordering::Relaxed);
    STAGE_BUNDLE_WALK_NS.store(0, Ordering::Relaxed);
}

fn print_stage_report(label: &str) {
    let iters = STAGE_ITER_COUNT.load(Ordering::Relaxed).max(1);
    let entities = iters * N_ENTITIES;

    let total = STAGE_TOTAL_NS.load(Ordering::Relaxed);
    let enq = STAGE_CMDS_ENQUEUE_NS.load(Ordering::Relaxed);
    let app = STAGE_APPLY_TOTAL_NS.load(Ordering::Relaxed);
    let arch = STAGE_ARCH_LOOKUP_NS.load(Ordering::Relaxed);
    let res = STAGE_ENT_RESERVE_NS.load(Ordering::Relaxed);
    let cre = STAGE_CREATE_ENTITY_NS.load(Ordering::Relaxed);
    let bw = STAGE_BUNDLE_WALK_NS.load(Ordering::Relaxed);

    eprintln!();
    eprintln!("════════════ STAGE BREAKDOWN — {} ════════════", label);
    eprintln!("iters={} (one iter = 10k spawns); total_entities={}", iters, entities);
    eprintln!("(per-entity nanoseconds; raw Instant overhead is NOT subtracted)");
    eprintln!();
    eprintln!("    total wall clock                    {:>10.2} ns/entity", total as f64 / entities as f64);
    eprintln!("    Commands::spawn enqueue (loop body) {:>10.2} ns/entity", enq as f64 / entities as f64);
    eprintln!("    CommandQueue::apply outer + per-cmd {:>10.2} ns/entity", app as f64 / entities as f64);
    eprintln!();
    eprintln!("  Direct (no Commands) breakdown:");
    eprintln!("    cached_archetype_id lookup          {:>10.2} ns/entity", arch as f64 / entities as f64);
    eprintln!("    EntityMaster reserve (counter rmw)  {:>10.2} ns/entity", res as f64 / entities as f64);
    eprintln!("    EcsMaster::create_entity            {:>10.2} ns/entity", cre as f64 / entities as f64);
    eprintln!("    Bundle::for_each_component_bytes    {:>10.2} ns/entity", bw as f64 / entities as f64);
    eprintln!("══════════════════════════════════════════════════");
    eprintln!();
}

// ===========================================================================
// Bench 0 — Instant::now() calibration
// ===========================================================================
//
// On Windows the system clock used by `Instant::now` is a QPC-based monotonic
// counter; each pair of `now()` calls costs ~20-30 ns. We measure that floor
// here so the reader can interpret the per-stage numbers.

fn bench_instant_now_calibration(c: &mut Criterion) {
    c.bench_function("p0_instant_now_pair", |b| {
        b.iter(|| {
            let t0 = Instant::now();
            let t1 = Instant::now();
            black_box(t1.duration_since(t0))
        });
    });
}

// ===========================================================================
// Bench 1 — Full Commands::spawn × 10k baseline, no decomposition
// ===========================================================================
//
// Identical to comparison.rs `bench_boyko_commands_spawn_10k`. This is the
// reference number we must explain.

fn bench_boyko_commands_spawn_10k_baseline(c: &mut Criterion) {
    register_profile_components();

    c.bench_function("p1_boyko_commands_spawn_10k_baseline", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            world.run_system(|mut cmds: BoykoCommands| {
                for i in 0..N_ENTITIES {
                    cmds.spawn(ProfilePosBundle {
                        pos: ProfilePosition {
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
// Bench 2 — Decomposed Commands::spawn: enqueue vs apply windows
// ===========================================================================
//
// We bracket the system body's enqueue loop with one Instant, then bracket
// the apply phase by reading the total `run_system` cost minus the enqueue.
// `run_system` triggers `SystemParam::apply` AFTER the body returns, so the
// difference `total - enqueue_inner` is exactly the apply window for the
// 10 000 commands plus the SystemParam::apply wrapper.

fn bench_boyko_commands_spawn_10k_decomposed(c: &mut Criterion) {
    register_profile_components();
    reset_stages();

    // The system closure passed to `run_system` must be `'static` (Bevy
    // parity). We thread the inner body timing through a static atomic
    // accumulator instead of a borrow.
    static ENQUEUE_NS_THIS_ITER: AtomicU64 = AtomicU64::new(0);

    c.bench_function("p2_boyko_commands_spawn_10k_decomposed", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            ENQUEUE_NS_THIS_ITER.store(0, Ordering::Relaxed);
            let t_start = Instant::now();

            world.run_system(|mut cmds: BoykoCommands| {
                let t_body_start = Instant::now();
                for i in 0..N_ENTITIES {
                    cmds.spawn(ProfilePosBundle {
                        pos: ProfilePosition {
                            x: i as f32,
                            y: 0.0,
                            z: 0.0,
                        },
                    });
                }
                let t_body_end = Instant::now();
                ENQUEUE_NS_THIS_ITER.store(
                    t_body_end.duration_since(t_body_start).as_nanos() as u64,
                    Ordering::Relaxed,
                );
            });

            let t_end = Instant::now();
            let total_ns = t_end.duration_since(t_start).as_nanos() as u64;
            let enqueue_ns = ENQUEUE_NS_THIS_ITER.load(Ordering::Relaxed);
            let apply_ns = total_ns.saturating_sub(enqueue_ns);

            STAGE_ITER_COUNT.fetch_add(1, Ordering::Relaxed);
            STAGE_TOTAL_NS.fetch_add(total_ns, Ordering::Relaxed);
            STAGE_CMDS_ENQUEUE_NS.fetch_add(enqueue_ns, Ordering::Relaxed);
            STAGE_APPLY_TOTAL_NS.fetch_add(apply_ns, Ordering::Relaxed);

            black_box(&world);
        });
    });

    print_stage_report("p2 Commands path (enqueue vs apply)");
}

// ===========================================================================
// Bench 3 — Direct path: EcsMaster::create_entity (no Commands) × 10k
// ===========================================================================
//
// This is the same final destination as `SpawnAtCommand::apply` —
// archetype resolve, entity allocate, archetype.create_entity, fast-store
// register — minus:
//   1. The Commands enqueue (`CommandQueue::push` × 10k)
//   2. The CommandQueue::apply outer loop dispatch (10k indirect calls
//      through the `consume_and_drop_glue` fn pointer)
//   3. The per-command read_unaligned of meta + payload
//   4. The MaybeUninit stack-slots dance in SpawnAtCommand::apply
//   5. The for_each_component_bytes callback per command
//
// Comparing p3 to p2 isolates the Commands tax.

fn bench_boyko_direct_create_entity_10k(c: &mut Criterion) {
    register_profile_components();

    c.bench_function("p3_boyko_direct_create_entity_10k", |b| {
        b.iter_with_setup(
            || {
                let mut world = EcsMaster::new();
                let arch = world.create_archetype(&[PROFILE_POS_ID]);
                (world, arch)
            },
            |(mut world, arch)| {
                for i in 0..N_ENTITIES {
                    let pos = ProfilePosition {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    };
                    let bytes: &[u8] = unsafe {
                        std::slice::from_raw_parts(
                            std::ptr::addr_of!(pos) as *const u8,
                            std::mem::size_of::<ProfilePosition>(),
                        )
                    };
                    world
                        .create_entity(arch, &[(PROFILE_POS_ID, bytes)])
                        .expect("create_entity must succeed");
                    // `pos` is Copy + memcpy'd into pool → no leak.
                }
                black_box(&world);
            },
        );
    });
}

// ===========================================================================
// Bench 4 — Direct path: EcsMaster::spawn_one × 10k
// ===========================================================================
//
// `spawn_one` is the typed convenience wrapper that takes the value by
// move and forwards to `create_entity`. The cost gap p4 - p3 should be
// near zero (just the slice-from-raw-parts boilerplate that p3 inlined).

fn bench_boyko_spawn_one_10k(c: &mut Criterion) {
    register_profile_components();

    c.bench_function("p4_boyko_spawn_one_10k", |b| {
        b.iter_with_setup(
            || {
                let mut world = EcsMaster::new();
                let arch = world.create_archetype(&[PROFILE_POS_ID]);
                (world, arch)
            },
            |(mut world, arch)| {
                for i in 0..N_ENTITIES {
                    world
                        .spawn_one(
                            arch,
                            ProfilePosition {
                                x: i as f32,
                                y: 0.0,
                                z: 0.0,
                            },
                        )
                        .expect("spawn_one must succeed");
                }
                black_box(&world);
            },
        );
    });
}

// ===========================================================================
// Bench 5 — Direct path with stage-internal instrumentation
// ===========================================================================
//
// Inside the per-entity loop we checkpoint:
//   a) cached_archetype_id (= the archetype lookup the apply path would
//      have done; here we use the pre-resolved id, so we measure just the
//      Acquire load + bounds check pattern from `ArchetypeMaster::has_archetype`)
//   b) entity reserve (`EntityCounter::reserve_entity` semantic via
//      `EntityMaster::next_id_atomic` — single atomic RMW)
//   c) `create_entity` end-to-end (= the work inside SpawnAtCommand::apply
//      from line 161 onwards minus the for_each_component_bytes callback)
//
// Each Instant pair costs ~25 ns so the absolute numbers include
// 4 × 25 ns = 100 ns of measurement floor per entity. The shape of the
// breakdown is still informative.

fn bench_boyko_direct_create_entity_10k_instrumented(c: &mut Criterion) {
    register_profile_components();
    reset_stages();

    c.bench_function("p5_boyko_direct_instrumented", |b| {
        b.iter_with_setup(
            || {
                let mut world = EcsMaster::new();
                let arch = world.create_archetype(&[PROFILE_POS_ID]);
                (world, arch)
            },
            |(mut world, arch)| {
                let t_start = Instant::now();
                let mut arch_ns: u64 = 0;
                let mut res_ns: u64 = 0;
                let mut cre_ns: u64 = 0;

                for i in 0..N_ENTITIES {
                    // (a) archetype lookup — proxy for `cached_archetype_id`
                    // cache hit: a single OnceLock::get Acquire load
                    // followed by a slice index. Measured as has_archetype
                    // (a single ArchetypeMaster lookup).
                    let t0 = Instant::now();
                    let exists = world.archetype_master().has_archetype(arch);
                    black_box(exists);
                    let t1 = Instant::now();

                    // (b) entity reserve — proxy for the atomic fetch_add
                    // that EntityCounter::reserve_entity does on the Commands
                    // path. We use the dispatcher equivalent.
                    let _e = world.entity_master().next_entity_id();
                    let t2 = Instant::now();

                    // (c) full create_entity — same code SpawnAtCommand::apply
                    // jumps into at line 161 via create_entity_at, minus the
                    // for_each_component_bytes setup costs.
                    let pos = ProfilePosition {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    };
                    let bytes: &[u8] = unsafe {
                        std::slice::from_raw_parts(
                            std::ptr::addr_of!(pos) as *const u8,
                            std::mem::size_of::<ProfilePosition>(),
                        )
                    };
                    world
                        .create_entity(arch, &[(PROFILE_POS_ID, bytes)])
                        .expect("create_entity must succeed");
                    let t3 = Instant::now();

                    arch_ns += t1.duration_since(t0).as_nanos() as u64;
                    res_ns += t2.duration_since(t1).as_nanos() as u64;
                    cre_ns += t3.duration_since(t2).as_nanos() as u64;
                }

                let t_end = Instant::now();
                let total_ns = t_end.duration_since(t_start).as_nanos() as u64;

                STAGE_ITER_COUNT.fetch_add(1, Ordering::Relaxed);
                STAGE_TOTAL_NS.fetch_add(total_ns, Ordering::Relaxed);
                STAGE_ARCH_LOOKUP_NS.fetch_add(arch_ns, Ordering::Relaxed);
                STAGE_ENT_RESERVE_NS.fetch_add(res_ns, Ordering::Relaxed);
                STAGE_CREATE_ENTITY_NS.fetch_add(cre_ns, Ordering::Relaxed);

                black_box(&world);
            },
        );
    });

    print_stage_report("p5 direct create_entity (per-stage)");
}

// ===========================================================================
// Bench 6 — Bundle::for_each_component_bytes isolated cost
// ===========================================================================
//
// Run the bundle's callback chain 10 000 times against a noop closure.
// This measures the macro-generated ManuallyDrop + sort_unstable + slice
// rebuild overhead on the boyko side. It is what SpawnAtCommand::apply
// pays in addition to the create_entity payload.
//
// SAFETY: ProfilePosBundle is Copy because all its fields are Copy and
// it derives no manual Drop. We can call for_each_component_bytes on a
// copy of the value safely.

fn bench_boyko_bundle_walk_only_10k(c: &mut Criterion) {
    register_profile_components();
    reset_stages();
    use boyko_ecs::ecs::core::bundle::Bundle as BoykoBundle;

    // Sink lives in a static so the optimiser cannot prove the closure
    // body is dead. We also publish through a black_box of the bundle to
    // foil dead-store elimination.
    static SINK: AtomicU64 = AtomicU64::new(0);

    c.bench_function("p6_boyko_bundle_walk_only_10k_1comp", |b| {
        b.iter(|| {
            let t0 = Instant::now();
            for i in 0..N_ENTITIES {
                let bundle = black_box(ProfilePosBundle {
                    pos: ProfilePosition {
                        x: i as f32,
                        y: 0.0,
                        z: 0.0,
                    },
                });
                bundle.for_each_component_bytes(|id, bytes| {
                    // Use volatile-ish sink so the compiler must produce
                    // an indirect callable for the closure.
                    SINK.fetch_add(
                        bytes.len() as u64 ^ id.0 as u64,
                        Ordering::Relaxed,
                    );
                });
            }
            let t1 = Instant::now();
            let ns = t1.duration_since(t0).as_nanos() as u64;
            STAGE_BUNDLE_WALK_NS.fetch_add(ns, Ordering::Relaxed);
            STAGE_ITER_COUNT.fetch_add(1, Ordering::Relaxed);
        });
    });

    // Tiny report: per-entity bundle-walk cost. The macro emits
    // ManuallyDrop locals + a 1-elem sort + slice rebuild.
    let iters = STAGE_ITER_COUNT.load(Ordering::Relaxed).max(1);
    let entities = iters * N_ENTITIES;
    let bw = STAGE_BUNDLE_WALK_NS.load(Ordering::Relaxed);
    eprintln!();
    eprintln!("══ BUNDLE WALK ONLY (1 comp) ══");
    eprintln!("  for_each_component_bytes  {:>8.2} ns/entity", bw as f64 / entities as f64);
    eprintln!("══════════════════════════════");
    eprintln!();
}

// ===========================================================================
// Bench 7 — 3-component bundle (matches the brief's hypothetical workload)
// ===========================================================================
//
// The original brief described a 3-component bundle. We add it as a
// secondary measurement so the future SpawnBatchCommand has a target.

fn bench_boyko_commands_spawn_10k_3comp(c: &mut Criterion) {
    register_profile_components();

    c.bench_function("p7_boyko_commands_spawn_10k_3comp", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            world.run_system(|mut cmds: BoykoCommands| {
                for i in 0..N_ENTITIES {
                    cmds.spawn(Profile3Bundle {
                        pos: ProfilePosition {
                            x: i as f32,
                            y: 0.0,
                            z: 0.0,
                        },
                        vel: ProfileVelocity {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        },
                        tag: ProfileTag { flags: i as u32 },
                    });
                }
            });
            black_box(&world);
        });
    });
}

// ===========================================================================
// Bench 8 — Bevy mirror of the same workload (sanity)
// ===========================================================================
//
// Runs once for the structural comparison so the relative cost is captured
// in this file too (instead of relying on memory of the head-to-head bench).

fn bench_bevy_commands_spawn_10k(c: &mut Criterion) {
    c.bench_function("p8_bevy_commands_spawn_10k", |b| {
        b.iter_with_setup(World::new, |mut world| {
            let _ = world.run_system_once(|mut cmds: BevyCommands| {
                for i in 0..N_ENTITIES {
                    cmds.spawn(BevyPositionProf {
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

fn bench_bevy_commands_spawn_10k_3comp(c: &mut Criterion) {
    c.bench_function("p9_bevy_commands_spawn_10k_3comp", |b| {
        b.iter_with_setup(World::new, |mut world| {
            let _ = world.run_system_once(|mut cmds: BevyCommands| {
                for i in 0..N_ENTITIES {
                    cmds.spawn((
                        BevyPositionProf {
                            x: i as f32,
                            y: 0.0,
                            z: 0.0,
                        },
                        BevyVelocityProf {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        },
                        BevyTagProf { flags: i as u32 },
                    ));
                }
            });
            black_box(&world);
        });
    });
}

// ===========================================================================
// Bench 10 — CommandQueue::push micro: cost of pushing N SpawnAtCommand bytes
// ===========================================================================
//
// `CommandQueue::push` is `pub(crate)`, so we measure the surface that
// drives it from outside: `Commands::add` (which calls `queue.push`).
// We push 10k SpawnAtCommand-equivalent payloads but use a no-op user
// command of the SAME size so we isolate the queue's write_unaligned cost
// from the spawn-side apply work.
//
// Size of SpawnAtCommand<ProfilePosBundle> = 8 (Entity) + 12 (Position) = 20 B
// rounded up to align of Entity (8) → 24 B + 8 B CommandMeta = 32 B slot.

fn bench_boyko_commandqueue_push_only_10k(c: &mut Criterion) {
    use boyko_ecs::ecs::core::commands::Command;

    /// Synthetic no-op command of the same byte size as the real
    /// SpawnAtCommand<ProfilePosBundle>. apply is empty; Drop is empty.
    #[repr(C)]
    struct NoopCommand {
        _entity: u64,
        _bundle: ProfilePosition, // 12 B
    }
    impl Command for NoopCommand {
        fn apply(self, _world: &mut EcsMaster) {
            // intentionally empty — push-only measurement
        }
    }

    c.bench_function("p10_boyko_commands_push_only_10k", |b| {
        b.iter_with_setup(EcsMaster::new, |mut world| {
            world.run_system(|mut cmds: BoykoCommands| {
                for i in 0..N_ENTITIES {
                    cmds.add(NoopCommand {
                        _entity: i as u64,
                        _bundle: ProfilePosition {
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
    name = profile_spawn;
    config = configure();
    targets =
        bench_instant_now_calibration,
        bench_boyko_commands_spawn_10k_baseline,
        bench_boyko_commands_spawn_10k_decomposed,
        bench_boyko_direct_create_entity_10k,
        bench_boyko_spawn_one_10k,
        bench_boyko_direct_create_entity_10k_instrumented,
        bench_boyko_bundle_walk_only_10k,
        bench_boyko_commands_spawn_10k_3comp,
        bench_bevy_commands_spawn_10k,
        bench_bevy_commands_spawn_10k_3comp,
        bench_boyko_commandqueue_push_only_10k,
}

criterion_main!(profile_spawn);
