//! Phase 12.5 Track P2 — query iter hot-path profiler.
//!
//! This bench isolates the four suspected contributors to the ~980 ns gap
//! between boyko and bevy on the 10k single-component iter benchmark
//! (boyko 7.88 us vs bevy 6.90 us, ratio 0.88x):
//!
//! 1. **Wrapper overhead** — boyko's existing g2 bench wraps `Query::iter`
//!    in `world.run_system(|q| ...)` which constructs a fresh
//!    `FunctionSystem` and a fresh `QueryDataState` on every iter
//!    (heap-allocates `matched_ids: Vec::with_capacity(16)`), then
//!    `initialize()` + `run_unsafe()` + `apply()`. Bevy's bench calls
//!    `state.iter(&world)` directly on a cached `QueryState` — no system,
//!    no per-call allocation, no apply pass.
//!
//! 2. **Archetype boundary cost** — at each archetype transition the inner
//!    `QueryIter::next` mints a new `*const Archetype`, calls
//!    `D::set_table_readonly` + `F::set_table_readonly` (forwards `meta`
//!    for Wave C Ref/Mut tick capture even when D = `&T`), and reads
//!    `entity_count()`. Bevy does the same shape but elides the meta
//!    forward when D doesn't need it.
//!
//! 3. **`set_change_ticks` propagation** — for the direct-iter path (no
//!    scheduler) the bookkeeping is one tick read on `world.current_tick()`
//!    inside `QueryDataState::new` and zero on subsequent iter() calls
//!    (the scheduler is the only writer). With a system wrapper the cost
//!    is the same — `set_change_ticks` is dispatcher-only.
//!
//! 4. **Per-row codegen** — boyko's `Iterator::next` is `#[inline]`,
//!    `<&T as QueryData>::fetch` is `#[inline]`, `(): QueryFilter` is
//!    `IS_ARCHETYPAL = true` so the per-row filter branch const-folds
//!    away. Bevy uses `#[inline(always)]` on both `next()` and `fetch()`.
//!    The asm comparison is in `docs/PHASE-12.5-PROFILE-QUERY.md`.
//!
//! # Cases
//!
//! - `p2_boyko_baseline_10k_run_system` — repeats the g2 bench shape:
//!   `world.run_system(|q: Query<&P>| ... )` per iter. Matches the
//!   current scoreboard measurement.
//! - `p2_boyko_cached_system_10k` — hoists the `FunctionSystem` outside
//!   the timed loop via `run_cached_system(&mut sys)`; subtracts the
//!   into_system + QueryDataState::new + Vec::with_capacity(16)
//!   per-call cost.
//! - `p2_boyko_run_system_once_10k` — same as `run_cached_system` but
//!   via `run_system_once(&mut sys)` (drops the apply pass; in this case
//!   the body has no Commands so apply is already a no-op for the
//!   `()` SystemParam — measured to confirm).
//! - `p2_boyko_direct_pool_10k` — bypasses Query entirely. Walks
//!   `archetype.component_pools().get_pool(id).get_raw(i)` for every
//!   row. Establishes the bottom-line "raw pointer arithmetic" baseline
//!   that any query API must approach.
//! - `p2_boyko_get_component_raw_10k` — walks `world.get_component_raw`
//!   for every spawned entity. Fast-random-access lookup cost (1
//!   indirection per entity vs Query's per-archetype boundary cost).
//! - `p2_bevy_baseline_10k` — Bevy's `state.iter(&world)` shape for
//!   apples-to-apples comparison with `p2_boyko_baseline_*`.
//!
//! Multi-archetype fanout cases were deferred — see the "case (F)" comment
//! below for the arena-sizing constraint.
//!
//! All numbers feed into `docs/PHASE-12.5-PROFILE-QUERY.md`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

// ── boyko imports ──────────────────────────────────────────────────────────
use boyko_ecs::ecs::core::component::component::Component as BoykoComponent;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query as BoykoQuery;
use boyko_ecs::ecs::core::system::into_system::IntoSystem;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

// ── bevy imports ───────────────────────────────────────────────────────────
use bevy_ecs::prelude::Component as BevyComponentDerive;
use bevy_ecs::prelude::*;

// ── Constants ──────────────────────────────────────────────────────────────

