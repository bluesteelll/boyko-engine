//! UI-ADVANCED rung **A1** — the sink, the four channels, the fused tick
//! (`docs/UI-PLAN-ANIMATION.md` A1, AD5, AD6, AD10, AD11, AD12, AM1, AM2, AM8).
//!
//! Eleven gates, and **every leg owns a red** (A0's standard). Eight of them are
//! runtime legs and live here; the other three are elsewhere for structural
//! reasons stated below.
//!
//! | Leg | Test | Owns red mutation |
//! |---|---|---|
//! | 1 | [`presence_is_running_and_the_reap_ends_it`] | 1 — `ui_tween_reap` skips the remove |
//! | 2 | [`the_tick_bumps_the_sink_on_both_routes`] + [`the_or_arm_is_not_vacuous`] | 2 — `Mut` → `&mut` (a PAIR of edits); 11b — a dense sink with the const-assert suppressed |
//! | 3 | [`a_rested_element_is_silent`] | 3 — delete the all-`None` `continue` AND the `set_if_neq` (a PAIR) |
//! | 4 | [`the_identity_default_has_two_routes`] | 4 — `#[derive(Default)]` |
//! | 5 | [`restarting_a_channel_replaces_and_rewinds_it`] | 5 — the restart resumes at half phase |
//! | 6 | `ui_a1_zero_alloc.rs` (its own binary — `#[global_allocator]`) | 6 — a `Vec::new()` in the tick body |
//! | 7 | [`a_paused_clock_stops_only_the_virtual_flagged_row`] | 7 — swap the clock select's default |
//! | 8 | `miri_a1_tween.rs` (its own binary — `#![cfg(miri)]`-shaped legs) | 8 — the reap drops the entry without clearing it |
//! | 9 | [`the_tick_composes_from_the_sink`] | 9 — base the composition on `UiVisual::default()` |
//! | 10 | [`the_sinks_equality_is_idempotent_under_nan`] | 10 — `#[derive(PartialEq)]` on `UiVisual` |
//! | 11 | the two `const _: () = assert!(…)` blocks in `components.rs` | 11a — `#[component(storage = "dense")]` on `UiVisual` ⇒ `error[E0080]` |
//!
//! # Leg 2 is the ONLY A1 gate that can see the storage decision (AD10)
//!
//! Its first half (a bare `Changed<UiVisual>`) is green under the storage error
//! that makes every animation invisible: MEASURED, a DENSE sink written through
//! AD5's exact `Mut::set_if_neq` is seen by a bare `Changed` (1 row) and not by
//! an `Or` (0 rows). So the second half runs the SAME write through a filter of
//! `ui_pack_inputs!(changed)`'s own shape and asserts the two AGREE.
//!
//! The `Or`'s other arm MUST be a component this fixture never writes — hence
//! [`Untouched`], and hence [`the_or_arm_is_not_vacuous`], which asserts that
//! arm reads zero in the measured window. If the other arm were `Changed` in the
//! frame under test the `Or` would be true THROUGH IT and the half would report
//! success whatever the sink's storage kind is: a gate that cannot fail, inside
//! the gate written to prevent one. A single-element `Or` is not a substitute —
//! it is a different type from the one `ui_pack_inputs!(changed)` expands to.
//!
//! # What every leg here does NOT prove
//!
//! None of these observes `UiRenderGeneration`. `UiVisual` joins
//! `ui_pack_inputs!` at rung **A4**, and `boyko-ui` cannot name a `boyko_render`
//! resource anyway (the dependency runs render → ui). The end-to-end half —
//! the real `ui_render_discovery` over the real filter with `UiVisual` in it —
//! is `boyko_render/tests/ui_a1_sink_reaches_discovery.rs`.

#![cfg(not(miri))]

use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::filter::{Changed, Or};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::*;
use boyko_macros::{Component, Resource};

