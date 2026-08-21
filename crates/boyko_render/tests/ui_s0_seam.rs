//! UI-ADVANCED rung S0 — the two-phase seam: the observer, G0-2, G0-3, G0-5
//! (`docs/UI-PLAN-SPRITES.md`; the architect's 2026-08-21 WorldView ruling).
//!
//! Every test here is **device-free**: a bare `EcsMaster`, no `RhiContext`, no
//! graphics type. Phase 1 of [`UiUploadSystem`]'s two-phase `run_dispatcher`
//! (gate → gather → pack into the staging box) is driven through
//! `EcsMaster::run_system_once` — the sanctioned dispatcher-solo mint — and
//! Phase 2 returns at its `RhiContext` projection because none is registered.
//!
//! * **The observer** — the re-pointed S0 item 7: one hardcoded panel, Phase 1
//!   packs it into the staging box; the staged records are asserted by value
//!   (scale folding, z-order, count). Cheaper than a windowed rung AND
//!   unit-testable.
//! * **G0-2** — the structural skip, asserted on the COMMAND CENSUS: over 10
//!   consecutive static frames the seam's counters (component probes, packs)
//!   record ZERO work — a census, not a timing delta.
//! * **G0-3** — the packed-count return: one mutation ⇒ the next dispatch
//!   packs, the count and the repacked rows are observable off the system, and
//!   the dispatch after that skips again.
//! * **G0-5** — the SEAM GATE, signature half: Phase 1's signature names no
//!   `!Send`/graphics type and Phase 2's names no world type, pinned by fn-
//!   pointer coercions that stop compiling if either signature grows the other
//!   phase's type. (The call-site half — re-fusing the phases fails to
//!   compile — is the trybuild fixture in `tests/ui_s0_seam_fusion/`.)

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling spawned `Entity` handles out of the `Send + Sync` one-shot system
// closure. Not engine code — compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::dispatcher_token::WorldView;
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_render::error::GpuColumnError;
use boyko_render::{
    ui_render_discovery, RhiContext, UiFramePlan, UiInstance, UiOrtho, UiRenderGeneration,
    UiUploadSystem,
};
use boyko_rhi_vulkan::swapchain::FrameWriteToken;
use boyko_ui::components::{ComputedClip, ComputedRect, StackIndex, UiBackground, UiRoot};

// ───────────────────────── shared plumbing ─────────────────────────────────

/// Builds the observer world: one root panel with two children, distinct stack
/// indices (child2 UNDER child1 by stack, so the z-sort is observable) and a
/// clip on child1. Returns the world and the child entities.
fn build_panel_world() -> (EcsMaster, Entity) {
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let root = {
            let mut e = cmds.spawn(ComputedRect { x: 10.0, y: 20.0, w: 200.0, h: 100.0 });
            e.insert(UiBackground { color: 0xFF10_2030, ..UiBackground::default() });
            e.insert(StackIndex(0));
            e.insert(UiRoot);
            e.id()
        };
        // child1: painted LAST (stack 2), carries a clip.
        {
            let mut e = cmds.spawn(ComputedRect { x: 20.0, y: 30.0, w: 50.0, h: 40.0 });
            e.insert(UiBackground { color: 0xFF40_5060, ..UiBackground::default() });
            e.insert(ComputedClip { x: 20.0, y: 30.0, w: 50.0, h: 40.0 });
            e.insert(StackIndex(2));
            e.set_parent(root);
        }
        // child2: emitted after child1 in DFS order but painted UNDER it
        // (stack 1) — the z-sort must reorder them in the staging box.
        {
            let mut e = cmds.spawn(ComputedRect { x: 60.0, y: 30.0, w: 30.0, h: 30.0 });
            e.insert(UiBackground { color: 0xFF70_8090, ..UiBackground::default() });
            e.insert(StackIndex(1));
            e.set_parent(root);
        }
        *probe.lock().expect("probe") = Some(root);
    });
    let root = sink.lock().expect("probe").expect("root spawned");
    (world, root)
}

/// One discovery schedule (the single production bump site).
fn discovery_schedule(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    b.add_system(ui_render_discovery);
    b.build(world)
}

/// Runs discovery frames until the generation holds for two consecutive
/// frames (the spawn itself is a change), then dispatches the upload system
/// once so the settled generation is PACKED and the gate is armed.
fn settle(world: &mut EcsMaster, schedule: &mut Schedule, sys: &mut UiUploadSystem) {
    let mut settled = 0;
    for _ in 0..8 {
        let before = world.resource::<UiRenderGeneration>().generation;
        schedule.run(world);
        if world.resource::<UiRenderGeneration>().generation == before {
            settled += 1;
            if settled == 2 {
                break;
            }
        } else {
            settled = 0;
        }
    }
    assert_eq!(settled, 2, "discovery must go quiet after the spawn settles");
    world.run_system_once(sys);
    assert!(!sys.staged().is_empty(), "the settle dispatch packed the panel");
}

// ───────────────────────── the observer (S0 item 7, re-pointed) ────────────