const N_ENTITIES: usize = 10_000;

/// Slot for the single benched component on the boyko side. Range chosen
/// to avoid every other reserved range in the codebase at the time of
/// writing (matches the choice in `comparison.rs`'s `BOYKO_POS_ID` to keep
/// the registry idempotent if both benches run together).
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

#[derive(BevyComponentDerive, Clone, Copy)]
#[allow(dead_code)]
struct BevyPosition {
    x: f32,
    y: f32,
    z: f32,
}

fn register_boyko_position() {
    register_layout::<BoykoPosition>(BOYKO_POS_ID.0);
}

static SUM_SINK: AtomicUsize = AtomicUsize::new(0);

// ── Setup helpers ───────────────────────────────────────────────────────────

/// Single archetype containing 10k Position entities.
fn setup_boyko_single() -> (EcsMaster, ArchetypeId) {
    register_boyko_position();
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[BOYKO_POS_ID]);
    for i in 0..N_ENTITIES {
        world
            .spawn_one(arch, BoykoPosition { x: i as f32, y: 0.0, z: 0.0 })
            .expect("spawn must succeed");
    }
    (world, arch)
}

fn setup_bevy_single() -> World {
    let mut world = World::new();
    for i in 0..N_ENTITIES {
        world.spawn(BevyPosition { x: i as f32, y: 0.0, z: 0.0 });
    }
    world
}

// ── Boyko: case (A) — baseline `run_system` shape (current g2). ────────────