use boyko_ui::animation::{
    start_tween_offset, start_tween_opacity, start_tween_tint, ui_clock_tick, ui_tween_reap,
    ui_visual_tick, UiClock, UiTweenScratch,
};
use boyko_ui::components::{
    EasingId, TweenOffset, TweenOpacity, TweenTint, UiVisual, TWEEN_FLAG_VIRTUAL_CLOCK,
};

/// One ordinary frame. Below the UI clock's 100 ms clamp, so nothing here is
/// truncated and `dt_real == dt_virtual` whenever the clock is neither paused
/// nor scaled.
const FRAME: Duration = Duration::from_millis(100);

/// A marker the fixtures carry and NEVER write after spawn — leg 2's inert `Or`
/// arm. It exists to make the filter an `Or` at all.
#[derive(Component, Clone, Copy, Debug)]
struct Untouched;

// ───────────────────────── probes ──────────────────────────────────────────

/// Per-frame change-detection readings, appended by the probe systems at the end
/// of every schedule run.
#[derive(Resource, Default)]
struct Probe {
    /// Rows matching a BARE `Changed<UiVisual>` (AM1's route).
    bare: Vec<usize>,
    /// Rows matching `Or<(Changed<Untouched>, Changed<UiVisual>)>` — the shape
    /// `ui_pack_inputs!(changed)` expands to (AM8's route).
    or_shaped: Vec<usize>,
    /// Rows matching `Changed<Untouched>` ALONE — the non-vacuity control for
    /// the arm above.
    arm_only: Vec<usize>,
    /// Rows carrying a `UiVisual` at all, changed or not.
    live_sinks: Vec<usize>,
}

fn probe_bare(q: Query<(), Changed<UiVisual>>, mut p: ResMut<Probe>) {
    let n = q.iter().count();
    p.bare.push(n);
}

#[allow(clippy::type_complexity)]
fn probe_or(q: Query<(), Or<(Changed<Untouched>, Changed<UiVisual>)>>, mut p: ResMut<Probe>) {
    let n = q.iter().count();
    p.or_shaped.push(n);
}

fn probe_arm_only(q: Query<(), Changed<Untouched>>, mut p: ResMut<Probe>) {
    let n = q.iter().count();
    p.arm_only.push(n);
}

fn probe_live(q: Query<&UiVisual>, mut p: ResMut<Probe>) {
    let n = q.iter().count();
    p.live_sinks.push(n);
}

// ───────────────────────── harness ─────────────────────────────────────────

/// A world holding [`Time`], [`UiClock`] and [`UiTweenScratch`] — everything the
/// A1 pair reads — plus the [`Probe`].
fn a1_world() -> EcsMaster {
    let mut world = EcsMaster::new();
    world.insert_resource(Time::default());
    world.insert_resource(UiClock::default());
    world.insert_resource(UiTweenScratch::default());
    world.insert_resource(Probe::default());
    world
}

/// `[ui_clock_tick → ui_visual_tick → ui_tween_reap → probes]`.
///
/// The probes are ordered AFTER the reap so their `Changed` window covers the
/// tick's write and nothing else, and the ordering is SET-free explicit edges —
/// registration order is not a pin.
fn a1_schedule(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let clock = b.add_system(ui_clock_tick).key();
    let tick = b.add_system(ui_visual_tick).after(clock).key();
    let reap = b.add_system(ui_tween_reap).after(tick).key();
    b.add_system(probe_bare).after(reap);
    b.add_system(probe_or).after(reap);
    b.add_system(probe_arm_only).after(reap);
    b.add_system(probe_live).after(reap);
    b.build(world)
}

fn frame(world: &mut EcsMaster, schedule: &mut Schedule, dt: Duration) {
    world.resource_mut::<Time>().advance_with(dt);
    schedule.run(world);
}

/// A bare node carrying only [`Untouched`]: no sink and no channel until a
/// helper puts them there.
fn spawn_node(world: &mut EcsMaster) -> Entity {
    world.run_system(|mut cmds: Commands| cmds.spawn(Untouched).id())
}

/// The live row count of a dense channel's store. `0` when the store was never
/// created, which is the same observable as "no rows".
fn live_count(world: &EcsMaster, id: ComponentId) -> usize {
    world.dense_registry().store(id).map_or(0, |s| s.live_count())
}

