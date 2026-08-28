//! UI-ADVANCED rung **A1** — the sink, the four channels, the fused tick
//! (`docs/UI-PLAN-ANIMATION.md` A1, AD5, AD6, AD10, AD11, AD12, AM1, AM2, AM8).
//!
//! Eleven gates. **Ten legs own a red; leg 5 does not, and that is recorded
//! rather than papered over** — A0's standard, honestly applied. Eight of the
//! eleven are runtime legs and live here; the other three are elsewhere for
//! structural reasons stated below.
//!
//! | Leg | Test | Owns red mutation |
//! |---|---|---|
//! | 1 | [`presence_is_running_and_the_reap_ends_it`] | 1 — `ui_tween_reap` skips the remove |
//! | 2 | [`the_tick_bumps_the_sink_on_both_routes`] | 2 — `Mut` → `&mut` (a PAIR of edits); 11b — a dense sink with the const-assert suppressed |
//! | 3 | [`a_rested_element_is_silent`] | 3 — delete the all-`None` `continue` AND the `set_if_neq` (a PAIR) |
//! | 4 | [`the_identity_default_has_two_routes`] | 4 — `#[derive(Default)]` |
//! | 5 | [`restarting_a_channel_replaces_and_rewinds_it`] | **NONE — the property is `NOT PROVED`, see below.** The substitute (the restart resumes at half phase) does red this test, but it edits the ONE insert path a fresh start and a restart share |
//! | 6 | `ui_a1_zero_alloc.rs` (its own binary — `#[global_allocator]`) | 6 — a `Vec::with_capacity(64)` behind a `black_box` in the tick body OR the reap's loop body |
//! | 7 | [`a_paused_clock_stops_only_the_virtual_flagged_row`] | 7 — swap the clock select's default |
//! | 8 | `miri_a1_tween.rs` (its own binary — `#![cfg(miri)]`-shaped legs) | 8 — the reap drops the entry without clearing it (the DRAIN is leg 8's gate; the stale-replay survival beside it is smoke — see that file) |
//! | 9 | [`the_tick_composes_from_the_sink`] | 9 — base the composition on `UiVisual::default()` |
//! | 10 | [`the_sinks_equality_is_idempotent_under_nan`] | 10 — `#[derive(PartialEq)]` on `UiVisual` |
//! | 11 | the two `const _: () = assert!(…)` blocks in `components.rs` | 11a — `#[component(storage = "dense")]` on `UiVisual` ⇒ `error[E0080]` |
//!
//! [`the_or_arm_is_not_vacuous`] is deliberately absent from that table.
//! It is leg 2's harness **CONTROL**, not a gate, and it must NOT be counted
//! toward "every leg owns a red": it reads only `Changed<Untouched>`, so no
//! mutation of the sink, of the tick, or of `UiVisual`'s storage can reach it.
//! Its own doc says so; the table used to credit it as a co-owner of leg 2's red
//! and that was wrong.
//!
//! # Leg 5's property is NOT PROVED, and no third substitute is offered
//!
//! The plan's mutation 5 — make `start_tween_tint` early-return when the channel
//! is already present — **cannot be written**. The helper's only world contact is
//! `&mut Commands`, and `Commands` declares NO component or resource access at all
//! (`boyko_ecs/src/ecs/core/system/params/commands.rs:389-399`, whose own comment
//! says exactly that), so it cannot branch on presence. The substitute that
//! shipped edits the single insert path a fresh start and a restart share, so it
//! falsifies the FRESH path and leaves the restart axis untouched.
//!
//! There is no third substitute because there is no defect to catch: the replace
//! goes through `DenseStore::insert_or_replace`, which overwrites the slot in
//! place, so no implementation of `start_tween_tint` can get a fresh insert right
//! and a restart wrong. That is the same tautology the rung already recorded for
//! the `live_count()` half, now extended to `from`/`to`/`elapsed`. This test stays
//! as a REGRESSION assertion over the shipped behaviour. An unprovable property
//! stated as proved is worse than one stated as open.
//!
//! Two further tests here guard the degenerate-`duration_ms` class rather than a
//! numbered A1 gate — [`a_degenerate_duration_creates_no_row`] (release-only) and
//! [`a_nan_inv_duration_completes_instead_of_running_forever`].
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
    EasingId, TweenOffset, TweenOpacity, TweenOpacityBundle, TweenTint, UiVisual,
    TWEEN_FLAG_VIRTUAL_CLOCK,
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
///
/// **2026-08-27: that tautology extends to `from`/`to`/`elapsed` too, so this is
/// a REGRESSION assertion and NOT a falsified gate.** Leg 5 owns no red — see the
/// module header. Do not "fix" that by inventing a third substitute mutation: the
/// one the plan specified is unwritable (`Commands` declares no reads, so the
/// helper cannot branch on presence) and the one that shipped edits the single
/// insert path a fresh start and a restart share.
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

