// Phase 10 Wave E Step 15 — criterion bench suite for change detection.
//
// Targets per Phase 10 plan §1.2 / §13.5:
//
//   1. bench_changed_filter_1024_rows                — target ≤ 1024 ns (1 ns/row).
//   2. bench_added_filter_first_frame                — same shape; all rows match.
//   3. bench_mut_deref_guard_overhead                — target ≤ 1 ns/row over baseline.
//   4. bench_or_added_changed_archetype_count_dominated — Round 2 W8 scaling.
//   5. bench_no_change_detection_baseline            — control / 0% delta target.
//
// # Hoisting
//
// Pool, world, schedule are hoisted across criterion iterations. Each
// `b.iter` body runs a single `schedule.run` — the dispatcher's frame
// tick bump + per-system `set_change_ticks` cost is included in the
// reported number.
//
// # Component id reservation
//
// Phase 10 Wave E benches use ids **396..=410** (disjoint from the
// integration tests' 380..=395 range and from Phase 9's 340-349).

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc, which
// is far more deterministic and exposes structural signals the system heap
// masks (the documented ±20-30% variance source). See docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::Arc;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::filter::Or;
use boyko_ecs::ecs::core::iters::query::{Added, Changed, Mut, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

// ── Component types ────────────────────────────────────────────────────────

const POS_ID: ComponentId = ComponentId(396);
const VEL_ID: ComponentId = ComponentId(397);
const A_ID: ComponentId = ComponentId(398);
const B_ID: ComponentId = ComponentId(399);
const TAG_ID: ComponentId = ComponentId(400);

#[repr(C)]
#[derive(Clone, Copy)]
struct Pos {
    x: f32,
    y: f32,
    z: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Vel {
    vx: f32,
    vy: f32,
    vz: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CompA(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct CompB(u32);

#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Tag(u32);

impl Component for Pos {
    fn component_id() -> ComponentId {
        POS_ID
    }
}
impl Component for Vel {
    fn component_id() -> ComponentId {
        VEL_ID
    }
}
impl Component for CompA {
    fn component_id() -> ComponentId {
        A_ID
    }
}
impl Component for CompB {
    fn component_id() -> ComponentId {
        B_ID
    }
}
impl Component for Tag {
    fn component_id() -> ComponentId {
        TAG_ID
    }
}

fn register_components() {
    register_layout::<Pos>(POS_ID.0);
    register_layout::<Vel>(VEL_ID.0);
    register_layout::<CompA>(A_ID.0);
    register_layout::<CompB>(B_ID.0);
    register_layout::<Tag>(TAG_ID.0);
}

fn build_pool(threads: usize) -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(threads).build()
}

// ── Bench 1: Changed<T> filter walk over 1024 rows ────────────────────────

/// Plan §1.2: per-row `Changed<T>` filter cost ≤ 1 ns. With 1024 rows the
/// schedule.run cost SHOULD stay near `1024 ns + frame dispatcher overhead`.
///
/// The system reads `Query<&Pos, Changed<Pos>>`. Frame N+1 reads after a
/// frame N write — every row matches on the first run because insertion
/// bumped the changed tick.
fn bench_changed_filter_1024_rows(c: &mut Criterion) {
    register_components();
    let pool = build_pool(4);
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[POS_ID]);
    for i in 0..1024 {
        world
            .spawn_one(arch, Pos { x: i as f32, y: 0.0, z: 0.0 })
            .unwrap();
    }
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&Pos, Changed<Pos>>| {
        for p in &q {
            black_box(p);
        }
    });
    let mut sched = builder.build(&mut world);

    c.bench_function("phase10_changed_filter_1024_rows", |b| {
        b.iter(|| {
            sched.run(black_box(&mut world));
        });
    });
}

// ── Bench 2: Added<T> first-frame walk ─────────────────────────────────────

/// Same shape as bench 1 but exercises `Added<T>` rather than `Changed<T>`.
/// At steady state after the first tick, no row has been newly added (no
/// migrations happen here), so the inner per-row predicate fires the
/// "filter rejects" branch — measures the *negative* filter cost.
fn bench_added_filter_first_frame(c: &mut Criterion) {
    register_components();
    let pool = build_pool(4);
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[POS_ID]);
    for i in 0..1024 {
        world
            .spawn_one(arch, Pos { x: i as f32, y: 0.0, z: 0.0 })
            .unwrap();
    }
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&Pos, Added<Pos>>| {
        for p in &q {
            black_box(p);
        }
    });
    let mut sched = builder.build(&mut world);

    c.bench_function("phase10_added_filter_1024_rows", |b| {
        b.iter(|| {
            sched.run(black_box(&mut world));
        });
    });
}

// ── Bench 3: Mut<T> deref guard overhead ───────────────────────────────────

