//! Dense plan D4 — per-slot change detection on dense components.
//!
//! Gates (the D4 change-detection contract — mirrors the archetypal Phase-10
//! contract exactly, indexed BY SLOT instead of by row):
//! * A freshly-inserted dense component is visible to `Added<Dense>` exactly the
//!   frame it is added, and NOT thereafter.
//! * A `Mut<Dense>` write (deref) makes the row visible to `Changed<Dense>`; an
//!   untouched dense component is NOT `Changed` on an idle frame.
//! * Deferred-change visibility: a dense component inserted via a Commands-applied
//!   op is visible to `Added`/`Changed` after the apply window (the same contract
//!   the archetypal path keeps).
//! * Per-slot tick correctness across remove/reuse: a slot reused by a fresh
//!   insert re-stamps both ticks, so a stale tick never leaks to the new tenant.
//!
//! All dense components use `#[component(storage = "dense")]`; the query path is
//! exercised through the parallel scheduler (`ScheduleBuilder` / `Schedule::run`)
//! so the `(last_run, this_run]` windows advance frame-to-frame exactly as the
//! archetypal change-detection suite drives them.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Added, Changed, Mut, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;
use boyko_macros::{Bundle, Component};

/// 16-byte POD dense "body" payload (the physics-body shape).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct DBody {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

/// A second dense component, used to prove per-component independence.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct DTag {
    v: u32,
}

/// A plain TABLE component the dense `DBody` rides alongside (so the entity is in
/// a real archetype; the dense column is the global store).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct TXform {
    px: f32,
    py: f32,
}

#[derive(Bundle)]
struct XformBody {
    t: TXform,
    b: DBody,
}

/// Single-dense-component spawn bundle (a bare dense component does not auto-impl
/// `Bundle` — like a bitset tag — so the reuse test wraps it).
#[derive(Bundle)]
struct TagOnly {
    d: DTag,
}

#[inline]
fn body(x: f32) -> DBody {
    DBody { x, y: x + 1.0, z: x + 2.0, w: x + 3.0 }
}