fn bench_boyko_baseline_run_system(c: &mut Criterion) {
    let (mut world, _arch) = setup_boyko_single();

    c.bench_function("p2_boyko_baseline_10k_run_system", |b| {
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

// ── Boyko: case (B) — hoisted FunctionSystem via run_cached_system. ────────

fn bench_boyko_cached_system(c: &mut Criterion) {
    let (mut world, _arch) = setup_boyko_single();

    // Hoist the closure → FunctionSystem outside the timed loop. The first
    // `run_cached_system` call still runs `initialize()` (cold path); every
    // subsequent call short-circuits via `state.is_some()` (FS1). After
    // criterion's warm-up the timed iters all hit the hot path.
    let closure = |q: BoykoQuery<&BoykoPosition>| {
        let mut sum = 0.0f32;
        for p in &q {
            sum += p.x;
        }
        SUM_SINK.store(black_box(sum) as usize, Ordering::Relaxed);
    };
    let mut sys = IntoSystem::<(), (), _>::into_system(closure);

    c.bench_function("p2_boyko_cached_system_10k", |b| {
        b.iter(|| {
            world.run_cached_system(&mut sys);
        });
    });
}

// ── Boyko: case (C) — cached + run_system_once (no apply pass). ─────────────

fn bench_boyko_run_system_once(c: &mut Criterion) {
    let (mut world, _arch) = setup_boyko_single();

    let closure = |q: BoykoQuery<&BoykoPosition>| {
        let mut sum = 0.0f32;
        for p in &q {
            sum += p.x;
        }
        SUM_SINK.store(black_box(sum) as usize, Ordering::Relaxed);
    };
    let mut sys = IntoSystem::<(), (), _>::into_system(closure);

    c.bench_function("p2_boyko_run_system_once_10k", |b| {
        b.iter(|| {
            world.run_system_once(&mut sys);
        });
    });
}

// ── Boyko: case (D) — direct pool walk (no Query, no system). ──────────────
//
// Reaches into `archetype.component_pools().get_pool(id).get_raw(row)` for
// every row. Establishes the floor: any Query API must approach this number.
// Note: `get_raw` returns Option<*const u8>; we cast to `*const BoykoPosition`.

fn bench_boyko_direct_pool(c: &mut Criterion) {
    let (world, arch) = setup_boyko_single();
    let arch_ref = world.archetype_master().get_archetype(arch).expect("arch must exist");
    let pool = arch_ref
        .component_pools()
        .get_pool(BOYKO_POS_ID)
        .expect("pool must exist");
    let count = pool.count();

    c.bench_function("p2_boyko_direct_pool_10k", |b| {
        b.iter(|| {
            let mut sum = 0.0f32;
            for i in 0..count {
                // SAFETY: i < count == pool size; `get_raw` returns a stable
                //   pointer into the arena slab that lives for the world's
                //   lifetime; bench is single-threaded with no concurrent
                //   structural mutation. The pool stores `BoykoPosition`
                //   per the `register_layout::<BoykoPosition>(BOYKO_POS_ID.0)`
                //   contract; bytes are valid for read.
                let raw = unsafe { pool.get_raw(i).unwrap_unchecked() };
                let pos = unsafe { &*(raw as *const BoykoPosition) };
                sum += pos.x;
            }
            SUM_SINK.store(black_box(sum) as usize, Ordering::Relaxed);
        });
    });
}

// ── Boyko: case (E) — get_component_raw fast-random-access walk. ───────────
//
// Walks all entities via the fast-random-access path
// `EcsMaster::get_component_raw(entity, id)`. Establishes the "1 lookup per
// entity" cost (3-4 cache lines per per Phase 7) for comparison against the
// "1 lookup per archetype + sequential" cost of Query.

fn bench_boyko_get_component_raw(c: &mut Criterion) {
    register_boyko_position();
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[BOYKO_POS_ID]);
    let mut entities = Vec::with_capacity(N_ENTITIES);
    for i in 0..N_ENTITIES {
        entities.push(
            world
                .spawn_one(arch, BoykoPosition { x: i as f32, y: 0.0, z: 0.0 })
                .expect("spawn must succeed"),
        );
    }

    c.bench_function("p2_boyko_get_component_raw_10k", |b| {
        b.iter(|| {
            let mut sum = 0.0f32;
            for &e in &entities {
                let raw = world.get_component_raw(e, BOYKO_POS_ID).unwrap();
                // SAFETY: get_component_raw returned Some, so raw points at
                //   a valid BoykoPosition byte slot.
                let pos = unsafe { &*(raw as *const BoykoPosition) };
                sum += pos.x;
            }
            SUM_SINK.store(black_box(sum) as usize, Ordering::Relaxed);
        });
    });
}

// ── Boyko: case (F) — fanout (multi-archetype) variant.
//
// DEFERRED: the default `EcsMaster` arena is 64 MB. Each Position pool
// reserves a 3 MB contiguous block (128 chunks × 2048 tiny slots ×
// 12 B); Phase 10 additionally allocates ~2 MB of per-pool `Box<[Tick]>`
// outside the arena. Even 3 separate archetypes hitting the same arena's
// free-block tracker exhausts available contiguous blocks under the
// current allocator policy (the panic fires inside
// `Arena::allocate_from_free_blocks` for the 3rd Pos pool's 3 MB
// request). Reproducing this with the bench would require either:
//   (a) bumping `DEFAULT_ARENA_SIZE` (out of scope — production change), or
//   (b) shrinking `DEFAULT_CHUNKS_PER_POOL` for benches (per-bench arena
//       sizing — non-trivial wiring through `EcsMaster::new`).
//
// The single-archetype results above already isolate the four suspected
// contributors (wrapper vs cached vs direct vs random access). Multi-
// archetype boundary cost is a follow-up: see Phase 12.5 plan §P2.

// ── Bevy baselines for cross-check. ─────────────────────────────────────────

fn bench_bevy_baseline(c: &mut Criterion) {
    let mut world = setup_bevy_single();
    let mut state: QueryState<&BevyPosition> = world.query();

    c.bench_function("p2_bevy_baseline_10k", |b| {
        b.iter(|| {
            let mut sum = 0.0f32;
            for p in state.iter(&world) {
                sum += p.x;
            }
            SUM_SINK.store(black_box(sum) as usize, Ordering::Relaxed);
        });
    });
}

// Bevy fanout removed for symmetry with the deferred boyko fanout — see
// the note above.

// ── Criterion wiring ───────────────────────────────────────────────────────

fn configure() -> Criterion {
    Criterion::default()
        .sample_size(50)
        .measurement_time(Duration::from_secs(3))
        .warm_up_time(Duration::from_millis(500))
}

criterion_group! {
    name = profile_query;
    config = configure();
    targets =
        bench_boyko_baseline_run_system,
        bench_boyko_cached_system,
        bench_boyko_run_system_once,
        bench_boyko_direct_pool,
        bench_boyko_get_component_raw,
        bench_bevy_baseline,
}

criterion_main!(profile_query);