fn sink_of(world: &EcsMaster, e: Entity) -> UiVisual {
    *world
        .get_component::<UiVisual>(e)
        .expect("the node carries a UiVisual sink")
}

// ───────────────────────── leg 1 ───────────────────────────────────────────

/// **A1 gate 1 — presence IS running (C2/D9), and the deferred reap ends it.**
///
/// Inserting a `TweenTint` starts the tween: the channel's `live_count()` is 1
/// and the sink appears (the `on_add` hook, which no earlier rung had an owner
/// for). When `elapsed` reaches the duration the tick writes the endpoint
/// EXACTLY and the reap removes the row in the same frame, so `live_count()`
/// returns to **0** — and the sink STAYS, holding the endpoint, because its last
/// value is the node's resting appearance (AM2).
///
/// Red mutation 1 (make the reap skip the remove) reds the `live_count()`
/// return; the endpoint assertion is what makes the leg more than a bookkeeping
/// check.
///
/// **Does NOT prove the removal is visible to a downstream reader in the same
/// frame** — nothing at A1 reads "no row ⇒ at rest"; that consumer arrives at A3.
#[test]
fn presence_is_running_and_the_reap_ends_it() {
    let mut world = a1_world();
    let e = spawn_node(&mut world);
    let mut sched = a1_schedule(&mut world);

    world.run_system(move |mut cmds: Commands| {
        start_tween_tint(&mut cmds, e, 0x0000_0000, 0xFFFF_FFFF, 200.0, EasingId::LINEAR, 0);
    });

    assert_eq!(
        live_count(&world, TweenTint::component_id()),
        1,
        "the helper inserted the channel"
    );
    assert_eq!(
        sink_of(&world, e),
        UiVisual::IDENTITY,
        "the channel's on_add hook materialized the sink at the identity — a channel whose sink \
         is missing is a tween that ticks into nothing"
    );

    // 100 ms of a 200 ms tween: running, not finished.
    frame(&mut world, &mut sched, FRAME);
    assert_eq!(
        live_count(&world, TweenTint::component_id()),
        1,
        "half-way through, the channel is still live"
    );
    assert_ne!(
        sink_of(&world, e).tint_mul,
        0xFFFF_FFFF,
        "half-way through, the sink is NOT at the endpoint — otherwise the frame below proves \
         nothing about completion"
    );

    // The frame that completes it.
    frame(&mut world, &mut sched, FRAME);
    assert_eq!(
        live_count(&world, TweenTint::component_id()),
        0,
        "the reap removed the completed channel IN THE SAME FRAME the tick finished it"
    );
    assert_eq!(
        sink_of(&world, e).tint_mul,
        0xFFFF_FFFF,
        "the endpoint is ASSIGNED at completion, not interpolated to"
    );
    assert_eq!(
        world.resource::<UiTweenScratch>().pending(),
        0,
        "and the completion list was drained, not merely read"
    );
}

// ───────────────────────── leg 2 ───────────────────────────────────────────

