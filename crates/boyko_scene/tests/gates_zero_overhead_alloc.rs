//! S2 propagation GATE — 0%-OVERHEAD on an unchanged tree.
//!
//! Two independent proofs that a still frame does ~no work:
//!
//! 1. **Zero affine composes** (the dirty-gate, value-driven), via the debug-only
//!    `STILL_FRAME_COMPOSES` counter the propagation maintains: a first run
//!    composes the whole tree (> 0), a following STILL run composes EXACTLY 0,
//!    and moving one interior node composes EXACTLY that subtree. This is the
//!    authoritative "did propagation do work" signal.
//!
//!    NOTE — the `STILL_FRAME_COMPOSES` counter (not the `GlobalTransform` change
//!    tick) is the work proxy used HERE because it counts EVERY recompose,
//!    including an idempotent re-descent whose value is unchanged. Post-F2 the
//!    `GlobalTransform` `changed_tick` IS advanced on a real move (the write now
//!    routes through `Mut::set_if_neq`, not the raw accessor) — that tick-fires /
//!    tick-quiet behavior is asserted separately in `gates_change_detection`. The
//!    compose counter remains the right "did propagation touch this node" signal
//!    for this still-frame gate.
//!
//! 2. **Alloc-free steady state** (Principle 5), via a counting global allocator
//!    and BASELINE SUBTRACTION (mirrors `boyko_ui`'s `zero_alloc` test). A
//!    parallel [`Schedule`]'s executor allocates a small FIXED number of bytes
//!    per frame for its own task/dispatch machinery, independent of the system
//!    body. So the propagation's OWN per-frame allocations are measured as the
//!    DELTA between a propagation schedule and a same-SHAPE single-exclusive-noop
//!    schedule over the same warmed tree. A DELTA of 0 proves the steady-state
//!    propagation allocates nothing of its own (its scratch buffers are reused).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Bundle;
use boyko_math::Vec3;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_scene::{GlobalTransform, Transform, propagate_transforms};

// ───────────────────────── counting allocator ─────────────────────────────

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static ARMED: AtomicBool = AtomicBool::new(false);

// SAFETY: forwards every call verbatim to the system allocator; the only added
// behavior is an atomic increment on alloc/realloc when armed.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// The counter `ARMED`/`ALLOCS` are PROCESS-GLOBAL: an armed window on one test
/// thread counts allocations from every OTHER test thread in this binary too.
/// Serialize all armed measurement so exactly one armed window is live at a time
/// (the same reason `boyko_ui`'s `zero_alloc` holds an `ARM_LOCK`).
static ARM_LOCK: Mutex<()> = Mutex::new(());

fn lock_arm() -> std::sync::MutexGuard<'static, ()> {
    ARM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn count_allocs(f: impl FnOnce()) -> usize {
    ALLOCS.store(0, Ordering::Relaxed);
    ARMED.store(true, Ordering::Relaxed);
    f();
    ARMED.store(false, Ordering::Relaxed);
    ALLOCS.load(Ordering::Relaxed)
}

// ───────────────────────── harness ─────────────────────────

#[derive(Bundle)]
struct SpatialBundle {
    transform: Transform,
    global: GlobalTransform,
}

#[inline]
fn spatial(transform: Transform) -> SpatialBundle {
    SpatialBundle { transform, global: GlobalTransform::IDENTITY }
}

/// An exclusive no-op with the SAME schedule SHAPE as the propagation schedule
/// (one exclusive `fn(&mut EcsMaster)` system), for the alloc baseline.
fn noop_exclusive(_w: &mut EcsMaster) {}

/// Spawns the same representative tree into `world` and returns the roots+nodes.
/// A grandparent → parent → 4 children fan, so the descent has both depth and
/// width; built through `Commands` so the `Children` reverse index is live.
fn seed_tree(world: &mut EcsMaster) -> Vec<Entity> {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let mut v = probe.lock().expect("probe");
        let g = cmds.spawn(spatial(Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)))).id();
        v.push(g);
        let p = {
            let mut e = cmds.spawn(spatial(Transform::from_translation(Vec3::new(0.0, 2.0, 0.0))));
            e.set_parent(g);
            e.id()
        };
        v.push(p);
        for i in 0..4 {
            let c = {
                let mut e = cmds
                    .spawn(spatial(Transform::from_translation(Vec3::new(i as f32, 0.0, 1.0))));
                e.set_parent(p);
                e.id()
            };
            v.push(c);
        }
    });
    sink.lock().expect("probe").clone()
}

fn build_propagation_schedule(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    b.add_system(propagate_transforms);
    b.build(world)
}

