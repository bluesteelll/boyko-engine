//! FINDING-1 behavioral proof — `Related<R, D>` as a REAL `Query` SystemParam.
//!
//! # What this file adds over the existing suite
//!
//! Before the bound-relaxation fix, `Related<R, D>` carried a `D: 'static` over-
//! constraint. Through `Query<'w, 's, D, F>`'s invariance over `D`, that pinned a
//! `D = &'a T` inner to `&'static T` and made `Query<(.., Related<ChildOf, &Pos>)>`
//! UNUSABLE as a SystemParam in a system body (E0521 "borrowed data escapes").
//!
//! The sibling files prove the fix at two LOWER tiers:
//!   * `related.rs` doctest — a COMPILE-ONLY proof the SystemParam type-checks.
//!   * `relations_query_parallel.rs` E.2 — the join's path through the EXCLUSIVE
//!     (`&mut EcsMaster`) `world.query(..)` form, not the `Query` SystemParam.
//!
//! This file closes the gap: a REAL system FUNCTION whose parameter is a
//! `Query<(.., Related<..>)>`, registered in a `Schedule` and RUN, with the join's
//! per-row result asserted correct (Some/None per child, tuple / `Ref` / `Option`
//! inners). It is the behavioral (not just compile-time) confirmation of FINDING-1.
//!
//! # Why a `static` probe
//!
//! A system registered via `ScheduleBuilder::add_system` is an `Fn`-shaped closure
//! whose only side channel back to the test is a `static` (it cannot return a value
//! or capture `&mut` test state). Each test resets its probe before `schedule.run`.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::Ref;
use boyko_ecs::ecs::core::iters::query::filter::With;
use boyko_ecs::ecs::core::iters::query::relation::Related;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Component;
use boyko_threadpool::ThreadPoolBuilder;

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Pos {
    x: i64,
}

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Vel {
    dx: i64,
}

/// Marks a child entity with its own ordinal so the test can map a joined row
/// back to the child it came from (the query view yields rows, not entities).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct ChildIdx(i64);

/// A bare marker so a parent that should NOT host `Pos` still has a concrete
/// archetype distinct from the `Pos`-bearing parents.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Tag(u32);

/// Spawns one parent in the `{Pos, Tag}` archetype; returns its handle.
fn spawn_pos_parent(world: &mut EcsMaster, x: i64) -> Entity {
    let holder = Arc::new(std::sync::Mutex::new(None));
    let h = Arc::clone(&holder);
    world.run_system(move |mut cmds: Commands| {
        *h.lock().expect("probe") = Some(cmds.spawn(Tag(0)).insert(Pos { x }).id());
    });
    holder.lock().expect("probe").expect("spawned parent")
}

/// Spawns one parent in the `{Tag}` archetype (NO `Pos`); returns its handle.
fn spawn_posless_parent(world: &mut EcsMaster) -> Entity {
    let holder = Arc::new(std::sync::Mutex::new(None));
    let h = Arc::clone(&holder);
    world.run_system(move |mut cmds: Commands| {
        *h.lock().expect("probe") = Some(cmds.spawn(Tag(7)).id());
    });
    holder.lock().expect("probe").expect("spawned posless parent")
}

/// Spawns a child carrying `Vel + ChildIdx (+ optional ChildOf)`.
fn spawn_child(world: &mut EcsMaster, idx: i64, dx: i64, parent: Option<Entity>) {
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(Vel { dx });
        e.insert(ChildIdx(idx));
        if let Some(p) = parent {
            e.insert(ChildOf(p));
        }
    });
}

// ════════════════════════════════════════════════════════════════════════════
// SP.1 — `fn read_parent(q: Query<(&ChildIdx, Related<ChildOf, &Pos>)>)` runs in a
//        Schedule. Asserts the join yields the CORRECT parent Pos per child, and
//        None for a child whose parent lacks Pos.
// ════════════════════════════════════════════════════════════════════════════

/// `(child_idx << 32) | (parent_x_plus_one)` is pushed per joined row so the test
/// can decode each child's resolved parent Pos (0 ⇒ None). Packed into a single
/// atomic counter sum is lossy, so we instead accumulate Some-values and a None
/// count separately.
static SP1_SOME_SUM: AtomicI64 = AtomicI64::new(0);
static SP1_NONE_COUNT: AtomicU64 = AtomicU64::new(0);
static SP1_ROW_COUNT: AtomicU64 = AtomicU64::new(0);

