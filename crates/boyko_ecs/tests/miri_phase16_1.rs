//! Phase 16.1 — Miri (Tree Borrows) for the C1 gated-system dispatch stamp.
//!
//! Phase 16.1 C1 stamps a GATED system's change-detection ticks at its DISPATCH
//! site (only on a frame it runs) instead of unconditionally at frame start. On
//! the concurrent path this is a PRE-PASS that takes `&mut self.systems[i]` for
//! every gated index in `to_spawn` BEFORE the `systems_ptr = self.systems
//! .as_mut_ptr()` raw lift (OQ-R2-1 resolved to the pre-pass form precisely so
//! the `&mut` retags do not invalidate `systems_ptr`'s provenance while a worker
//! still holds it).
//!
//! The code-reviewer flagged the specific TB envelope to exercise: a
//! **conflict-deferred gated system** — a gated system that CANNOT dispatch in
//! round K (blocked by a conflict with a running system) and is therefore
//! pre-pass-stamped in round K+1, while round-K worker closures may still hold
//! the round-K `systems_ptr` (the same Tree-Borrows envelope as the existing
//! `apply_window_drain` reborrow). This file drives exactly that shape under
//! `-Zmiri-tree-borrows` and asserts no UB.
//!
//! ## Why this is sound (and why TB accepts it post-9.3c)
//!
//! `systems_ptr` targets the `Schedule::systems` Vec's SEPARATE heap buffer
//! (outside the `Schedule` allocation, so no `&mut self` protector covers it —
//! Phase 9.3c). The pre-pass `&mut self.systems[i]` retags complete and are
//! released BEFORE `systems_ptr` is minted each round, and the conflict graph
//! (SCH3) guarantees no two concurrently-dispatched systems alias the same slot.
//! A round-K+1 stamp therefore never aliases a round-K worker's live `*mut
//! SystemBox` (different index, and the round-K pointer was minted from a borrow
//! that ended before this round began). The cross-worker completion handshake is
//! the Phase-9.3c `CompletionCell` lineage, already TB-clean
//! (`miri_schedule_parallel.rs`).
//!
//! ## Run
//! ```bash
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation \
//!   -Zmiri-permissive-provenance -Zmiri-ignore-leaks" \
//!   cargo +nightly miri test -p boyko-ecs --test miri_phase16_1
//! ```
//! `-Zmiri-ignore-leaks` is required for the same reason as
//! `miri_schedule_parallel.rs` (crossbeam-epoch GC nodes unreclaimed at exit).
#![cfg(miri)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Changed, Mut, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_macros::Component;
use boyko_threadpool::ThreadPoolBuilder;

#[derive(Component)]
#[repr(C)]
struct MShared {
    v: u32,
}

#[derive(Component)]
#[repr(C)]
struct MOther {
    v: u32,
}

/// Conflict-deferred gated system under Miri-TB. Three systems on a 2-worker
/// pool:
///   * `writer` (plain): writes `MShared` — dispatches round 1.
///   * `reader_other` (plain): reads `MOther`, disjoint from `writer` ⇒
///     dispatches CONCURRENTLY on the second worker in round 1 (this is what
///     forces a live round-1 worker holding the round-1 `systems_ptr`).
///   * `gated` (`.run_if(|| true)`): also writes `MShared` ⇒ CONFLICTS with
///     `writer` ⇒ cannot dispatch in round 1 ⇒ deferred to a later round, where
///     the C1 PRE-PASS stamps `&mut self.systems[gated]` before minting that
///     round's `systems_ptr`.
///
/// Change detection is live (the gated body holds a `Changed<MShared>` query and
/// `writer` mutates via `Mut`), so the dispatch stamp's `set_change_ticks` is
/// semantically exercised, not just structurally.
#[test]
fn miri_conflict_deferred_gated_system_dispatch_stamp_no_ub() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    // Tiny entity sets (Miri is ~100× slower — keep counts minimal).
    let arch_s = world.create_archetype(&[MShared::component_id()]);
    let arch_o = world.create_archetype(&[MOther::component_id()]);
    for i in 0..2 {
        world.spawn_one(arch_s, MShared { v: i }).expect("spawn shared");
        world.spawn_one(arch_o, MOther { v: i }).expect("spawn other");
    }

    let gated_runs = Arc::new(AtomicUsize::new(0));
    let gated_runs_cl = Arc::clone(&gated_runs);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));

    // round-1 writer (plain) — writes MShared.
    builder.add_system(|mut q: Query<Mut<MShared>>| {
        for mut s in &mut q {
            s.v = s.v.wrapping_add(1);
        }
    });
    // round-1 disjoint reader (plain) — runs on the 2nd worker concurrently with
    // the writer, so a live round-1 worker holds the round-1 systems_ptr while
    // the next round pre-pass-stamps the gated system.
    builder.add_system(|q: Query<&MOther>| {
        let _ = q.iter().count();
    });
    // conflict-deferred GATED system — writes MShared (conflicts with the
    // writer) so it cannot dispatch round 1; gated by a true condition so it DOES
    // run in a later round, hitting the C1 dispatch-stamp pre-pass. Its body
    // reads Changed<MShared> so the stamped window is used.
    builder
        .add_system(move |q: Query<&MShared, Changed<MShared>>| {
            gated_runs_cl.fetch_add(1, Ordering::Relaxed);
            let _ = q.iter().count();
        })
        .run_if(|| true);

    let mut schedule = builder.build(&mut world);

    // Two frames: frame 1 exercises the first dispatch stamp; frame 2 exercises
    // the resume-window path (prev this_run now non-sentinel).
    schedule.run(&mut world);
    schedule.run(&mut world);

    assert_eq!(
        gated_runs.load(Ordering::Relaxed),
        2,
        "the conflict-deferred gated system runs once per frame (true condition)"
    );
}

/// A SKIPPED gated system on the concurrent path: the gate is false, so the
/// system is `mark_skipped` (stamps NOTHING) while OTHER (plain) systems dispatch
/// across 2 workers. Confirms the C1 pre-pass is not entered for a skipped index
/// and the skip-cascade coexists with live workers under TB.
#[test]
fn miri_skipped_gated_system_with_live_workers_no_ub() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let arch_s = world.create_archetype(&[MShared::component_id()]);
    let arch_o = world.create_archetype(&[MOther::component_id()]);
    for i in 0..2 {
        world.spawn_one(arch_s, MShared { v: i }).expect("spawn shared");
        world.spawn_one(arch_o, MOther { v: i }).expect("spawn other");
    }

    let skipped_runs = Arc::new(AtomicUsize::new(0));
    let skipped_runs_cl = Arc::clone(&skipped_runs);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Two plain disjoint systems dispatch concurrently on the 2 workers.
    builder.add_system(|mut q: Query<Mut<MShared>>| {
        for mut s in &mut q {
            s.v = s.v.wrapping_add(1);
        }
    });
    builder.add_system(|q: Query<&MOther>| {
        let _ = q.iter().count();
    });
    // A gated system whose gate is FALSE ⇒ skipped every frame (NOT stamped).
    builder
        .add_system(move || {
            skipped_runs_cl.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(|| false);

    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);
    schedule.run(&mut world);

    assert_eq!(
        skipped_runs.load(Ordering::Relaxed),
        0,
        "the false-gated system is skipped every frame (never stamped, never run)"
    );
}
