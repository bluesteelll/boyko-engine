//! Phase 10 Wave E Step 15 — end-to-end integration tests for
//! `Added<T>` / `Changed<T>` filters, `Ref<T>` / `Mut<T>` data, and
//! multi-frame change detection lifecycles.
//!
//! See `docs/PHASE-10-CHANGE-DETECTION-PLAN.md` §13.2 for the full plan.
//!
//! # Component id reservation
//!
//! Phase 10 Wave E tests claim ids **380..=410** (per orchestrator brief).
//! Each test uses a UNIQUE per-test set so the global component-id registry
//! never collides across tests regardless of execution order.
//!
//! # Closure tick-state shared probes
//!
//! Closures passed to `EcsMaster::run_closure_once` / `ScheduleBuilder::add_system`
//! must be `Send + Sync + 'static`. We smuggle observations out of the
//! closure via `Arc<AtomicU32>` / `Arc<AtomicUsize>` probes (the same
//! pattern as `tests/query_dsl_smoke.rs`).
//!
//! # Test isolation against shared mutable state
//!
//! Tests that touch process-global state (e.g. `static` probes between
//! frames) acquire a `TEST_MUTEX` to avoid interleaving with other tests
//! that touch the same probe. Tests using local probes do not need the
//! mutex.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::filter::Or;
use boyko_ecs::ecs::core::iters::query::{Added, Changed, Mut, Query, Ref, With};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_threadpool::ThreadPoolBuilder;
use boyko_macros::Component;

/// Process-wide mutex for tests that touch shared probe state across
/// schedule frames. Tests that own their probes do NOT need this.
static TEST_MUTEX: Mutex<()> = Mutex::new(());

// ── Component types (slot range 380..=410) ─────────────────────────────────

#[derive(Component)]
#[repr(C)]
struct CdPos380 {
    x: f32,
    y: f32,
}

#[derive(Component)]
#[repr(C)]
struct CdHealth382 {
    hp: u32,
}

#[derive(Component, PartialEq)]
#[repr(C)]
struct CdEqProbe386 {
    v: u32,
}

#[derive(Component)]
#[repr(C)]
struct CdBypass387 {
    raw: u32,
}

#[derive(Component)]
#[repr(C)]
struct CdLifecycle388 {
    counter: u32,
}

#[derive(Component)]
#[repr(C)]
struct CdParA389 {
    a: u32,
}

#[derive(Component)]
#[repr(C)]
struct CdParB390 {
    b: u32,
}

#[derive(Component)]
#[repr(C)]
struct CdRefIntro391 {
    n: u32,
}

#[derive(Component)]
#[repr(C)]
struct CdOrNullA392 {
    a: u32,
}

#[derive(Component)]
#[repr(C)]
struct CdOrNullB393 {
    b: u32,
}

// ── Test 1: Added<T> matches new spawns; not on subsequent frames ──────────

/// Plan §13.2 `added_filter_basic_spawn_query`:
/// spawn entity with component, run a system with `Query<&T, Added<T>>`
/// the same frame — it MUST match. Run again next frame — `Added<T>` MUST
/// NOT match anymore (the entity is no longer "newly added" relative to
/// the system's `last_run`).
#[test]
fn added_filter_basic_spawn_query() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    // Spawn an entity BEFORE the schedule starts running.
    let arch = world.create_archetype(&[CdPos380::component_id()]);
    world
        .spawn_one(arch, CdPos380 { x: 1.0, y: 2.0 })
        .expect("spawn");

    // Shared probe for "how many Added<CdPos380> rows did this frame see?"
    static MATCHES: AtomicUsize = AtomicUsize::new(0);
    MATCHES.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&CdPos380, Added<CdPos380>>| {
        for _ in &q {
            MATCHES.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1 — Added matches the row that pre-exists; `last_run` is
    // `current - MAX_CHANGE_AGE`, so any row's `added` tick (which is
    // `Tick(0)` at world creation) is in `(last_run, this_run]`.
    schedule.run(&mut world);
    assert_eq!(
        MATCHES.load(Ordering::Relaxed),
        1,
        "frame 1: Added<CdPos380> must match the pre-existing entity (the row's added tick lies in the system's window)"
    );

    // Frame 2 — same schedule reruns. The system's `last_run` is now the
    // previous frame's `this_run`; nothing was added between frames, so
    // Added<CdPos380> MUST NOT match.
    MATCHES.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        MATCHES.load(Ordering::Relaxed),
        0,
        "frame 2: no spawns between runs, Added<CdPos380> must yield zero rows"
    );
}