/// **A1 gate 2 — the tick bumps the sink's tick, on BOTH routes (AM1 + AM8).**
///
/// A bare `Changed<UiVisual>` sees the animating row, and a filter of
/// `ui_pack_inputs!(changed)`'s own shape —
/// `Or<(Changed<Untouched>, Changed<UiVisual>)>` — sees EXACTLY the same rows,
/// on the animating frames and on the frame after the reap.
///
/// The agreement is the storage gate. MEASURED: the two routes agree for a table
/// sink (1 and 1) and DISAGREE for a dense one (1 and **0**), because the
/// kernel's `Or` overrides none of the dense hooks. This is the only A1 gate that
/// can see AD10, and A4 — where the symptom first appears as a frozen picture —
/// is a rung and a cross-plan dependency away.
///
/// **Does NOT prove the write reaches `UiRenderGeneration`** — see the module
/// header; that is `boyko_render`'s A1 leg and, in full, rung A4.
#[test]
fn the_tick_bumps_the_sink_on_both_routes() {
    let mut world = a1_world();
    let e = spawn_node(&mut world);
    let mut sched = a1_schedule(&mut world);

    world.run_system(move |mut cmds: Commands| {
        start_tween_opacity(&mut cmds, e, 0.0, 1.0, 400.0, EasingId::LINEAR, 0);
    });

    // Frame 1 is discarded: the spawn and the inserts stamped their own ticks,
    // so it cannot separate "the tick wrote" from "the insert wrote".
    for _ in 0..4 {
        frame(&mut world, &mut sched, FRAME);
    }

    let p = world.resource::<Probe>();
    // Frames 2 and 3 animate; frame 4 is the completing frame (400 ms reached),
    // which also writes. All three must be seen on both routes.
    assert_eq!(
        &p.bare[1..4],
        &[1, 1, 1][..],
        "a bare Changed<UiVisual> sees the animating row on every animating frame"
    );
    assert_eq!(
        p.or_shaped[1..4],
        p.bare[1..4],
        "and the discovery filter's OWN SHAPE sees exactly the same rows — a dense sink makes \
         this half read 0 while the half above still reads 1 (AM8, MEASURED)"
    );

    // The frame after the reap: nothing live, nothing written, both routes zero.
    frame(&mut world, &mut sched, FRAME);
    let p = world.resource::<Probe>();
    assert_eq!(p.bare[4], 0, "the frame after the reap writes nothing");
    assert_eq!(
        p.or_shaped[4], 0,
        "and the Or agrees — the routes agree in the negative direction too"
    );
}

/// **Leg 2's non-vacuity control.** The `Or`'s other arm must read ZERO in the
/// window leg 2 measures.
///
/// If [`Untouched`] were `Changed` on a measured frame the `Or` would be true
/// THROUGH IT, and leg 2's second half would report success for a dense sink as
/// happily as for a table one — the exact "gate that cannot fail" shape the leg
/// exists to close. This asserts the arm is inert, and separately that it CAN
/// fire, so "0" means "not written" rather than "not wired".
#[test]
fn the_or_arm_is_not_vacuous() {
    let mut world = a1_world();
    let e = spawn_node(&mut world);
    let mut sched = a1_schedule(&mut world);

    world.run_system(move |mut cmds: Commands| {
        start_tween_opacity(&mut cmds, e, 0.0, 1.0, 400.0, EasingId::LINEAR, 0);
    });
    for _ in 0..4 {
        frame(&mut world, &mut sched, FRAME);
    }
    assert_eq!(
        &world.resource::<Probe>().arm_only[1..4],
        &[0, 0, 0][..],
        "the Or's other arm is INERT across leg 2's measured window — nothing writes Untouched"
    );

    // …and it is wired: writing it makes the same probe read 1 on the next frame.
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(Untouched);
    });
    frame(&mut world, &mut sched, FRAME);
    let p = world.resource::<Probe>();
    assert_eq!(
        p.arm_only[p.arm_only.len() - 1],
        1,
        "the arm is a real term, not a type that never matches — a zero above therefore means \
         NOT WRITTEN, not NOT WIRED"
    );
}

// ───────────────────────── leg 3 ───────────────────────────────────────────