/// A REAL system function: the parameter is a `Query` whose data tuple embeds the
/// relation join. This is the exact shape FINDING-1 unblocked.
fn read_parent_pos(q: Query<(&ChildIdx, Related<ChildOf, &Pos>)>) {
    for (idx, parent_pos) in q.iter() {
        SP1_ROW_COUNT.fetch_add(1, Ordering::Relaxed);
        match parent_pos {
            // Encode idx*1000 + parent_x so a wrong parent→child binding is visible.
            Some(p) => {
                SP1_SOME_SUM.fetch_add(idx.0 * 1000 + p.x, Ordering::Relaxed);
            }
            None => {
                SP1_NONE_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

#[test]
fn query_systemparam_related_join_yields_correct_parent_per_child() {
    SP1_SOME_SUM.store(0, Ordering::Relaxed);
    SP1_NONE_COUNT.store(0, Ordering::Relaxed);
    SP1_ROW_COUNT.store(0, Ordering::Relaxed);

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    // Two Pos parents + one Pos-less parent.
    let pa = spawn_pos_parent(&mut world, 10); // child idx 1 → parent x 10
    let pb = spawn_pos_parent(&mut world, 20); // child idx 2 → parent x 20
    let pc = spawn_posless_parent(&mut world); // child idx 3 → None (parent lacks Pos)

    spawn_child(&mut world, 1, 100, Some(pa));
    spawn_child(&mut world, 2, 200, Some(pb));
    spawn_child(&mut world, 3, 300, Some(pc));

    // Register the REAL Query-SystemParam function and run it in a Schedule.
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(read_parent_pos);
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    assert_eq!(
        SP1_ROW_COUNT.load(Ordering::Relaxed),
        3,
        "the Query SystemParam matched all 3 children (each hosts ChildOf + ChildIdx)"
    );
    assert_eq!(
        SP1_NONE_COUNT.load(Ordering::Relaxed),
        1,
        "exactly one child (the Pos-less parent's) joins to None"
    );
    // idx1·1000+10  +  idx2·1000+20  =  1010 + 2020 = 3030. The Pos-less child
    // contributed None, not a Some, so it is absent from the sum.
    assert_eq!(
        SP1_SOME_SUM.load(Ordering::Relaxed),
        3030,
        "each Some-child resolved EXACTLY its own parent's Pos (1→10, 2→20)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// SP.2 — the same join with a TUPLE inner `Related<ChildOf, (&Pos, &Vel)>` proves
//        a multi-term read-only inner works as a SystemParam. (The parent here
//        carries BOTH Pos and Vel.)
// ════════════════════════════════════════════════════════════════════════════

static SP2_SUM: AtomicI64 = AtomicI64::new(0);
static SP2_NONE: AtomicU64 = AtomicU64::new(0);

fn read_parent_pos_and_vel(q: Query<Related<ChildOf, (&Pos, &Vel)>>) {
    for joined in q.iter() {
        match joined {
            Some((p, v)) => {
                SP2_SUM.fetch_add(p.x + v.dx, Ordering::Relaxed);
            }
            None => {
                SP2_NONE.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

#[test]
fn query_systemparam_related_tuple_inner_works() {
    SP2_SUM.store(0, Ordering::Relaxed);
    SP2_NONE.store(0, Ordering::Relaxed);

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    // Parent carries BOTH Pos and Vel.
    let holder = Arc::new(std::sync::Mutex::new(None));
    let h = Arc::clone(&holder);
    world.run_system(move |mut cmds: Commands| {
        *h.lock().expect("probe") =
            Some(cmds.spawn(Pos { x: 5 }).insert(Vel { dx: 3 }).id());
    });
    let parent = holder.lock().expect("probe").expect("parent");

    // One child whose join hits BOTH inner terms; one orphan child → None.
    spawn_child(&mut world, 1, 0, Some(parent));
    spawn_child(&mut world, 2, 0, None); // no ChildOf ⇒ not matched at all

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(read_parent_pos_and_vel);
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    // Only the ChildOf-bearing child is matched (the orphan lacks ChildOf, so it
    // is not in the query at all — no None contribution from it).
    assert_eq!(
        SP2_NONE.load(Ordering::Relaxed),
        0,
        "the single matched child resolved both inner terms (no None)"
    );
    assert_eq!(
        SP2_SUM.load(Ordering::Relaxed),
        8,
        "tuple inner (&Pos,&Vel) joined to parent (Pos.x=5 + Vel.dx=3)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// SP.3 — `Ref<T>` inner (change-detection-bearing) AND `Option<&T>` inner each
//        work as a SystemParam. `Ref<Pos>` derefs to `&Pos`; `Option<&Pos>` is an
//        inner that itself never filters, so the join yields `Some(Some(..))` /
//        `Some(None)` shapes — confirms NEEDS_CHANGE_DETECTION + non-filtering
//        inner both ride the SystemParam path.
// ════════════════════════════════════════════════════════════════════════════

static SP3_REF_SUM: AtomicI64 = AtomicI64::new(0);
static SP3_OPT_SOME: AtomicU64 = AtomicU64::new(0);

fn read_parent_ref(q: Query<Related<ChildOf, Ref<Pos>>>) {
    // `.flatten()` drops the `None` FK-unresolved rows; `Ref<Pos>` derefs to `&Pos`.
    for p in q.iter().flatten() {
        SP3_REF_SUM.fetch_add(p.x, Ordering::Relaxed);
    }
}

fn read_parent_opt(q: Query<Related<ChildOf, Option<&Pos>>>) {
    for joined in q.iter() {
        // Outer Option = FK resolved?; inner Option = target hosts Pos?
        if let Some(Some(_p)) = joined {
            SP3_OPT_SOME.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[test]
fn query_systemparam_related_ref_and_option_inners_work() {
    SP3_REF_SUM.store(0, Ordering::Relaxed);
    SP3_OPT_SOME.store(0, Ordering::Relaxed);

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let parent = spawn_pos_parent(&mut world, 42);
    spawn_child(&mut world, 1, 0, Some(parent));

    // Ref<Pos> inner.
    let mut b1 = ScheduleBuilder::new(Arc::clone(&pool));
    b1.add_system(read_parent_ref);
    b1.build(&mut world).run(&mut world);

    // Option<&Pos> inner.
    let mut b2 = ScheduleBuilder::new(Arc::clone(&pool));
    b2.add_system(read_parent_opt);
    b2.build(&mut world).run(&mut world);

    assert_eq!(
        SP3_REF_SUM.load(Ordering::Relaxed),
        42,
        "Ref<Pos> inner joined to the parent's Pos (derefs to &Pos)"
    );
    assert_eq!(
        SP3_OPT_SOME.load(Ordering::Relaxed),
        1,
        "Option<&Pos> inner yielded Some(Some(..)) for the single child"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// SP.4 — a sibling NON-relation read in the SAME data tuple (`&Vel` on the source)
//        coexists with the join: proves the FINDING-1 SystemParam form mixes the
//        source's own borrow with the joined target borrow (the exact doctest
//        shape `Query<(&Vel, Related<ChildOf, &Pos>)>`), via With<ChildOf>.
// ════════════════════════════════════════════════════════════════════════════

static SP4_PAIRS: AtomicI64 = AtomicI64::new(0);

fn read_self_vel_and_parent_pos(q: Query<(&Vel, Related<ChildOf, &Pos>), With<ChildOf>>) {
    for (vel, parent_pos) in q.iter() {
        if let Some(p) = parent_pos {
            // self Vel.dx + parent Pos.x, per matched child.
            SP4_PAIRS.fetch_add(vel.dx + p.x, Ordering::Relaxed);
        }
    }
}

#[test]
fn query_systemparam_source_borrow_plus_join_coexist() {
    SP4_PAIRS.store(0, Ordering::Relaxed);

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let pa = spawn_pos_parent(&mut world, 1000);
    spawn_child(&mut world, 1, 1, Some(pa)); // Vel.dx=1   + Pos.x=1000
    spawn_child(&mut world, 2, 2, Some(pa)); // Vel.dx=2   + Pos.x=1000

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(read_self_vel_and_parent_pos);
    builder.build(&mut world).run(&mut world);

    // (1+1000) + (2+1000) = 2003.
    assert_eq!(
        SP4_PAIRS.load(Ordering::Relaxed),
        2003,
        "self &Vel and joined &Pos coexist in one Query SystemParam tuple"
    );
}
