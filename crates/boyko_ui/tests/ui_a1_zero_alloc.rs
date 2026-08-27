//! **A1 gate 6 — zero per-frame allocation on the steady animating path**
//! (`docs/UI-PLAN-ANIMATION.md` A1 gate 6, Principle 5).
//!
//! The crate's established shape: a counting global allocator plus BASELINE
//! SUBTRACTION (`zero_alloc.rs`, `p4_bind_zero_alloc.rs`,
//! `text_emit_zero_alloc.rs`). An absolute "0" is not assertable here for the
//! same reason it is not there — `Schedule::run`'s parallel executor allocates a
//! small fixed number of bytes per frame for its own task machinery, a property
//! of the executor and not of the systems' bodies.
//!
//! So both arms run a schedule of **identical SHAPE** — two normal systems and
//! one exclusive system — over an **identical world** (the same animated nodes,
//! spawned the same way), and the delta is the A1 pair's own per-frame cost:
//!
//! * baseline = `[ui_clock_tick, noop_normal, noop_exclusive]`
//! * pair     = `[ui_clock_tick, ui_visual_tick, ui_tween_reap]`
//!
//! `ui_clock_tick` is in BOTH arms deliberately: the tick under test reads the
//! clock, so removing it from the baseline would make the baseline a different
//! schedule rather than a shapeless one.
//!
//! # Why the tweens are long, and why the buffer is warmed with short ones first
//!
//! The measured window is the **steady animating** path: every row mid-tween,
//! nothing completing, so [`UiTweenScratch`]'s retained buffer is not pushed to.
//! It is nonetheless warmed to a real high-water mark first, by a burst of
//! completions, so a `0` below cannot be read as "the buffer was never used".

// Test-harness plumbing only: a file-static `Mutex<()>` serializes the tests that
// arm the process-global allocator counter. Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]
#![cfg(not(miri))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::prelude::*;
use boyko_macros::Component;

use boyko_ui::animation::{
    start_tween_offset, start_tween_opacity, start_tween_tint, ui_clock_tick, ui_tween_reap,
    ui_visual_tick, UiClock, UiTweenScratch,
};
use boyko_ui::components::{EasingId, TweenTint, UiVisual};

// ───────────────────────── armed-window serialization ─────────────────────

static ARM_LOCK: Mutex<()> = Mutex::new(());

fn lock_arm() -> std::sync::MutexGuard<'static, ()> {
    ARM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

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

fn count_allocs(f: impl FnOnce()) -> usize {
    ALLOCS.store(0, Ordering::Relaxed);
    ARMED.store(true, Ordering::Relaxed);
    f();
    ARMED.store(false, Ordering::Relaxed);
    ALLOCS.load(Ordering::Relaxed)
}

// ───────────────────────── fixtures ────────────────────────────────────────

const FRAME: Duration = Duration::from_millis(16);
/// Nodes in the fixture. Small enough to stay fast, plural enough that a
/// per-row allocation would be counted several times over.
const NODES: usize = 32;

#[derive(Component, Clone, Copy, Debug)]
struct Node;

fn noop_normal(_q: Query<&UiVisual>) {}
fn noop_exclusive(_w: &mut EcsMaster) {}

