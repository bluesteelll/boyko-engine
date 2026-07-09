//! Phase 16.1 (W4) — the DISCRIMINATING dormancy suite for tick-aware run
//! conditions.
//!
//! The sibling file `phase16_1_tick_conditions.rs` only proves the *every-frame*
//! case (a `Changed<T>` condition gating true/false/true on consecutive frames).
//! That case is behavior-preserving across the Phase 16.1 fix, so it does NOT
//! discriminate it. THIS file targets the two coupled correctness changes that
//! ONLY manifest when a system or condition is DORMANT (gated off) for several
//! frames and then resumes:
//!
//! * **Gap #1 — condition dormancy.** A condition now advances its
//!   `(last_run, this_run]` window only on a frame it is actually evaluated
//!   (`run_condition`), not unconditionally at frame start. So a `Changed<C>`
//!   condition that is gated dormant for N frames, on resume, observes ALL
//!   changes accrued while dormant (Bevy "since-last-actual-run" parity) instead
//!   of only "changes since the previous frame". Pre-fix: the unconditional
//!   frame-start condition bump advanced the window every frame, so a mutation
//!   that happened during a dormant frame was already aged out of the window by
//!   the time the condition resumed → the condition reported FALSE (missed it).
//!
//! * **C1 — SYSTEM-body dormancy (THE key new defect).** A gated system now
//!   advances its own ticks only at its dispatch site, on a frame it runs; a
//!   skipped frame leaves them FROZEN. So a `run_if`-gated system with a
//!   `Changed<C>` BODY query, dormant for N frames, on resume sees every change
//!   that happened while it was dormant. Pre-fix: the unconditional frame-start
//!   SYSTEM bump (`schedule.rs:218-221`) advanced the gated system's `last_run`
//!   every frame even on the frames it was SKIPPED, so on resume its window
//!   started at the *last skipped frame* and silently dropped the dormant
//!   mutation → the body's `Changed<C>` query counted ZERO. This is silent data
//!   loss and is the primary thing Phase 16.1 C1 fixes (most users put
//!   `Changed<T>` in system bodies, not in conditions).
//!
//! # How each test discriminates the fix
//!
//! Every dormancy test asserts that the resumed frame's observation is NON-ZERO.
//! Under the pre-fix unconditional-frame-start-bump behavior the corresponding
//! window had already advanced past the dormant mutation, so the observation
//! would have been ZERO — i.e. each `assert` here is a direct regression net for
//! exactly one of the two pre-fix bumps.
//!
//! # Why the wraparound-clamp cases (plan W4 items 4-5) are NOT here
//!
//! Driving the `should_run_check_ticks` cold path requires advancing the world
//! tick past `CHECK_TICK_THRESHOLD` (= 518_400_000), which no integration test
//! can reach in bounded time, and `Schedule::check_change_ticks` /
//! `System::set_change_ticks` / `SystemMeta::{last_run,this_run}` /
//! `Tick::check_tick` are all `pub(crate)` — unreachable from this external test
//! crate. Those cases therefore live as in-crate unit + property tests in
//! `schedule.rs`'s `#[cfg(test)] mod tests` (the only place the clamp entry point
//! is reachable), exactly as the plan §Tests prescribes ("Unit: ...
//! `Schedule::check_change_ticks` clamps system + own + set condition").
//!
//! # Harness discipline (matches `phase16_run_conditions.rs` / `phase17_states.rs`)
//!
//! Single-worker pool (deterministic serial dispatch), per-test `Arc<Atomic*>`
//! probes captured by the closures — NO shared global `static`s, so the tests
//! never flake under parallel `cargo test`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Changed, Mut, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Res;
use boyko_macros::{Component, Resource};
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

// ── Component / resource types (per-test unique component ids minted lazily) ───

#[derive(Component)]
#[repr(C)]
struct DormantHp {
    hp: u32,
}

#[derive(Component)]
#[repr(C)]
struct DormantMana {
    mana: u32,
}

#[derive(Component)]
#[repr(C)]
struct DormantArmor {
    armor: u32,
}

/// A boolean gate resource flipped between frames via `insert_resource` to drive
/// a `Res`-reading `run_if` condition true/false across the dormant span.
#[derive(Resource)]
struct Gate(bool);

