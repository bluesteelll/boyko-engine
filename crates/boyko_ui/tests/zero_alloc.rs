//! Zero-per-frame-allocation test for the layout pair (Principle 5), via a
//! counting global allocator and BASELINE SUBTRACTION.
//!
//! # Why baseline subtraction (and not an absolute "0")
//!
//! The layout pair is driven the way a host drives it: through a parallel
//! [`Schedule`]. `Schedule::run`'s parallel executor allocates a small FIXED
//! number of bytes per frame for its own task/dispatch machinery (measured: ~7
//! per frame for a 2-system schedule), entirely independent of the systems'
//! bodies. That overhead is a property of the executor, not of the layout code,
//! and it cannot be driven to zero from inside `boyko_ui`. (Driving the pair via
//! `run_cached_system` to bypass the executor is NOT a valid alternative: that
//! path does not advance the per-system change-detection tick window the way
//! `Schedule::run` does, so discovery never observes a change and apply never runs
//! — the layout work would be silently skipped, making any "0 allocs" vacuous.)
//!
//! So the test measures the layout pair's OWN per-frame allocations as the DELTA
//! between two schedules with identical system SHAPE (one normal/parallel system
//! + one exclusive system) over the same warmed UI tree:
//!   * baseline = `[noop_normal, noop_exclusive]`
//!   * pair     = `[ui_layout_discovery, ui_layout_apply]`
//!
//! A DELTA of 0 proves the layout pair allocates nothing of its own per frame
//! (the plan's "0 heap allocations per frame in the steady state of BOTH
//! systems"). Each path is warmed to high-water before the counter is armed.
//!
//! The cases asserted (each at DELTA 0): (a) unchanged/steady-state frame,
//! (b) a non-structural size tweak within high-water, (c) a resize frame at the
//! same root count (cached root list reused — no per-frame `query_entities`).

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ───────────────────────── armed-window serialization ─────────────────────
//
// The counting allocator's `ARMED`/`ALLOCS` are PROCESS-GLOBAL: an armed window
// on one test thread also counts allocations made by every OTHER test thread in
// this binary. The default test runner runs these three tests on separate
// threads in parallel, so without serialization a sibling's allocations land in
// this test's armed window (observed: an impossible NEGATIVE "own alloc" delta of
// -10, and inflated steady-state counts). All armed measurement here must hold
// this lock so exactly one armed window is live at a time. Mirrors the
// `PROBE_LOCK` guard the in-crate `layout.rs` change-detection tests use for the
// same process-global-`static` reason. Poison-tolerant (a panicking measurement
// must not cascade a `PoisonError` into the others).
static ARM_LOCK: Mutex<()> = Mutex::new(());

fn lock_arm() -> std::sync::MutexGuard<'static, ()> {
    ARM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_ui::components::{ComputedRect, UiLayout, UiRoot};
use boyko_ui::layout::{ui_layout_apply, ui_layout_discovery};
use boyko_ui::resources::{LayoutScratch, UiViewport};
use boyko_ui::units::{LayoutType, Unit};

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

// ───────────────────────── world / schedule builders ──────────────────────

fn col(width: Unit, height: Unit) -> UiLayout {
    UiLayout { layout_type: LayoutType::Column, width, height, ..UiLayout::default() }
}
fn px(v: f32) -> Unit {
    Unit::Px(v)
}

fn noop_normal(_q: Query<&ComputedRect>) {}
fn noop_exclusive(_w: &mut EcsMaster) {}

/// A world seeded with the UI resources and a representative tree (root + mixed
/// fixed/stretch/nested children, so all depth pools grow), returning the world
/// and the first non-root child handle (for the size-tweak case).
fn seeded_world() -> (EcsMaster, Entity) {
    let mut world = EcsMaster::new();
    world.insert_resource(UiViewport { width: 400.0, height: 800.0, scale_factor: 1.0, generation: 0 });
    world.insert_resource(LayoutScratch::with_seeds());

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let mut v = probe.lock().expect("probe");
        let root = {
            let mut e = cmds.spawn(col(px(400.0), px(800.0)));
            e.insert(ComputedRect::default());
            e.insert(UiRoot);
            e.id()
        };
        v.push(root);
        let a = {
            let mut e = cmds.spawn(col(px(100.0), px(40.0)));
            e.insert(ComputedRect::default());
            e.set_parent(root);
            e.id()
        };
        v.push(a);
        for _ in 0..2 {
            let s = {
                let mut e = cmds.spawn(col(px(100.0), Unit::Stretch(1.0)));
                e.insert(ComputedRect::default());
                e.set_parent(root);
                e.id()
            };
            v.push(s);
        }
        for _ in 0..2 {
            let g = {
                let mut e = cmds.spawn(col(px(50.0), px(20.0)));
                e.insert(ComputedRect::default());
                e.set_parent(a);
                e.id()
            };
            v.push(g);
        }
    });
    let ids = sink.lock().expect("probe").clone();
    (world, ids[1])
}