/// A world with [`NODES`] animated nodes, the retained scratch already at a real
/// high-water mark, and every row mid-tween on a duration long enough that none
/// completes inside the measured window.
fn seeded_world() -> EcsMaster {
    let mut world = EcsMaster::new();
    world.insert_resource(Time::default());
    world.insert_resource(UiClock::default());
    world.insert_resource(UiTweenScratch::default());

    let nodes: Vec<Entity> = world.run_system(|mut cmds: Commands| {
        (0..NODES).map(|_| cmds.spawn(Node).id()).collect::<Vec<_>>()
    });

    // Warm the retained completion buffer: SHORT tweens on every node, driven to
    // completion, so `UiTweenScratch`'s Vec reaches its high-water capacity
    // before the measured window — a `0` delta below is then "no growth", not
    // "never used".
    for _ in 0..3 {
        let batch = nodes.clone();
        world.run_system(move |mut cmds: Commands| {
            for &e in &batch {
                start_tween_tint(&mut cmds, e, 0, 0xFFFF_FFFF, 1.0, EasingId::LINEAR, 0);
                start_tween_opacity(&mut cmds, e, 0.0, 1.0, 1.0, EasingId::LINEAR, 0);
            }
        });
        world.resource_mut::<Time>().advance_with(FRAME);
        world.run_system(ui_clock_tick);
        world.run_system(ui_visual_tick);
        world.run_system(ui_tween_reap);
    }

    // The steady state under test: long tweens, nothing completing.
    let batch = nodes;
    world.run_system(move |mut cmds: Commands| {
        for &e in &batch {
            start_tween_tint(&mut cmds, e, 0, 0xFFFF_FFFF, 600_000.0, EasingId::LINEAR, 0);
            start_tween_offset(
                &mut cmds,
                e,
                [0.0, 0.0],
                [-400.0, 0.0],
                600_000.0,
                EasingId::LINEAR,
                0,
            );
        }
    });
    world
}

fn build_pair(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let clock = b.add_system(ui_clock_tick).key();
    let tick = b.add_system(ui_visual_tick).after(clock).key();
    b.add_system(ui_tween_reap).after(tick);
    b.build(world)
}

fn build_baseline(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let clock = b.add_system(ui_clock_tick).key();
    let noop = b.add_system(noop_normal).after(clock).key();
    b.add_system(noop_exclusive).after(noop);
    b.build(world)
}

/// Worst-of-N armed `run()` count after warming the schedule.
fn warmed_allocs(world: &mut EcsMaster, sched: &mut Schedule) -> usize {
    for _ in 0..8 {
        world.resource_mut::<Time>().advance_with(FRAME);
        sched.run(world);
    }
    (0..4)
        .map(|_| {
            world.resource_mut::<Time>().advance_with(FRAME);
            count_allocs(|| sched.run(world))
        })
        .max()
        .unwrap_or(0)
}

// ───────────────────────── the gate ────────────────────────────────────────

/// **A1 gate 6.** The tick + reap pair allocates nothing of its own on the
/// steady animating path.
///
/// Red mutation 6 (a `Vec::new()` in the tick body) reds this: the tick runs once
/// per frame and the allocation lands inside the armed window, above a baseline
/// that has none.
#[test]
fn the_steady_animating_path_allocates_zero_over_baseline() {
    let _arm = lock_arm();

    let mut wb = seeded_world();
    let mut sb = build_baseline(&mut wb);
    let base = warmed_allocs(&mut wb, &mut sb);

    let mut wp = seeded_world();
    let mut sp = build_pair(&mut wp);
    let pair = warmed_allocs(&mut wp, &mut sp);

    // Non-vacuity of the SUBTRACTION first: the method means nothing while the
    // baseline allocates nothing, because then every delta below is trivially
    // satisfied and a schedule that never ran would pass.
    assert!(
        base > 0,
        "the baseline schedule allocated nothing, so the subtraction has no subtrahend"
    );
    // Non-vacuity of the FIXTURE: the rows must actually be mid-tween in the
    // measured window, or the tick's body is an early-out and gate 6 measures a
    // `continue`.
    assert_eq!(
        wp.dense_registry()
            .store(TweenTint::component_id())
            .map_or(0, |s| s.live_count()),
        NODES,
        "every node is still mid-tween after the measured frames — otherwise this gate is \
         measuring the all-None early-out, not the animating path"
    );
    assert!(
        pair <= base,
        "steady animating state: the A1 pair must allocate no more than the scheduler baseline \
         (baseline {base}, pair {pair})"
    );
}
