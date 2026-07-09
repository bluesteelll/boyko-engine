//! Phase 16.1 (B-1) — end-to-end proof that tick-based run conditions work.
//!
//! The B-1 fix bumps every run-condition's `(last_run, this_run)` tick snapshot
//! at frame start (in `Schedule::run`) with the SAME `this_run` as the system
//! bodies. Without it, a condition's ticks stay frozen at the `initialize`
//! sentinel (`current - MAX_CHANGE_AGE`) forever, so a `Changed<T>` / `Added<T>`
//! / `Ref<T>` condition reads EVERY per-row tick as "changed since last run" and
//! silently reports ALWAYS-TRUE. The in-module `schedule.rs` test
//! (`run_condition_ticks_advance_per_frame`) proves the ticks advance; THIS file
//! proves the user-visible consequence: a `.run_if(<T changed this frame>)` gate
//! actually gates — true frame 1, FALSE on an idle frame, true again after a
//! real mutation.
//!
//! # Why this is the discriminating test
//!
//! A frozen-tick (broken-B-1) condition would observe the idle frame 2 as
//! "changed" and run the gated body anyway, so the `frame 2 ⇒ body did NOT run`
//! assertion is exactly the regression net. The `frame 3 after mutation ⇒ body
//! runs again` assertion proves the gate is not stuck-false either.
//!
//! # How "T changed this frame" is expressed in a `-> bool` condition
//!
//! boyko's `Query<'w, 's, D, F>` is a `SystemParam`, and a run condition is any
//! `impl IntoSystem<(), bool, M>`. So the idiomatic spelling is a closure
//! `|q: Query<&T, Changed<T>>| q.iter().count() > 0` — it materialises the
//! per-row `Changed<T>` filter against the condition's own tick window and
//! returns whether any row changed.
//!
//! NOTE: `Query::is_empty()` is the WRONG primitive here — it reports whether
//! any *archetype* is matched, NOT whether any *row* passed the per-row tick
//! filter (`query.rs` docs are explicit). `Changed<T>` is non-archetypal, so we
//! must drive the iterator (`iter().count()`) to evaluate the row-level ticks.
//!
//! # Harness discipline (matches `phase16_run_conditions.rs`)
//!
//! Single-worker pool (deterministic serial dispatch), per-test
//! `Arc<AtomicUsize>` / `Arc<AtomicBool>` probes captured by the closures — NO
//! shared global `static`s, so the tests never flake under parallel
//! `cargo test`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Changed, Mut, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_macros::Component;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

// ── Harness ──────────────────────────────────────────────────────────────────

/// Single-worker pool — serial dispatch ⇒ deterministic firing order.
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

fn load(c: &Arc<AtomicUsize>) -> usize {
    c.load(Ordering::Relaxed)
}

// ── Component types (per-test unique; ids minted lazily on first use) ─────────

#[derive(Component)]
#[repr(C)]
struct TickHealth {
    hp: u32,
}

#[derive(Component)]
#[repr(C)]
struct TickPos {
    x: f32,
}

// =============================================================================
// §1 — THE core B-1 proof: a `Changed<T>` condition gates true / false / true
//      across three frames, mutation driven by a writer system in-frame.
// =============================================================================

/// A system gated by `.run_if(|q: Query<&TickHealth, Changed<TickHealth>>|
/// q.iter().count() > 0)`. A writer system (ordered BEFORE the gated system, so
/// the condition — evaluated when the gated system becomes ready — observes the
/// writer's effect within the SAME frame) mutates `TickHealth` only when a flag
/// is set.
///
/// * Frame 1: the pre-existing row's insert tick lies in the condition's first
///   `(current - MAX_CHANGE_AGE, this_run]` window ⇒ condition TRUE ⇒ gated runs
///   (the standard first-frame "everything is changed" semantic).
/// * Frame 2 (writer flag OFF, no mutation): the only change tick is frame 1's,
///   which is `== last_run` and therefore OUTSIDE the strict-lower-bound window
///   `(last_run, this_run]` ⇒ condition FALSE ⇒ gated does NOT run. THIS is the
///   assertion a frozen-tick (broken B-1) condition fails — it would see frame 2
///   as changed and run the body.
/// * Frame 3 (writer flag ON): the writer bumps `changed_tick = this_run(3)`,
///   which lies in frame 3's window ⇒ condition TRUE ⇒ gated runs again. Proves
///   the gate is not stuck-false.
#[test]
fn changed_condition_gates_true_false_true_across_frames() {
    let pool = serial_pool();
    let mut world = EcsMaster::new();

    // Pre-existing entity carrying TickHealth (spawned before the schedule).
    let arch = world.create_archetype(&[TickHealth::component_id()]);
    world
        .spawn_one(arch, TickHealth { hp: 100 })
        .expect("spawn TickHealth entity");

    // Probe for "did the gated body run this frame?" and the writer's flag.
    let gated_runs = counter();
    let gated_runs_cl = Arc::clone(&gated_runs);
    let should_mutate = Arc::new(AtomicBool::new(false));
    let should_mutate_cl = Arc::clone(&should_mutate);

    let mut builder = ScheduleBuilder::new(pool);

    // Writer: mutates TickHealth via Mut<T> (bumps the changed tick) only when
    // the flag is set. `Mut::deref_mut` is the change-detection write path.
    let writer = builder
        .add_system(move |mut q: Query<Mut<TickHealth>>| {
            if should_mutate_cl.load(Ordering::Relaxed) {
                for mut h in &mut q {
                    h.hp = h.hp.wrapping_sub(1);
                }
            }
        })
        .key();

    // Gated system: empty body (just records that it ran), ordered AFTER the
    // writer, gated on "did TickHealth change this frame?".
    builder
        .add_system(move || {
            gated_runs_cl.fetch_add(1, Ordering::Relaxed);
        })
        .after(writer)
        .run_if(|q: Query<&TickHealth, Changed<TickHealth>>| q.iter().count() > 0);

    let mut schedule = builder.build(&mut world);

    // ── Frame 1: insert tick in first window ⇒ TRUE ⇒ gated runs. ──
    schedule.run(&mut world);
    assert_eq!(
        load(&gated_runs),
        1,
        "frame 1: Changed<TickHealth> condition true on first run (insert tick in window) ⇒ gated body runs"
    );

    // ── Frame 2: no mutation ⇒ FALSE ⇒ gated does NOT run. ──
    // This is the discriminating assertion: a silently-always-true (broken B-1)
    // condition would run the body here.
    schedule.run(&mut world);
    assert_eq!(
        load(&gated_runs),
        1,
        "frame 2: no mutation ⇒ Changed condition FALSE ⇒ gated body must NOT run \
         (a frozen-tick / always-true condition would have incremented this to 2)"
    );

    // ── Frame 3: writer mutates ⇒ TRUE ⇒ gated runs again. ──
    should_mutate.store(true, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        load(&gated_runs),
        2,
        "frame 3: writer bumped changed_tick to this_run ⇒ Changed condition TRUE ⇒ gated runs again"
    );

    // ── Frame 4: flag back off ⇒ FALSE again ⇒ no further run. ──
    // Confirms the window re-closes after the change ages out — the gate is
    // genuinely tick-driven, not a one-way latch.
    should_mutate.store(false, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        load(&gated_runs),
        2,
        "frame 4: idle again ⇒ Changed condition FALSE ⇒ gated body must NOT run"
    );
}

