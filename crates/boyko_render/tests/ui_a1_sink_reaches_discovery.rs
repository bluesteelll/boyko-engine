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

fn world_with_one_animated_node() -> (EcsMaster, Entity) {
    let mut world = EcsMaster::new();
    world.insert_resource(Time::default());
    world.insert_resource(UiClock::default());
    world.insert_resource(UiTweenScratch::default());
    world.insert_resource(UiRenderGeneration::default());
    world.insert_resource(Readings::default());

    let node = world.run_system(|mut cmds: Commands| cmds.spawn(ComputedRect::default()).id());
    world.run_system(move |mut cmds: Commands| {
        // 400 ms at 100 ms a frame: live across the whole measured window.
        start_tween_opacity(&mut cmds, node, 0.0, 1.0, 400.0, EasingId::LINEAR, 0);
    });
    (world, node)
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
