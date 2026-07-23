//! E — parallel-soundness of the relation join with the scheduler conflict graph.
//!
//! # Sequential-only join, but a valid SystemParam (FINDING-1 fixed)
//!
//! `Related<R, D>: QueryData` no longer carries a `D: 'static` bound, so
//! `Query<(.., Related<ChildOf, &Pos>)>` IS usable as a **SystemParam** in a system
//! body and iterates sequentially via `.iter()`. The join is still SEQUENTIAL-ONLY:
//! `.par_iter()` const-rejects a `Related` term (the parallel chunk runner has no
//! world cell to resolve the FK target's archetype per row). So a *parallel* fan-out
//! over a `Related` join is not constructible by design, but the SystemParam itself
//! is. This file tests the parallel-soundness properties that ARE real:
//!
//! * E.1 — the conflict-graph PREDICATE (`Access::conflicts_with`, the exact
//!   function `ConflictGraph::build` consumes) over the components a `Related<R,
//!   &Pos>` join READS (`Pos` + the FK column): a Pos-writer serializes with a
//!   Pos-reader; two Pos-readers parallelize; a ChildOf-FK-reader serializes with a
//!   ChildOf writer. This is exactly the classification the join's declared reads
//!   would receive.
//! * E.2 — the join's REAL execution path: an exclusive (`&mut EcsMaster`) system
//!   runs `world.query::<Related<ChildOf, &Pos>>()` alongside a `&mut Pos` writer in
//!   the same `Schedule`. The exclusive system holds the whole-world borrow, so the
//!   executor serializes it against the writer (sound: the `Related` resolve needs
//!   `&mut self` exclusivity). Results are asserted correct under the 4-worker pool.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::relation::Related;
use boyko_ecs::ecs::core::iters::query::filter::With;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::system::System;
use boyko_ecs::ecs::core::system::{Commands, IntoSystem};
use boyko_macros::Component;
use boyko_threadpool::ThreadPoolBuilder;

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Pos {
    x: i64,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Tag(u32);

/// Initializes a function system against `world` so its `Access` is populated.
fn init_sys<S: System>(world: &mut EcsMaster, mut sys: S) -> S {
    sys.initialize(world);
    sys
}

// ════════════════════════════════════════════════════════════════════════════
// E.1 — the conflict-graph predicate over the components a Related join reads.
//
// A `Related<ChildOf, &Pos>` join reads `Pos` (the inner) + `ChildOf` (the FK).
// These assertions use `Access::conflicts_with` — the SAME predicate
// `ConflictGraph::build` uses to decide serialization — proving the join's reads
// would serialize against a Pos/ChildOf writer and parallelize against a reader.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn pos_reader_serializes_with_pos_writer() {
    let mut world = EcsMaster::new();
    let writer = init_sys(
        &mut world,
        IntoSystem::into_system(|mut q: Query<&mut Pos>| {
            let _ = q.iter_mut().count();
        }),
    );
    // The Related<ChildOf, &Pos> join's inner read is exactly this `&Pos` read.
    let reader = init_sys(
        &mut world,
        IntoSystem::into_system(|q: Query<&Pos>| {
            let _ = (&q).into_iter().count();
        }),
    );
    assert!(
        writer.access().conflicts_with(reader.access()),
        "a Pos writer serializes with a Pos reader — the join's inner Pos read \
         receives this classification"
    );
    assert!(reader.access().conflicts_with(writer.access()), "symmetric");
}

#[test]
fn two_pos_readers_run_in_parallel() {
    let mut world = EcsMaster::new();
    let r1 = init_sys(
        &mut world,
        IntoSystem::into_system(|q: Query<&Pos>| {
            let _ = (&q).into_iter().count();
        }),
    );
    let r2 = init_sys(
        &mut world,
        IntoSystem::into_system(|q: Query<&Pos>| {
            let _ = (&q).into_iter().count();
        }),
    );
    assert!(
        !r1.access().conflicts_with(r2.access()),
        "two Pos readers do NOT conflict ⇒ may run in parallel (the join's inner \
         read is a read, so it parallelizes with another reader)"
    );
}

#[test]
fn childof_fk_reader_serializes_with_childof_writer() {
    // The join ALSO reads the FK column (`ChildOf`). A ChildOf reader therefore
    // serializes with a ChildOf writer.
    let mut world = EcsMaster::new();
    let reader = init_sys(
        &mut world,
        IntoSystem::into_system(|q: Query<&ChildOf>| {
            let _ = (&q).into_iter().count();
        }),
    );
    let writer = init_sys(
        &mut world,
        IntoSystem::into_system(|mut q: Query<&mut ChildOf>| {
            let _ = q.iter_mut().count();
        }),
    );
    assert!(
        reader.access().conflicts_with(writer.access()),
        "the join reads the ChildOf FK column ⇒ serializes with a &mut ChildOf writer"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// E.2 — behavioral: the join's REAL path (exclusive system + world.query) runs
//        correctly alongside a parallel writer in a real Schedule.
// ════════════════════════════════════════════════════════════════════════════

/// Sum probe written by the exclusive Related-reader system.
static RELATED_SUM: AtomicU64 = AtomicU64::new(0);

#[test]
fn exclusive_related_reader_and_parallel_writer_correct_results() {
    RELATED_SUM.store(0, Ordering::Relaxed);

    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let mut world = EcsMaster::new();

    // Parent Pos{10}; 3 children with their own Pos + a ChildOf FK to the parent.
    let parent_holder = Arc::new(std::sync::Mutex::new(None));
    let ph = Arc::clone(&parent_holder);
    world.run_system(move |mut cmds: Commands| {
        let parent = cmds.spawn(Tag(0)).insert(Pos { x: 10 }).id();
        *ph.lock().unwrap() = Some(parent);
    });
    let parent = parent_holder.lock().unwrap().unwrap();
    world.run_system(move |mut cmds: Commands| {
        for i in 0..3u32 {
            cmds.spawn(Tag(i + 1))
                .insert(Pos { x: 100 + i as i64 })
                .insert(ChildOf(parent));
        }
    });

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // A non-exclusive writer.
    builder.add_system(|mut q: Query<&mut Pos>| {
        for p in q.iter_mut() {
            p.x += 1;
        }
    });
    // The EXCLUSIVE Related reader (the join's real, sequential-only path through
    // `world.query`). It holds the whole-world borrow, so the executor serializes
    // it against the writer — sound, since the `Related` resolve needs `&mut self`.
    builder.add_system(|world: &mut EcsMaster| {
        let mut sum = 0i64;
        for p in world
            .query::<Related<ChildOf, &Pos>, With<ChildOf>>()
            .iter()
            .flatten()
        {
            sum += p.x;
        }
        RELATED_SUM.store(sum as u64, Ordering::Relaxed);
    });

    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    // Serialized writer vs exclusive reader ⇒ the reader saw either all-pre (30) or
    // all-post (33) parent Pos across the 3 child rows — never torn.
    let related = RELATED_SUM.load(Ordering::Relaxed) as i64;
    assert!(
        related == 3 * 10 || related == 3 * 11,
        "exclusive Related reader summed the parent's Pos.x over 3 child rows \
         consistently (30 or 33), got {related}"
    );

    // The writer bumped every Pos exactly once.
    let final_sum: i64 = world.query::<&Pos, ()>().iter().map(|p| p.x).sum();
    assert_eq!(
        final_sum, 317,
        "after the frame all 4 Pos bumped once: 11+101+102+103 = 317"
    );
}
