//! KE5 — run-condition combinators, pinned by the **five D5 tests**.
//!
//! Kernel backlog `KE5`; ruling `D5` in `docs/aether-v2/DECISIONS.md` enumerates
//! the five and states that the numbering is load-bearing, so each test below
//! names its number in its title and its doc comment.
//!
//! > **D5. Run-condition combinators fold EAGERLY — no short-circuit.** Not
//! > (only) because `run_once` mutates on evaluation: a condition's change-tick
//! > window advances only when it actually runs, so a short-circuited RHS
//! > freezes its `Changed` window and observes a bogus burst later.
//! > `CombinedSystem` runs both children every reached frame, unions access,
//! > forwards tick maintenance to both.
//!
//! # What each test would report against a LAZY fold
//!
//! The implementation these tests reject is not a strawman: it is the careful
//! lazy fold — short-circuit `run_unsafe`, and advance a child's tick snapshot
//! at the moment that child actually runs (which is `EcsMaster::run_condition`'s
//! own rule for a leaf). Measured against exactly that implementation before the
//! eager fold landed:
//!
//! | test | lazy fold |
//! |---|---|
//! | D5(1) `both_children_evaluate_every_reached_frame` | RED — RHS call count 0 where 3 is required |
//! | D5(2) `run_once_on_the_rhs_of_a_true_or_still_consumes_its_run` | RED — the gated body runs on a second frame |
//! | D5(3) `no_bogus_changed_burst_from_the_would_be_skipped_side` | RED — the frozen window bursts on the next frame |
//! | D5(4) `combined_access_is_the_union_of_the_children` | green either way (access is not a run-time property) |
//! | D5(5) `tick_maintenance_is_forwarded_to_both_children` | RED — the RHS's snapshot never advances |
//!
//! D5(4) is green on both sides **by design** and is kept as the
//! over-correction guard: it is the one test that would go red if a future
//! change narrowed the union to make the eager fold cheaper.
//!
//! # Harness discipline
//!
//! Copied from `phase16_1_tick_conditions.rs`: a single-worker pool for
//! deterministic serial dispatch, and per-test `Arc<Atomic*>` probes captured by
//! the closures — never a shared `static`, so nothing flakes under a parallel
//! `cargo test`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::change_detection::{MAX_CHANGE_AGE, Tick};
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Changed, Mut, Query};
use boyko_ecs::ecs::core::schedule::{ConditionExt, ScheduleBuilder, not, run_once};
use boyko_ecs::ecs::core::resources::resource::Resource;
use boyko_ecs::ecs::core::system::Res;
use boyko_ecs::ecs::core::system::access::Access;
use boyko_ecs::ecs::core::system::system::System;
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

// ── Component / resource types ───────────────────────────────────────────────

#[derive(Component)]
#[repr(C)]
struct Ke5Health {
    hp: u32,
}

#[derive(Component)]
#[repr(C)]
struct Ke5Pos {
    x: f32,
}

/// A resource read by one side of a combinator, so D5(4) has a resource axis to
/// assert on as well as a component axis — a union that dropped one side would
/// otherwise be invisible whenever both children happen to touch the same axis.
#[derive(Resource)]
struct Ke5Config(u32);

// =============================================================================
// D5(1) — both children evaluate every reached frame
// =============================================================================

