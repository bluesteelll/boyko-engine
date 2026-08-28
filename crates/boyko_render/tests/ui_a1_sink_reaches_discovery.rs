//! UI-ADVANCED rung **A1**, the half `boyko-ui` structurally cannot assert:
//! does the animation sink's write reach the RENDER path?
//! (`docs/UI-PLAN-ANIMATION.md` A1 gate 2, AD10, AM8; `docs/OPEN-QUESTIONS.md`.)
//!
//! `boyko-ui` names no render crate — the dependency runs `boyko-render →
//! boyko-ui` and `boyko_render/Cargo.toml` states the acyclicity as a rule — so
//! A1's own gate 2 can only run a filter of the discovery filter's SHAPE. This
//! file runs the real `ui_render_discovery`, the real `UiRenderGeneration` and
//! the real `Or` machinery in the crate that owns them, against a world driven
//! by the real `ui_visual_tick`.
//!
//! # Two claims, and they point in opposite directions on purpose
//!
//! 1. **[`the_sink_is_seen_through_a_real_or_next_to_a_real_pack_input`]** — the
//!    tick's `Mut::set_if_neq` write on a TABLE sink IS visible through an `Or`
//!    that also carries a genuine pack input (`Changed<ComputedRect>`), which is
//!    exactly the term `ui_pack_inputs!(changed)` grows at rung A4. This is the
//!    end-to-end form of AM8: a DENSE sink reads **0** here while a bare
//!    `Changed` still reads 1, and the symptom is a frozen picture with nothing
//!    red anywhere.
//! 2. **[`a1_folds_nothing_into_the_pack_yet`]** — and TODAY the real
//!    `ui_render_discovery` does **not** bump on a frame whose only change is the
//!    sink, because `UiVisual` is not a member of `ui_pack_inputs!` until A4.
//!    That is A4's disarmed gate stated as an assertion, and it is the reason
//!    A1 owes the golden pins no movement: if a pin moved at this rung, that
//!    would be a regression, not a re-bless.
//!
//! Claim 2 is what makes claim 1 non-trivial: the sink reaches the render path
//! through the FILTER's shape and not yet through the pack, so the two together
//! say precisely how much of the seam A1 has built.
//!
//! CPU-only: no GPU, no window.

#![cfg(not(miri))]

use std::time::Duration;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::filter::{Changed, Or};
use boyko_ecs::prelude::*;
use boyko_macros::Resource;

use boyko_render::{ui_render_discovery, UiRenderGeneration};
use boyko_ui::animation::{
    start_tween_opacity, ui_clock_tick, ui_tween_reap, ui_visual_tick, UiClock, UiTweenScratch,
};
use boyko_ui::components::{ComputedRect, EasingId, UiVisual};

const FRAME: Duration = Duration::from_millis(100);

/// Per-frame readings taken AFTER the discovery system on every frame.
#[derive(Resource, Default)]
struct Readings {
    /// Rows matching `Or<(Changed<ComputedRect>, Changed<UiVisual>)>` — the term
    /// `ui_pack_inputs!(changed)` grows at A4, spelled here against a real pack
    /// input.
    or_with_sink: Vec<usize>,
    /// Rows matching `Changed<ComputedRect>` ALONE — the non-vacuity control.
    /// A non-zero reading in the measured window would make the `Or` above true
    /// through the WRONG arm and the gate incapable of failing.
    pack_input_only: Vec<usize>,
    /// `UiRenderGeneration` as the frame ended.
    generation: Vec<u64>,
}

#[allow(clippy::type_complexity)]
fn read_or(
    q: Query<(), Or<(Changed<ComputedRect>, Changed<UiVisual>)>>,
    control: Query<(), Changed<ComputedRect>>,
    generation: Res<UiRenderGeneration>,
    mut out: ResMut<Readings>,
) {
    let n = q.iter().count();
    let c = control.iter().count();
    let g = generation.generation;
    out.or_with_sink.push(n);
    out.pack_input_only.push(c);
    out.generation.push(g);
}