/// **A1 gate 3 — a rested element is silent (AM2).**
///
/// An entity whose tween has completed KEEPS its `UiVisual` row and, on every
/// subsequent still frame, is not `Changed<UiVisual>`. Both halves are asserted
/// together: a live-sink count of ≥ 1 and a changed count of 0 is what "rested
/// but retained" means — the changed count alone is also satisfied by a sink
/// that was removed.
///
/// Red mutation 3 is a PAIR (delete the all-`None` `continue` AND replace
/// `set_if_neq` with the plain deref write), because either edit alone is
/// silent: with the `continue` in place the deref write never reaches a rested
/// row, and with `set_if_neq` in place the deleted `continue` writes nothing
/// (MEASURED `[0,0,0,0]` for each single edit).
#[test]
fn a_rested_element_is_silent() {
    let mut world = a1_world();
    let e = spawn_node(&mut world);
    let mut sched = a1_schedule(&mut world);

    world.run_system(move |mut cmds: Commands| {
        start_tween_tint(&mut cmds, e, 0x0000_0000, 0xFFFF_FFFF, 100.0, EasingId::LINEAR, 0);
    });

    // One frame completes the 100 ms tween and the reap clears it.
    frame(&mut world, &mut sched, FRAME);
    assert_eq!(
        live_count(&world, TweenTint::component_id()),
        0,
        "precondition: the channel is reaped, so the frames below are RESTED frames"
    );

    let settled = world.resource::<Probe>().bare.len();
    for _ in 0..4 {
        frame(&mut world, &mut sched, FRAME);
    }

    let p = world.resource::<Probe>();
    assert_eq!(
        &p.bare[settled..],
        &[0, 0, 0, 0][..],
        "a rested row bumps nothing: the all-None continue fires BEFORE the sink is touched"
    );
    assert!(
        p.live_sinks[settled..].iter().all(|&n| n >= 1),
        "and the sink is RETAINED — its last value is the node's resting appearance, so removing \
         it would snap the element back"
    );
}

// ───────────────────────── leg 4 ───────────────────────────────────────────

/// **A1 gate 4 — the identity default, by TWO routes (AD6).**
///
/// `UiVisual::default()` equals `UiVisual::IDENTITY` field by field, and
/// `IDENTITY`'s four fields are additionally asserted against literals written
/// into this test. Neither route derives from the other, which is what makes the
/// comparison capable of failing — the `default_mode_is_off` precedent.
///
/// **The half A1 originally claimed here cannot be built and is not attempted:**
/// "`UiVisual` does not `#[derive(Default)]`" is not a statement any Rust test
/// can make, because a derived and a hand-written `impl Default` are the SAME
/// trait impl to the type system.
#[test]
fn the_identity_default_has_two_routes() {
    let d = UiVisual::default();
    let i = UiVisual::IDENTITY;

    assert_eq!(d.tint_mul, i.tint_mul);
    assert_eq!(d.opacity, i.opacity);
    assert_eq!(d.offset_px, i.offset_px);
    assert_eq!(d.scale, i.scale);

    // The literals, so the two routes are not merely equal to each other.
    assert_eq!(i.tint_mul, 0xFFFF_FFFF, "no tint");
    assert_eq!(i.opacity, 1.0, "fully opaque — a derived Default gives 0");
    assert_eq!(i.offset_px, [0.0, 0.0], "no offset");
    assert_eq!(i.scale, [1.0, 1.0], "unit scale — a derived Default gives [0, 0]");
}

// ───────────────────────── leg 5 ───────────────────────────────────────────