/// Plan §1.2: per-row `Mut<T>::deref_mut` tick bump cost ≤ 1 ns. Compare
/// against the `&mut T` baseline (bench 5) to isolate the bump cost.
fn bench_mut_deref_guard_overhead(c: &mut Criterion) {
    register_components();
    let pool = build_pool(4);
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[POS_ID]);
    for i in 0..1024 {
        world
            .spawn_one(arch, Pos { x: i as f32, y: 0.0, z: 0.0 })
            .unwrap();
    }
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut q: Query<Mut<Pos>>| {
        for mut p in &mut q {
            p.x += 1.0;
            black_box(&mut *p);
        }
    });
    let mut sched = builder.build(&mut world);

    c.bench_function("phase10_mut_deref_guard_1024_rows", |b| {
        b.iter(|| {
            sched.run(black_box(&mut world));
        });
    });
}

// ── Bench 4: Or<(Added<A>, Changed<B>)> archetype-count dominated ─────────

/// Plan §13.5 Round 2 W8: `Or<F>::aggregate_include` is a no-op
/// (filter.rs M8 contract), so `Or<(Added<A>, Changed<B>)>` walks EVERY
/// archetype — including those that contain neither A nor B. This bench
/// measures the per-archetype walk overhead at increasing archetype
/// counts. Linear scaling expected.
// The `Query<&CompA, Or<(Added<CompA>, Changed<CompB>)>>` closure param is the
// query DSL type being benched; an alias would not aid readability (it relies
// on SystemParam lifetime elision at the closure-arg position).
#[allow(clippy::type_complexity)]
fn bench_or_added_changed_archetype_count_dominated(c: &mut Criterion) {
    register_components();
    let pool = build_pool(4);
    let mut world = EcsMaster::new();
    // Build 16 archetypes, only one of which contains BOTH A and B.
    // Use `spawn_two` so both components are provided to the archetype.
    let arch_ab = world.create_archetype(&[A_ID, B_ID]);
    for _ in 0..256 {
        world
            .spawn_two(arch_ab, CompA(1), CompB(2))
            .unwrap();
    }
    // 15 decoy archetypes containing only `Pos`.
    for _ in 0..15 {
        let arch = world.create_archetype(&[POS_ID]);
        // single row keeps the archetype "live"
        world
            .spawn_one(arch, Pos { x: 0.0, y: 0.0, z: 0.0 })
            .unwrap();
    }
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&CompA, Or<(Added<CompA>, Changed<CompB>)>>| {
        for a in &q {
            black_box(a);
        }
    });
    let mut sched = builder.build(&mut world);

    c.bench_function("phase10_or_added_changed_16_archetypes", |b| {
        b.iter(|| {
            sched.run(black_box(&mut world));
        });
    });
}

// ── Bench 5: No-change-detection baseline ──────────────────────────────────

/// Plan §1.2: `Query::iter` overhead added by Phase 10 when neither
/// `Added` nor `Changed` is used must be 0 ns. This bench is the control
/// for benches 1-3 — same world / row count / system arity, no Phase 10
/// filter.
fn bench_no_change_detection_baseline(c: &mut Criterion) {
    register_components();
    let pool = build_pool(4);
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[POS_ID]);
    for i in 0..1024 {
        world
            .spawn_one(arch, Pos { x: i as f32, y: 0.0, z: 0.0 })
            .unwrap();
    }
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&Pos>| {
        for p in &q {
            black_box(p);
        }
    });
    let mut sched = builder.build(&mut world);

    c.bench_function("phase10_no_change_detection_baseline", |b| {
        b.iter(|| {
            sched.run(black_box(&mut world));
        });
    });
}

// ── Bench 6: schedule frame tick overhead control ─────────────────────────

/// Direct measurement of the Phase 10 dispatcher overhead — frame-start
/// `change_tick.fetch_add(1)` + per-system `set_change_ticks` for a
/// single trivial system on an empty world.
fn bench_phase10_dispatcher_overhead(c: &mut Criterion) {
    register_components();
    let pool = build_pool(4);
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|_q: Query<&Pos>| {});
    let mut sched = builder.build(&mut world);

    c.bench_function("phase10_dispatcher_overhead_one_system", |b| {
        b.iter(|| {
            sched.run(black_box(&mut world));
        });
    });
}

criterion_group! {
    name = phase10_benches;
    config = Criterion::default()
        .sample_size(50)
        .measurement_time(Duration::from_secs(2));
    targets =
        bench_changed_filter_1024_rows,
        bench_added_filter_first_frame,
        bench_mut_deref_guard_overhead,
        bench_or_added_changed_archetype_count_dominated,
        bench_no_change_detection_baseline,
        bench_phase10_dispatcher_overhead,
}
criterion_main!(phase10_benches);