// ════════════════════════════════════════════════════════════════════════════
// Added<Dense>: a freshly-inserted dense component is Added exactly its frame.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn added_dense_matches_only_the_insert_frame() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    // Spawn 3 entities each with a dense `DBody` (+ a table `TXform`) before the
    // schedule starts.
    world.run_system(|mut cmds: Commands| {
        for i in 0..3u32 {
            let x = i as f32;
            cmds.spawn(XformBody { t: TXform { px: x, py: -x }, b: body(x) });
        }
    });

    static ADDED_SEEN: AtomicUsize = AtomicUsize::new(0);
    ADDED_SEEN.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&DBody, Added<DBody>>| {
        for _ in &q {
            ADDED_SEEN.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1: the 3 pre-spawned dense members were Added (their slot added-tick
    // lies in the system's first window).
    schedule.run(&mut world);
    assert_eq!(
        ADDED_SEEN.load(Ordering::Relaxed),
        3,
        "frame 1: Added<DBody> must match every pre-spawned dense member"
    );

    // Frame 2: no inserts between frames — Added<DBody> must yield zero.
    ADDED_SEEN.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        ADDED_SEEN.load(Ordering::Relaxed),
        0,
        "frame 2: no dense inserts → Added<DBody> matches nothing"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Changed<Dense>: a Mut<Dense> deref makes the row Changed; idle frames don't.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn changed_dense_matches_after_mut_deref() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    world.run_system(|mut cmds: Commands| {
        cmds.spawn(XformBody { t: TXform { px: 0.0, py: 0.0 }, b: body(10.0) });
    });

    static MUTATIONS: AtomicUsize = AtomicUsize::new(0);
    static CHANGED_SEEN: AtomicUsize = AtomicUsize::new(0);
    MUTATIONS.store(0, Ordering::Relaxed);
    CHANGED_SEEN.store(0, Ordering::Relaxed);
    let should_mutate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let should_mutate_w = Arc::clone(&should_mutate);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Writer: mutates the dense `DBody` through the `Mut` deref guard when flagged.
    builder.add_system(move |mut q: Query<Mut<DBody>>| {
        if should_mutate_w.load(Ordering::Relaxed) {
            for mut b in &mut q {
                b.x += 1.0; // deref_mut → bumps the dense slot's changed tick
                MUTATIONS.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
    // Reader: counts `Changed<DBody>` rows.
    builder.add_system(|q: Query<&DBody, Changed<DBody>>| {
        for _ in &q {
            CHANGED_SEEN.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1: insert bumped changed_tick = current_tick, so Changed matches once.
    schedule.run(&mut world);
    assert_eq!(
        CHANGED_SEEN.load(Ordering::Relaxed),
        1,
        "frame 1: the insert's changed tick lies in the first window → Changed matches"
    );

    // Frame 2: idle (writer flag off) — Changed must NOT match.
    CHANGED_SEEN.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        CHANGED_SEEN.load(Ordering::Relaxed),
        0,
        "frame 2: untouched dense member is NOT Changed"
    );

    // Frame 3: writer mutates once; the reader (ordered after the writer by the
    // W/R conflict on the dense node) observes the change.
    should_mutate.store(true, Ordering::Relaxed);
    CHANGED_SEEN.store(0, Ordering::Relaxed);
    MUTATIONS.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(MUTATIONS.load(Ordering::Relaxed), 1, "frame 3: writer mutates one row");
    assert_eq!(
        CHANGED_SEEN.load(Ordering::Relaxed),
        1,
        "frame 3: the Mut deref bumped the dense slot's changed tick → Changed matches"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Deferred-change visibility: a dense insert in a Commands-applied op is visible
// to Added AFTER the apply window (the archetypal deferred-change contract).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn deferred_dense_insert_visible_to_added_after_apply_window() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    static SPAWN_FRAME: AtomicUsize = AtomicUsize::new(0);
    static ADDED_SEEN: AtomicUsize = AtomicUsize::new(0);
    SPAWN_FRAME.store(0, Ordering::Relaxed);
    ADDED_SEEN.store(0, Ordering::Relaxed);
    // Frame counter so the spawner runs on exactly frame 1.
    let frame = Arc::new(AtomicUsize::new(0));
    let frame_spawner = Arc::clone(&frame);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Spawner: on frame 1 only, queue a dense spawn via Commands (deferred to the
    // apply window).
    builder.add_system(move |mut cmds: Commands| {
        if frame_spawner.load(Ordering::Relaxed) == 1 {
            cmds.spawn(XformBody { t: TXform { px: 1.0, py: 1.0 }, b: body(5.0) });
        }
    });
    // Reader: counts Added<DBody>.
    builder.add_system(|q: Query<&DBody, Added<DBody>>| {
        for _ in &q {
            ADDED_SEEN.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1: the spawner queues the dense insert; it is applied in the apply
    // window. Per the deferred-change contract (the apply-window tick bump), the
    // Added is observable on the NEXT frame's reader window (the reader on frame 1
    // ran before the apply / may not see the post-apply stamp).
    frame.store(1, Ordering::Relaxed);
    schedule.run(&mut world);

    // Frame 2: the reader observes the deferred dense insert as Added.
    frame.store(2, Ordering::Relaxed);
    ADDED_SEEN.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        ADDED_SEEN.load(Ordering::Relaxed),
        1,
        "the deferred (Commands-applied) dense insert is visible to Added after the apply window"
    );

    // Frame 3: no longer Added.
    frame.store(3, Ordering::Relaxed);
    ADDED_SEEN.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        ADDED_SEEN.load(Ordering::Relaxed),
        0,
        "frame 3: the dense member is no longer Added"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Per-slot tick correctness across remove/reuse: a reused slot re-stamps its
// ticks, so a stale tick never leaks to the new tenant.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn reused_slot_restamps_ticks_no_stale_leak() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    static ADDED_SEEN: AtomicUsize = AtomicUsize::new(0);
    ADDED_SEEN.store(0, Ordering::Relaxed);

    // Spawn two DTag entities, despawn both (tombstones their dense slots), let a
    // frame pass so their original Added window goes stale, then spawn one fresh
    // DTag — it MUST reuse a freed slot and re-stamp the Added tick to the new
    // frame, so it shows up as Added again (a stale tick would NOT match).
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&DTag, Added<DTag>>| {
        for _ in &q {
            ADDED_SEEN.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Seed two DTag members + despawn them (directly, outside the schedule, so the
    // free-list has reusable slots before the reuse spawn).
    let e0 = world.run_system(|mut cmds: Commands| cmds.spawn(TagOnly { d: DTag { v: 1 } }).id());
    let e1 = world.run_system(|mut cmds: Commands| cmds.spawn(TagOnly { d: DTag { v: 2 } }).id());
    world.delete_entity(e0);
    world.delete_entity(e1);

    // Run a couple of idle frames so any prior Added/Changed window is far behind.
    schedule.run(&mut world);
    schedule.run(&mut world);
    ADDED_SEEN.store(0, Ordering::Relaxed);

    // Spawn a fresh DTag — it reuses a freed slot (LIFO). Its insert re-stamps the
    // added tick at the CURRENT tick.
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(TagOnly { d: DTag { v: 99 } });
    });

    schedule.run(&mut world);
    assert_eq!(
        ADDED_SEEN.load(Ordering::Relaxed),
        1,
        "a reused dense slot re-stamps its added tick → the fresh member is Added (no stale-tick leak)"
    );
}