/// **A1 gate 5 — a restart REPLACES and REWINDS (D9 reason 3).**
///
/// `start_tween_tint` on an entity that already carries one replaces
/// `from`/`to`/`duration` and restarts `elapsed` at 0, at the same dense slot.
/// This is the property rung A3's reversing transition depends on, and it was
/// asserted nowhere.
///
/// The `live_count()` half alone is a KERNEL TAUTOLOGY and is kept only as a
/// companion: every insert route for a dense id on an existing entity goes
/// through `DenseStore::insert_or_replace`, which overwrites the slot in place,
/// so `live_count()` — `column.count() − free.len()` — is arithmetically
/// incapable of moving and no implementation of the helper could have made it.
#[test]
fn restarting_a_channel_replaces_and_rewinds_it() {
    let mut world = a1_world();
    let e = spawn_node(&mut world);
    let mut sched = a1_schedule(&mut world);

    world.run_system(move |mut cmds: Commands| {
        start_tween_tint(&mut cmds, e, 0x0000_0000, 0xFFFF_FFFF, 2000.0, EasingId::LINEAR, 0);
    });
    // 500 ms into a 2 s tween: running, and unmistakably NOT at phase zero.
    for _ in 0..5 {
        frame(&mut world, &mut sched, FRAME);
    }
    let mid = *world
        .get_component::<TweenTint>(e)
        .expect("the channel is live part-way through");
    assert!(
        mid.elapsed > 0.1,
        "precondition: the running row carries a NON-ZERO phase (got {}), or 'rewound' below \
         would be indistinguishable from 'never advanced'",
        mid.elapsed
    );

    world.run_system(move |mut cmds: Commands| {
        start_tween_tint(&mut cmds, e, 0xFF00_0000, 0xFF00_00FF, 2000.0, EasingId::LINEAR, 0);
    });

    let row = *world
        .get_component::<TweenTint>(e)
        .expect("the restarted channel is live");
    assert_eq!(row.from, 0xFF00_0000, "the restart REPLACED `from`");
    assert_eq!(row.to, 0xFF00_00FF, "the restart REPLACED `to`");
    assert_eq!(row.elapsed, 0.0, "and REWOUND the phase to zero");
    assert_eq!(
        live_count(&world, TweenTint::component_id()),
        1,
        "arity one per channel: the replace reused the slot"
    );

    // One frame later the sink reads the NEW `from` side, not a continuation of
    // the old tween.
    frame(&mut world, &mut sched, FRAME);
    let tint = sink_of(&world, e).tint_mul;
    assert_eq!(
        tint >> 24,
        0xFF,
        "the composed value comes from the NEW endpoints (alpha 0xFF), not the old ones"
    );
}

// ───────────────────────── leg 7 ───────────────────────────────────────────

/// **A1 gate 7 — the per-row clock SELECT (D15's opt-in, AD9 (1)).**
///
/// With `Time` paused, a default-clock tween advances and a `virtual`-flagged
/// tween does not. Both rows are in the same world, ticked by the same system on
/// the same frames, so the only thing separating them is bit 0 of `flags`.
///
/// This tests the SELECT, not the fields' arithmetic — `dt_real`'s clamp and
/// `dt_virtual`'s scaling are A0 legs 3 and 4. It is nonetheless the first gate
/// anywhere that `dt_real` has a reader at all: without D15's bit, AD9's rule
/// hands every consumer `dt_virtual` and `dt_real` is a dead datum.
#[test]
fn a_paused_clock_stops_only_the_virtual_flagged_row() {
    let mut world = a1_world();
    let real_row = spawn_node(&mut world);
    let virtual_row = spawn_node(&mut world);
    let mut sched = a1_schedule(&mut world);

    world.run_system(move |mut cmds: Commands| {
        start_tween_opacity(&mut cmds, real_row, 0.0, 1.0, 1000.0, EasingId::LINEAR, 0);
        start_tween_opacity(
            &mut cmds,
            virtual_row,
            0.0,
            1.0,
            1000.0,
            EasingId::LINEAR,
            TWEEN_FLAG_VIRTUAL_CLOCK,
        );
    });
    world.run_system(|mut time: ResMut<Time>| time.pause());

    for _ in 0..3 {
        frame(&mut world, &mut sched, FRAME);
    }

    let clock = *world.resource::<UiClock>();
    assert_eq!(clock.dt_virtual(), 0.0, "precondition: paused ⇒ the virtual delta is zero");
    assert!(
        clock.dt_real() > 0.0,
        "precondition: the field the flagged row must NOT read is LIVE — without this the leg \
         proves nothing about which field was selected"
    );

    assert!(
        sink_of(&world, real_row).opacity > 0.0,
        "the DEFAULT row is on dt_real: a pause-menu fade fades while the game is paused"
    );
    assert_eq!(
        sink_of(&world, virtual_row).opacity, 0.0,
        "the FLAGGED row is on dt_virtual: it pauses with the game"
    );
}

// ───────────────────────── leg 9 ───────────────────────────────────────────

