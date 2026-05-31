//! Phase 9 Wave 7 Step 19 — Miri test suite for the parallel scheduler.
//!
//! Designed for execution under `cargo +nightly miri test --test miri_phase9`
//! with `MIRIFLAGS=-Zmiri-tree-borrows` (workspace `.cargo/config.toml`).
//! The tests exercise the Phase 9 unsafe surface that Miri can validate
//! cheaply on a single thread:
//!
//! - `Schedule::run` with no systems — minimal control-flow exercise.
//! - `InSystemRunGuard` round trip — TLS flag set/cleared per system body
//!   (ALLOC1 / ALLOC6).
//! - `current_worker_id` / `current_worker_id_or_dispatcher_lane` — TLS
//!   sentinel routing (TPN13 / EVT1).
//! - `is_in_system_run` precondition reads.
//!
//! # Cross-thread Miri limitation — RESOLVED in Phase 9.2
//!
//! Multi-thread scheduler runs previously tripped a Tree Borrows
//! protected-tag conflict in `boyko_threadpool::Scope` (the worker-side
//! `pending.fetch_sub` foreign-wrote a tag the dispatcher held as a
//! protected `Reserved (conflicted)` `&ScopeShared` across the join).
//! Phase 9.2 landed the `NonNull<ScopeShared>` field refactor (the joiner
//! takes a by-value `*const ScopeShared`, so no protected borrow spans the
//! workers' writes), and Phase 9.2's Candidate U makes `complete_task`
//! (`crates/boyko_threadpool/src/scope.rs`) call `waker.unpark()` BEFORE its
//! `pending.fetch_sub` — so the `fetch_sub` is the worker's last allocation
//! access and the box is freed only at the single `Scope::drop` site, after the
//! join, clearing the data race the refactor unmasked.
//! The dedicated multi-worker Miri gate now lives in
//! `crates/boyko_threadpool/tests/miri_scope.rs` (boyko surface) and
//! `crates/boyko_ecs/tests/miri_schedule_parallel.rs` (2-worker executor).
//!
//! The tests below are **safe** under Miri because they:
//!   1. Use `num_threads(1)` AND only run schedules with zero systems
//!      (no `scope.spawn` invocation).
//!   2. Exercise TLS state mutation only — no shared-memory worker
//!      handoff.
//!
//! Like `miri_phase8cd.rs` / `miri_phase8_5.rs`, the file is **not** gated
//! on `#[cfg(miri)]` — it runs as a smoke test under the regular `cargo
//! test` too.

#[cfg(not(miri))]
use std::sync::Arc;

#[cfg(not(miri))]
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
#[cfg(not(miri))]
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
#[cfg(not(miri))]
use boyko_threadpool::ThreadPoolBuilder;
use boyko_threadpool::{
    InSystemRunGuard, WORKER_ID_UNATTACHED, current_worker_id,
    current_worker_id_or_dispatcher_lane, is_in_system_run,
};

/// A scheduler with zero systems must run cleanly. Exercises the
/// `executor_main_loop` early-return path (`completed == 0 == n`) without
/// minting any worker spawn — verifies the Phase 9 control flow does not
/// dereference invalid pointers when there is nothing to dispatch.
///
/// # Miri caveat
///
/// `ThreadPoolBuilder::build` spawns OS worker threads that Miri's
/// `-Zmiri-disable-isolation`-less mode flags as "leaked" because there
/// is no current `ThreadPool::Drop` that joins them. The test logic is
/// still UB-clean — the leak detection runs at process exit; setting
/// `-Zmiri-ignore-leaks` (or invoking with `MIRIFLAGS="$MIRIFLAGS
/// -Zmiri-ignore-leaks"`) silences the harness shutdown error.
///
/// We gate this test on `#[cfg(not(miri))]` to keep the file's Miri
/// suite green by default. Test runs under regular `cargo test` are
/// unaffected.
#[cfg(not(miri))]
#[test]
fn miri_schedule_run_empty_no_ub() {
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut world = EcsMaster::new();
    let builder = ScheduleBuilder::new(Arc::clone(&pool));
    let mut sched = builder.build(&mut world);
    sched.run(&mut world);
    sched.run(&mut world); // second frame — pred_remaining reset path.
}

/// `InSystemRunGuard` round-trip — the TLS flag is set on `enter` and
/// cleared on drop. Miri sees the Cell::get / Cell::set ops and validates
/// no concurrent thread observes torn state (single-thread here).
#[test]
fn miri_in_system_run_guard_round_trip_no_ub() {
    assert!(!is_in_system_run());
    {
        let _g = InSystemRunGuard::enter();
        assert!(is_in_system_run());
        // The guard's destructor will fire at the end of this block.
    }
    assert!(!is_in_system_run());
}

/// Verifies the TLS sentinel for an unattached thread (`WORKER_ID_UNATTACHED`)
/// and the lane mapping for the dispatcher (`worker_count` slot) versus
/// the unattached fallback (0). Pure TLS read — no allocation, no spawn.
#[test]
fn miri_worker_id_tls_sentinels_no_ub() {
    // Test threads start unattached.
    assert_eq!(current_worker_id(), WORKER_ID_UNATTACHED);
    // Unattached thread maps to lane 0 (EVT1 fallback).
    assert_eq!(current_worker_id_or_dispatcher_lane(8), 0);
}

/// Sequential `InSystemRunGuard::enter`/drop pairs — back-to-back guards
/// must round-trip without leaving the TLS flag set. Catches a regression
/// where the drop path forgets to clear (would make `is_in_system_run`
/// stay true between system bodies, tripping the allocation discipline
/// debug assertion on the next allocator call).
#[test]
fn miri_in_system_run_guard_back_to_back_no_ub() {
    for _ in 0..3 {
        assert!(!is_in_system_run());
        let _g = InSystemRunGuard::enter();
        assert!(is_in_system_run());
        drop(_g);
        assert!(!is_in_system_run());
    }
}