/// **D5(1).** Instrument each side with a call counter and assert both advance
/// on a frame where the LHS alone would have decided the result.
///
/// Two directions, because the two operators short-circuit on opposite verdicts
/// and a fold that is eager for one is not thereby eager for the other:
///
/// * `or` with a **true** LHS — a lazy `||` never reaches the RHS.
/// * `and` with a **false** LHS — a lazy `&&` never reaches the RHS.
///
/// Both counters must equal the frame count in both directions.
#[test]
fn d5_1_both_children_evaluate_every_reached_frame() {
    const FRAMES: usize = 3;

    // ── direction A: `or` with a decisive (true) LHS ──
    {
        let pool = serial_pool();
        let mut world = EcsMaster::new();

        let lhs_calls = counter();
        let rhs_calls = counter();
        let lhs_cl = Arc::clone(&lhs_calls);
        let rhs_cl = Arc::clone(&rhs_calls);

        let mut builder = ScheduleBuilder::new(pool);
        builder.add_system(|| {}).run_if(
            (move || {
                lhs_cl.fetch_add(1, Ordering::Relaxed);
                true // decisive for `or`
            })
            .or(move || {
                rhs_cl.fetch_add(1, Ordering::Relaxed);
                false
            }),
        );
        let mut schedule = builder.build(&mut world);
        for _ in 0..FRAMES {
            schedule.run(&mut world);
        }

        assert_eq!(load(&lhs_calls), FRAMES, "`or` LHS runs every frame");
        assert_eq!(
            load(&rhs_calls),
            FRAMES,
            "`or` RHS must run every frame even though the true LHS already \
             decided the verdict — a short-circuiting fold reports 0 here",
        );
    }

    // ── direction B: `and` with a decisive (false) LHS ──
    {
        let pool = serial_pool();
        let mut world = EcsMaster::new();

        let lhs_calls = counter();
        let rhs_calls = counter();
        let lhs_cl = Arc::clone(&lhs_calls);
        let rhs_cl = Arc::clone(&rhs_calls);

        let mut builder = ScheduleBuilder::new(pool);
        builder.add_system(|| {}).run_if(
            (move || {
                lhs_cl.fetch_add(1, Ordering::Relaxed);
                false // decisive for `and`
            })
            .and(move || {
                rhs_cl.fetch_add(1, Ordering::Relaxed);
                true
            }),
        );
        let mut schedule = builder.build(&mut world);
        for _ in 0..FRAMES {
            schedule.run(&mut world);
        }

        assert_eq!(load(&lhs_calls), FRAMES, "`and` LHS runs every frame");
        assert_eq!(
            load(&rhs_calls),
            FRAMES,
            "`and` RHS must run every frame even though the false LHS already \
             decided the verdict",
        );
    }
}

/// **D5(1), `Not` arm.** `not(c)` has nothing to short-circuit, but its child
/// must still be reached every frame — the same window-freeze argument applies
/// to a negation that stopped calling its child under some condition.
#[test]
fn d5_1_not_reaches_its_child_every_frame_and_inverts() {
    const FRAMES: usize = 3;
    let pool = serial_pool();
    let mut world = EcsMaster::new();

    let child_calls = counter();
    let gated_runs = counter();
    let child_cl = Arc::clone(&child_calls);
    let gated_cl = Arc::clone(&gated_runs);

    let mut builder = ScheduleBuilder::new(pool);
    builder
        .add_system(move || {
            gated_cl.fetch_add(1, Ordering::Relaxed);
        })
        .run_if(not(move || {
            child_cl.fetch_add(1, Ordering::Relaxed);
            false
        }));
    let mut schedule = builder.build(&mut world);
    for _ in 0..FRAMES {
        schedule.run(&mut world);
    }

    assert_eq!(load(&child_calls), FRAMES, "the negated child runs each frame");
    assert_eq!(
        load(&gated_runs),
        FRAMES,
        "`not(false)` is true ⇒ the gated body runs every frame",
    );
}

// =============================================================================
// D5(2) — `run_once` on the RHS of an `or` whose LHS is true still consumes its run
// =============================================================================

/// **D5(2).** The RHS's one-shot latch flips even though the combined result
/// never needed it.
///
/// Uses the real [`run_once`], not a re-implementation, because the ruling is
/// about that exact built-in. The observation is indirect by necessity — the
/// latch lives in the condition's `Local<bool>` — so the schedule is arranged to
/// make the latch's state decide a later frame's verdict:
///
/// * **Frame 1** — LHS `true`. Eager: the RHS `run_once` runs, returns `true`,
///   and **consumes** its one shot. Combined `true` ⇒ the body runs.
/// * **Frame 2** — LHS flipped to `false`, so the `or` now depends entirely on
///   the RHS. Eager: `run_once` is spent ⇒ `false` ⇒ combined `false` ⇒ the body
///   does **not** run.
///
/// A short-circuiting fold never reaches the RHS on frame 1, so its latch is
/// still armed on frame 2 and fires there: the body runs twice. `1` versus `2`
/// is the whole test.
#[test]
fn d5_2_run_once_on_the_rhs_of_a_true_or_still_consumes_its_run() {
    let pool = serial_pool();
    let mut world = EcsMaster::new();

    let gated_runs = counter();
    let gated_cl = Arc::clone(&gated_runs);
    let lhs_verdict = Arc::new(AtomicBool::new(true));
    let lhs_cl = Arc::clone(&lhs_verdict);

    let mut builder = ScheduleBuilder::new(pool);
    builder
        .add_system(move || {
            gated_cl.fetch_add(1, Ordering::Relaxed);
        })
        .run_if((move || lhs_cl.load(Ordering::Relaxed)).or(run_once));
    let mut schedule = builder.build(&mut world);

    // Frame 1 — LHS true; the RHS's single run is consumed here.
    schedule.run(&mut world);
    assert_eq!(load(&gated_runs), 1, "frame 1: `true or run_once` ⇒ body runs");

    // Frame 2 — LHS false; the verdict is now the RHS's alone.
    lhs_verdict.store(false, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        load(&gated_runs),
        1,
        "frame 2: `run_once` was already consumed on frame 1 (eager fold) ⇒ \
         `false or false` ⇒ the body must NOT run. A short-circuiting fold \
         leaves the latch armed and reports 2 here",
    );

    // Frame 3 — nothing re-arms a spent `run_once`.
    schedule.run(&mut world);
    assert_eq!(load(&gated_runs), 1, "frame 3: still spent");
}