/// **A1 gate 9 — the composition base is `*sink`, not the identity (AD12).**
///
/// The four channels own DISJOINT fields, so composing means *overwrite the
/// fields whose channel is live and carry the rest*. A node whose `TweenOffset`
/// finished at −400 px, then given a `TweenTint`, keeps `offset_px[0] == −400`
/// on the tint's first frame and every frame after.
///
/// This is the first A1 gate that runs TWO channels, and that is why the defect
/// it catches survived eight gates: an identity-base implementation — which
/// silently undoes every finished animation — passes every single-channel leg,
/// and would have surfaced first as an A3 transition resetting a slid-in panel.
#[test]
fn the_tick_composes_from_the_sink() {
    let mut world = a1_world();
    let e = spawn_node(&mut world);
    let mut sched = a1_schedule(&mut world);

    world.run_system(move |mut cmds: Commands| {
        start_tween_offset(&mut cmds, e, [0.0, 0.0], [-400.0, 0.0], 100.0, EasingId::LINEAR, 0);
    });
    frame(&mut world, &mut sched, FRAME);
    assert_eq!(
        sink_of(&world, e).offset_px[0],
        -400.0,
        "precondition: the offset tween finished and the endpoint was assigned"
    );
    assert_eq!(
        live_count(&world, TweenOffset::component_id()),
        0,
        "precondition: and its channel was reaped, so nothing re-writes the field below"
    );

    world.run_system(move |mut cmds: Commands| {
        start_tween_tint(&mut cmds, e, 0x0000_0000, 0xFFFF_FFFF, 1000.0, EasingId::LINEAR, 0);
    });
    for i in 0..3 {
        frame(&mut world, &mut sched, FRAME);
        assert_eq!(
            sink_of(&world, e).offset_px[0],
            -400.0,
            "frame {i} of a LATER tint tween: the finished offset is CARRIED, not reset — from \
             UiVisual::default() it would read 0 and the panel would jump home"
        );
    }
}

// ───────────────────────── leg 10 ──────────────────────────────────────────

/// **A1 gate 10 — the sink's equality is idempotent (AD11).**
///
/// A *plateau* tween (`from == to`, which an author writes whenever a transition
/// targets the state it is already in) over a sink carrying one NaN field is not
/// `Changed<UiVisual>` on any still frame after the first. MEASURED both ways:
/// `[0, 0, 0]` bytewise, `[1, 1, 1]` derived.
///
/// This is the RELEASE-side half of AM1's *"a tick that bumps every frame
/// defeats the render gate as surely as one that never bumps"*. Every
/// `debug_assert!` in this rung compiles out, and the public helpers take author
/// `from`/`to` values, so a NaN is reachable in a shipping build with no kernel
/// bug — and one such row bumps the single global `UiRenderGeneration`, which
/// disarms the per-slot skip for the WHOLE UI, on every frame, forever.
///
/// The duration is long enough that the channel is still LIVE across the
/// measured frames. If it completed, the reap would fire, the row would go
/// all-`None`, gate 3's `continue` would take over and this leg would measure
/// nothing.
#[test]
fn the_sinks_equality_is_idempotent_under_nan() {
    let mut world = a1_world();
    // The sink is spawned DIRECTLY with a NaN — which is how one reaches release:
    // not through the helpers' debug asserts, but through any author write.
    let e = world.run_system(|mut cmds: Commands| {
        let mut e = cmds.spawn(Untouched);
        e.insert(UiVisual {
            tint_mul: 0xFFFF_FFFF,
            opacity: f32::NAN,
            offset_px: [0.0, 0.0],
            scale: [1.0, 1.0],
        });
        e.id()
    });
    let mut sched = a1_schedule(&mut world);

    world.run_system(move |mut cmds: Commands| {
        // A PLATEAU: from == to, so every frame composes the value already held.
        start_tween_tint(&mut cmds, e, 0xFFFF_FFFF, 0xFFFF_FFFF, 10_000.0, EasingId::LINEAR, 0);
    });

    for _ in 0..4 {
        frame(&mut world, &mut sched, FRAME);
    }

    assert_eq!(
        live_count(&world, TweenTint::component_id()),
        1,
        "precondition: the plateau channel is STILL LIVE across the measured frames — if it had \
         completed, the reap would have turned this into gate 3"
    );
    assert!(
        sink_of(&world, e).opacity.is_nan(),
        "precondition: the NaN is still in the sink — it is what the equality has to survive"
    );
    assert_eq!(
        &world.resource::<Probe>().bare[1..4],
        &[0, 0, 0][..],
        "a value-preserving frame does not bump, NaN included: under #[derive(PartialEq)] this \
         reads [1, 1, 1] and the whole UI's per-slot skip is disarmed forever"
    );
}