// =============================================================================
// §1 — Gap #1: a `Changed<C>` CONDITION advances its window ONLY when evaluated
//      (`run_condition` eval-site checkpoint), and the every-frame case is
//      behavior-preserving.
//
// IMPORTANT (OQ-1 / plan R2): boyko's condition fold is EAGER with NO
// short-circuit (`should_run &= r`). `evaluate_ready_conditions` runs EVERY own
// AND every gating-set condition of a system the moment that system is *reached*
// (`pred_remaining == 0`, not running, not completed) — regardless of whether a
// sibling condition or a set gate already returned false. Empirically verified:
// an own `Changed<C>` condition sitting next to a false `Res<Gate>` sibling, and
// a set-level `Changed<C>` condition on a once-per-frame-reached member, are BOTH
// evaluated on EVERY frame the schedule runs. Consequently a CONDITION is NEVER
// topologically dormant in a single public-API schedule: there is no public
// drive that suppresses a reachable system's condition evaluation for N frames.
//
// The genuine condition-dormancy proof for the Gap #1 MECHANISM (a frozen window
// observing a change accrued while the condition was NOT evaluated) therefore
// lives as an in-crate unit test in `schedule.rs`'s `#[cfg(test)] mod tests`
// (`run_condition_advances_window_only_when_evaluated` /
// `dormant_condition_sees_change_accrued_while_skipped`), where the eval site can
// be driven directly. This integration test pins the REACHABLE, discriminating
// consequence: a `Changed<C>` condition evaluated every frame still gates
// correctly per-frame (the eval-site checkpoint reproduces the old frame-start
// window EXACTLY for the every-frame case).
// =============================================================================