// =============================================================================
// D5(3) — no bogus `Changed` burst from the would-be-skipped side
// =============================================================================

/// **D5(3).** The negative half of D5(1): with eager folding, the next frame
/// sees no accumulated window.
///
/// This is the test that catches a lazy fold which *looks* correct — one that
/// short-circuits `run_unsafe` and advances each child's tick snapshot at the
/// moment that child runs, which is precisely `EcsMaster::run_condition`'s rule
/// for a leaf condition. Such a fold passes any assertion about *verdicts on the
/// frame the RHS is reached*; what it gets wrong is the **window** the RHS is
/// reached with.
///
/// Arrangement, all on one `or`:
///
/// * **Frame 1** — a writer mutates `Ke5Health`; the LHS is `true`, so the `or`
///   is already decided. Eager: the RHS (`Changed<Ke5Health>`) runs anyway, sees
///   the change, and its window closes past frame 1.
/// * **Frame 2** — LHS flipped to `false`, writer idle. Eager: the RHS's window
///   is `(frame 1, frame 2]`, which contains no change ⇒ `false` ⇒ the body does
///   not run.
///
/// Under the lazy fold the RHS never ran on frame 1, so on frame 2 its window
/// still opens at its initialisation sentinel and **frame 1's mutation is inside
/// it** — the burst. The body runs a second time and the test is red.
#[test]
fn d5_3_no_bogus_changed_burst_from_the_would_be_skipped_side() {
    let pool = serial_pool();
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[Ke5Health::component_id()]);
    world
        .spawn_one(arch, Ke5Health { hp: 100 })
        .expect("spawn Ke5Health entity");

    let gated_runs = counter();
    let gated_cl = Arc::clone(&gated_runs);
    let lhs_verdict = Arc::new(AtomicBool::new(true));
    let lhs_cl = Arc::clone(&lhs_verdict);
    let should_mutate = Arc::new(AtomicBool::new(true));
    let should_mutate_cl = Arc::clone(&should_mutate);

    let mut builder = ScheduleBuilder::new(pool);
    // Writer runs BEFORE the gated system so the condition — evaluated when the
    // gated system becomes ready — observes the mutation in the SAME frame.
    let writer = builder
        .add_system(move |mut q: Query<Mut<Ke5Health>>| {
            if should_mutate_cl.load(Ordering::Relaxed) {
                for mut h in &mut q {
                    h.hp = h.hp.wrapping_sub(1);
                }
            }
        })
        .key();
    builder
        .add_system(move || {
            gated_cl.fetch_add(1, Ordering::Relaxed);
        })
        .after(writer)
        .run_if(
            (move || lhs_cl.load(Ordering::Relaxed))
                .or(|q: Query<&Ke5Health, Changed<Ke5Health>>| q.iter().count() > 0),
        );
    let mut schedule = builder.build(&mut world);

    // Frame 1 — LHS true (decisive for `or`) AND a real mutation happens.
    schedule.run(&mut world);
    assert_eq!(load(&gated_runs), 1, "frame 1: LHS true ⇒ body runs");

    // Frame 2 — LHS false, writer idle. The verdict is the RHS's alone.
    lhs_verdict.store(false, Ordering::Relaxed);
    should_mutate.store(false, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        load(&gated_runs),
        1,
        "frame 2: the RHS already observed frame 1's mutation (it ran even \
         though the `or` was decided), so its window is empty now ⇒ body must \
         NOT run. A lazy fold's frozen window bursts here and reports 2",
    );

    // Frame 3 — a fresh mutation must still be seen: the gate is tick-driven,
    // not a one-way latch stuck at false.
    should_mutate.store(true, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        load(&gated_runs),
        2,
        "frame 3: a real change ⇒ the RHS fires again",
    );
}