/// One node carrying a real pack input ([`ComputedRect`]) and one opacity tween
/// of the caller's duration.
fn world_with_one_animated_node_ms(duration_ms: f32) -> (EcsMaster, Entity) {
    let mut world = EcsMaster::new();
    world.insert_resource(Time::default());
    world.insert_resource(UiClock::default());
    world.insert_resource(UiTweenScratch::default());
    world.insert_resource(UiRenderGeneration::default());
    world.insert_resource(Readings::default());

    let node = world.run_system(|mut cmds: Commands| cmds.spawn(ComputedRect::default()).id());
    world.run_system(move |mut cmds: Commands| {
        start_tween_opacity(&mut cmds, node, 0.0, 1.0, duration_ms, EasingId::LINEAR, 0);
    });
    (world, node)
}

fn world_with_one_animated_node() -> (EcsMaster, Entity) {
    // 400 ms at 100 ms a frame: live across the whole measured window.
    world_with_one_animated_node_ms(400.0)
}

/// `[ui_clock_tick → ui_visual_tick → ui_tween_reap → ui_render_discovery → read_or]`.
fn schedule(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let clock = b.add_system(ui_clock_tick).key();
    let tick = b.add_system(ui_visual_tick).after(clock).key();
    let reap = b.add_system(ui_tween_reap).after(tick).key();
    let disc = b.add_system(ui_render_discovery).after(reap).key();
    b.add_system(read_or).after(disc);
    b.build(world)
}

/// `[read_or → ui_clock_tick → ui_visual_tick → ui_tween_reap → ui_render_discovery]`
/// — the SAME five systems as [`schedule`], the SAME world, and the ONE thing
/// changed is where the reader sits.
///
/// `ui_render_discovery` is kept here even though nothing reads its generation in
/// the ordering test: dropping it would make the two schedules differ in TWO
/// ways, and then a difference in the readings could no longer be attributed to
/// the reader's position alone.
fn schedule_reader_first(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let rd = b.add_system(read_or).key();
    let clock = b.add_system(ui_clock_tick).after(rd).key();
    let tick = b.add_system(ui_visual_tick).after(clock).key();
    let reap = b.add_system(ui_tween_reap).after(tick).key();
    b.add_system(ui_render_discovery).after(reap);
    b.build(world)
}

fn frame(world: &mut EcsMaster, sched: &mut Schedule) {
    world.resource_mut::<Time>().advance_with(FRAME);
    sched.run(world);
}

/// **Claim 1 — the sink is visible through a REAL `Or` alongside a REAL pack
/// input.**
///
/// The tick's write on the animating frames is seen; the frame after the reap is
/// not. The control arm (`Changed<ComputedRect>`) reads zero throughout, so the
/// `Or` is true through the SINK's arm and nothing else — without that control
/// this test would report success for a dense sink exactly as happily.
#[test]
fn the_sink_is_seen_through_a_real_or_next_to_a_real_pack_input() {
    let (mut world, node) = world_with_one_animated_node();
    let mut sched = schedule(&mut world);

    // Frame 1 is discarded: the spawn and the inserts stamped their own ticks.
    for _ in 0..5 {
        frame(&mut world, &mut sched);
    }

    let r = world.resource::<Readings>();
    assert_eq!(
        &r.pack_input_only[1..5],
        &[0, 0, 0, 0][..],
        "the control arm is INERT across the measured window — nothing writes ComputedRect, so a \
         hit on the Or below is the SINK's arm and not this one"
    );
    assert_eq!(
        &r.or_with_sink[1..4],
        &[1, 1, 1][..],
        "the fused tick's set_if_neq write on a TABLE sink IS seen through the real Or — a DENSE \
         sink reads 0 here (AM8, MEASURED) and every animation renders nothing"
    );
    assert_eq!(
        r.or_with_sink[4], 0,
        "and the frame after the reap is silent, so the 1s above are writes and not a filter that \
         matches everything"
    );

    assert!(
        world.get_component::<UiVisual>(node).is_some(),
        "the sink is retained after the reap (AM2)"
    );
}

