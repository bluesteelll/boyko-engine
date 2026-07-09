//! Bug #56 regression — deferred-added components are observed by
//! `Added<T>` / `Changed<T>` EXACTLY ONCE, on the frame after the deferred
//! command applied (never the same frame, never twice).
//!
//! # The bug
//!
//! Deferred command applies (`SpawnAt` / `Insert` / migration / `spawn_batch`)
//! stamp a component's `added` / `changed` tick by reading `current_tick()` at
//! apply time — and the apply runs at the schedule's apply-window barrier. Before
//! the fix, `Schedule::run` bumped the change tick exactly once (frame-start), so
//! a deferred apply stamped at the SAME `this_run` every reader in the frame was
//! pinned to. A reader's `Added<T>` window is `(last_run, this_run]`; the next
//! frame's `last_run` equals THIS frame's `this_run`, so a component stamped at
//! `this_run` lands exactly on the EXCLUSIVE lower boundary of the next frame's
//! window and is NEVER observed (`Added` count == 0 — the bug).
//!
//! # The fix
//!
//! `Schedule::run` now bumps the change tick a SECOND time at the apply-window
//! barrier (after systems/conditions/state captured the frame-start `this_run`,
//! before any deferred drain — `schedule.rs` ~266-282). Deferred applies now
//! stamp at `this_run + 1`, strictly between this frame's reader window
//! (`…, this_run]`) and the next frame's window (`(this_run, this_run + 2]`).
//! The deferred-added component therefore lands inside the NEXT frame's window
//! exactly once and is gone by the frame after (Bevy's ApplyDeferred sync-point
//! analogue).
//!
//! # Observation model & harness
//!
//! `Added<T>` / `Changed<T>` advance their `last_run` only inside `Schedule::run`,
//! so every case drives the deferred op AND its observation through a multi-frame
//! schedule (the model of `phase10_change_detection.rs` /
//! `phase14b_insert_migration_correctness.rs`). Per-frame match counts are
//! smuggled out of the `Send + Sync` system closures via module-level `static`
//! probes guarded by a process-wide mutex. Each test resets its probes under the
//! lock. Component ids are minted lazily from the global atomic counter (disjoint
//! across the binary).
//!
//! Pools are pinned to ONE worker so the spawner-before-reader ordering is the
//! linearised schedule (the reader is ordered AFTER the spawner via an
//! intra-archetype/`.after` edge where it matters); the deferred apply for a
//! frame happens at that frame's apply-window regardless of worker count.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Added, Changed, Mut, Query};
use boyko_ecs::ecs::core::schedule::{ScheduleBuilder, run_once};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

const REL: Ordering = Ordering::Relaxed;

/// Serializes tests sharing module-level probe `static`s across frames.
static TEST_MUTEX: Mutex<()> = Mutex::new(());

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

// ════════════════════════════════════════════════════════════════════════════
// Case 1 — deferred spawn: Added<C> seen EXACTLY ONCE, the frame after apply.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct SpawnAddedC {
    v: u32,
}

#[derive(Bundle)]
struct SpawnAddedBundle {
    c: SpawnAddedC,
}

/// Per-frame `Added<SpawnAddedC>` match counts, indexed by frame number (1-based
/// pushes). Guarded by `TEST_MUTEX`.
static C1_PER_FRAME_HITS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
/// Frame-1-only spawn gate (the spawner closure runs every frame; it spawns only
/// when this is set, which the harness clears after frame 1).
static C1_DO_SPAWN: AtomicBool = AtomicBool::new(false);
/// Scratch counter the reader accumulates into within a single frame.
static C1_FRAME_HITS: AtomicUsize = AtomicUsize::new(0);

