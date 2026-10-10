//! GATE 3 (no_alloc) — the data-bind pair allocates NOTHING of its own per
//! frame in steady state, over many bound nodes, via a counting global allocator
//! and BASELINE SUBTRACTION (the same methodology as the P1 `zero_alloc.rs`).
//!
//! `Schedule::run`'s parallel executor allocates a small FIXED number of bytes
//! per frame for its own task machinery, independent of the systems' bodies. So
//! the test measures the bind pair's OWN per-frame allocations as the DELTA
//! between two schedules of identical SHAPE (one normal + one exclusive system)
//! over the same warmed bound tree:
//!   * baseline = `[noop_normal, noop_exclusive]`
//!   * pair     = `[ui_bind_discovery, ui_bind_apply]`
//!
//! A DELTA of 0 proves the bind pair allocates nothing of its own per frame.
//!
//! Two steady-state cases at DELTA 0:
//!   (a) a STILL frame (no source changed — discovery returns false, apply
//!       early-returns): the 0%-gate path must not allocate;
//!   (b) a DIRTY frame (every source changed → every widget re-formats): the
//!       apply path's retained scratch buffers (widget query, arch-id scratch,
//!       stack-buffer format) must allocate nothing once warmed.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_macros::{Bindable, Component};

use boyko_ui::binding::bind_system::{ui_bind_apply, ui_bind_discovery, UiBindScratch};
use boyko_ui::binding::components::{BindText, TemplateId, UiTextBuffer, NO_FIELD};
use boyko_ui::binding::Bindable;

// ───────────────────────── armed-window serialization ──────────────────────

static ARM_LOCK: Mutex<()> = Mutex::new(());

fn lock_arm() -> std::sync::MutexGuard<'static, ()> {
    ARM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// ───────────────────────── counting allocator ──────────────────────────────

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

/// Runs `f` with the counter armed, returning the alloc count.
fn count_allocs(f: impl FnOnce()) -> usize {
    ALLOCS.store(0, Ordering::Relaxed);
    ARMED.store(true, Ordering::Relaxed);
    f();
    ARMED.store(false, Ordering::Relaxed);
    ALLOCS.load(Ordering::Relaxed)
}

// ───────────────────────── bindable source ─────────────────────────────────

#[derive(Component, Bindable, Clone, Copy, Debug)]
#[repr(C)]
struct Health {
    current: f32,
    max: f32,
}

// ───────────────────────── noop baseline systems ───────────────────────────

use boyko_ecs::ecs::core::iters::query::query::Query;

use boyko_macros::Resource;

/// In-schedule mutation queue (the realistic gameplay-system mutate path; an
/// out-of-band `run_system` mutate collides with the schedule's `last_run`, a
/// harness artifact — see p4_bind.rs).
#[derive(Resource, Default)]
struct MutQueue {
    pending: Vec<(Entity, Health)>,
}

/// In-schedule mutator drained ahead of the bind/baseline systems.
#[allow(clippy::needless_pass_by_ref_mut)]
fn mutator_system(world: &mut EcsMaster) {
    let pending = std::mem::take(&mut world.resource_mut::<MutQueue>().pending);
    for (e, h) in pending {
        if let Some(mut g) = world.get_component_mut::<Health>(e) {
            *g = h;
        }
    }
}

/// Baseline normal system (same shape slot as `ui_bind_discovery`'s scheduling
/// role: a scheduled FunctionSystem). Touches a trivial query so it is a real
/// system, not elided.
fn noop_normal(_q: Query<&UiTextBuffer>) {}

/// Baseline exclusive system (same shape slot as `ui_bind_apply`).
#[allow(clippy::needless_pass_by_ref_mut)]
fn noop_exclusive(_world: &mut EcsMaster) {}

// ───────────────────────── world builders ──────────────────────────────────

const N_WIDGETS: usize = 256;