/// **The ordering axis, which AD10's const-assert does not cover.**
///
/// AD10 hardens the STORAGE axis at compile time. The ORDERING axis has the
/// identical silent-frozen-picture symptom and is guarded by nothing, so it is
/// guarded here. 20 frames, one tween live across all of them, the same fixture
/// in both arms — only the reader's position differs.
#[test]
fn the_reader_must_be_ordered_after_the_tick_or_every_write_is_lost() {
    // 100 s at 100 ms a frame: live across all 20 frames in both arms.
    let (mut wa, _) = world_with_one_animated_node_ms(100_000.0);
    let mut sa = schedule(&mut wa);
    for _ in 0..20 {
        frame(&mut wa, &mut sa);
    }
    let after = wa.resource::<Readings>().or_with_sink.clone();
    let after_ctl = wa.resource::<Readings>().pack_input_only.clone();

    let (mut wb, _) = world_with_one_animated_node_ms(100_000.0);
    let mut sb = schedule_reader_first(&mut wb);
    for _ in 0..20 {
        frame(&mut wb, &mut sb);
    }
    let before = wb.resource::<Readings>().or_with_sink.clone();
    let before_ctl = wb.resource::<Readings>().pack_input_only.clone();

    println!("reader AFTER  the tick: or={after:?}\n  control={after_ctl:?}");
    println!("reader BEFORE the tick: or={before:?}\n  control={before_ctl:?}");

    assert_eq!(
        after_ctl[1..20].iter().sum::<usize>(),
        0,
        "control: the Or's pack-input arm is inert across the ANIMATING window, so every hit \
         counted there is the sink's"
    );
    assert_eq!(
        before_ctl[1..20].iter().sum::<usize>(),
        0,
        "control, reader-first arm"
    );
    // 19, not 20, and the window is the SAME `[1..20]` the two controls above
    // prove inert. Frame 1's hit is real but is NOT attributable to the sink:
    // the spawn stamps the pack-input components in that frame too, so the `Or`
    // is true through BOTH arms there and a `sum() == 20` would credit the sink
    // with a hit the control cannot exclude. Asserting the 19 frame-by-frame is
    // also strictly stronger than asserting their total, and it mirrors the
    // reader-first arm's `&[0; 19]` below.
    assert_eq!(
        &after[1..20],
        &[1; 19][..],
        "reader AFTER the tick sees the sink write on every one of the 19 animating frames the \
         control proves the pack-input arm is inert for — so every one of these hits is the \
         sink's, and none is the spawn's"
    );
    assert_eq!(
        after[0], 1,
        "frame 1 hits too, but through BOTH Or arms (the spawn stamps the pack inputs), so it is \
         asserted separately and credited to neither arm alone"
    );
    assert_eq!(
        &before[1..20],
        &[0; 19][..],
        "reader BEFORE the tick sees NOTHING after frame 1 — a write stamped at frame N's \
         this_run is above frame N's reader (it has not happened) and at-or-below the EXCLUSIVE \
         lower bound of frame N+1's (last_run, this_run] window. The repaint is LOST, not late."
    );
    assert_eq!(
        before.iter().sum::<usize>(),
        1,
        "and the single hit is frame 1's out-of-schedule spawn/insert stamp, not an animating frame"
    );
}

/// **Claim 2 — A1 folds NOTHING into the pack yet, and the golden pins must not
/// move.**
///
/// `UiVisual` is not a member of `ui_pack_inputs!` until rung A4, so the real
/// `ui_render_discovery` holds its generation across a frame whose only change is
/// the sink. The control asserts the discovery is WIRED — a real pack-input write
/// does bump it — so a held generation means "the sink is not a member", not
/// "the system never ran".
#[test]
fn a1_folds_nothing_into_the_pack_yet() {
    let (mut world, node) = world_with_one_animated_node();
    let mut sched = schedule(&mut world);

    for _ in 0..4 {
        frame(&mut world, &mut sched);
    }
    let r = world.resource::<Readings>();
    let held = r.generation[1];
    assert_eq!(
        &r.generation[1..4],
        &[held, held, held][..],
        "three animating frames, and UiRenderGeneration HOLDS: A1 adds no pack input, so no \
         golden image can move at this rung"
    );

    // …and the discovery is wired: a real pack-input write bumps it.
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(node).insert(ComputedRect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 });
    });
    frame(&mut world, &mut sched);
    let r = world.resource::<Readings>();
    assert_eq!(
        r.generation[r.generation.len() - 1],
        held + 1,
        "a genuine pack-input change DOES bump — so the held generation above means the sink is \
         not a member, not that the system never ran"
    );
}