fn build_baseline_schedule(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    b.add_system(noop_exclusive);
    b.build(world)
}

/// Builds a PERSISTENT noop "ticker" schedule whose `run` advances the world
/// change tick without running propagation. It must be built ONCE and REUSED:
/// a freshly-built-then-run schedule consumes a different tick window than a
/// reused one, which would desync the propagation schedule's `last_run` against
/// a post-run mutation (empirically a fresh ticker per edit misses the edit).
/// A SEPARATE schedule, so advancing the tick does not run (and thus does not
/// advance the `last_run` of) the propagation system. See the
/// `gates_composition_structural` harness doc.
fn build_ticker(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    b.add_system(noop_exclusive);
    b.build(world)
}

/// Worst-of-N armed steady-state `run()` alloc count, after warming.
fn warmed_idle_allocs(world: &mut EcsMaster, sched: &mut Schedule) -> usize {
    for _ in 0..8 {
        sched.run(world);
    }
    (0..4).map(|_| count_allocs(|| sched.run(world))).max().unwrap_or(0)
}

// ════════════════════════════════════════════════════════════════════════════
// 0%-OVERHEAD (work side) — a still frame composes ZERO affines; moving one
// interior node composes EXACTLY that subtree (the dirty-gate is value-driven).
// Uses the authoritative debug-only `STILL_FRAME_COMPOSES` counter (the
// GlobalTransform tick is bypassed by the raw write — see the module doc / F2).
// ════════════════════════════════════════════════════════════════════════════

#[cfg(debug_assertions)]
#[test]
fn still_frame_composes_zero_then_dirty_subtree_only() {
    use boyko_scene::propagation::STILL_FRAME_COMPOSES;

    // `STILL_FRAME_COMPOSES` is a PROCESS-GLOBAL counter reset at the start of
    // every `propagate_transforms` call. The sibling alloc test ALSO runs
    // propagation; if it interleaves it would reset/bump this counter mid-check.
    // Hold the shared lock so exactly one propagation-measuring test runs at a
    // time (the same process-global-`static` discipline the alloc test uses).
    let _arm = lock_arm();

    let mut world = EcsMaster::new();
    // Persistent ticker, built ONCE and reused (see `build_ticker`).
    let mut ticker = build_ticker(&mut world);
    ticker.run(&mut world); // lift the tick off ZERO before any spawn
    let nodes = seed_tree(&mut world); // [g, p, c0, c1, c2, c3] — 6 spatial nodes
    let mut sched = build_propagation_schedule(&mut world);

    // Run 1: everything is dirty (first run) — the whole tree composes.
    sched.run(&mut world);
    assert!(
        STILL_FRAME_COMPOSES.load(Ordering::Relaxed) > 0,
        "first run composes the whole tree"
    );

    // Run 2: nothing changed — ZERO affine composes (the still-frame 0%-gate).
    sched.run(&mut world);
    assert_eq!(
        STILL_FRAME_COMPOSES.load(Ordering::Relaxed),
        0,
        "a fully-static frame performs zero affine composes"
    );

    // Run 3: move ONLY the parent (nodes[1]); EXACTLY its subtree recomposes
    // (parent + its 4 children = 5 nodes), proving the gate is value-driven.
    // Advance the tick first (REUSED ticker) so the edit lands in the next window.
    ticker.run(&mut world);
    let p = nodes[1];
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(p).insert(Transform::from_translation(Vec3::new(0.0, 99.0, 0.0)));
    });
    sched.run(&mut world);
    assert_eq!(
        STILL_FRAME_COMPOSES.load(Ordering::Relaxed),
        5,
        "moving the parent recomposes exactly its 5-node subtree (parent + 4 kids)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// 0%-OVERHEAD (alloc side) — steady state allocates nothing over the scheduler
// baseline (DELTA == 0).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn steady_state_propagation_allocates_zero_over_baseline() {
    let _arm = lock_arm();

    let mut wb = EcsMaster::new();
    let _ = seed_tree(&mut wb);
    let mut sb = build_baseline_schedule(&mut wb);
    let base = warmed_idle_allocs(&mut wb, &mut sb);

    let mut wp = EcsMaster::new();
    let _ = seed_tree(&mut wp);
    let mut sp = build_propagation_schedule(&mut wp);
    let pair = warmed_idle_allocs(&mut wp, &mut sp);

    assert!(
        pair <= base,
        "steady-state propagation must allocate no more than the scheduler baseline \
         (baseline {base}, propagation {pair}; propagation's own per-frame allocs = {})",
        pair as i64 - base as i64
    );
}