// =============================================================================
// §2 — control: a body-equivalent `Changed<T>` reader SYSTEM (not a condition)
//      observes the identical true/false/true pattern. Pins that the condition
//      path and the system path agree (the B-1 fix made conditions match
//      systems).
// =============================================================================

/// The same scenario as §1 but the change-detection check lives in a normal
/// reader SYSTEM body (`Query<&TickPos, Changed<TickPos>>`) rather than a
/// condition. Reader systems already advanced their ticks pre-B-1 (Phase 10), so
/// this is the reference behaviour the condition path must reproduce. Running
/// both in the same suite makes a divergence between the two paths obvious.
#[test]
fn changed_reader_system_matches_condition_semantics() {
    let pool = serial_pool();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[TickPos::component_id()]);
    world.spawn_one(arch, TickPos { x: 0.0 }).expect("spawn TickPos entity");

    let changed_seen = counter();
    let changed_seen_cl = Arc::clone(&changed_seen);
    let should_mutate = Arc::new(AtomicBool::new(false));
    let should_mutate_cl = Arc::clone(&should_mutate);

    let mut builder = ScheduleBuilder::new(pool);

    let writer = builder
        .add_system(move |mut q: Query<Mut<TickPos>>| {
            if should_mutate_cl.load(Ordering::Relaxed) {
                for mut p in &mut q {
                    p.x += 1.0;
                }
            }
        })
        .key();

    // Reader records how many rows it saw as Changed this frame.
    builder
        .add_system(move |q: Query<&TickPos, Changed<TickPos>>| {
            let n = q.iter().count();
            if n > 0 {
                changed_seen_cl.fetch_add(1, Ordering::Relaxed);
            }
        })
        .after(writer);

    let mut schedule = builder.build(&mut world);

    // Frame 1: insert tick in window ⇒ reader sees the row as changed.
    schedule.run(&mut world);
    assert_eq!(load(&changed_seen), 1, "frame 1: reader sees the freshly-inserted row as Changed");

    // Frame 2: idle ⇒ reader sees nothing.
    schedule.run(&mut world);
    assert_eq!(load(&changed_seen), 1, "frame 2: idle ⇒ reader sees zero Changed rows");

    // Frame 3: mutation ⇒ reader sees the row as changed again.
    should_mutate.store(true, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(load(&changed_seen), 2, "frame 3: writer bumped tick ⇒ reader sees the row as Changed");
}

// =============================================================================
// §3 — a `Changed<T>` condition that is ALWAYS false after frame 1 still lets
//      the frame terminate (skip-successor sanity for the tick-condition path).
// =============================================================================

/// A gated system with NO writer ever touching `TickHealth`: after frame 1 the
/// condition is false forever. Across many frames the body runs exactly once
/// (frame 1), and every frame terminates cleanly. Guards against a tick-driven
/// condition wedging the executor when it goes (and stays) false.
#[test]
fn changed_condition_false_after_first_frame_terminates_every_frame() {
    let pool = serial_pool();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[TickHealth::component_id()]);
    world.spawn_one(arch, TickHealth { hp: 1 }).expect("spawn");

    let runs = counter();
    let runs_cl = Arc::clone(&runs);

    let mut builder = ScheduleBuilder::new(pool);
    builder
        .add_system(move || {
            runs_cl.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(|q: Query<&TickHealth, Changed<TickHealth>>| q.iter().count() > 0);

    let mut schedule = builder.build(&mut world);

    // Frame 1 runs (first-frame all-changed); frames 2..=5 are idle ⇒ skipped.
    for _ in 0..5 {
        schedule.run(&mut world);
    }

    assert_eq!(
        load(&runs),
        1,
        "the gated body runs only on frame 1; the never-changing component leaves the condition false for frames 2..=5"
    );
}
