//! **VG rung R2d-5 — the NARROW framing: the gate rung R2d-6 will flip.**
//!
//! Two batches of [`vb_inst_cull_scene::BATCH_INSTANCES`] instances each, framed so that exactly
//! ONE instance per batch — the one at [`vb_inst_cull_scene::OFFSCREEN_LOCAL_INDEX`], interior to
//! its batch — is wholly outside the frustum while its batch's UNION box straddles it. That is the
//! whole point of the arrangement: the per-BATCH cull rung R2c already ships rejects NOTHING here,
//! so anything this gate observes changing is per-INSTANCE granularity and nothing else.
//!
//! # What THIS rung asserts, and why every number is what the build produces
//!
//! `vb_batch_cull.comp.hlsl` ships with its level-2 `keep` predicate HARDWIRED `true` (rung R2d-3),
//! and `vb_raster.vs.hlsl` reads a survivor list that is therefore the IDENTITY (rung R2d-4). So on
//! THIS build:
//!
//! | field | this rung | rung R2d-6 (armed) |
//! |---|---|---|
//! | `batches` | 2 | 2 — the host still submits both batches |
//! | `visible` | 2 | 2 — level 1 keeps both; their union boxes straddle the frustum |
//! | `inst` | `[3, 3]` | `[2, 2]` — one survivor per batch removed |
//! | `vis` | `0:0,1,2` and `3:3,4,5` | `0:0,2` and `3:3,5` — the culled LOCAL index 1 is gone |
//!
//! `visible` is deliberately NOT the observable here: it counts BATCHES, and no batch is rejected
//! at either rung. `inst` and `vis` are.
//!
//! # `#[ignore]`
//!
//! Needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`,
//! the conventions every windowed test in this crate follows. The margin test below is NOT ignored:
//! it is pure CPU and is what keeps the fixture's teeth from being filed off in CI.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::frustum::instance_visible_after_cull;
use boyko_render::{GeometryLegs, Material, MeshGeometryTableSlot, RenderPath, RenderPathConfig};

mod vb_inst_cull_scene;

use vb_inst_cull_scene::{
    BATCH_COUNT, BATCH_INSTANCES, EXTENT, INSTANCE_COUNT, NARROW, OFFSCREEN_LOCAL_INDEX, WIDE,
};

fn setup(
    commands: Commands,
    meshes: NonSendResMut<Assets<MeshGpu>>,
    materials: ResMut<Assets<Material>>,
    geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    vb_inst_cull_scene::fixture_setup_system(commands, meshes, materials, geo_table, dev, &NARROW);
}

/// **The gate.** Boots the narrow framing with the cull-readback probe armed and asserts every
/// number this build produces.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it to read the per-instance cull's survivor regions"]
fn vb_inst_cull_narrow_reports_the_inert_regions() {
    let probe = vb_inst_cull_scene::probe_in_process("narrow", || {
        let mut app = App::new();
        app.add_plugins(EnginePlugins::window(
            "boyko_engine vb inst cull narrow",
            EXTENT.0,
            EXTENT.1,
        ));
        app.add_startup_system(setup);
        app.insert_resource(RenderPathConfig {
            path: RenderPath::VisibilityBuffer,
            legs: GeometryLegs::Mesh,
        });
        app.run();
    });

    assert_eq!(
        probe.batches, BATCH_COUNT,
        "the fixture must produce exactly {BATCH_COUNT} draw batches (two distinct mesh ids) -- \
         got {:?}. One batch means the two spheres were bucketed together and nothing below says \
         anything about per-batch bases",
        probe.raw
    );
    assert_eq!(
        probe.visible as usize, BATCH_COUNT,
        "level 1 must keep BOTH batches: each batch's UNION box spans from an on-screen instance \
         to the off-screen one, so it straddles the frustum and is not wholly outside any plane. A \
         smaller number here means the per-BATCH cull is rejecting a batch that contains visible \
         geometry -- got {:?}",
        probe.raw
    );
    let mut list = probe.list.clone();
    list.sort_unstable();
    assert_eq!(
        list,
        (0..BATCH_COUNT as u32).collect::<Vec<_>>(),
        "the compacted batch list must name both batches (sorted: the shader appends them with an \
         atomic bump, so their ORDER is a race and only the set is meaningful) -- got {:?}",
        probe.raw
    );

    // ---- the two observables the arming rung flips -------------------------------------------
    assert_eq!(
        probe.inst.as_slice(),
        [BATCH_INSTANCES as u32; BATCH_COUNT].as_slice(),
        "with `keep` hardwired `true` (`vb_batch_cull.comp.hlsl`, rung R2d-3) every instance \
         survives, so each record's post-cull `instanceCount` is still {BATCH_INSTANCES}. Rung \
         R2d-6 makes this [{}, {}]. A value BELOW {BATCH_INSTANCES} on THIS build means the level-2 \
         predicate is not the constant the module claims -- got {:?}",
        BATCH_INSTANCES - 1,
        BATCH_INSTANCES - 1,
        probe.raw
    );
    assert_eq!(probe.drawn_instances() as usize, INSTANCE_COUNT);

    assert_eq!(
        probe.vis.len(),
        BATCH_COUNT,
        "one survivor region per drawn batch -- got {:?}",
        probe.raw
    );
    for (b, (base, members)) in probe.vis.iter().enumerate() {
        assert_eq!(
            *base as usize,
            b * BATCH_INSTANCES,
            "batch {b}'s region base must be the gather's prefix sum -- got {:?}",
            probe.raw
        );
        assert_eq!(
            *members,
            (0..BATCH_INSTANCES as u32).map(|i| base + i).collect::<Vec<_>>(),
            "batch {b}'s survivor region must be the IDENTITY run `base .. base + {BATCH_INSTANCES}` \
             on this build. Rung R2d-6 removes the entry at local index \
             {OFFSCREEN_LOCAL_INDEX} -- got {:?}",
            probe.raw
        );
    }
    assert!(probe.regions_are_identity(), "got {:?}", probe.raw);

    // FIXTURE PROPERTY 1, observed rather than assumed: the second batch's region does not start
    // at 0, so `visible[base + id]` and `visible[id]` are distinguishable expressions.
    assert!(
        probe.vis[1].0 > 0,
        "the second batch must have base_instance > 0 or a dropped `base` term is undetectable -- \
         got {:?}",
        probe.raw
    );
}