// ───────────────────────── the immortal row (F9) ───────────────────────────

/// **A degenerate `duration_ms` creates NO ROW — the entry-point refusal.**
///
/// `inv_duration` is `1000.0 / duration_ms`, and every `debug_assert!` guarding
/// that division compiles out. MEASURED 2026-08-27 in release: each of `+inf`,
/// `-inf`, `-0.0` and any negative finite `duration_ms` produced a row that
/// returns `Some(t)` FOREVER — never completing, never entering `done`, never
/// reachable by the reap. **By the render-gate criterion — how many frames
/// bump `set_if_neq` — the negative finite shape is the worst of the four**: the
/// sink DIVERGES (−0.25, −0.5, −0.75, −1.0, −1.25 over five
/// 100 ms frames of a `0.0 → 0.25` tween; `-49.999977` over the original 50-frame
/// `0.0 → 1.0` run) and bumps `set_if_neq` on EVERY frame, which at rung A4 is
/// the whole UI's per-slot repaint skip disarmed permanently — not "a wrong
/// picture on one node".
///
/// **`NaN` is refused too but is NOT one of the immortal four**, and saying
/// otherwise is what let this gate's NaN arm pass vacuously — see the section on
/// the pre-tick reading below.
///
/// `+0.0` and the denormals are deliberately NOT in this list: MEASURED, they
/// yield `inv_duration = +inf`, so `t = inf` completes on the first frame with a
/// non-zero delta and the endpoint is assigned. They are benign, and
/// [`a_zero_or_denormal_duration_snaps_to_the_endpoint`] is the gate that holds
/// them on the accepting side. **`-0.0` is NOT benign and IS in this list** —
/// it yields `-inf`, so the sink is pinned at `-inf` from frame 1 on and the row is
/// immortal like the other three. It is NOT the worst: by the criterion above a
/// constant `-inf` bumps `set_if_neq` ZERO times after the first frame, tying it
/// with `+inf` and `-inf`. The worst by that criterion is the negative finite
/// shape, stated above.
///
/// # The row's absence is asserted BEFORE the tick, and that is load-bearing
///
/// MEASURED 2026-08-27, release, with the guard's `return` deleted: a NaN
/// duration yields `inv_duration = NaN`, so `t` is NaN, and `advance`'s `t < 1.0`
/// spelling puts a NaN `t` on the COMPLETING side — the row completes on frame 1,
/// assigns its endpoint, and the reap removes it. **A NaN duration is therefore
/// NOT immortal under the shipped `advance`**, and a post-frame `live_count == 0`
/// is satisfied by "the row was created and reaped" exactly as it is by "no row
/// was ever created". The earlier spelling of this gate looked only after the
/// frame and used a `0.0 → 1.0` fixture, so its NaN arm asserted `live_count == 0`
/// (true — reaped) and `sink == IDENTITY` (true — the endpoint `1.0` IS
/// `IDENTITY.opacity`): **the NaN arm could not fail.** It reddened only from
/// `+inf`, the second iteration.
///
/// Two changes fix that, and both are needed:
/// 1. the row count is read after the commands apply and BEFORE any tick, so
///    "never created" is the only reading that passes; and
/// 2. the endpoint is `0.25`, which is NOT `UiVisual::IDENTITY.opacity` (`1.0`),
///    and the sink is asserted ABSENT rather than equal to the identity — so
///    "no sink" and "a sink that completed" are different readings.
///
/// # What each refused shape does WITHOUT the guard
///
/// MEASURED 2026-08-27, release, 5 frames of 100 ms, a `0.0 → 0.25` opacity
/// tween, guard's `return` deleted:
///
/// | `duration_ms` | `inv_duration` | rows over 5 frames | sink |
/// |---|---|---|---|
/// | `NaN` | `NaN` | 1 → **0** | completes to `0.25` on frame 1 |
/// | `+inf` | `0` | 1,1,1,1,1 | frozen at `from` (`0.0`) forever |
/// | `-inf` | `-0` | 1,1,1,1,1 | frozen at `from` (`0.0`) forever |
/// | `-100.0` | `-10` | 1,1,1,1,1 | **diverges**: −0.25, −0.5, −0.75, −1.0, −1.25 |
/// | `-0.0` | `-inf` | 1,1,1,1,1 | **`-inf`** on frame 1 and every frame after |
///
/// So five of the six degenerate shapes are refused, four of those five are
/// genuinely immortal, and the NaN member is refused at the door even though
/// `advance` would also survive it — the two defences OVERLAP on NaN and that is
/// recorded rather than claimed as coverage.
///
/// **Release-only**, because [`invalid_tween_duration`] keeps a
/// `debug_assert!(false, …)` so an authoring mistake still names itself loudly in
/// a debug build. That makes this test INVISIBLE to a debug `cargo test` run —
/// the debug and release binaries have different test SETS, and only the release
/// one carries this gate.
///
/// Red mutation: delete the `return` from the guard in `tween_helpers!` ⇒ EVERY
/// one of the five arms reds on the pre-tick `live_count`, the NaN arm included.
///
/// # The five arms are TWO mechanisms plus one shape caught by both
///
/// MEASURED 2026-08-27, one conjunct of the guard dropped at a time:
///
/// | mutation | `NaN` | `+inf` | `-inf` | `-100.0` | `-0.0` |
/// |---|---|---|---|---|---|
/// | drop `is_finite()` | **LEAK** | **LEAK** | refused | refused | refused |
/// | drop `is_sign_positive()` | refused | refused | refused | **LEAK** | **LEAK** |
///
/// `is_finite()` is what `NaN` and `+inf` gate (`f32::NAN` is bits `0x7fc00000`,
/// so `is_sign_positive()` returns **true** for it); `is_sign_positive()` is what
/// `-100.0` and `-0.0` gate; and **`-inf` leaks under NEITHER** — both conjuncts
/// catch it, so its arm has zero discriminating power against either natural
/// weakening of the predicate. This is NOT five independent covers, and the
/// `⇒ EVERY one of the five arms reds` above must not be read as five. It is
/// two, plus one redundant against these two mutations. `-inf`'s arm stays
/// because it IS independently gateable by a contrived predicate — that is the
/// "one fix away" structure, not vacuity — and saying so here is cheaper than
/// having the next reader infer five covers from five rows.
#[cfg(not(debug_assertions))]
#[test]
fn a_degenerate_duration_creates_no_row() {
    for (label, duration_ms) in [
        ("NaN", f32::NAN),
        ("+inf", f32::INFINITY),
        ("-inf", f32::NEG_INFINITY),
        ("-100.0 (negative finite)", -100.0f32),
        ("-0.0 (negative zero)", -0.0f32),
    ] {
        let mut world = a1_world();
        let e = spawn_node(&mut world);
        let mut sched = a1_schedule(&mut world);

        world.run_system(move |mut cmds: Commands| {
            // The endpoint is 0.25 and NOT `UiVisual::IDENTITY.opacity` (1.0), so
            // a completed tween and an absent sink are DIFFERENT readings.
            start_tween_opacity(&mut cmds, e, 0.0, 0.25, duration_ms, EasingId::LINEAR, 0);
        });

        // BEFORE the tick. `run_system` has applied the commands, so a row that
        // the helper created exists NOW — and one created here and reaped in
        // frame 1 is indistinguishable from one never created if the only
        // reading is taken afterwards. That is precisely how the NaN arm used
        // to pass.
        assert_eq!(
            live_count(&world, TweenOpacity::component_id()),
            0,
            "duration_ms = {label}: the helper must refuse AT THE DOOR — the commands have been \
             applied and no row may exist yet. This is the reading the NaN arm needs: after a \
             frame, a NaN row has already completed and been reaped and reads 0 either way"
        );
        assert!(
            world.get_component::<UiVisual>(e).is_none(),
            "duration_ms = {label}: with no row, the channel's on_add hook never ran, so the node \
             carries NO UiVisual at all — an absence `unwrap_or(IDENTITY)` cannot distinguish \
             from a sink that happens to hold the identity"
        );

        frame(&mut world, &mut sched, FRAME);

        assert_eq!(
            live_count(&world, TweenOpacity::component_id()),
            0,
            "duration_ms = {label}: and a frame does not conjure the row either"
        );
        assert!(
            world.get_component::<UiVisual>(e).is_none(),
            "duration_ms = {label}: nothing wrote a sink — under the unguarded helper this reads \
             a frozen, diverging or -inf opacity"
        );
    }
}