// =============================================================================
// D5(4) — combined access is the union of the children's access
// =============================================================================

/// **D5(4).** Asserted on the composed access set, not on observed behaviour,
/// so a scheduler that happens to serialise cannot hide a missing entry.
///
/// [`Access`]'s bitmask fields are crate-private, so the public probe is
/// [`Access::conflicts_with`]: a probe surface that writes exactly one component
/// conflicts with the composite iff the composite declares a read or write of
/// that component. Each side is probed separately, plus a **negative control**
/// on a third component neither child touches — without it, a composite that
/// wrongly declared `Access::universal()` would pass every positive probe.
#[test]
fn d5_4_combined_access_is_the_union_of_the_children() {
    let mut world = EcsMaster::new();
    world.insert_resource(Ke5Config(1));

    // LHS reads a component; RHS reads a resource. Disjoint axes, so a union
    // that dropped either side is visible.
    let mut combined =
        (|q: Query<&Ke5Health>| q.iter().count() > 0).or(|c: Res<Ke5Config>| c.0 > 0);
    combined.initialize(&mut world);

    let mut probe_lhs = Access::new();
    probe_lhs.add_component_write(Ke5Health::component_id());
    assert!(
        combined.access().conflicts_with(&probe_lhs),
        "the LHS's read of Ke5Health must appear in the composite's access",
    );

    let mut probe_rhs = Access::new();
    probe_rhs.add_resource_write(Ke5Config::resource_id());
    assert!(
        combined.access().conflicts_with(&probe_rhs),
        "the RHS's read of Ke5Config must appear in the composite's access",
    );

    // Negative control: a component neither child names must NOT conflict.
    // This is what makes the two assertions above non-vacuous — it fails if the
    // composite declared a universal (or otherwise over-broad) surface.
    let mut probe_unrelated = Access::new();
    probe_unrelated.add_component_write(Ke5Pos::component_id());
    assert!(
        !combined.access().conflicts_with(&probe_unrelated),
        "the union must be exactly the children's access, not a widened or \
         universal surface — Ke5Pos is touched by neither child",
    );

    // `Not` carries its child's access verbatim, by the same argument.
    let mut negated = not(|q: Query<&Ke5Health>| q.iter().count() > 0);
    negated.initialize(&mut world);
    assert!(
        negated.access().conflicts_with(&probe_lhs),
        "`not(c)` declares c's access",
    );
    assert!(
        !negated.access().conflicts_with(&probe_unrelated),
        "`not(c)` declares NOTHING beyond c's access",
    );
}

// =============================================================================
// D5(5) — tick maintenance is forwarded to both children
// =============================================================================

