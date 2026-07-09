// Phase 9 Wave 7 Step 20 — Criterion bench suite for the parallel scheduler.
//
// Targets per Phase 9 plan §1.2 / §13.5:
//
//   1. bench_schedule_run_empty                  — control / dispatcher overhead
//   2. bench_schedule_run_50_systems             — target ≤ 20 µs (Round 3 C-NEW-2)
//   3. bench_par_iter_4096_entities              — target ≥ 4× single-thread on
//                                                  an 8-core box (PAR1 throughput)
//   4. bench_schedule_run_two_disjoint           — minimum 2-system parallel path
//   5. bench_schedule_run_exclusive_only         — exclusive system inline path
//
// # Pool hoisting
//
// The pool is built ONCE per criterion group and reused across iterations.
// `ThreadPoolBuilder::build` is expensive (spawns OS threads + sets affinity);
// measuring it inside the timed loop would dwarf the per-frame dispatcher
// cost we care about. The schedule is also built once per Criterion `iter`
// loop — the dispatcher overhead is what we measure, not the build cost
// (which has its own target in §1.2 and is exercised in
// `bench_schedule_build` below).
//
// # World state
//
// All benches use an empty `EcsMaster` except for `par_iter_4096_entities`,
// which spawns a single archetype with 4096 rows. Component ids are taken
// from the 340-349 reserved Phase 9 bench range (disjoint from §scheduler_*
// integration tests which use 320-329).

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

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

// ── Component types ────────────────────────────────────────────────────────

const VAL_ID: ComponentId = ComponentId(340);

#[repr(C)]
#[derive(Clone, Copy)]
struct Val(u32);

impl Component for Val {
    fn component_id() -> ComponentId {
        VAL_ID
    }
}

fn register_test_components() {
    register_layout::<Val>(VAL_ID.0);
}

// ── Pool factories ─────────────────────────────────────────────────────────

fn build_pool(num_threads: usize) -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(num_threads).build()
}

// ── Bench 1 — empty schedule dispatcher overhead ───────────────────────────

/// Measures the per-frame cost of `Schedule::run` on a zero-system schedule.
/// This isolates the executor's outer install entry + early-return path
/// (`completed == n == 0`). Useful as a control for scaling tests.
fn bench_schedule_empty(c: &mut Criterion) {
    let pool = build_pool(4);
    let mut world = EcsMaster::new();
    let builder = ScheduleBuilder::new(Arc::clone(&pool));
    let mut sched = builder.build(&mut world);

    c.bench_function("phase9_schedule_run_empty", |b| {
        b.iter(|| {
            sched.run(black_box(&mut world));
        });
    });
}

// ── Bench 2 — 50-system dispatcher overhead (Round 3 C-NEW-2) ─────────────

/// 50 trivial exclusive systems registered. Target: per-frame dispatch ≤
/// 20 µs (Round 3 C-NEW-2 — relaxed from the original 5 µs target because
/// apply cost dominates the per-system contribution). Bevy single-threaded
/// dispatcher saturates at ~470 ns/sys; boyko at ≤ 400 ns/sys beats Bevy on
/// raw throughput at this scale.
///
/// The systems are exclusive (`fn(&mut EcsMaster)`) — they serialize on the
/// dispatcher and so measure the dispatcher's outer-loop cost without
/// worker spawn churn. The "real" parallelism benches sit below in
/// `bench_par_iter_*` and `bench_two_disjoint`.
fn bench_schedule_50_systems(c: &mut Criterion) {
    let pool = build_pool(8);
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));

    static EXEC_COUNT: AtomicUsize = AtomicUsize::new(0);
    EXEC_COUNT.store(0, Ordering::Relaxed);

    for _ in 0..50 {
        builder.add_system(|_w: &mut EcsMaster| {
            EXEC_COUNT.fetch_add(1, Ordering::Relaxed);
        });
    }

    let mut sched = builder.build(&mut world);

    c.bench_function("phase9_schedule_run_50_exclusive_systems", |b| {
        b.iter(|| {
            sched.run(black_box(&mut world));
        });
    });
}

// ── Bench 3 — par_iter on a 4096-entity archetype ──────────────────────────

/// Single-archetype `par_iter().for_each(...)` over 4096 rows. The body is
/// a cheap atomic increment — kept tiny so the bench measures dispatch
/// overhead per chunk, not user code. Target (plan §13.5): ≥ 4× speedup
/// vs a single-thread `iter()` walk on an 8-core box, with per-chunk
/// dispatch cost ≤ 200 ns (the soft minimum below which fork-join
/// overhead would dominate).
fn bench_par_iter_4096_entities(c: &mut Criterion) {
    register_test_components();
    let pool = build_pool(8);
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[VAL_ID]);
    for i in 0..4096u32 {
        world
            .spawn_one(arch, Val(i))
            .expect("seed must succeed");
    }

    static SUM: AtomicUsize = AtomicUsize::new(0);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&Val>| {
        q.par_iter().for_each(|v: &Val| {
            SUM.fetch_add(v.0 as usize, Ordering::Relaxed);
        });
    });
    let mut sched = builder.build(&mut world);

    c.bench_function("phase9_par_iter_4096_entities", |b| {
        b.iter(|| {
            SUM.store(0, Ordering::Relaxed);
            sched.run(black_box(&mut world));
        });
    });
}

// ── Bench 4 — two disjoint concurrent systems ──────────────────────────────

/// Two systems with no resource / component overlap. The conflict graph
/// permits parallel dispatch; on an 8-thread pool both run simultaneously.
/// Measures the minimum parallel dispatch cost (one round-trip through
/// the apply window with two completions to drain).
fn bench_schedule_two_disjoint(c: &mut Criterion) {
    let pool = build_pool(8);
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));

    static A: AtomicUsize = AtomicUsize::new(0);
    static B: AtomicUsize = AtomicUsize::new(0);

    builder.add_system(|| {
        A.fetch_add(1, Ordering::Relaxed);
    });
    builder.add_system(|| {
        B.fetch_add(1, Ordering::Relaxed);
    });

    let mut sched = builder.build(&mut world);

    c.bench_function("phase9_schedule_run_two_disjoint", |b| {
        b.iter(|| {
            sched.run(black_box(&mut world));
        });
    });
}

// ── Bench 5 — single exclusive system inline path ──────────────────────────

/// An exclusive system runs inline on the dispatcher under
/// `cell.world_mut()` (EXC1). Measures the no-spawn cost: the dispatcher
/// recognises `is_exclusive`, skips the spawn round-trip, and runs +
/// applies in place.
fn bench_schedule_one_exclusive(c: &mut Criterion) {
    let pool = build_pool(4);
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));

    static C: AtomicUsize = AtomicUsize::new(0);

    builder.add_system(|_w: &mut EcsMaster| {
        C.fetch_add(1, Ordering::Relaxed);
    });

    let mut sched = builder.build(&mut world);

    c.bench_function("phase9_schedule_run_one_exclusive", |b| {
        b.iter(|| {
            sched.run(black_box(&mut world));
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(50)
        .measurement_time(Duration::from_secs(2))
        .warm_up_time(Duration::from_millis(500));
    targets =
        bench_schedule_empty,
        bench_schedule_50_systems,
        bench_par_iter_4096_entities,
        bench_schedule_two_disjoint,
        bench_schedule_one_exclusive
}
criterion_main!(benches);
