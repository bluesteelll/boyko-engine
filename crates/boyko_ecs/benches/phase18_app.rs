// Phase 18 — the regression gate bench for the `App` facade.
// RE-BASELINED in Phase 20 (plan ★m2) — see the method note below; comparisons
// against pre-Phase-20 criterion records of Group A have no validity.
//
// THE claim under test (the Phase-20 declared-envelope form): `App`'s per-frame
// loop adds at most the P20-B1(b) driver budget — **≤ 250 ns/frame** (Time
// advance + 3 predictable branches + the empty event swap; measured directly
// by `benches/app_overhead.rs`) — over a raw `EcsMaster` + `Schedule` loop.
// At the µs scale of 50 exclusive dispatches that envelope is within run
// noise, so A ≈ B is the expected reading. The facade's plugin / tuple /
// TypeId machinery is all cold setup-only code; `Schedule::run` itself is
// byte-identical (gate P20-B1(a), asm-verified).
//
// # Method
//
//   Group A — `app_run_n_50_systems`: build an `App` with 50 trivial exclusive
//             systems, then time `app.run_n_with_delta(1, 16 ms)` per criterion
//             iteration — the full Phase-20 frame driver (steps ①-⑤) with a
//             SCRIPTED delta, so `Instant::now` jitter stays out of the timed
//             loop (plan D11/Q7: every TIMED artifact routes through
//             `run_n_with_delta`). Phase 18 originally timed `run_n(1)`
//             (self-clocked); Phase 20 re-pointed it — the RE-BASELINE.
//   Group B — `raw_schedule_run_50_systems`: the SAME 50 systems wired by hand
//             into an `EcsMaster` + `ScheduleBuilder` + `Schedule`, timed as
//             `schedule.run(&mut world)` per iteration.
//
// Both use an 8-worker pool (matching `phase9_scheduler.rs`) and 50 exclusive
// systems (matching `phase9_schedule_run_50_exclusive_systems`, so the absolute
// numbers are cross-comparable). Exclusive systems serialise on the dispatcher,
// isolating the per-frame dispatch cost from worker-spawn churn — exactly the
// path the App frame driver lowers to.
//
// # Acceptance
//
// A − B ≤ the P20-B1(b) envelope (250 ns) ⇒ the facade adds no more than the
// declared driver budget per frame. The two groups run back-to-back so they
// share machine/thermal state; on a noisy box the RELATIVE delta is the
// load-bearing number, not the absolute times.
//
// # Why not measure `App::new` / `finish` here
//
// Those are cold, one-shot config-phase costs (they run once per app, never per
// frame); the 0%-gate is exclusively about the per-FRAME hot path, so the build
// is hoisted out of every timed loop, identically on both sides.

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

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::prelude::App;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};
use criterion::{Criterion, black_box, criterion_group, criterion_main};

const NUM_SYSTEMS: usize = 50;
const NUM_THREADS: usize = 8;

/// Shared exec counter so the system bodies are not optimised away. Process-
/// global static (benches are single-threaded at the harness level and these
/// two groups never run concurrently).
static EXEC_COUNT: AtomicUsize = AtomicUsize::new(0);

fn build_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(NUM_THREADS).build()
}

// ── Group A — App frame-driver hot path ──────────────────────────────────────

/// 16 ms scripted frame delta for Group A (the Phase-20 deterministic loop).
const FRAME_16MS: Duration = Duration::from_millis(16);

/// 50 trivial exclusive systems registered through the `App` facade; time one
/// frame via `App::run_n_with_delta(1, 16 ms)` — the facade's lowered
/// per-frame loop with the clock scripted (Phase 20 ★m2 re-baseline).
fn bench_app_run_n_50_systems(c: &mut Criterion) {
    let mut app = App::with_pool(build_pool());
    for _ in 0..NUM_SYSTEMS {
        app.add_systems(|_w: &mut EcsMaster| {
            EXEC_COUNT.fetch_add(1, Ordering::Relaxed);
        });
    }
    // Finish once OUTSIDE the timed loop so the timed body is purely the frame
    // loop (run_n_with_delta still calls finish(), but the second call is the
    // cold no-op).
    app.finish();

    c.bench_function("phase18_app_run_n_50_systems", |b| {
        b.iter(|| {
            black_box(&mut app).run_n_with_delta(black_box(1), black_box(FRAME_16MS));
        });
    });
}

// ── Group B — raw Schedule::run baseline ─────────────────────────────────────

/// The SAME 50 exclusive systems hand-wired into a bare `EcsMaster` +
/// `Schedule`; time one frame via `schedule.run(&mut world)`. This is the
/// no-facade baseline the facade must match.
fn bench_raw_schedule_run_50_systems(c: &mut Criterion) {
    let pool = build_pool();
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    for _ in 0..NUM_SYSTEMS {
        builder.add_system(|_w: &mut EcsMaster| {
            EXEC_COUNT.fetch_add(1, Ordering::Relaxed);
        });
    }
    let mut schedule = builder.build(&mut world);

    c.bench_function("phase18_raw_schedule_run_50_systems", |b| {
        b.iter(|| {
            schedule.run(black_box(&mut world));
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(50)
        .measurement_time(Duration::from_secs(2))
        .warm_up_time(Duration::from_millis(500));
    // Order matters: A then B, back-to-back, so they share thermal/machine state
    // and the relative delta is meaningful on a noisy box.
    targets = bench_app_run_n_50_systems, bench_raw_schedule_run_50_systems
}
criterion_main!(benches);
