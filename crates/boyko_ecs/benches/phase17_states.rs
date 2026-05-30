// Phase 17 — Criterion microbenches for application/game states (plan §7).
//
// Two benches:
//
//   1. transition_pass_cost — the per-frame cost of the built-in transition
//      pass with 0 (baseline), 1, and 4 registered state types, each cycling
//      its state every frame (so the pass takes the real-transition branch,
//      not the early-out). Driven WITHOUT run conditions (via systems that
//      call `ResMut<NextState<S>>::set`), so it is independent of the
//      condition-wiring bug (tester report F1). Informational — no hard
//      threshold (plan §7.2a target < 100 ns/frame for 4 states is a soft
//      guide, not a gate).
//
//   2. in_state_gate_cost — N systems all `.run_if(in_state(active))` vs N
//      ungated systems (plan §7.2b). This wires `in_state` through `.run_if`.
//      It was previously blocked by bug F1 (the opaque `impl FnMut(Res<..>) ->
//      bool` return could not satisfy the `SystemParamFunction` HRTB bound); the
//      conditions now return `impl System<Out = bool>`, so both halves compile +
//      run by default.
//
// # Pool / world hoisting
//
// Matches `phase9_scheduler.rs`: the pool is built once per group; each bench
// builds its schedule + world once outside the timed loop, so only
// `Schedule::run` is measured.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::state::{NextState, States};
use boyko_ecs::ecs::core::system::ResMut;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

// ── State types (up to 4 orthogonal axes) ───────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
enum S0 {
    #[default]
    A,
    B,
}
impl States for S0 {}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
enum S1 {
    #[default]
    A,
    B,
}
impl States for S1 {}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
enum S2 {
    #[default]
    A,
    B,
}
impl States for S2 {}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
enum S3 {
    #[default]
    A,
    B,
}
impl States for S3 {}

fn build_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// Registers a `NextState<S>`-toggling system on the builder via an INLINE
/// closure (capturing a fresh per-system parity atomic). The closure is inlined
/// at the `add_system` call site so the macro can mint a fresh parity atomic per
/// `S` without a helper that returns a bare `impl FnMut(ResMut<..>)`. The system
/// flips `NextState<S>` A↔B every frame, forcing the transition pass onto its
/// real-transition branch.
macro_rules! add_toggler {
    ($builder:expr, $S:ty, $a:expr, $b:expr) => {{
        let parity = AtomicUsize::new(0);
        $builder.add_system(move |mut next: ResMut<NextState<$S>>| {
            let p = parity.fetch_add(1, Ordering::Relaxed);
            next.set(if p % 2 == 0 { $b } else { $a });
        });
    }};
}

// ── Bench 1 — transition_pass_cost (0 / 1 / 4 registered states) ─────────────

fn bench_transition_pass_cost(c: &mut Criterion) {
    let mut group = c.benchmark_group("phase17_transition_pass_cost");

    // Baseline: a schedule with one trivial system and NO state ⇒ the state
    // pass is a single `state_entries.is_empty()` early-out.
    {
        let pool = build_pool();
        let mut world = EcsMaster::new();
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        builder.add_system(|| {});
        let mut sched = builder.build(&mut world);
        group.bench_function("0_states_baseline", |b| {
            b.iter(|| {
                sched.run(black_box(&mut world));
            });
        });
    }

    // 1 registered state, cycling A↔B every frame (real transition per frame).
    {
        let pool = build_pool();
        let mut world = EcsMaster::new();
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        builder.init_state::<S0>();
        add_toggler!(builder, S0, S0::A, S0::B);
        let mut sched = builder.build(&mut world);
        group.bench_function("1_state", |b| {
            b.iter(|| {
                sched.run(black_box(&mut world));
            });
        });
    }

    // 4 registered states, all cycling every frame.
    {
        let pool = build_pool();
        let mut world = EcsMaster::new();
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        builder.init_state::<S0>();
        builder.init_state::<S1>();
        builder.init_state::<S2>();
        builder.init_state::<S3>();
        add_toggler!(builder, S0, S0::A, S0::B);
        add_toggler!(builder, S1, S1::A, S1::B);
        add_toggler!(builder, S2, S2::A, S2::B);
        add_toggler!(builder, S3, S3::A, S3::B);
        let mut sched = builder.build(&mut world);
        group.bench_function("4_states", |b| {
            b.iter(|| {
                sched.run(black_box(&mut world));
            });
        });
    }

    group.finish();
}

// ── Bench 2 — in_state_gate_cost (BLOCKED by F1; gated) ──────────────────────

/// The ungated control: N trivial systems, no conditions. Always compiles, so
/// the bench harness is never empty even with the feature off.
fn bench_in_state_gate_cost(c: &mut Criterion) {
    let mut group = c.benchmark_group("phase17_in_state_gate_cost");
    const N: usize = 16;

    {
        let pool = build_pool();
        let mut world = EcsMaster::new();
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        for _ in 0..N {
            builder.add_system(|| {});
        }
        let mut sched = builder.build(&mut world);
        group.bench_function("ungated_control_16_systems", |b| {
            b.iter(|| {
                sched.run(black_box(&mut world));
            });
        });
    }

    // N systems all `.run_if(in_state(active))`. Previously blocked by bug F1
    // (`.run_if(in_state(..))` did not compile); the conditions now return
    // `impl System<Out = bool>`, so this half builds + runs by default.
    {
        use boyko_ecs::ecs::core::schedule::in_state;
        let pool = build_pool();
        let mut world = EcsMaster::new();
        let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
        builder.init_state::<S0>();
        for _ in 0..N {
            builder.add_system(|| {}).run_if(in_state(S0::A));
        }
        let mut sched = builder.build(&mut world);
        group.bench_function("in_state_gated_16_systems", |b| {
            b.iter(|| {
                sched.run(black_box(&mut world));
            });
        });
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(100)
        .measurement_time(Duration::from_secs(3))
        .warm_up_time(Duration::from_millis(500));
    targets = bench_transition_pass_cost, bench_in_state_gate_cost
}
criterion_main!(benches);