fn build_pair_schedule(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let k = b.add_system(ui_layout_discovery).key();
    b.add_system(ui_layout_apply).after(k);
    b.build(world)
}

fn build_baseline_schedule(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let k = b.add_system(noop_normal).key();
    b.add_system(noop_exclusive).after(k);
    b.build(world)
}

/// Worst-of-N armed `run()` allocation count, after warming the schedule.
fn warmed_idle_allocs(world: &mut EcsMaster, sched: &mut Schedule) -> usize {
    for _ in 0..8 {
        sched.run(world);
    }
    (0..4).map(|_| count_allocs(|| sched.run(world))).max().unwrap_or(0)
}

fn set_layout(world: &mut EcsMaster, e: Entity, l: UiLayout) {
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(l);
    });
}

fn resize(world: &mut EcsMaster, w: f32, h: f32) {
    let vp = world.resource_mut::<UiViewport>();
    vp.width = w;
    vp.height = h;
    vp.generation = vp.generation.wrapping_add(1);
}

// ───────────────────────── tests (DELTA == 0) ─────────────────────────────

#[test]
fn unchanged_frame_layout_pair_allocates_zero_over_baseline() {
    let _arm = lock_arm();
    let (mut wb, _) = seeded_world();
    let mut sb = build_baseline_schedule(&mut wb);
    let base = warmed_idle_allocs(&mut wb, &mut sb);

    let (mut wp, _) = seeded_world();
    let mut sp = build_pair_schedule(&mut wp);
    let pair = warmed_idle_allocs(&mut wp, &mut sp);

    assert_eq!(
        pair, base,
        "steady-state: layout pair must allocate no more than the scheduler baseline \
         (baseline {base}, pair {pair}; the layout pair's own per-frame allocs = {})",
        pair as i64 - base as i64
    );
}

#[test]
fn non_structural_size_tweak_layout_pair_allocates_zero_over_baseline() {
    let _arm = lock_arm();
    // Baseline path also gets the tweak (so the command-apply cost cancels out of
    // the DELTA), then measure the relayout frame.
    let (mut wb, ab) = seeded_world();
    let mut sb = build_baseline_schedule(&mut wb);
    for _ in 0..8 {
        sb.run(&mut wb);
    }
    set_layout(&mut wb, ab, col(px(100.0), px(48.0)));
    let base = count_allocs(|| sb.run(&mut wb));

    let (mut wp, ap) = seeded_world();
    let mut sp = build_pair_schedule(&mut wp);
    for _ in 0..8 {
        sp.run(&mut wp);
    }
    set_layout(&mut wp, ap, col(px(100.0), px(48.0)));
    let pair = count_allocs(|| sp.run(&mut wp));

    assert!(
        pair <= base,
        "non-structural size-tweak relayout must add no allocations over baseline \
         (baseline {base}, pair {pair})"
    );
}

#[test]
fn resize_frame_layout_pair_allocates_zero_over_baseline() {
    let _arm = lock_arm();
    // Resize is a direct resource write (no command apply) and is NOT a root-SET
    // change, so apply reuses the cached `roots` list (no per-frame
    // `query_entities`). The DELTA over the (also-resized) baseline must be 0.
    let (mut wb, _) = seeded_world();
    let mut sb = build_baseline_schedule(&mut wb);
    for _ in 0..8 {
        sb.run(&mut wb);
    }
    resize(&mut wb, 440.0, 860.0);
    let base = count_allocs(|| sb.run(&mut wb));

    let (mut wp, _) = seeded_world();
    let mut sp = build_pair_schedule(&mut wp);
    for _ in 0..8 {
        sp.run(&mut wp);
    }
    resize(&mut wp, 440.0, 860.0);
    let pair = count_allocs(|| sp.run(&mut wp));

    assert!(
        pair <= base,
        "resize relayout (same root count) must add no allocations over baseline \
         — cached roots reused, no query_entities (baseline {base}, pair {pair})"
    );
}