/// **An over-ceiling duration is ACCEPTED and never completes** — the disclosure
/// in `invalid_tween_duration`'s doc, made falsifiable.
///
/// `elapsed` (an `f32` accumulating `+= dt`) saturates at `2^19 s` = 6.068 days
/// at 60 Hz, so any `duration_ms` above `5.24288e8` yields a row that can never
/// reach `t = 1.0`. This gate does not wait 6 days; it pins the OBSERVABLE:
/// the row is still live after N frames and the sink changed on every one of
/// them, which is the `set_if_neq` bump the A4 repaint skip cannot survive.
///
/// It exists so the guard's "this class is NOT closed" disclosure cannot rot: if
/// a future rung adds an upper bound at or below `f32::MAX`, this test reds and
/// the doc must be rewritten in the same edit.
///
/// **It reds on a bound at the DOOR and nothing else.** The disclosure is a
/// property of the SYSTEM, and a second termination condition can be added
/// downstream, where nothing here looks: MEASURED 2026-08-28, `&& *elapsed <
/// 3_600.0` on `advance`'s completion test — "no tween runs longer than an hour"
/// — left this test, this binary and all 1427 tests of both crates at EXIT=0
/// while the disclosure was false verbatim. That half is
/// `the_termination_condition_is_pinned_to_the_disclosure`'s
/// (`tests/ui_a1_source_census.rs`).
///
/// # The DATUM, and why the observables alone were not enough
///
/// The three observables above — row live, sink moving, bare `Changed` firing —
/// are all DOWNSTREAM of the duration, and every one of them is produced just as
/// faithfully by a duration the door quietly changed as by the one the caller
/// passed. They refute a bound shaped as a REFUSAL and nothing else.
///
/// MEASURED 2026-08-28, one line after the guard in `tween_helpers!` —
/// `let duration_ms = duration_ms.min(1e4);`, a 10-second ceiling and the most
/// natural way to actually write a cap:
///
/// ```text
/// cargo test -p boyko-ui --release --test ui_a1_tween  ⇒ EXIT=0, 14/14, this test "... ok"
/// cargo test -p boyko-ui --all-targets --release       ⇒ EXIT=0, 342 passed, 0 failed
/// ```
///
/// After that edit no accepted `duration_ms` is over the ceiling at all — every
/// tween completes inside ~625 frames — so the disclosure this gate exists to
/// protect had become FALSE while the gate exited 0. The bound was extremely
/// weak: `FRAME` is 100 ms and `FRAMES` is 5, so `elapsed` reaches only 0.5 s,
/// and a clamp anywhere from ~500 ms up to `f32::MAX` (the true `5.24288e8` ms
/// ceiling included) left all three observables intact.
///
/// | clamp | this test |
/// |---|---|
/// | `.min(5.24288e8)` — exactly the true ceiling | GREEN, 14/14 |
/// | `.min(1e4)` — 10 s | GREEN, 14/14, and 342/342 crate-wide |
/// | `.min(5e1)` — 50 ms | RED, naming `duration_ms = 1e10` |
///
/// So the gate also asserts the DATUM: the row's stored `inv_duration` must be
/// bit-exactly `1000.0 / duration_ms` recomputed from the value passed IN. That
/// is the one number the door produces from the duration. Re-measured 2026-08-28,
/// all four bounds red:
///
/// | bound | this test | names |
/// |---|---|---|
/// | `.min(5.24288e8)` | RED, EXIT=101 | `1e10`, `0x36000000` vs `0x33d6bf95` |
/// | `.min(1e4)` | RED, EXIT=101 | `1e10`, `0x3dcccccd` vs `0x33d6bf95` |
/// | `.min(5e1)` | RED, EXIT=101 | `1e10`, `0x41a00000` vs `0x33d6bf95` |
/// | `&& duration_ms < 1e20` | RED, EXIT=101 | `1e30`, on the live-row assertion |
///
/// and the three refusal-shaped bounds the previous round proved stayed red on
/// the live-row assertion: `< 5.24288e8` names `1e10`, `< 1e20` names `1e30`,
/// `< 1e38` names `f32::MAX`.
///
/// # ⚠️ What the datum assertion pins is THREE POINTS, not a class
///
/// It used to say the stored reciprocal "moves under every transformation —
/// clamp, floor, round, scale". **That is false, and the counterexample is a
/// transformation of exactly the shape the disclosure names.** MEASURED
/// 2026-08-28, one line after the guard in `tween_helpers!`:
///
/// ```text
/// let duration_ms = if duration_ms > 5.0e8 && duration_ms <= 1.0e9 { 1.0e4 } else { duration_ms };
/// cargo test -p boyko-ui --all-targets --release --no-fail-fast  ⇒ EXIT=0, 342 passed
/// ```
///
/// All three arms are preserved bit-exactly, so all three datum assertions pass —
/// while every accepted duration in `(5.24288e8, 1e9]`, squarely inside the
/// disclosed class, now completes in ten seconds. A gate that samples three points
/// refutes a transformation that MOVES one of those three points, and nothing
/// more; the disclosure says *every*, and an open class has no finite sample.
///
/// What this assertion actually pins, stated at its true strength: **at
/// `1e10`, `1e30` and `f32::MAX`, the door stores the reciprocal of the value
/// passed in, bit for bit.** That refutes every GLOBAL transformation (the three
/// `.min` rows above, which all move `1e10`) and every refusal at or below
/// `f32::MAX`. It does not refute a transformation that leaves those three floats
/// alone.
///
/// The class itself is closed by
/// `the_termination_condition_is_pinned_to_the_disclosure`
/// (`tests/ui_a1_source_census.rs`), which pins the normalized SOURCE of `advance`
/// and of the `tween_helpers!` `$start` body — so a second termination condition
/// anywhere in either, sub-range or not, reds without anyone having sampled the
/// right float. The two are complementary and neither subsumes the other: the
/// source pin cannot show that the pinned source BEHAVES as the disclosure says,
/// and this gate cannot show that no unsampled duration behaves differently.
///
/// The datum assertion is coupled to the row storing the RECIPROCAL. A rung that
/// stores the duration itself must rewrite this assertion — and the disclosure —
/// in the same edit, which is the intended coupling and not an accident.
///
/// MEASURED by exact `f32` simulation 2026-08-27: the `elapsed` ceiling is
/// `524288 s` (`2^19`) at `dt = 1/60 s` AND at 16 ms, and `2097152 s` (`2^21`)
/// at the 100 ms [`UiClock`] clamp — so even the SMALLEST of the three durations
/// used here (`1e10`, which needs `elapsed = 1e7 s`) is over the ceiling by 5x at
/// the most generous `dt`, and this is NOT a "the test is just too short"
/// artifact.
///
/// # Why THREE durations, and why the largest is `f32::MAX`
///
/// The disclosure this gate protects covers an open-ended CLASS — *every*
/// accepted `duration_ms` above the ceiling — and a gate that pins ONE point
/// only refutes a bound that happens to fall below that point. MEASURED
/// 2026-08-27 against the single-point (`1e10`) spelling: adding
/// `&& duration_ms < 1e20` to the guard in `tween_helpers!` left this test
/// GREEN, the whole `ui_a1_tween` binary green 14/14, and
/// `cargo test -p boyko-ui --all-targets --release` at EXIT=0 over 52 targets.
/// An upper bound WAS added and the gate that exists to notice said nothing.
///
/// The third arm is what closes the REFUSAL half of the class rather than
/// sampling it: `f32::MAX` is the largest finite `f32`, so **every** upper bound
/// strictly below it refuses that arm, wherever the bound is put. `1e10` and
/// `1e30` are not redundant — they LOCALIZE the bound, so a red names roughly
/// where the new ceiling sits instead of only that one exists. The argument is
/// sound for `duration_ms < X` and says **nothing** about `duration_ms.min(X)`;
/// the other half is the datum assertion's, below.
///
/// Runs in BOTH profiles: every value here is ACCEPTED, so
/// `invalid_tween_duration`'s `debug_assert!(false, …)` is never reached.
///
/// Red mutation: add a second conjunct — `duration_ms < 5.24288e8`,
/// `duration_ms < 1e20`, or any other finite upper bound — to the guard in
/// `tween_helpers!` ⇒ at least the `f32::MAX` arm is refused and the FIRST
/// assertion (the row exists at all) reds. Second red mutation: a GLOBAL clamp
/// instead of a refusal — `let duration_ms = duration_ms.min(X);` after the
/// guard, for any finite `X` ⇒ the SECOND assertion (the stored `inv_duration`)
/// reds, at every `X` including the true ceiling, because a global `min` moves
/// `1e10`. A clamp that leaves all three sampled floats alone does NOT red this
/// and is not meant to — see the section above, and
/// `tests/ui_a1_source_census.rs` for the gate that does.
#[test]
fn an_over_ceiling_duration_is_accepted_and_never_completes() {
    /// Three points spanning the accepted over-ceiling class, from just over the
    /// 100 ms-clamp ceiling to the top of the finite `f32` range.
    ///
    /// * `1e10` — over the 60 Hz `elapsed` ceiling (`5.24288e8` ms) by 19x and
    ///   over the 100 ms-clamp ceiling by 5x. "115 days" in the plan's prose.
    /// * `1e30` — a decade-scale step above it, to localize a bound.
    /// * `f32::MAX` — the largest finite `f32`; no finite upper bound can admit
    ///   it, which is what makes this a CLASS gate rather than three samples.
    const OVER_CEILING_MS: [(&str, f32); 3] =
        [("1e10", 1e10), ("1e30", 1e30), ("f32::MAX", f32::MAX)];
    const FRAMES: usize = 5;

    for (label, duration_ms) in OVER_CEILING_MS {
        let mut world = a1_world();
        let e = spawn_node(&mut world);
        let mut sched = a1_schedule(&mut world);

        world.run_system(move |mut cmds: Commands| {
            start_tween_opacity(&mut cmds, e, 0.0, 1.0, duration_ms, EasingId::LINEAR, 0);
        });

        // (c) ACCEPTED — the contrast with `a_degenerate_duration_creates_no_row`,
        // where this very reading is 0 for all five refused shapes.
        assert_eq!(
            live_count(&world, TweenOpacity::component_id()),
            1,
            "duration_ms = {label}: an over-ceiling duration is ACCEPTED at the door — the guard \
             tests only finiteness and the sign bit, so a row exists. If this reads 0, a \
             REFUSAL-shaped upper bound at or below {label} was added; rewrite \
             `invalid_tween_duration`'s 'what this predicate does NOT close' section in this \
             edit. (The `f32::MAX` arm reds under every finite REFUSAL bound; the two smaller \
             arms say where it sits. A bound shaped as a CLAMP is invisible here and is caught \
             by the datum assertion immediately below — see this test's doc.)"
        );

        // The DATUM, not a consequence of it. Everything else in this test is
        // downstream of the duration: the row stays live, the sink keeps moving.
        // Those observables are produced just as faithfully by a CLAMPED duration
        // as by an unclamped one, so they close only the refusal-shaped half of
        // the class. `inv_duration` is the number the door actually stored, so
        // this catches every transformation that MOVES one of the three floats
        // below — which is every global one. It does NOT catch a transformation
        // that leaves all three alone: MEASURED 2026-08-28, a sub-range rewrite
        // over `(5.0e8, 1.0e9]` kept all three bit-exact and the crate green. The
        // class is closed by `the_termination_condition_is_pinned_to_the_
        // disclosure` in `tests/ui_a1_source_census.rs`, which pins the SOURCE of
        // the predicate instead of sampling its outputs.
        let stored = world
            .get_component::<TweenOpacity>(e)
            .expect("the accepted row exists — the assertion above just proved it")
            .inv_duration;
        let want = 1000.0f32 / duration_ms;
        assert_eq!(
            stored.to_bits(),
            want.to_bits(),
            "duration_ms = {label}: the row stored inv_duration = {stored:e} (bits {:#010x}), but \
             the door's own arithmetic on the value PASSED IN gives {want:e} (bits {:#010x}). \
             Something between the guard and the insert transformed the duration. A CLAMP is the \
             natural shape — MEASURED 2026-08-28, one line `let duration_ms = \
             duration_ms.min(1e4);` after the guard left this whole binary green 14/14 and \
             `boyko-ui --all-targets --release` green at 342 passed, while every accepted \
             duration was then UNDER the ceiling and `invalid_tween_duration`'s 'what this \
             predicate does NOT close' disclosure had become false. A clamp anywhere between \
             500 ms and f32::MAX (the true 5.24288e8 ms ceiling included) was invisible to the \
             live-row and sink-bump assertions below. Rewrite that disclosure in this edit",
            stored.to_bits(),
            want.to_bits()
        );

        let mut seen = [0.0f32; FRAMES];
        for (i, slot) in seen.iter_mut().enumerate() {
            frame(&mut world, &mut sched, FRAME);
            *slot = sink_of(&world, e).opacity;
            // (a) still live, every frame — never completes, so never enters
            // `done` and the reap can never reach it.
            assert_eq!(
                live_count(&world, TweenOpacity::component_id()),
                1,
                "duration_ms = {label}, frame {i}: the row is STILL live. `elapsed` would have \
                 to reach {} s for `t` to reach 1.0, and it saturates four orders of magnitude \
                 short of that — but read that as the reason the row is immortal GIVEN the \
                 duration the row actually holds AND GIVEN that `t < 1.0` is the only way to \
                 stop, not as a claim about the duration passed in. A transformation of the \
                 duration makes both true again at a new value (that half is the datum \
                 assertion's, above), and a SECOND termination condition in `advance` makes the \
                 sentence false outright while this assertion still passes — that half is \
                 `tests/ui_a1_source_census.rs`'s",
                duration_ms / 1000.0
            );
        }

        // (b) the sink changed on EVERY frame — the `set_if_neq` bump.
        for i in 1..FRAMES {
            assert_ne!(
                seen[i],
                seen[i - 1],
                "duration_ms = {label}, frame {i}: the sink must DIFFER from the previous frame \
                 — that difference is the `set_if_neq` bump, and an immortal row emits it \
                 forever. Values: {seen:?}"
            );
        }
        assert_eq!(
            &world.resource::<Probe>().bare[..FRAMES],
            &[1usize; FRAMES][..],
            "duration_ms = {label}: the bump is not merely a value change — a bare \
             `Changed<UiVisual>` reader sees this row on EVERY frame, permanently. That is what \
             the A4 per-slot repaint skip cannot survive, and it is strictly worse than the \
             REFUSED `+inf`, whose constant `t = 0` bumps ZERO times after the first frame"
        );
    }
}