/// The S0 observer: Phase 1 alone, device-free, bare `EcsMaster`. The panel's
/// three nodes land in the staging box packed (scale-folded, premultiplied)
/// and z-sorted painter's-order — asserted by value, not by a window.
#[test]
fn observer_phase1_packs_the_panel_device_free() {
    let (mut world, _root) = build_panel_world();
    let mut schedule = discovery_schedule(&mut world);

    // scale 2.0 so the logical→physical fold is observable in the records.
    let mut sys = UiUploadSystem::new(2.0);
    let mut settled = 0;
    for _ in 0..8 {
        let before = world.resource::<UiRenderGeneration>().generation;
        schedule.run(&mut world);
        if world.resource::<UiRenderGeneration>().generation == before {
            settled += 1;
            if settled == 2 {
                break;
            }
        } else {
            settled = 0;
        }
    }
    assert_eq!(settled, 2, "discovery must go quiet after the spawn settles");

    world.run_system_once(&mut sys);
    let staged = sys.staged();
    assert_eq!(staged.len(), 3, "the packed-count: root + two children");

    // Painter's order after the z-sort: root (stack 0), child2 (stack 1),
    // child1 (stack 2) — DFS emitted child1 BEFORE child2, so equality here
    // proves the sort ran, not just the gather.
    assert_eq!(staged[0].min_px, [20.0, 40.0], "root at logical (10,20) × scale 2");
    assert_eq!(staged[0].size_px, [400.0, 200.0], "root 200×100 × scale 2");
    assert_eq!(staged[1].min_px, [120.0, 60.0], "child2 (stack 1) paints second");
    assert_eq!(staged[2].min_px, [40.0, 60.0], "child1 (stack 2) paints last");
    // child1's clip folded to a physical-px AABB (min.xy, max.xy).
    assert_eq!(staged[2].clip, [40.0, 60.0, 140.0, 140.0], "child1's own clip packs");
}

// ───────────────────────── G0-2 ────────────────────────────────────────────

/// G0-2: the structural skip, asserted on the COMMAND CENSUS. Ten consecutive
/// static dispatches after the settle: the probe census and the pack census
/// both record ZERO — the gate returns before ONE component is probed.
#[test]
fn g0_2_static_frames_record_zero_census() {
    let (mut world, _root) = build_panel_world();
    let mut schedule = discovery_schedule(&mut world);
    let mut sys = UiUploadSystem::new(1.0);
    settle(&mut world, &mut schedule, &mut sys);

    let probes_before = sys.probes();
    let repacks_before = sys.repacks();
    for _ in 0..10 {
        schedule.run(&mut world); // discovery: nothing changed, no bump
        world.run_system_once(&mut sys); // the gate must skip BEFORE the gather
    }
    assert_eq!(
        sys.probes().wrapping_sub(probes_before),
        0,
        "G0-2: a static frame issues ZERO component probes (gate ahead of the gather)"
    );
    assert_eq!(
        sys.repacks().wrapping_sub(repacks_before),
        0,
        "G0-2: a static frame executes ZERO packs"
    );
}

// ───────────────────────── G0-3 ────────────────────────────────────────────

/// G0-3: the packed-count return. One mutation ⇒ the next dispatch packs
/// (census +1, the count observable off the system, the repacked row carrying
/// the NEW value), and the dispatch after that skips again.
#[test]
fn g0_3_one_mutation_one_repack_count_returned() {
    let (mut world, root) = build_panel_world();
    let mut schedule = discovery_schedule(&mut world);
    let mut sys = UiUploadSystem::new(1.0);
    settle(&mut world, &mut schedule, &mut sys);

    // Mutate ONE pack input once.
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(root).insert(ComputedRect { x: 15.0, y: 25.0, w: 200.0, h: 100.0 });
    });
    schedule.run(&mut world); // discovery bumps exactly once

    let repacks_before = sys.repacks();
    world.run_system_once(&mut sys);
    assert_eq!(
        sys.repacks().wrapping_sub(repacks_before),
        1,
        "the changed frame executes exactly ONE pack"
    );
    assert_eq!(sys.staged().len(), 3, "the packed-count is returned and observable");
    assert_eq!(
        sys.staged()[0].min_px,
        [15.0, 25.0],
        "the repacked row carries the mutated rect (the count is not a stale re-serve)"
    );

    // The frame after: static again — the gate re-arms on the new generation.
    schedule.run(&mut world);
    let probes_before = sys.probes();
    world.run_system_once(&mut sys);
    assert_eq!(
        sys.probes().wrapping_sub(probes_before),
        0,
        "the frame after the repack skips again (zero probes)"
    );
}

// ───────────────────────── G0-5 (signature half) ───────────────────────────

/// G0-5, the SEAM GATE's signature half: each phase's exact signature is
/// pinned by an fn-pointer coercion. Phase 1 names NO `!Send`/graphics type
/// (a `WorldView` in, a count out); Phase 2 names NO world type (the
/// `RhiContext`, the packed rows, the ortho, the write proof). Re-fusing the
/// seam by widening either signature with the other phase's type stops this
/// test compiling. The call-site half — holding both borrows at once fails
/// borrowck — is `tests/ui_s0_seam_fusion/refused_refusion.rs`.
#[test]
fn g0_5_seam_signatures_do_not_cross() {
    // Phase 1 (device-free): world view in, packed count out.
    let _phase1: fn(&mut UiUploadSystem, &WorldView<'_>) -> usize =
        UiUploadSystem::gather_into_staging;
    // Phase 2 (exclusive): the !Send context in, no WorldView anywhere.
    let _phase2: fn(
        &mut RhiContext,
        &[UiInstance],
        UiOrtho,
        &FrameWriteToken,
    ) -> Result<UiFramePlan, GpuColumnError> = UiUploadSystem::upload_staging;
}