/// **D5(5).** Each child's last-run tick advances on every reached frame —
/// the mechanism tests D5(2) and D5(3) depend on.
///
/// Driven directly against the composite rather than through a schedule,
/// because the property is about the two tick-maintenance channels
/// (`set_change_ticks`, `check_change_tick`) and both are `System` methods the
/// scheduler calls on the *boxed composite*, never on its interior. The children
/// are reachable only through the composite, so if it does not forward, nothing
/// else will.
///
/// Also pins the *derivation*: a child's new `last_run` is that child's OWN
/// previous `this_run`, exactly as `EcsMaster::run_condition` derives it for a
/// leaf condition — not the composite's `last_run` handed down verbatim.
#[test]
fn d5_5_tick_maintenance_is_forwarded_to_both_children() {
    let mut world = EcsMaster::new();

    let mut combined = (|| true).and(|| false);
    combined.initialize(&mut world);

    // ── set_change_ticks, frame N ──
    combined.set_change_ticks(Tick::new(5), Tick::new(9));
    assert_eq!(
        combined.lhs().meta().this_run(),
        Tick::new(9),
        "the LHS's this_run advanced",
    );
    assert_eq!(
        combined.rhs().meta().this_run(),
        Tick::new(9),
        "the RHS's this_run advanced — a fold that ticks only the side it \
         intends to run leaves this at the initialisation sentinel",
    );

    // ── set_change_ticks, frame N+1: the per-child derivation ──
    combined.set_change_ticks(Tick::new(9), Tick::new(12));
    assert_eq!(
        combined.lhs().meta().last_run(),
        Tick::new(9),
        "the LHS's new last_run is its OWN previous this_run",
    );
    assert_eq!(
        combined.rhs().meta().last_run(),
        Tick::new(9),
        "the RHS's new last_run is its OWN previous this_run",
    );
    assert_eq!(combined.rhs().meta().this_run(), Tick::new(12));

    // ── check_change_tick: the wraparound clamp reaches the children too ──
    // A snapshot older than MAX_CHANGE_AGE would silently flip
    // `Tick::is_newer_than`; the children are invisible to
    // `Schedule::check_change_ticks`, so the composite must clamp them.
    let current = Tick::new(MAX_CHANGE_AGE.wrapping_add(1_000));
    combined.set_change_ticks(Tick::new(1), Tick::new(1));
    combined.check_change_tick(current);
    let oldest_valid = Tick::new(current.get().wrapping_sub(MAX_CHANGE_AGE));
    assert_eq!(
        combined.lhs().meta().last_run(),
        oldest_valid,
        "the LHS's ancient last_run was clamped",
    );
    assert_eq!(
        combined.rhs().meta().last_run(),
        oldest_valid,
        "the RHS's ancient last_run was clamped",
    );

    // `Not` forwards on the same two channels.
    let mut negated = not(|| true);
    negated.initialize(&mut world);
    negated.set_change_ticks(Tick::new(3), Tick::new(4));
    assert_eq!(negated.inner().meta().this_run(), Tick::new(4));
    negated.set_change_ticks(Tick::new(4), Tick::new(6));
    assert_eq!(
        negated.inner().meta().last_run(),
        Tick::new(4),
        "`not`'s child gets the same per-child derivation",
    );
}

// =============================================================================
// Composition — the shapes the chained `.run_if(a).run_if(b)` fold cannot spell
// =============================================================================

/// Nesting works in both directions: an `and` inside an `or`, and a `not`
/// around a combinator. This is KE5's reason to exist beyond the operator
/// itself — `.run_if(a).run_if(b)` can only ever produce a flat conjunction.
#[test]
fn combinators_nest_in_both_directions() {
    let pool = serial_pool();
    let mut world = EcsMaster::new();

    let runs = counter();
    let runs_cl = Arc::clone(&runs);

    let mut builder = ScheduleBuilder::new(pool);
    // (true && false) || !(false)  ==  false || true  ==  true
    builder
        .add_system(move || {
            runs_cl.fetch_add(1, Ordering::Relaxed);
        })
        .run_if((|| true).and(|| false).or(not(|| false)));
    let mut schedule = builder.build(&mut world);
    schedule.run(&mut world);

    assert_eq!(
        load(&runs),
        1,
        "(true and false) or not(false) ⇒ true ⇒ the body runs",
    );
}

/// The verdict truth table, end to end through the real schedule, for all four
/// LHS/RHS assignments of `or` and `and`. Guards against an operator swap that
/// every eagerness test above would happily tolerate.
#[test]
fn or_and_and_produce_the_boolean_truth_tables_end_to_end() {
    for (lhs, rhs, expect_or, expect_and) in [
        (false, false, false, false),
        (false, true, true, false),
        (true, false, true, false),
        (true, true, true, true),
    ] {
        // `or`
        {
            let mut world = EcsMaster::new();
            let runs = counter();
            let runs_cl = Arc::clone(&runs);
            let mut builder = ScheduleBuilder::new(serial_pool());
            builder
                .add_system(move || {
                    runs_cl.fetch_add(1, Ordering::Relaxed);
                })
                .run_if((move || lhs).or(move || rhs));
            let mut schedule = builder.build(&mut world);
            schedule.run(&mut world);
            assert_eq!(
                load(&runs) == 1,
                expect_or,
                "{lhs} or {rhs} must be {expect_or}",
            );
        }
        // `and`
        {
            let mut world = EcsMaster::new();
            let runs = counter();
            let runs_cl = Arc::clone(&runs);
            let mut builder = ScheduleBuilder::new(serial_pool());
            builder
                .add_system(move || {
                    runs_cl.fetch_add(1, Ordering::Relaxed);
                })
                .run_if((move || lhs).and(move || rhs));
            let mut schedule = builder.build(&mut world);
            schedule.run(&mut world);
            assert_eq!(
                load(&runs) == 1,
                expect_and,
                "{lhs} and {rhs} must be {expect_and}",
            );
        }
    }
}