/// **A `+0.0` or denormal `duration_ms` SNAPS to the endpoint — it is accepted.**
///
/// The companion to [`a_degenerate_duration_creates_no_row`], and the gate that
/// pins the entry guard's SPELLING rather than merely its existence.
///
/// `1000.0 / +0.0` is `+inf`, so `t = elapsed * inf` is `+inf` on the first frame
/// with a non-zero delta, `t < 1.0` is false, and the channel completes with its
/// endpoint ASSIGNED — the same one-frame snap a `1e-40` duration gets. A node
/// authored with a zero duration must therefore end up AT its target, not
/// silently unanimated.
///
/// MEASURED 2026-08-27, release, `0.0 → 0.25` opacity: with the guard spelled
/// `duration_ms > 0.0`, `+0.0` produced **no row and no `UiVisual` at all** —
/// `0.0f32 > 0.0` is false — while `1e-40` produced a row that completed to
/// `0.25` on frame 1. The two shapes are behaviourally identical (`inv_duration`
/// is `+inf` for both) and the predicate separated them.
///
/// `-0.0` is on the OTHER side and must stay there: it yields `-inf`, and the
/// sink reads `-inf` on frame 1 and forever after. `is_sign_positive()` is the
/// spelling that admits `+0.0` and refuses `-0.0`; `> 0.0` refuses both and
/// `>= 0.0` admits both.
///
/// Runs in BOTH profiles — every duration here is ACCEPTED, so
/// [`invalid_tween_duration`]'s `debug_assert!(false, …)` is never reached and
/// this gate is not release-only the way its companion is.
///
/// Red mutation: restore the guard to `duration_ms.is_finite() && duration_ms >
/// 0.0` ⇒ the `+0.0` arm reds on the row count. (The denormal arms stay green
/// under it — they are what made the regression invisible.)
#[test]
fn a_zero_or_denormal_duration_snaps_to_the_endpoint() {
    for (label, duration_ms) in [
        ("+0.0", 0.0f32),
        ("denormal 1e-40", 1e-40f32),
        ("f32::MIN_POSITIVE", f32::MIN_POSITIVE),
    ] {
        let mut world = a1_world();
        let e = spawn_node(&mut world);
        let mut sched = a1_schedule(&mut world);

        world.run_system(move |mut cmds: Commands| {
            start_tween_opacity(&mut cmds, e, 0.0, 0.25, duration_ms, EasingId::LINEAR, 0);
        });

        assert_eq!(
            live_count(&world, TweenOpacity::component_id()),
            1,
            "duration_ms = {label}: ACCEPTED — the helper created the row. Under a `> 0.0` guard \
             the +0.0 arm reads 0 here and the node is silently never animated"
        );
        assert_eq!(
            sink_of(&world, e).opacity,
            1.0,
            "duration_ms = {label}: the channel's on_add hook materialized the sink at the \
             identity, before any tick"
        );

        frame(&mut world, &mut sched, FRAME);

        assert_eq!(
            sink_of(&world, e).opacity,
            0.25,
            "duration_ms = {label}: the endpoint is ASSIGNED on the first frame with a non-zero \
             delta — a zero-duration tween SNAPS to its target, it is not a no-op"
        );
        assert_eq!(
            live_count(&world, TweenOpacity::component_id()),
            0,
            "duration_ms = {label}: and the row completed, so the reap reached it — the snap \
             leaves nothing immortal behind"
        );
    }
}

