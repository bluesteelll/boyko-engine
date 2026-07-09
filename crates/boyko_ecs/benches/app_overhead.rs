// Phase 20 — App frame-driver overhead gates (plan §Metrics P20-B1(b) / P20-B2).
//
// The Phase-20 frame driver (`App::update_with_delta`, plan D1) wraps every
// frame in: ① `Time::advance_with` → ② margin-aware check-ticks compare →
// ③ gated event swap → ④ fixed catch-up loop → ⑤ Main `Schedule::run`. The
// runs themselves are opaque, byte-identical units (gate P20-B1(a), asm-
// verified); THESE benches bound what the driver adds AROUND them.
//
// # Groups and gates
//
//   `app_overhead/empty_main` — gate **P20-B1(b)**: one finished App (1-thread
//       pool), empty Main, NO Fixed schedule, no events, no states;
//       `update_with_delta(16 ms)` per iteration. This is the declared additive
//       per-frame envelope of the driver — Time advance + 3 predictable
//       branches + the empty event swap + one empty `Schedule::run` —
//       **≤ 250 ns/frame**. (With no Fixed schedule, `FixedTime` stays inert:
//       the accumulator is gated on `fixed.is_some()`, so unbounded iteration
//       never builds a substep backlog.)
//
//   `app_overhead/fixed_loop_1_substep` — feeds gate **P20-B2**: the same App
//       shape plus an EMPTY Fixed schedule, driven with exactly one 64 Hz
//       timestep (15.625 ms) per iteration — every frame expends EXACTLY one
//       substep and leaves `overstep == 0`, so iterations are identical.
//
//   `app_overhead/bare_empty_schedule_run` — the subtraction reference for
//       P20-B2: a bare `Schedule::run` of an empty schedule on the same
//       1-thread pool shape (the cost of the one extra run the substep adds).
//
//   **P20-B2** = `fixed_loop_1_substep` − `bare_empty_schedule_run` −
//   `empty_main` ≤ 100 ns/substep (one `resource_mut::<FixedTime>` re-borrow +
//   integer-ns Duration math + the swap-gate counter update). Equivalently:
//   app-frame minus bare-run ≤ 100 ns + the measured (b) envelope.
//
// # Pool hoisting
//
// As in `phase9_scheduler.rs` / `phase18_app.rs`: the pool and the App are
// built ONCE outside the timed loop (`ThreadPoolBuilder::build` spawns OS
// threads); `finish()` is called once so the timed body is purely the frame
// driver. A 1-thread pool minimizes worker wake noise — the empty schedule's
// early-return path never dispatches to a worker anyway.

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

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::prelude::{App, CoreSchedule};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};
use criterion::{Criterion, black_box, criterion_group, criterion_main};

/// One 64 Hz timestep exactly (the engine default, 15 625 000 ns): the
/// `fixed_loop_1_substep` frame expends exactly one substep with a ZERO
/// remainder, so the accumulator state is identical on every iteration.
const ONE_STEP: Duration = Duration::from_nanos(15_625_000);

/// A 16 ms frame delta for the no-Fixed group (the value is irrelevant to the
/// driver cost — `FixedTime` is inert without a Fixed schedule).
const FRAME_16MS: Duration = Duration::from_millis(16);

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

// ── Group (a) — P20-B1(b): the empty-frame driver envelope ──────────────────

/// One finished App, empty Main, no Fixed: `update_with_delta(16 ms)` is the
/// full driver envelope + one empty `Schedule::run`. Gate: ≤ 250 ns/frame.
fn bench_empty_main(c: &mut Criterion) {
    let mut app = App::with_pool(serial_pool());
    app.finish();

    c.bench_function("app_overhead/empty_main", |b| {
        b.iter(|| {
            black_box(&mut app).update_with_delta(black_box(FRAME_16MS));
        });
    });
}

// ── Group (b) — P20-B2 numerator: one substep per frame ─────────────────────

/// Empty Main + EMPTY Fixed schedule, exactly one substep per frame (the
/// 15.625 ms delta is an exact timestep multiple, so `overstep` returns to 0
/// every iteration). Reported; the gate is the subtraction documented in the
/// header.
fn bench_fixed_loop_1_substep(c: &mut Criterion) {
    let mut app = App::with_pool(serial_pool());
    // An empty config closure still creates the Fixed builder (lazy creation
    // on first `*_in(Fixed, …)` touch), so `finish` builds an empty Fixed
    // schedule and the driver's fixed branch is taken every frame.
    app.add_systems_cfg_in(CoreSchedule::Fixed, |_b| {});
    app.finish();

    c.bench_function("app_overhead/fixed_loop_1_substep", |b| {
        b.iter(|| {
            black_box(&mut app).update_with_delta(black_box(ONE_STEP));
        });
    });
}

// ── Reference — bare empty Schedule::run (the P20-B2 subtrahend) ────────────

/// A bare `Schedule::run` of an empty schedule, no App: what one extra run
/// (the substep's `fixed.run(w)`) costs on its own.
fn bench_bare_empty_schedule_run(c: &mut Criterion) {
    let pool = serial_pool();
    let mut world = EcsMaster::new();
    let builder = ScheduleBuilder::new(Arc::clone(&pool));
    let mut schedule = builder.build(&mut world);

    c.bench_function("app_overhead/bare_empty_schedule_run", |b| {
        b.iter(|| {
            schedule.run(black_box(&mut world));
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(100)
        .measurement_time(Duration::from_secs(3))
        .warm_up_time(Duration::from_secs(1));
    // Back-to-back so the three groups share machine/thermal state — the
    // P20-B2 verdict is a subtraction across them.
    targets = bench_empty_main, bench_fixed_loop_1_substep, bench_bare_empty_schedule_run
}
criterion_main!(benches);
