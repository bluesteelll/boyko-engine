//! Phase 9.1 W4 — Miri-2: the integrated 2-worker `Schedule::run` executor
//! under Miri (plan D4 / M1).
//!
//! Distinct from Miri-1 (`boyko-threadpool/tests/miri_scope.rs`, which proves
//! the pool's `Scope::spawn` transmute in isolation), this test exercises the
//! surface only the ECS adds: the type-erased world cell shared across **two**
//! worker threads inside the real `Schedule::run` executor, which dispatches
//! systems via `pool.install(|scope| scope.spawn(…))` (`schedule.rs`).
//!
//! Scope is the **absolute minimum** that still forces cross-worker cell
//! sharing (plan D4 budget: < ~3 min wall under Miri):
//!   - exactly **2 systems** over **disjoint** components (`MPosition` vs
//!     `MVelocity`) so the conflict graph lets them run concurrently — the point
//!     is the cross-worker handshake, not throughput,
//!   - a tiny entity set, **1 frame**,
//!   - a **2-worker** pool (`num_threads(2)`).
//!
//! The setup mirrors the (compiling, native) `scheduler_par_iter_concurrent_
//! systems.rs` integration test exactly — `create_archetype` + `spawn_one`,
//! closure systems, `let q: Query<&T> = world.query();` read-back — so the only
//! variable under test is "2-worker `Schedule::run` under Miri". This file
//! deliberately targets the 2-worker path under Miri, which the existing
//! multithreaded integration tests gate off (`#![cfg(not(miri))]`) and which
//! `miri_phase9.rs` sidesteps by using `num_threads(1)` + zero systems.
//!
//! ## D5.1 OUTCOME (recorded 2026-05-30): blocked under BOTH borrow models — TB
//! by the same `scope.rs:96` over-approximation; SB by a crossbeam-deque
//! integer-to-pointer-cast limitation.
//!
//! Running this test (`-- --ignored`) does NOT cleanly reach the
//! ECS-specific cross-worker world-cell sharing it targets, for a model-specific
//! reason in each direction:
//!   - **Tree Borrows** reproduces the **same** `scope.rs:96` protected-tag
//!     conflict as Miri-1 (`ScopeShared::complete_task`'s `pending.fetch_sub`
//!     foreign-write while the dispatcher holds a protected `&ScopeShared` in
//!     `join_workers_until_drained`) — proven a TB over-approximation, not a
//!     bug, via the `std::thread::scope` equivalence harness (see
//!     `boyko_threadpool/tests/miri_scope.rs` module doc, D5.1 (ii)).
//!   - **Stacked Borrows** (this Miri's *default* when TB is not forced) trips
//!     first inside **crossbeam-deque**'s `steal_batch_*` →
//!     `crossbeam-epoch::internal.rs:549` `&*local_ptr` retag, an
//!     integer-to-pointer-cast pattern crossbeam uses that neither SB nor TB
//!     supports (`Stealer` is reached on a worker thread before the scope join).
//!     That is a third-party crate limitation, explicitly out of the proof
//!     surface (the deque is loom/Miri-opaque by plan §9; covered by the D6
//!     stress test on real hardware).
//!
//! Either way the ECS cell handshake is unreachable in *this* Miri without
//! production hardening, so the test is `#[ignore]`-by-default (kept compiling +
//! ready): it will run once the `*const ScopeShared` joiner hardening lands (the
//! D5.1 candidate fix, confirmed clean in the std harness) AND a Miri/crossbeam
//! combination handles the deque's exposed-provenance casts. Until then the
//! cross-worker world-cell path is covered by the native
//! `scheduler_par_iter_concurrent_systems` integration test.
//!
//! ## Run (plan §5)
//! ```bash
//! MIRIFLAGS="-Zmiri-disable-isolation" \
//!   cargo +nightly miri test -p boyko-ecs --test miri_schedule_parallel -- --ignored
//! ```
#![cfg(miri)]

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_macros::Component;
use boyko_threadpool::ThreadPoolBuilder;

// `Query` is the system-param read/write handle (used by the closure systems);
// `EcsMaster::query()` returns a `QueryView` for direct world read-back.

#[derive(Component)]
#[repr(C)]
struct MPosition {
    x: f32,
}

#[derive(Component)]
#[repr(C)]
struct MVelocity {
    x: f32,
}

/// Drives the real 2-worker `Schedule::run` for a single frame over two
/// disjoint-component entity sets. Asserts no Miri UB / data race in the
/// cross-worker world-cell sharing, and that the `Position`-mutating system
/// applied its write.
#[test]
#[ignore = "blocked by the same TB over-approximation as Miri-1 (scope.rs:96 \
            protected tag); the pool join trips before the ECS cell surface is \
            reached. See module doc / D5.1 (ii). Run with -- --ignored."]
fn miri_two_worker_schedule_disjoint_systems_one_frame() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    // Disjoint single-component archetypes (mirrors the native concurrent test).
    let arch_p = world.create_archetype(&[MPosition::component_id()]);
    let arch_v = world.create_archetype(&[MVelocity::component_id()]);
    for i in 0..4 {
        world
            .spawn_one(arch_p, MPosition { x: i as f32 })
            .expect("spawn position");
        world
            .spawn_one(arch_v, MVelocity { x: i as f32 })
            .expect("spawn velocity");
    }

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // System A: mutate `MPosition` only.
    builder.add_system(|mut q: Query<&mut MPosition>| {
        for p in q.iter_mut() {
            p.x += 1.0;
        }
    });
    // System B: read `MVelocity` only (disjoint from A → runs concurrently on
    // the second worker, sharing the type-erased world cell with A).
    builder.add_system(|q: Query<&MVelocity>| {
        let _ = q.iter().count();
    });
    let mut schedule = builder.build(&mut world);

    // The frame under proof: dispatches both systems across 2 workers.
    schedule.run(&mut world);

    // Read back via a single-threaded query. `EcsMaster::query::<D, F>()`
    // returns a `QueryView` (borrows the world mutably for the view's lifetime);
    // `view.iter()` yields `&MPosition` per row — the engine's own read-back form
    // (see `ecs_master::query` doc + `query_dsl_smoke.rs`).
    let view = world.query::<&MPosition, ()>();
    let mut sum = 0.0f32;
    let mut count = 0;
    for p in view.iter() {
        sum += p.x;
        count += 1;
    }
    assert_eq!(count, 4, "all positions visited");
    // Each x was incremented by 1: (0+1)+(1+1)+(2+1)+(3+1) = 10.
    assert_eq!(sum, 10.0, "move system applied +1 to every position");
}