/// Builds a world with `N_WIDGETS` `Health` sources each driving a `BindText`
/// widget, then warms it. Returns the world + sources (for the dirty-frame case).
/// Both schedule shapes carry the SAME `mutator_system` ahead of the pair so the
/// mutation churn cancels out of the alloc delta.
fn build_bound_world(schedule_pair: bool) -> (EcsMaster, Schedule, Vec<Entity>) {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let mut scratch = UiBindScratch::default();
    Health::register_bind_accessor();
    scratch.register_bound_id(Health::component_id());
    world.insert_resource(scratch);
    world.insert_resource(MutQueue::default());

    let mut builder = ScheduleBuilder::new(pool);
    let mutate = builder.add_system(mutator_system).key();
    if schedule_pair {
        let discovery = builder.add_system(ui_bind_discovery).after(mutate).key();
        builder.add_system(ui_bind_apply).after(discovery);
    } else {
        let n = builder.add_system(noop_normal).after(mutate).key();
        builder.add_system(noop_exclusive).after(n);
    }
    let schedule = builder.build(&mut world);

    // Spawn sources + widgets.
    let sources: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let sp = Arc::clone(&sources);
    let comp = Health::component_id();
    world.run_system(move |mut cmds: Commands| {
        let mut v = sp.lock().unwrap();
        for i in 0..N_WIDGETS {
            let src = cmds.spawn(Health { current: i as f32, max: 100.0 }).id();
            let mut ec = cmds.spawn(BindText {
                source: src,
                comp,
                field: 0,
                field2: NO_FIELD,
                template: TemplateId::Value,
            });
            ec.insert(UiTextBuffer::default());
            v.push(src);
        }
    });
    let srcs = sources.lock().unwrap().clone();

    (world, schedule, srcs)
}

/// Enqueues a bump of every source so the next `schedule.run` is dirty (the
/// in-schedule mutator applies them, every widget reformats). The enqueue is a
/// `Vec::extend` into the retained `MutQueue` — its own steady-state allocation
/// is part of BOTH paths (pair + baseline both run `mutator_system`), so it
/// cancels out of the delta.
fn touch_all(world: &mut EcsMaster, srcs: &[Entity]) {
    let q = &mut world.resource_mut::<MutQueue>().pending;
    for (i, &s) in srcs.iter().enumerate() {
        q.push((s, Health { current: (i + 1) as f32, max: 100.0 }));
    }
}

// ───────────────────────── tests ───────────────────────────────────────────

#[test]
fn bind_still_frame_zero_own_allocations() {
    let _g = lock_arm();

    // Pair path: warm to high-water (touch once so every sink formats + the
    // scratch buffers grow to steady capacity, then settle on still frames).
    let (mut pw, mut ps, psrcs) = build_bound_world(true);
    touch_all(&mut pw, &psrcs);
    for _ in 0..6 {
        ps.run(&mut pw);
    }
    // Now still frames: no source changed → discovery false, apply early-returns.
    let pair = count_allocs(|| {
        for _ in 0..8 {
            ps.run(&mut pw);
        }
    });

    // Baseline path: identical shape + warm (same one-time touch).
    let (mut bw, mut bs, bsrcs) = build_bound_world(false);
    touch_all(&mut bw, &bsrcs);
    for _ in 0..6 {
        bs.run(&mut bw);
    }
    let base = count_allocs(|| {
        for _ in 0..8 {
            bs.run(&mut bw);
        }
    });

    assert!(
        pair <= base,
        "still-frame bind pair must not allocate beyond the executor baseline \
         (pair={pair}, baseline={base}, delta={})",
        pair as isize - base as isize
    );
}

#[test]
fn bind_dirty_frame_zero_own_allocations_steady_state() {
    let _g = lock_arm();

    // Pair path warmed to high-water with repeated dirty frames so every retained
    // scratch buffer reaches steady capacity.
    let (mut pw, mut ps, psrcs) = build_bound_world(true);
    for _ in 0..6 {
        touch_all(&mut pw, &psrcs);
        ps.run(&mut pw);
    }
    let pair = count_allocs(|| {
        for _ in 0..8 {
            touch_all(&mut pw, &psrcs);
            ps.run(&mut pw);
        }
    });

    // Baseline path: identical shape + the SAME per-frame touch_all source bumps
    // (so the command-queue churn cancels out of the delta).
    let (mut bw, mut bs, bsrcs) = build_bound_world(false);
    for _ in 0..6 {
        touch_all(&mut bw, &bsrcs);
        bs.run(&mut bw);
    }
    let base = count_allocs(|| {
        for _ in 0..8 {
            touch_all(&mut bw, &bsrcs);
            bs.run(&mut bw);
        }
    });

    assert!(
        pair <= base,
        "dirty-frame bind pair (over {N_WIDGETS} widgets) must allocate nothing of \
         its own beyond the baseline (pair={pair}, baseline={base}, delta={})",
        pair as isize - base as isize
    );
}