/// A "spawner" deferred-spawns one `SpawnAddedC` on frame 1 ONLY; a "reader"
/// counts `Added<SpawnAddedC>` each frame. Over 4 frames the deferred-spawned
/// component must be observed EXACTLY ONCE, on frame 2 (the frame after the
/// frame-1 apply-window), and NEVER on frame 3/4. Pre-fix this count was 0.
#[test]
fn deferred_spawn_added_seen_exactly_once() {
    let _guard = TEST_MUTEX.lock().expect("test mutex");
    C1_PER_FRAME_HITS.lock().expect("probe").clear();
    C1_DO_SPAWN.store(false, REL);
    C1_FRAME_HITS.store(0, REL);

    let pool = serial_pool();
    let mut world = EcsMaster::new();
    // Pre-register so the spawn bundle's archetype is resolvable lazily.
    let _ = SpawnAddedC::component_id();

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Spawner: deferred-spawn one entity on the armed frame only.
    let spawner = builder
        .add_system(move |mut cmds: Commands| {
            if C1_DO_SPAWN.load(REL) {
                cmds.spawn(SpawnAddedBundle { c: SpawnAddedC { v: 42 } });
            }
        })
        .key();
    // Reader: count Added<C> rows this frame. Ordered AFTER the spawner so that
    // within a single frame the reader cannot observe before the spawner queued
    // (the apply happens at the apply-window regardless; the edge only fixes the
    // intra-frame order for determinism).
    builder
        .add_system(|q: Query<&SpawnAddedC, Added<SpawnAddedC>>| {
            for _ in &q {
                C1_FRAME_HITS.fetch_add(1, REL);
            }
        })
        .after(spawner);
    let mut schedule = builder.build(&mut world);

    // Drive 4 frames; frame 1 arms the spawn, the rest do not.
    for frame in 1..=4 {
        C1_FRAME_HITS.store(0, REL);
        C1_DO_SPAWN.store(frame == 1, REL);
        schedule.run(&mut world);
        C1_PER_FRAME_HITS
            .lock()
            .expect("probe")
            .push(C1_FRAME_HITS.load(REL));
    }

    let hits = C1_PER_FRAME_HITS.lock().expect("probe").clone();
    assert_eq!(hits.len(), 4, "exactly 4 frames recorded");
    // Frame 1: the entity is spawned at frame 1's apply-window, stamped at
    // this_run+1 — strictly ABOVE frame 1's reader window, so NOT seen in frame 1.
    assert_eq!(
        hits[0], 0,
        "frame 1: the deferred-spawned C is stamped at apply-window (this_run+1), \
         above frame 1's reader window — not seen the same frame. Got {hits:?}"
    );
    // Frame 2: frame 2's window is (this_run_1, this_run_1+2], which CONTAINS the
    // this_run_1+1 stamp — seen exactly once.
    assert_eq!(
        hits[1], 1,
        "Bug#56: frame 2 (the frame AFTER the deferred apply) must observe Added<C> exactly once. \
         Got {hits:?} (pre-fix this slot was 0 — the bug)"
    );
    // Frames 3/4: the stamp is below the window's exclusive lower bound — gone.
    assert_eq!(
        hits[2], 0,
        "frame 3: Added<C> must NOT re-fire. Got {hits:?}"
    );
    assert_eq!(
        hits[3], 0,
        "frame 4: Added<C> must NOT re-fire. Got {hits:?}"
    );

    let total: usize = hits.iter().sum();
    assert_eq!(
        total, 1,
        "the deferred-spawned C is observed by Added<C> EXACTLY once across all frames. Got {hits:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case 2 — deferred insert onto an existing entity: Added<C2>/Changed<C2> seen
//   exactly once, the frame after apply. (The freshly-added component fires
//   BOTH Added and Changed; we pin Added — the discriminating "exactly once".)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct InsBase {
    a: u32,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct InsAdded {
    b: u32,
}

#[derive(Bundle)]
struct InsAddedBundle {
    c: InsAdded,
}

static C2_PER_FRAME_ADDED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static C2_DO_INSERT: AtomicBool = AtomicBool::new(false);
static C2_FRAME_ADDED: AtomicUsize = AtomicUsize::new(0);

/// An entity has `{InsBase}` before the schedule. A "mutator" deferred-inserts
/// `InsAdded` onto it on frame 1 ONLY (an insert-migration, since InsAdded is new
/// to the archetype). A reader counts `Added<InsAdded>` each frame. The fresh
/// component must be observed exactly once, on frame 2.
#[test]
fn deferred_insert_added_seen_exactly_once() {
    let _guard = TEST_MUTEX.lock().expect("test mutex");
    C2_PER_FRAME_ADDED.lock().expect("probe").clear();
    C2_DO_INSERT.store(false, REL);
    C2_FRAME_ADDED.store(0, REL);

    let pool = serial_pool();
    let mut world = EcsMaster::new();

    let base_arch = world.create_archetype(&[InsBase::component_id()]);
    let entity = world
        .spawn_one(base_arch, InsBase { a: 1 })
        .expect("spawn {InsBase}");
    let _ = InsAdded::component_id();

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let mutator = builder
        .add_system(move |mut cmds: Commands| {
            if C2_DO_INSERT.load(REL) {
                cmds.entity(entity).insert(InsAddedBundle { c: InsAdded { b: 9 } });
            }
        })
        .key();
    builder
        .add_system(|q: Query<&InsAdded, Added<InsAdded>>| {
            for _ in &q {
                C2_FRAME_ADDED.fetch_add(1, REL);
            }
        })
        .after(mutator);
    let mut schedule = builder.build(&mut world);

    for frame in 1..=4 {
        C2_FRAME_ADDED.store(0, REL);
        C2_DO_INSERT.store(frame == 1, REL);
        schedule.run(&mut world);
        C2_PER_FRAME_ADDED
            .lock()
            .expect("probe")
            .push(C2_FRAME_ADDED.load(REL));
    }

    let hits = C2_PER_FRAME_ADDED.lock().expect("probe").clone();
    assert!(
        world.has_component(entity, InsAdded::component_id()),
        "the deferred insert migrated the entity (InsAdded now present)"
    );
    assert_eq!(
        hits,
        vec![0, 1, 0, 0],
        "Bug#56: the deferred-inserted InsAdded is observed by Added<InsAdded> exactly once, \
         on frame 2 (the frame AFTER apply). Got {hits:?} (pre-fix frame-2 slot was 0)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case 3 — same-frame DIRECT write through Mut<C> stays visible to a LATER
//   system reading Changed<C> in the SAME frame. (Guards against the rejected
//   "shift the frame-start bump" alternative: a direct write must remain visible
//   intra-frame.)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct DirectC {
    v: u32,
}

static C3_SAME_FRAME_SEEN: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static C3_FRAME_SEEN: AtomicUsize = AtomicUsize::new(0);

/// A writer mutates `DirectC` via `Mut<C>` (a DIRECT through-query write, NOT a
/// deferred command) every frame; a LATER-ordered reader (via `.after`) reads
/// `Changed<DirectC>` in the SAME frame. The change MUST be visible in that same
/// frame, every frame — the apply-window bump must not have pushed the
/// direct-write stamp out of the same-frame reader window.
#[test]
fn same_frame_direct_change_still_visible() {
    let _guard = TEST_MUTEX.lock().expect("test mutex");
    C3_SAME_FRAME_SEEN.lock().expect("probe").clear();
    C3_FRAME_SEEN.store(0, REL);

    let pool = serial_pool();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[DirectC::component_id()]);
    world.spawn_one(arch, DirectC { v: 0 }).expect("spawn {DirectC}");

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let writer = builder
        .add_system(|mut q: Query<Mut<DirectC>>| {
            for mut c in &mut q {
                c.v = c.v.wrapping_add(1);
            }
        })
        .key();
    builder
        .add_system(|q: Query<&DirectC, Changed<DirectC>>| {
            for _ in &q {
                C3_FRAME_SEEN.fetch_add(1, REL);
            }
        })
        .after(writer);
    let mut schedule = builder.build(&mut world);

    for _ in 0..3 {
        C3_FRAME_SEEN.store(0, REL);
        schedule.run(&mut world);
        C3_SAME_FRAME_SEEN
            .lock()
            .expect("probe")
            .push(C3_FRAME_SEEN.load(REL));
    }

    let seen = C3_SAME_FRAME_SEEN.lock().expect("probe").clone();
    assert_eq!(
        seen,
        vec![1, 1, 1],
        "a DIRECT Mut<C> write must be observed by a same-frame, later-ordered Changed<C> reader \
         EVERY frame — the apply-window bump must not evict the intra-frame direct-write stamp. \
         Got {seen:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case 4 — deferred spawn_batch: every batched entity observed by Added<C>
//   exactly once, the frame after apply.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct BatchC {
    i: u32,
}

#[derive(Bundle)]
struct BatchBundle {
    c: BatchC,
}

const BATCH_N: usize = 64;

static C4_PER_FRAME_HITS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static C4_DO_BATCH: AtomicBool = AtomicBool::new(false);
static C4_FRAME_HITS: AtomicUsize = AtomicUsize::new(0);

/// A spawner `spawn_batch`es `BATCH_N` entities with `BatchC` on frame 1 only; a
/// reader counts `Added<BatchC>` each frame. The whole batch must be observed
/// exactly once, on frame 2, with the FULL count (every row), and never again.
#[test]
fn deferred_spawn_batch_added_once() {
    let _guard = TEST_MUTEX.lock().expect("test mutex");
    C4_PER_FRAME_HITS.lock().expect("probe").clear();
    C4_DO_BATCH.store(false, REL);
    C4_FRAME_HITS.store(0, REL);

    let pool = serial_pool();
    let mut world = EcsMaster::new();
    let _ = BatchC::component_id();

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let spawner = builder
        .add_system(move |mut cmds: Commands| {
            if C4_DO_BATCH.load(REL) {
                cmds.spawn_batch((0..BATCH_N as u32).map(|i| BatchBundle { c: BatchC { i } }))
                    .expect("batch within MAX_BATCH_HINT")
                    .for_each(drop);
            }
        })
        .key();
    builder
        .add_system(|q: Query<&BatchC, Added<BatchC>>| {
            for _ in &q {
                C4_FRAME_HITS.fetch_add(1, REL);
            }
        })
        .after(spawner);
    let mut schedule = builder.build(&mut world);

    for frame in 1..=4 {
        C4_FRAME_HITS.store(0, REL);
        C4_DO_BATCH.store(frame == 1, REL);
        schedule.run(&mut world);
        C4_PER_FRAME_HITS
            .lock()
            .expect("probe")
            .push(C4_FRAME_HITS.load(REL));
    }

    let hits = C4_PER_FRAME_HITS.lock().expect("probe").clone();
    assert_eq!(
        hits,
        vec![0, BATCH_N, 0, 0],
        "Bug#56: all {BATCH_N} batched entities are observed by Added<BatchC> exactly once, \
         on frame 2, then never again. Got {hits:?}"
    );
    assert_eq!(
        world.entity_count(),
        BATCH_N,
        "all batched entities are live"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case 5 — a `Changed<C>` run condition fires the frame AFTER a deferred insert
//   of C, not the same frame. (Run-condition tick-window analogue of case 2.)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct CondBase {
    a: u32,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct CondAdded {
    b: u32,
}

#[derive(Bundle)]
struct CondAddedBundle {
    c: CondAdded,
}

static C5_PER_FRAME_RAN: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static C5_DO_INSERT: AtomicBool = AtomicBool::new(false);
static C5_FRAME_RAN: AtomicUsize = AtomicUsize::new(0);

/// A gated system runs only when its `Added<CondAdded>` run condition is true.
/// A separate mutator deferred-inserts `CondAdded` on frame 1 ONLY (gated via
/// `run_once`). The gated body must run exactly once, on frame 2 (the frame after
/// the deferred apply makes `Added<CondAdded>` true), and never on frame 3/4.
///
/// This exercises the Phase-16.1 condition-tick path together with the Bug#56
/// apply-window bump: a tick-based run condition must observe the deferred-added
/// component on the next frame, not prematurely (frame F) and not late.
#[test]
fn condition_added_not_premature() {
    let _guard = TEST_MUTEX.lock().expect("test mutex");
    C5_PER_FRAME_RAN.lock().expect("probe").clear();
    C5_DO_INSERT.store(false, REL);
    C5_FRAME_RAN.store(0, REL);

    let pool = serial_pool();
    let mut world = EcsMaster::new();

    let base_arch = world.create_archetype(&[CondBase::component_id()]);
    let entity = world
        .spawn_one(base_arch, CondBase { a: 1 })
        .expect("spawn {CondBase}");
    let _ = CondAdded::component_id();

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Mutator: deferred-insert CondAdded on frame 1 only (run_once gate).
    let mutator = builder
        .add_system(move |mut cmds: Commands| {
            cmds.entity(entity).insert(CondAddedBundle { c: CondAdded { b: 3 } });
        })
        .run_if(run_once)
        .key();
    // Gated system: runs only when Added<CondAdded> fires. Ordered after the
    // mutator for intra-frame determinism.
    builder
        .add_system(|| {
            C5_FRAME_RAN.fetch_add(1, REL);
        })
        .after(mutator)
        .run_if(|q: Query<&CondAdded, Added<CondAdded>>| q.iter().next().is_some());
    let mut schedule = builder.build(&mut world);

    for _ in 1..=4 {
        C5_FRAME_RAN.store(0, REL);
        schedule.run(&mut world);
        C5_PER_FRAME_RAN
            .lock()
            .expect("probe")
            .push(C5_FRAME_RAN.load(REL));
    }

    let ran = C5_PER_FRAME_RAN.lock().expect("probe").clone();
    assert!(
        world.has_component(entity, CondAdded::component_id()),
        "the deferred insert migrated CondAdded onto the entity"
    );
    assert_eq!(
        ran,
        vec![0, 1, 0, 0],
        "Bug#56 + Phase16.1: an Added<CondAdded> run condition fires the gated body exactly once, \
         on frame 2 (frame AFTER the deferred insert applied), not the same frame and not late. \
         Got {ran:?}"
    );
}