/// **A NaN `inv_duration` that bypassed the entry point still completes** — the
/// `t < 1.0` spelling, as defence in depth.
///
/// The row is hand-built and inserted directly, so
/// [`a_degenerate_duration_creates_no_row`]'s guard is not on this path at all:
/// this is the residual `TweenXBundle` route, and it is what the completion
/// test's spelling covers.
///
/// A NaN `t` is unordered against both `1.0` comparisons. `t >= 1.0` puts it on
/// the RUNNING side (immortal); `t < 1.0` puts it on the COMPLETING side, so the
/// endpoint is assigned and the reap can reach the row. MEASURED free — 14.002 vs
/// 14.008 ns per 4-channel row.
///
/// It covers NaN and NOTHING ELSE: a non-positive or infinite `inv_duration`
/// still yields a finite `t < 1.0` forever, which is why the whole class is
/// refused at the door instead.
///
/// Red mutation: revert the completion test to `if t >= 1.0 { None } else
/// { Some(t) }` ⇒ the row survives the frame and this reads `live_count == 1`.
#[test]
fn a_nan_inv_duration_completes_instead_of_running_forever() {
    let mut world = a1_world();
    let e = spawn_node(&mut world);
    let mut sched = a1_schedule(&mut world);

    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(TweenOpacityBundle {
            tween: TweenOpacity {
                from: 0.0,
                to: 1.0,
                elapsed: 0.0,
                inv_duration: f32::NAN,
                easing: EasingId::LINEAR,
                flags: 0,
                _pad: [0; 2],
            },
        });
    });
    assert_eq!(
        live_count(&world, TweenOpacity::component_id()),
        1,
        "precondition: the hand-built row is live, so the frame below measures its COMPLETION \
         rather than its absence"
    );

    frame(&mut world, &mut sched, FRAME);

    assert_eq!(
        live_count(&world, TweenOpacity::component_id()),
        0,
        "a NaN `t` lands on the COMPLETING side of `t < 1.0`, so the row was finished and reaped \
         — under `t >= 1.0` it runs forever and is never reachable by the reap"
    );
    assert_eq!(
        sink_of(&world, e).opacity,
        1.0,
        "and completing means the endpoint was ASSIGNED — under `t >= 1.0` the sink instead reads \
         the NaN that `lerp1(from, to, NaN)` produces"
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