// ───────────────────────── M4b (§4) ────────────────────────────────────────

/// **Measurement obligation M4b (§4) — the UI clamp is doing something
/// VISIBLE**, recorded here because A1 is the rung that owns it (no tween
/// exists at A0, so half of M4 was unmeasurable at the rung it was first
/// assigned to).
///
/// A running tween's `elapsed` advance across ONE synthetic 2 000 ms frame
/// delta, clamped vs unclamped. The two arms differ only in
/// [`UiClock::set_max_delta`] — same frame, same tween, same system — so the
/// pair isolates the clamp and nothing else.
///
/// | arm | `UiClock::max_delta` | `dt_real` | tween `elapsed` advance |
/// |---|---|---|---|
/// | clamped (default) | 0.1 s | **0.1 s** | **0.1 s** |
/// | unclamped | 1 000 s | **2.0 s** | **2.0 s** |
///
/// against A0's already-recorded `dt_real` pair of 2.0 s / 0.1 s: the tween's
/// advance tracks the clock's field exactly, which is the "visible" half M4
/// could not show. Unclamped, one alt-tab stall runs 2 s of animation in a
/// single frame — every transition shorter than that jumps straight to its end
/// on resume, which reads as a glitch and not as an animation.
///
/// This is a REPORTED COMPARISON, not gate 7's pass/fail on the same subject:
/// gate 7 asserts WHICH FIELD the row selects; this records WHAT THE CLAMP IS
/// WORTH once the field is chosen.
#[test]
fn m4b_the_clamp_is_visible_in_a_tweens_elapsed() {
    /// The alt-tab stall AM6 is about: twenty times the UI's clamp.
    const HITCH: Duration = Duration::from_secs(2);

    fn advance_over_one_hitch(max_delta: Option<f32>) -> (f32, f32) {
        let mut world = a1_world();
        if let Some(m) = max_delta {
            world.resource_mut::<UiClock>().set_max_delta(m);
        }
        let e = spawn_node(&mut world);
        let mut sched = a1_schedule(&mut world);
        world.run_system(move |mut cmds: Commands| {
            // Long enough that the hitch cannot complete it in either arm.
            start_tween_opacity(&mut cmds, e, 0.0, 1.0, 100_000.0, EasingId::LINEAR, 0);
        });
        frame(&mut world, &mut sched, HITCH);
        let dt_real = world.resource::<UiClock>().dt_real();
        let elapsed = world
            .get_component::<TweenOpacity>(e)
            .expect("the tween is still running in both arms")
            .elapsed;
        (dt_real, elapsed)
    }

    let (clamped_dt, clamped_elapsed) = advance_over_one_hitch(None);
    let (unclamped_dt, unclamped_elapsed) = advance_over_one_hitch(Some(1000.0));

    println!(
        "M4b — one 2000 ms frame delta:\n  clamped   (max_delta 0.1 s): dt_real {clamped_dt} s, \
         tween elapsed advance {clamped_elapsed} s\n  unclamped (max_delta 1000 s): dt_real \
         {unclamped_dt} s, tween elapsed advance {unclamped_elapsed} s"
    );

    assert_eq!(clamped_dt, 0.1, "M4b: the clamped arm truncates the hitch to max_delta");
    assert_eq!(clamped_elapsed, 0.1, "M4b: and the tween advances by exactly that");
    assert_eq!(unclamped_dt, 2.0, "M4b: the unclamped arm hands the raw 2 s through");
    assert_eq!(
        unclamped_elapsed, 2.0,
        "M4b: and the tween eats 2 s of animation in ONE frame — every transition shorter than \
         that jumps straight to its end on resume"
    );
}