/// A `.run_if(|q: Query<&DormantHp, Changed<DormantHp>>| ...)` condition (the
/// ONLY gate, so it is reached and evaluated every frame) gates true/false/true
/// as `DormantHp` is mutated. This proves the Gap #1 eval-site checkpoint keeps
/// the every-frame window identical to the deleted frame-start bump: a condition
/// evaluated every frame sees `prev == last frame's this_run`, so its
/// `(last_run, this_run]` window is the same one the old loop produced.
///
/// (The dormant-resume case is the in-crate unit test; see the module note.)
#[test]
fn changed_condition_eval_site_checkpoint_gates_per_frame() {
    let pool = serial_pool();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[DormantHp::component_id()]);
    world
        .spawn_one(arch, DormantHp { hp: 100 })
        .expect("spawn DormantHp entity");

    let body_runs = counter();
    let body_runs_cl = Arc::clone(&body_runs);
    let mutate_now = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mutate_now_cl = Arc::clone(&mutate_now);

    let mut builder = ScheduleBuilder::new(pool);

    // Always-on writer ordered BEFORE the gated system, so the condition (run
    // when the gated system becomes ready) observes the write in the same frame.
    let writer = builder
        .add_system(move |mut q: Query<Mut<DormantHp>>| {
            if mutate_now_cl.load(Ordering::Relaxed) {
                for mut h in &mut q {
                    h.hp = h.hp.wrapping_sub(1);
                }
            }
        })
        .key();

    builder
        .add_system(move || {
            body_runs_cl.fetch_add(1, Ordering::Relaxed);
        })
        .after(writer)
        .run_if(|q: Query<&DormantHp, Changed<DormantHp>>| q.iter().count() > 0);

    let mut schedule = builder.build(&mut world);

    // Frame 1: insert tick in the first window ⇒ TRUE ⇒ runs (1).
    schedule.run(&mut world);
    assert_eq!(load(&body_runs), 1, "frame 1: insert tick in window ⇒ condition true ⇒ runs");

    // Frame 2: idle ⇒ the frame-1 tick is now == last_run, outside (last_run,
    // this_run] ⇒ FALSE ⇒ skipped (still 1). A pre-fix frozen-tick condition
    // would have reported always-true here.
    schedule.run(&mut world);
    assert_eq!(load(&body_runs), 1, "frame 2: idle ⇒ condition false ⇒ skipped");

    // Frame 3: mutate ⇒ changed_tick = this_run ∈ (last_run, this_run] ⇒ TRUE.
    mutate_now.store(true, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(load(&body_runs), 2, "frame 3: mutation ⇒ condition true ⇒ runs again");

    // Frame 4: idle ⇒ window re-closes ⇒ FALSE ⇒ skipped (still 2).
    mutate_now.store(false, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(load(&body_runs), 2, "frame 4: idle ⇒ condition false ⇒ skipped");
}

// =============================================================================
// §2 — C1: a SYSTEM with a `Changed<C>` BODY query, dormant across a mutation,
//      SEES it on resume. THIS is the key new defect C1 fixes.
// =============================================================================

/// A `run_if(Gate)`-gated system whose BODY runs `Query<&DormantMana,
/// Changed<DormantMana>>` and records how many rows it matched *on its most
/// recent run*. An always-on writer mutates `DormantMana` on the dormant frame.
///
/// Drive:
/// * Frame 1: gate true → body runs; first-frame all-changed ⇒ it matches the
///   row → records ≥1, AND stamps its own ticks to frame 1.
/// * Frames 2..=4: gate FALSE → body skipped → its ticks stay FROZEN at frame 1.
///   Writer mutates `DormantMana` on frame 3 (changed_tick = frame-3 this_run).
/// * Frame 5: gate true → body runs again. Its window is
///   `(frame1_this_run, frame5_this_run]` (last_run frozen at frame 1), which
///   STILL contains the frame-3 mutation ⇒ the body's `Changed` query matches
///   the row ⇒ records ≥1.
///
/// Pre-fix discrimination (the silent-data-loss net): the unconditional
/// frame-start SYSTEM bump at `schedule.rs:218-221` advanced this gated system's
/// `last_run` EVERY frame, including the skipped frames 2..=4. So on frame 5 its
/// window would be `(frame4_this_run, frame5_this_run]` — the frame-3 mutation
/// is aged out → the body's `Changed` query matches ZERO rows on resume. The
/// last assert (`recent_matched > 0`) is exactly that regression net: it FAILS
/// (0) pre-fix and PASSES (≥1) post-fix.
#[test]
fn changed_body_query_dormant_across_mutation_sees_it_on_resume() {
    let pool = serial_pool();
    let mut world = EcsMaster::new();
    world.insert_resource(Gate(true));

    let arch = world.create_archetype(&[DormantMana::component_id()]);
    world
        .spawn_one(arch, DormantMana { mana: 50 })
        .expect("spawn DormantMana entity");

    // `recent_matched` is OVERWRITTEN with this run's Changed-row count, so we
    // inspect exactly the most-recent run (not an accumulation across frames).
    let recent_matched = counter();
    let recent_matched_cl = Arc::clone(&recent_matched);
    let body_runs = counter();
    let body_runs_cl = Arc::clone(&body_runs);
    let mutate_now = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mutate_now_cl = Arc::clone(&mutate_now);

    let mut builder = ScheduleBuilder::new(pool);

    let writer = builder
        .add_system(move |mut q: Query<Mut<DormantMana>>| {
            if mutate_now_cl.load(Ordering::Relaxed) {
                for mut m in &mut q {
                    m.mana = m.mana.wrapping_add(1);
                }
            }
        })
        .key();

    // The gated system: its BODY holds the Changed<C> query (the common case).
    builder
        .add_system(move |q: Query<&DormantMana, Changed<DormantMana>>| {
            body_runs_cl.fetch_add(1, Ordering::Relaxed);
            recent_matched_cl.store(q.iter().count(), Ordering::Relaxed);
        })
        .after(writer)
        .run_if(|g: Res<Gate>| g.0);

    let mut schedule = builder.build(&mut world);

    // Frame 1: gate true ⇒ body runs; first-frame all-changed ⇒ matches the row.
    schedule.run(&mut world);
    assert_eq!(load(&body_runs), 1, "frame 1: gate true ⇒ gated body runs");
    assert!(
        load(&recent_matched) >= 1,
        "frame 1: first-frame all-changed ⇒ body's Changed query matches the row"
    );

    // Frames 2..=4: gate FALSE ⇒ body skipped ⇒ its ticks FROZEN at frame 1.
    world.insert_resource(Gate(false));
    schedule.run(&mut world); // frame 2 — skipped
    mutate_now.store(true, Ordering::Relaxed);
    schedule.run(&mut world); // frame 3 — writer mutates DormantMana; body skipped
    mutate_now.store(false, Ordering::Relaxed);
    schedule.run(&mut world); // frame 4 — skipped
    assert_eq!(load(&body_runs), 1, "frames 2..=4: gate false ⇒ body never runs (still 1)");

    // Frame 5: gate true ⇒ body runs again. With its last_run FROZEN at frame 1
    // (C1), its window still spans the frame-3 mutation ⇒ matches the row.
    world.insert_resource(Gate(true));
    schedule.run(&mut world);
    assert_eq!(load(&body_runs), 2, "frame 5: gate true ⇒ body runs again");
    assert!(
        load(&recent_matched) >= 1,
        "frame 5 (THE C1 PROOF): a gated system's `Changed<C>` BODY query, dormant since \
         frame 1, observes the frame-3 mutation on resume because its ticks stayed FROZEN \
         while skipped. A pre-fix unconditional frame-start SYSTEM bump would have advanced \
         last_run every skipped frame, aging the mutation out ⇒ this would be 0 (silent data \
         loss)."
    );
}

/// C1 corollary — a gated system dormant across a mutation, on resume, must NOT
/// keep re-reporting that change on the FOLLOWING frame it runs (the resumed run
/// correctly checkpoints its `last_run` to the resume frame). Without a correct
/// checkpoint at the dispatch site the window could remain anchored in the past
/// and double-count.
///
/// Drive: same gated `Changed<DormantArmor>` body, gate true every frame this
/// time. Mutate on frame 2 only. Frame 2 the body sees the change (>=1); frame 3
/// (no mutation, gate still true) the body must see ZERO — proving the resume /
/// run checkpoint advanced past the frame-2 mutation.
#[test]
fn gated_body_run_checkpoints_so_change_is_seen_exactly_once() {
    let pool = serial_pool();
    let mut world = EcsMaster::new();
    world.insert_resource(Gate(true));

    let arch = world.create_archetype(&[DormantArmor::component_id()]);
    world
        .spawn_one(arch, DormantArmor { armor: 10 })
        .expect("spawn DormantArmor entity");

    let recent_matched = counter();
    let recent_matched_cl = Arc::clone(&recent_matched);
    let mutate_now = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mutate_now_cl = Arc::clone(&mutate_now);

    let mut builder = ScheduleBuilder::new(pool);
    let writer = builder
        .add_system(move |mut q: Query<Mut<DormantArmor>>| {
            if mutate_now_cl.load(Ordering::Relaxed) {
                for mut a in &mut q {
                    a.armor = a.armor.wrapping_add(1);
                }
            }
        })
        .key();
    builder
        .add_system(move |q: Query<&DormantArmor, Changed<DormantArmor>>| {
            recent_matched_cl.store(q.iter().count(), Ordering::Relaxed);
        })
        .after(writer)
        .run_if(|g: Res<Gate>| g.0);

    let mut schedule = builder.build(&mut world);

    // Frame 1: first-frame all-changed (consume the insert window).
    schedule.run(&mut world);

    // Frame 2: mutate ⇒ body sees the change.
    mutate_now.store(true, Ordering::Relaxed);
    schedule.run(&mut world);
    assert!(load(&recent_matched) >= 1, "frame 2: body sees the frame-2 mutation");

    // Frame 3: no mutation, gate still true ⇒ body runs but sees ZERO changed
    // rows (its last_run advanced to frame 2 when it ran ⇒ the frame-2 change is
    // now behind the window).
    mutate_now.store(false, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        load(&recent_matched),
        0,
        "frame 3: the frame-2 change is reported EXACTLY ONCE — the resumed run checkpointed \
         its last_run to frame 2, so an idle frame 3 sees zero changed rows (no double-count, \
         no stuck window)"
    );
}

// =============================================================================
// §3 — Behavior preservation: an UNGATED system + UNGATED condition advance
//      ticks EVERY frame and `Changed<C>` works per-frame. Re-pins the
//      every-frame contract the dormancy fix must not regress.
// =============================================================================

/// An UNgated (`has_condition` clear) system with a `Changed<C>` body query
/// observes the classic per-frame true/false/true pattern: frame 1 (insert
/// window) matches, an idle frame matches nothing, a mutation frame matches
/// again. This is the path that is byte-identical across the Phase 16.1 fix
/// (plain systems are still stamped at frame start); the test guards against a
/// regression that broke the common every-frame case while fixing dormancy.
#[test]
fn ungated_system_changed_body_works_every_frame() {
    let pool = serial_pool();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[DormantHp::component_id()]);
    world.spawn_one(arch, DormantHp { hp: 7 }).expect("spawn");

    let recent_matched = counter();
    let recent_matched_cl = Arc::clone(&recent_matched);
    let mutate_now = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mutate_now_cl = Arc::clone(&mutate_now);

    let mut builder = ScheduleBuilder::new(pool);
    let writer = builder
        .add_system(move |mut q: Query<Mut<DormantHp>>| {
            if mutate_now_cl.load(Ordering::Relaxed) {
                for mut h in &mut q {
                    h.hp = h.hp.wrapping_add(1);
                }
            }
        })
        .key();
    // No `.run_if` ⇒ ungated ⇒ stamped at frame start every frame.
    builder
        .add_system(move |q: Query<&DormantHp, Changed<DormantHp>>| {
            recent_matched_cl.store(q.iter().count(), Ordering::Relaxed);
        })
        .after(writer);

    let mut schedule = builder.build(&mut world);

    // Frame 1: insert tick in the first window ⇒ matches.
    schedule.run(&mut world);
    assert!(load(&recent_matched) >= 1, "frame 1: ungated body sees the freshly-inserted row");

    // Frame 2: idle ⇒ matches nothing.
    schedule.run(&mut world);
    assert_eq!(load(&recent_matched), 0, "frame 2: idle ⇒ ungated body sees zero changed rows");

    // Frame 3: mutate ⇒ matches again.
    mutate_now.store(true, Ordering::Relaxed);
    schedule.run(&mut world);
    assert!(load(&recent_matched) >= 1, "frame 3: mutation ⇒ ungated body sees the changed row");

    // Frame 4: idle again ⇒ matches nothing (window re-closes).
    mutate_now.store(false, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(load(&recent_matched), 0, "frame 4: idle ⇒ ungated body sees zero changed rows");
}

/// An UNgated `.run_if(|q: Query<&C, Changed<C>>| ...)` condition (the gate is
/// the `Changed` condition itself, evaluated EVERY frame) reproduces the
/// every-frame true/false/true gating documented in
/// `phase16_1_tick_conditions.rs`. Re-pinned here so the dormancy file is a
/// complete behavioral net: the Gap #1 eval-site checkpoint must keep the
/// every-frame case identical to the old frame-start bump (for a condition run
/// every frame, `prev` == last frame's `this_run`).
#[test]
fn ungated_changed_condition_gates_per_frame() {
    let pool = serial_pool();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[DormantMana::component_id()]);
    world.spawn_one(arch, DormantMana { mana: 1 }).expect("spawn");

    let body_runs = counter();
    let body_runs_cl = Arc::clone(&body_runs);
    let mutate_now = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mutate_now_cl = Arc::clone(&mutate_now);

    let mut builder = ScheduleBuilder::new(pool);
    let writer = builder
        .add_system(move |mut q: Query<Mut<DormantMana>>| {
            if mutate_now_cl.load(Ordering::Relaxed) {
                for mut m in &mut q {
                    m.mana = m.mana.wrapping_add(1);
                }
            }
        })
        .key();
    // The ONLY condition is the Changed condition — evaluated every frame the
    // system is reached (which is every frame, since there is no other gate).
    builder
        .add_system(move || {
            body_runs_cl.fetch_add(1, Ordering::Relaxed);
        })
        .after(writer)
        .run_if(|q: Query<&DormantMana, Changed<DormantMana>>| q.iter().count() > 0);

    let mut schedule = builder.build(&mut world);

    // Frame 1: insert window ⇒ true ⇒ runs (1).
    schedule.run(&mut world);
    assert_eq!(load(&body_runs), 1, "frame 1: insert tick in window ⇒ condition true ⇒ runs");

    // Frame 2: idle ⇒ false ⇒ skipped (still 1).
    schedule.run(&mut world);
    assert_eq!(load(&body_runs), 1, "frame 2: idle ⇒ condition false ⇒ skipped");

    // Frame 3: mutate ⇒ true ⇒ runs (2).
    mutate_now.store(true, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(load(&body_runs), 2, "frame 3: mutation ⇒ condition true ⇒ runs again");

    // Frame 4: idle ⇒ false ⇒ skipped (still 2).
    mutate_now.store(false, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(load(&body_runs), 2, "frame 4: idle ⇒ condition false ⇒ skipped");
}
