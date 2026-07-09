//! Phase 15 — Miri validation for the build-time set-ordering expansion.
//!
//! Run under:
//!
//! ```powershell
//! $env:MIRIFLAGS = "-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-ecs --test miri_phase15
//! ```
//!
//! `-Zmiri-tree-borrows` is the workspace default (`.cargo/config.toml`);
//! `-Zmiri-ignore-leaks` MUST be appended because each test constructs an
//! `Arc<ThreadPool>` whose OS worker threads Miri reports as "leaked" at
//! process exit (the harness shutdown check, NOT a UB — see the gating note
//! below). Every test body itself is UB-clean; the 4 tests pass with zero
//! Miri errors.
//!
//! # Why these tests exist
//!
//! Phase 15 added **no `unsafe`** — the entire feature is build-time graph
//! manipulation (`HashMap` interning, `Vec` edge expansion, the iterative
//! Tarjan SCC / Kahn topo-sort / hierarchy-flatten DFS, and the derive
//! codegen). UB risk is consequently low, but Miri still validates that the
//! new collection-heavy `try_build` pipeline has no provenance / aliasing /
//! uninitialised-read defect introduced by the expansion code.
//!
//! Each test drives `ScheduleBuilder::try_build` over a representative shape:
//!
//! 1. `miri_set_expansion_builds_clean` — `in_set` members + `configure_set`
//!    ordering (the D1 cartesian expansion path).
//! 2. `miri_hierarchy_flatten_builds_clean` — nested `configure_set(..).in_set`
//!    (the D3 transitive-membership DFS).
//! 3. `miri_enum_derive_intern_clean` — enum-variant `SystemSet` interning via
//!    the derive (distinct discriminants → distinct ids).
//! 4. `miri_cycle_detection_clean` — the error path (`OrderingCycle`) exercises
//!    the Tarjan SCC + enrichment String building.
//!
//! # Single-thread only / thread-pool gating
//!
//! `Schedule::run` spawns worker tasks via `Scope::spawn`, whose raw-pointer
//! handshake hits a known Tree Borrows protected-tag conflict under Miri
//! (documented in `miri_phase9.rs`; deferred to Phase 9.1). Phase 15 does NOT
//! touch that path, so these tests stop at `try_build` and never call `run`.
//!
//! `ThreadPool::Drop` joins its workers, so the `Arc<ThreadPool>` constructed
//! per test is released cleanly (no leaked threads) when the builder/schedule
//! is dropped at end of scope. Should a future Miri build flag the worker
//! threads, run with `-Zmiri-ignore-leaks` appended to `MIRIFLAGS` (mirrors the
//! `miri_phase9.rs` caveat) — the build logic remains UB-clean regardless.
//!
//! Like the other `miri_phase*.rs` files this is NOT gated on `#[cfg(miri)]`,
//! so it also runs as a fast smoke test under the regular `cargo test`.

use std::sync::Arc;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{ScheduleBuildError, ScheduleBuilder};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_macros::SystemSet;

#[derive(SystemSet)]
struct MiriSetS;

#[derive(SystemSet)]
struct MiriSetT;

#[derive(SystemSet)]
struct MiriSetM;

#[derive(SystemSet)]
struct MiriSetP;

#[derive(SystemSet)]
struct MiriSetR;

#[derive(SystemSet)]
enum MiriCombat {
    Target,
    Damage,
}

/// Single-worker pool — these tests never dispatch, so one worker is enough and
/// keeps the per-test thread footprint minimal under Miri.
fn miri_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// D1 — set-ordering cartesian expansion path. `a`,`b` in S; `x` in T;
/// `configure_set(S).before(T)` expands to {a→x, b→x}. The build (interning +
/// expand_set_edges + Tarjan + Kahn) must be provenance-clean.
#[test]
fn miri_set_expansion_builds_clean() {
    let mut builder = ScheduleBuilder::new(miri_pool());
    builder.add_system(|| {}).in_set(MiriSetS);
    builder.add_system(|| {}).in_set(MiriSetS);
    builder.add_system(|| {}).in_set(MiriSetT);
    builder.configure_set(MiriSetS).before(MiriSetT);

    let mut world = EcsMaster::new();
    let schedule = builder
        .try_build(&mut world)
        .expect("set expansion build must succeed");
    assert_eq!(schedule.len(), 3);
    // Drop `schedule` here → drops the Arc<ThreadPool> → joins workers.
}

/// D3 — hierarchy-flatten DFS path. `a` in M; M nested under P; P before R;
/// `r` in R. The transitive-membership DFS + dedup must be clean.
#[test]
fn miri_hierarchy_flatten_builds_clean() {
    let mut builder = ScheduleBuilder::new(miri_pool());
    builder.add_system(|| {}).in_set(MiriSetM);
    builder.add_system(|| {}).in_set(MiriSetR);
    builder.configure_set(MiriSetM).in_set(MiriSetP);
    builder.configure_set(MiriSetP).before(MiriSetR);

    let mut world = EcsMaster::new();
    let schedule = builder
        .try_build(&mut world)
        .expect("hierarchy flatten build must succeed");
    assert_eq!(schedule.len(), 2);
}

/// Enum-variant interning via the derive: distinct variants resolve to distinct
/// ids; the same variant resolves to one id. Exercises `set_id_of_value`'s
/// `(TypeId, discriminant)` HashMap keying under Miri.
#[test]
fn miri_enum_derive_intern_clean() {
    let mut builder = ScheduleBuilder::new(miri_pool());
    let target_a = builder.configure_set(MiriCombat::Target).id();
    let target_b = builder.configure_set(MiriCombat::Target).id();
    let damage = builder.configure_set(MiriCombat::Damage).id();
    assert_eq!(target_a, target_b);
    assert_ne!(target_a, damage);

    builder.add_system(|| {}).in_set(MiriCombat::Target);
    builder.add_system(|| {}).in_set(MiriCombat::Damage);
    builder
        .configure_set(MiriCombat::Target)
        .before(MiriCombat::Damage);

    let mut world = EcsMaster::new();
    let schedule = builder
        .try_build(&mut world)
        .expect("enum-variant ordering build must succeed");
    assert_eq!(schedule.len(), 2);
}

/// Error path — a set-induced 2-cycle drives the Tarjan SCC detection and the
/// `enrich_system_name` String building. Miri validates the error path's
/// allocations / iterator provenance.
#[test]
fn miri_cycle_detection_clean() {
    let mut builder = ScheduleBuilder::new(miri_pool());
    let s = builder.add_system(|| {}).in_set(MiriSetS).key();
    builder.add_system(|| {}).before(s).in_set(MiriSetT);
    builder.configure_set(MiriSetS).before(MiriSetT);

    let mut world = EcsMaster::new();
    // `Schedule` is not `Debug`, so match instead of `expect_err`.
    let err = match builder.try_build(&mut world) {
        Ok(_schedule) => panic!("set-induced cycle must error"),
        Err(e) => e,
    };
    assert!(matches!(err, ScheduleBuildError::OrderingCycle { .. }));
}
