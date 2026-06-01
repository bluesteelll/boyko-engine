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
//! ## RESOLVED in Phase 9.2 (with a carried-forward crossbeam caveat)
//!
//! The boyko-surface blocker that earlier `#[ignore]`d this test — the
//! `ScopeShared` join protected-tag conflict + the post-decrement `waker`
//! data race — is fixed: the landed `NonNull<ScopeShared>` field cleared the
//! Tree-Borrows protector, and Phase 9.2's Candidate U makes `complete_task`
//! (see `boyko_threadpool/src/scope.rs`) call `waker.unpark()` BEFORE its
//! `pending.fetch_sub` — so the `fetch_sub` is the worker's last allocation
//! access and the box is freed only at the single `Scope::drop` site after the
//! join, clearing the data race. The boyko Scope surface is thus verified by
//! `boyko_threadpool/tests/miri_scope.rs` (TB + data-race clean, default seed;
//! UB-clean across 16 seeds).
//!
//! ## RESOLVED in Phase 9.3c (this test is now ENABLED)
//!
//! Two follow-on blockers were cleared after Phase 9.2:
//!   - **Phase 9.3a** made every executor wait site Miri-cooperative
//!     (`#[cfg(miri)] yield_now()` on the Step-5 park, the `apply_window_drain`
//!     transient-empty pop loop, and the worker backoff), so the 2-worker run
//!     completes instead of livelocking.
//!   - **Phase 9.3c** relocated the cross-thread completion state
//!     (`completion_queue` + `pending_apply`) out of the inline
//!     `Schedule.executor_scratch` into a SEPARATE heap allocation
//!     (`CompletionChannel`) owned as a bare `NonNull` and reached only through
//!     a non-retagging `CompletionCell` lineage — so the worker's completion
//!     push is no longer a foreign write to bytes covered by the dispatcher's
//!     `&mut self` Tree-Borrows protector. This mirrors the Phase 9.2
//!     `NonNull<ScopeShared>` relocation; the `write access forbidden` is gone.
//!
//! The boyko `Scope` surface remains independently gated by the green
//! `boyko_threadpool/tests/miri_scope.rs`; the native
//! `scheduler_par_iter_concurrent_systems` integration test covers the
//! cross-worker world-cell path on real hardware.
//!
//! **Carried-forward caveat (from Phase 9.1):** this 2-worker path also drives
//! `crossbeam-deque::steal_batch_and_pop`, whose exposed-provenance int-to-ptr
//! casts Tree Borrows may flag *independently of boyko*. If a future toolchain
//! surfaces a `crossbeam-*`-frame TB error (not a `scope.rs`/`schedule.rs`
//! frame), that is a third-party limitation — re-`#[ignore]` it with a
//! crossbeam-specific reason and rely on `miri_scope.rs` as the boyko-surface
//! gate. A `scope.rs`/`schedule.rs`-frame failure is a real regression.
//! (As of Phase 9.3c, with `-Zmiri-permissive-provenance` set, NO crossbeam-deque
//! TB *error* surfaces — only the crossbeam-epoch *leaks* handled below.)
//!
//! ## Run (plan §5 / §12)
//! ```bash
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation \
//!   -Zmiri-permissive-provenance -Zmiri-ignore-leaks" \
//!   cargo +nightly miri test -p boyko-ecs --test miri_schedule_parallel
//! ```
//!
//! `-Zmiri-ignore-leaks` is REQUIRED (same as `miri_scope.rs`): with the Phase
//! 9.3b worker join, the 2 worker threads + main thread each register a
//! `crossbeam_epoch::internal::Local` and leave `SealedBag` GC nodes that
//! crossbeam-epoch never reclaims at process exit (it has no global shutdown
//! hook) — Miri's leak checker flags those 7 third-party allocations. They are
//! NOT boyko allocations: the `CompletionChannel` `Box` is freed in
//! `Drop for ExecutorScratch`. With the flag set, the run is TB-clean and
//! UB-free (`1 passed; 0 failed`).
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