/// **The fixture's own teeth, on the CPU — NOT `#[ignore]`d.**
///
/// The GPU gate above asserts INERT numbers, so it stays green even if the fixture stops being able
/// to detect anything. This test is what fails instead: it checks that the narrow framing rejects
/// exactly the interior instance of each batch, that it does so by a MARGIN rather than marginally,
/// that the on-screen instances are inside by their centres rather than by a corner, and that the
/// per-BATCH cull rejects NOTHING — which is the premise the whole rung rests on.
#[test]
fn the_fixture_margins_are_not_marginal() {
    vb_inst_cull_scene::assert_fixture_invariants();

    // ---- which instances the narrow framing rejects -------------------------------------------
    let expected: Vec<u32> = (0..BATCH_COUNT)
        .map(|b| vb_inst_cull_scene::global_instance_id(b, OFFSCREEN_LOCAL_INDEX))
        .collect();
    assert_eq!(
        vb_inst_cull_scene::instance_rejections(&NARROW),
        expected,
        "the narrow framing must reject exactly the instance at local index \
         {OFFSCREEN_LOCAL_INDEX} of each batch"
    );

    // ---- and the per-BATCH cull rejects nothing, which is why the rung exists ------------------
    assert!(
        vb_inst_cull_scene::batch_rejections(&NARROW).is_empty(),
        "the per-BATCH cull must reject NOTHING here: if it rejected a batch, the GPU gate's \
         `inst` change could be attributed to level 1 and the fixture would say nothing about \
         per-INSTANCE granularity"
    );

    // ---- margins: rejected by a wide margin, kept by their centres ----------------------------
    let planes = vb_inst_cull_scene::frustum_planes(&NARROW, EXTENT.0, EXTENT.1);
    let ring = vb_inst_cull_scene::instance_rows();
    for batch in 0..BATCH_COUNT {
        let local_aabb = vb_inst_cull_scene::batch_local_bounds(batch);
        // INFLATED four-fold and still outside ⇒ the rejection has ≥ 3 half-extents of slack, so a
        // small change to the sphere or the framing cannot silently turn it into a keep.
        let inflated = vb_inst_cull_scene::scaled_bounds(local_aabb, 4.0);
        let g = vb_inst_cull_scene::global_instance_id(batch, OFFSCREEN_LOCAL_INDEX) as usize;
        assert!(
            !instance_visible_after_cull(&planes, &ring[g], inflated),
            "batch {batch}'s off-screen instance is only MARGINALLY outside: a 4x-inflated box \
             re-enters the frustum. Move it further out rather than weakening the assertion"
        );
        // Collapsed to their CENTRES and still inside ⇒ the keeps are not corner-grazing.
        let point = vb_inst_cull_scene::scaled_bounds(local_aabb, 0.0);
        for local in 0..BATCH_INSTANCES {
            if local == OFFSCREEN_LOCAL_INDEX {
                continue;
            }
            let g = vb_inst_cull_scene::global_instance_id(batch, local) as usize;
            assert!(
                instance_visible_after_cull(&planes, &ring[g], point),
                "batch {batch}'s local instance {local} is only marginally INSIDE: its centre is \
                 outside the frustum and only its extent reaches in"
            );
        }
    }

    // ---- the control's own premise, so the two framings are checked in one place --------------
    assert!(
        vb_inst_cull_scene::instance_rejections(&WIDE).is_empty(),
        "the wide framing is the narrow test's CONTROL and must contain every instance; with a \
         rejection in it the two runs would differ for two reasons at once"
    );
}