// ── Test 2: Changed<T> matches after Mut::deref_mut ─────────────────────────

/// Plan §13.2 `changed_filter_after_mutation`:
/// frame 1 - no mutation; `Changed<T>` matches initially (insert bumps
/// the tick). After running the writer system in frame 2 (`Mut<T>::deref_mut`),
/// the `Changed<T>` reader system in frame 3 MUST observe the change.
#[test]
fn changed_filter_after_mutation() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[CdHealth382::component_id()]);
    world
        .spawn_one(arch, CdHealth382 { hp: 100 })
        .expect("spawn");

    static MUTATIONS: AtomicU32 = AtomicU32::new(0);
    static CHANGED_SEEN: AtomicU32 = AtomicU32::new(0);
    MUTATIONS.store(0, Ordering::Relaxed);
    CHANGED_SEEN.store(0, Ordering::Relaxed);
    let should_mutate = Arc::new(AtomicBool::new(false));
    let should_mutate_w = Arc::clone(&should_mutate);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Writer system: bumps tick when `should_mutate` flag set.
    builder.add_system(move |mut q: Query<Mut<CdHealth382>>| {
        if should_mutate_w.load(Ordering::Relaxed) {
            for mut h in &mut q {
                h.hp = h.hp.wrapping_sub(1);
                MUTATIONS.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
    // Reader system: counts rows matching `Changed<CdHealth382>`.
    builder.add_system(|q: Query<&CdHealth382, Changed<CdHealth382>>| {
        for _ in &q {
            CHANGED_SEEN.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1 — entity is freshly inserted; insert bumped `changed_tick =
    // current_tick`. The Changed filter SHOULD match on the first frame
    // (the row's change tick equals the frame's `this_run`).
    schedule.run(&mut world);
    let f1_changed = CHANGED_SEEN.load(Ordering::Relaxed);
    assert_eq!(
        f1_changed, 1,
        "frame 1: insert-tick lies in the system's first window, so Changed must match the freshly-spawned row"
    );

    // Frame 2 — no mutation, no new inserts. Changed must NOT match.
    CHANGED_SEEN.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        CHANGED_SEEN.load(Ordering::Relaxed),
        0,
        "frame 2: idle frame, Changed must yield zero rows"
    );

    // Frame 3 — writer runs once with mutate flag set. Within the SAME
    // frame, the reader system runs AFTER the writer (DAG ordering via
    // intra-archetype write/read conflict), so it observes the change.
    should_mutate.store(true, Ordering::Relaxed);
    CHANGED_SEEN.store(0, Ordering::Relaxed);
    MUTATIONS.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        MUTATIONS.load(Ordering::Relaxed),
        1,
        "frame 3: writer must mutate exactly one row"
    );
    assert_eq!(
        CHANGED_SEEN.load(Ordering::Relaxed),
        1,
        "frame 3: reader runs after writer (W/R conflict orders them); the row's changed_tick lies in (last_run, this_run]"
    );
}

// ── Test 3: Or<(Added<A>, Changed<B>)> ──────────────────────────────────────

/// Plan §13.2 `or_added_changed_composition`:
/// `Or<(Added<A>, Changed<B>)>` matches when either filter fires.
#[test]
// The `Query<&A, Or<(Added<A>, Changed<B>)>>` closure param is the query DSL
// type under test; an alias would not aid readability (it relies on
// SystemParam lifetime elision at the closure-arg position).
#[allow(clippy::type_complexity)]
fn or_filter_added_or_changed() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    // Single archetype carrying BOTH components so both filters share the
    // same row population.
    let arch = world.create_archetype(&[
        CdParA389::component_id(),
        CdParB390::component_id(),
    ]);
    world
        .spawn_two(arch, CdParA389 { a: 1 }, CdParB390 { b: 2 })
        .expect("spawn 1");
    world
        .spawn_two(arch, CdParA389 { a: 3 }, CdParB390 { b: 4 })
        .expect("spawn 2");

    static OR_MATCHES: AtomicUsize = AtomicUsize::new(0);
    OR_MATCHES.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&CdParA389, Or<(Added<CdParA389>, Changed<CdParB390>)>>| {
        for _ in &q {
            OR_MATCHES.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1: both `Added<A>` and `Changed<B>` fire because both ticks
    // were just bumped to `this_run` at insert time. Or<>'s expected
    // result is 2 (both rows matched).
    schedule.run(&mut world);
    assert_eq!(
        OR_MATCHES.load(Ordering::Relaxed),
        2,
        "frame 1: insert bumps both added & changed ticks; Or matches all 2 rows"
    );

    // Frame 2: no inserts, no writes. Neither branch fires → 0 matches.
    OR_MATCHES.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        OR_MATCHES.load(Ordering::Relaxed),
        0,
        "frame 2: idle, Or yields zero"
    );
}

// ── Test 4: Or<> with archetypal null-base path (Round 2 C4) ────────────────

/// Plan §13.2 `or_with_changed_archetype_lacking_c_no_panic` (Round 2 C4):
/// `Or<(With<A>, Changed<B>)>` runs on an archetype that contains A but
/// lacks B. The `Changed<B>::set_table_*` writes `tick_base = null`; the
/// `Or` evaluator MUST handle that gracefully — `With<A>` returns true
/// (archetypal), short-circuiting the OR to true; the null-base branch
/// in `Changed<B>::filter_fetch` is the safety net.
#[test]
#[allow(clippy::type_complexity)] // query DSL type under test; see note on or_filter_added_or_changed
fn or_filter_with_archetypal_null_base() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    // Archetype with only A (no B).
    let arch_a = world.create_archetype(&[CdOrNullA392::component_id()]);
    world.spawn_one(arch_a, CdOrNullA392 { a: 1 }).unwrap();
    world.spawn_one(arch_a, CdOrNullA392 { a: 2 }).unwrap();

    static OR_MATCHES: AtomicUsize = AtomicUsize::new(0);
    OR_MATCHES.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // `Or<(With<A>, Changed<B>)>` selects: any row in an archetype that
    // either contains A (always true here) OR has a freshly-changed B.
    // For an A-only archetype, B's tick column is absent — the Changed<B>
    // null-base safety branch returns false; `With<A>` is true; OR is true.
    builder.add_system(|q: Query<&CdOrNullA392, Or<(With<CdOrNullA392>, Changed<CdOrNullB393>)>>| {
        for _ in &q {
            OR_MATCHES.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(
        OR_MATCHES.load(Ordering::Relaxed),
        2,
        "Or<(With<A>, Changed<B>)>: A-only archetype matches via With branch (null-base for Changed<B>)"
    );
}

// ── Test 5: set_if_neq preserves tick on equal write ────────────────────────

/// Plan §13.2 `changed_filter_set_if_neq_no_bump`:
/// `Mut<T>::set_if_neq` with EQUAL value MUST NOT bump the changed tick;
/// downstream `Changed<T>` MUST NOT match next frame.
#[test]
fn mut_set_if_neq_preserves_tick() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[CdEqProbe386::component_id()]);
    world.spawn_one(arch, CdEqProbe386 { v: 42 }).unwrap();

    static SAW_CHANGE: AtomicUsize = AtomicUsize::new(0);
    static SETIF_RETURNED_TRUE: AtomicUsize = AtomicUsize::new(0);
    SAW_CHANGE.store(0, Ordering::Relaxed);
    SETIF_RETURNED_TRUE.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Writer: assigns the SAME value via set_if_neq.
    builder.add_system(|mut q: Query<Mut<CdEqProbe386>>| {
        for mut e in &mut q {
            // Equal value — must NOT bump.
            if e.set_if_neq(CdEqProbe386 { v: 42 }) {
                SETIF_RETURNED_TRUE.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
    // Reader: counts Changed.
    builder.add_system(|q: Query<&CdEqProbe386, Changed<CdEqProbe386>>| {
        for _ in &q {
            SAW_CHANGE.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1: insert tick bumps both columns; Changed fires.
    schedule.run(&mut world);
    // Reset counters; nothing should fire in frame 2 because set_if_neq
    // with equal value MUST NOT bump.
    SAW_CHANGE.store(0, Ordering::Relaxed);
    SETIF_RETURNED_TRUE.store(0, Ordering::Relaxed);
    schedule.run(&mut world);

    assert_eq!(
        SETIF_RETURNED_TRUE.load(Ordering::Relaxed),
        0,
        "set_if_neq with equal value must return false"
    );
    assert_eq!(
        SAW_CHANGE.load(Ordering::Relaxed),
        0,
        "set_if_neq(equal) preserves changed_tick; Changed<T> MUST NOT match"
    );
}

// ── Test 6: bypass_change_detection skips tick bump ─────────────────────────

/// Plan §2.5 MUT5 / §13.2 `mut_bypass_change_detection`:
/// `Mut<T>::bypass_change_detection` MUST NOT bump the changed tick;
/// downstream `Changed<T>` MUST NOT match next frame.
#[test]
fn mut_bypass_change_detection() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[CdBypass387::component_id()]);
    world.spawn_one(arch, CdBypass387 { raw: 7 }).unwrap();

    static SAW_CHANGE: AtomicUsize = AtomicUsize::new(0);
    static WROTE: AtomicUsize = AtomicUsize::new(0);
    SAW_CHANGE.store(0, Ordering::Relaxed);
    WROTE.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut q: Query<Mut<CdBypass387>>| {
        for mut e in &mut q {
            // Mutate via bypass — must NOT bump the tick.
            let raw = e.bypass_change_detection();
            raw.raw = raw.raw.wrapping_add(1);
            WROTE.fetch_add(1, Ordering::Relaxed);
        }
    });
    builder.add_system(|q: Query<&CdBypass387, Changed<CdBypass387>>| {
        for _ in &q {
            SAW_CHANGE.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1: insert tick bumps changed; Changed matches once.
    schedule.run(&mut world);

    // Frame 2: writer uses bypass — tick must stay at frame-1 value;
    // Changed must NOT match.
    SAW_CHANGE.store(0, Ordering::Relaxed);
    WROTE.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        WROTE.load(Ordering::Relaxed),
        1,
        "bypass path must write to underlying value"
    );
    assert_eq!(
        SAW_CHANGE.load(Ordering::Relaxed),
        0,
        "bypass_change_detection must NOT bump the tick; Changed must yield zero"
    );
}

// ── Test 7: Ref<T>::is_added / is_changed observable in same system ─────────

/// Plan §13.2 / Round 3 O1: `Ref<T>::is_added` and `is_changed` exposed
/// in the same system that wrote MUST reflect the bump (inclusive
/// lower-bound semantic).
#[test]
fn ref_is_added_and_is_changed_observable_within_system() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[CdRefIntro391::component_id()]);
    world.spawn_one(arch, CdRefIntro391 { n: 99 }).unwrap();

    static ADDED_ROWS: AtomicUsize = AtomicUsize::new(0);
    static CHANGED_ROWS: AtomicUsize = AtomicUsize::new(0);
    static TOTAL_ROWS: AtomicUsize = AtomicUsize::new(0);
    ADDED_ROWS.store(0, Ordering::Relaxed);
    CHANGED_ROWS.store(0, Ordering::Relaxed);
    TOTAL_ROWS.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // A system reading `Ref<T>` MUST observe `is_added() == true` for a
    // row freshly inserted, and `is_changed() == true` for the same row
    // (insert bumps both ticks).
    builder.add_system(|q: Query<Ref<CdRefIntro391>>| {
        for r in &q {
            TOTAL_ROWS.fetch_add(1, Ordering::Relaxed);
            if r.is_added() {
                ADDED_ROWS.fetch_add(1, Ordering::Relaxed);
            }
            if r.is_changed() {
                CHANGED_ROWS.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(TOTAL_ROWS.load(Ordering::Relaxed), 1, "iterated 1 row");
    assert_eq!(
        ADDED_ROWS.load(Ordering::Relaxed),
        1,
        "freshly-inserted row's Ref::is_added must report true"
    );
    assert_eq!(
        CHANGED_ROWS.load(Ordering::Relaxed),
        1,
        "freshly-inserted row's Ref::is_changed must report true (insert bumps both ticks)"
    );
}

// ── Test 8: Multi-frame lifecycle (5 frames) ────────────────────────────────

/// Plan §13.2 `multi_frame_added_lifecycle` extended: track entity across
/// frames. Entity is spawned BEFORE frame 1; mutated on frame 4. Expected:
/// * Frame 1: insert-tick-in-window → Added matches; Changed matches.
/// * Frames 2, 3: no operation → neither matches.
/// * Frame 4: writer fires → Changed matches; Added does NOT.
/// * Frame 5: no operation → neither matches.
#[test]
fn multi_frame_lifecycle() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[CdLifecycle388::component_id()]);
    world.spawn_one(arch, CdLifecycle388 { counter: 0 }).unwrap();

    static ADDED_HITS: AtomicUsize = AtomicUsize::new(0);
    static CHANGED_HITS: AtomicUsize = AtomicUsize::new(0);
    ADDED_HITS.store(0, Ordering::Relaxed);
    CHANGED_HITS.store(0, Ordering::Relaxed);

    let writer_active = Arc::new(AtomicBool::new(false));
    let writer_active_cl = Arc::clone(&writer_active);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Writer: bumps counter on demand.
    builder.add_system(move |mut q: Query<Mut<CdLifecycle388>>| {
        if writer_active_cl.load(Ordering::Relaxed) {
            for mut e in &mut q {
                e.counter += 1;
            }
        }
    });
    // Reader for Added.
    builder.add_system(|q: Query<&CdLifecycle388, Added<CdLifecycle388>>| {
        for _ in &q {
            ADDED_HITS.fetch_add(1, Ordering::Relaxed);
        }
    });
    // Reader for Changed.
    builder.add_system(|q: Query<&CdLifecycle388, Changed<CdLifecycle388>>| {
        for _ in &q {
            CHANGED_HITS.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // ── Frame 1 ────────────────────────────────────────────────────────
    schedule.run(&mut world);
    assert_eq!(
        ADDED_HITS.load(Ordering::Relaxed),
        1,
        "frame 1: Added must match the pre-existing row"
    );
    assert_eq!(
        CHANGED_HITS.load(Ordering::Relaxed),
        1,
        "frame 1: insert bumps changed_tick; Changed must match"
    );

    // ── Frames 2, 3 — no operation ─────────────────────────────────────
    for f in 2..=3 {
        ADDED_HITS.store(0, Ordering::Relaxed);
        CHANGED_HITS.store(0, Ordering::Relaxed);
        schedule.run(&mut world);
        assert_eq!(
            ADDED_HITS.load(Ordering::Relaxed),
            0,
            "frame {f}: no spawn, Added must NOT match"
        );
        assert_eq!(
            CHANGED_HITS.load(Ordering::Relaxed),
            0,
            "frame {f}: no write, Changed must NOT match"
        );
    }

    // ── Frame 4 — writer fires ─────────────────────────────────────────
    writer_active.store(true, Ordering::Relaxed);
    ADDED_HITS.store(0, Ordering::Relaxed);
    CHANGED_HITS.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    writer_active.store(false, Ordering::Relaxed);
    assert_eq!(
        ADDED_HITS.load(Ordering::Relaxed),
        0,
        "frame 4: no new spawn, Added must NOT match"
    );
    assert_eq!(
        CHANGED_HITS.load(Ordering::Relaxed),
        1,
        "frame 4: writer bumped changed_tick to this_run; Changed must match"
    );

    // ── Frame 5 — no operation, writer flag reset ──────────────────────
    ADDED_HITS.store(0, Ordering::Relaxed);
    CHANGED_HITS.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        ADDED_HITS.load(Ordering::Relaxed),
        0,
        "frame 5: idle, Added must NOT match"
    );
    assert_eq!(
        CHANGED_HITS.load(Ordering::Relaxed),
        0,
        "frame 5: idle, Changed must NOT match"
    );
}

// ── Test 9: Parallel disjoint systems — Changed correctness ────────────────

/// Plan §13.2 `parallel_changed_correctness`: two systems, one writes
/// component A, one reads component B (disjoint). Changed<A> on the
/// downstream reader system must work correctly.
#[test]
fn parallel_systems_no_race() {
    let _guard = TEST_MUTEX.lock().unwrap();

    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let mut world = EcsMaster::new();

    let arch_a = world.create_archetype(&[CdParA389::component_id()]);
    let arch_b = world.create_archetype(&[CdParB390::component_id()]);
    for _ in 0..16 {
        world.spawn_one(arch_a, CdParA389 { a: 1 }).unwrap();
        world.spawn_one(arch_b, CdParB390 { b: 10 }).unwrap();
    }

    static WROTE_A: AtomicUsize = AtomicUsize::new(0);
    static READ_B: AtomicUsize = AtomicUsize::new(0);
    static CHANGED_A_DOWNSTREAM: AtomicUsize = AtomicUsize::new(0);
    WROTE_A.store(0, Ordering::Relaxed);
    READ_B.store(0, Ordering::Relaxed);
    CHANGED_A_DOWNSTREAM.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Writer writes A. Reader reads B. They are scheduling-disjoint.
    builder.add_system(|mut q: Query<Mut<CdParA389>>| {
        for mut e in &mut q {
            e.a = e.a.wrapping_add(1);
            WROTE_A.fetch_add(1, Ordering::Relaxed);
        }
    });
    builder.add_system(|q: Query<&CdParB390>| {
        for _ in &q {
            READ_B.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Two frames: write A; on frame 2 verify Changed<A> can be observed
    // for the next reader pipeline build.
    schedule.run(&mut world);
    schedule.run(&mut world);

    assert_eq!(WROTE_A.load(Ordering::Relaxed), 32, "writer wrote A across 2 frames");
    assert_eq!(READ_B.load(Ordering::Relaxed), 32, "reader saw B across 2 frames");

    // Build a SECOND schedule with a downstream Changed<A> reader. By the
    // time this schedule runs, the previous schedule's `this_run` has
    // already bumped past the world's `current_tick`; the new reader's
    // `last_run` (initialised via `SystemMeta::new`) is `current -
    // MAX_CHANGE_AGE`, so the per-row changed_ticks (last written in the
    // most recent prior frame) lie inside the window → Changed matches.
    let mut builder2 = ScheduleBuilder::new(Arc::clone(&pool));
    builder2.add_system(|q: Query<&CdParA389, Changed<CdParA389>>| {
        for _ in &q {
            CHANGED_A_DOWNSTREAM.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule2 = builder2.build(&mut world);
    schedule2.run(&mut world);

    assert_eq!(
        CHANGED_A_DOWNSTREAM.load(Ordering::Relaxed),
        16,
        "downstream Changed<A> reader observes all 16 prior writes"
    );
}
