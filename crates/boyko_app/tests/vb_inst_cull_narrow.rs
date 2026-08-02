//! **VG rung R2d-6 — the NARROW framing: THE GATE, armed.**
//!
//! Two batches of [`vb_inst_cull_scene::BATCH_INSTANCES`] instances each, framed so that exactly
//! ONE instance per batch — the one at [`vb_inst_cull_scene::OFFSCREEN_LOCAL_INDEX`], interior to
//! its batch — is wholly outside the frustum while its batch's UNION box straddles it. That is the
//! whole point of the arrangement: the per-BATCH cull rung R2c ships rejects NOTHING here, so
//! anything this gate observes changing is per-INSTANCE granularity and nothing else.
//!
//! # What THIS rung asserts, and what moved
//!
//! Rung R2d-6 replaced `vb_batch_cull.comp.hlsl`'s level-2 `keep` — hardwired `true` since rung
//! R2d-3 — with the instance's own world box against the six pushed planes. `vb_raster.vs.hlsl`
//! has read the survivor list since rung R2d-4, so the compaction now reaches the image:
//!
//! | field | rung R2d-5 (inert) | THIS rung (armed) |
//! |---|---|---|
//! | `batches` | 2 | 2 — the host still submits both batches |
//! | `visible` | 2 | 2 — level 1 keeps both; their union boxes straddle the frustum |
//! | `inst` | `[3, 3]` | **`[2, 2]`** — one survivor per batch removed |
//! | `vis` | `0:0,1,2` and `3:3,4,5` | **`0:0,2` and `3:3,5`** — the culled LOCAL index 1 is gone |
//!
//! `visible` is deliberately NOT the observable here: it counts BATCHES, and no batch is rejected
//! at either rung. `inst` and `vis` are.
//!
//! # The survivor regions are no longer the IDENTITY, and that is asserted rather than implied
//!
//! [`vb_inst_cull_scene::OFFSCREEN_LOCAL_INDEX`] is INTERIOR to its batch by const assertion, so a
//! compacted region (`base, base + 2`) holds a value that a stale identity run does not
//! (`base + 2` at compacted slot 1). The gate below asserts
//! `!probe.regions_are_identity()` for exactly that reason: without it, a raster reading LAST
//! frame's list, or a cull that wrote the identity and merely lowered the record word, would still
//! satisfy every other assertion here.
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

/// Instances of each batch that survive the narrow framing: every local index except
/// [`OFFSCREEN_LOCAL_INDEX`]. Derived from the fixture's own constants rather than typed in, so a
/// fixture edit moves the expectation with the geometry.
const SURVIVORS_PER_BATCH: usize = BATCH_INSTANCES - 1;

/// Batch `batch`'s survivor region under the armed cull: its surviving instances' GLOBAL ring
/// indices in COMPACTION order — ascending local index with [`OFFSCREEN_LOCAL_INDEX`] removed,
/// which is the order the shader's `k` cursor writes them in.
fn expected_region(batch: usize) -> Vec<u32> {
    (0..BATCH_INSTANCES)
        .filter(|local| *local != OFFSCREEN_LOCAL_INDEX)
        .map(|local| vb_inst_cull_scene::global_instance_id(batch, local))
        .collect()
}

/// **The gate.** Boots the narrow framing with the cull-readback probe armed and asserts every
/// number this build produces.
///
/// RENAMED at rung R2d-6 (`..._reports_the_inert_regions`): the regions are no longer inert, and a
/// test whose name states the opposite of what it asserts is worse than no name.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it to read the per-instance cull's survivor regions"]
fn vb_inst_cull_narrow_drops_the_offscreen_instance() {
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

    // ---- the two observables the arming rung flipped ------------------------------------------
    assert_eq!(
        probe.inst.as_slice(),
        [SURVIVORS_PER_BATCH as u32; BATCH_COUNT].as_slice(),
        "the armed level-2 predicate (`vb_batch_cull.comp.hlsl`, rung R2d-6) rejects the instance \
         at local index {OFFSCREEN_LOCAL_INDEX} of each batch, so each record's post-cull \
         `instanceCount` -- the word `vkCmdDrawIndexedIndirect` fetches -- is \
         {SURVIVORS_PER_BATCH}. [{BATCH_INSTANCES}, {BATCH_INSTANCES}] means the predicate is \
         still the constant `true` rung R2d-5 shipped: armed in name only, and byte-identical on \
         every on-screen golden. Anything BELOW {SURVIVORS_PER_BATCH} means it is rejecting \
         geometry the camera can see -- and `vb_inst_cull_wide.rs`, the control, is where that \
         reads unambiguously -- got {:?}",
        probe.raw
    );
    assert_eq!(
        probe.drawn_instances() as usize,
        INSTANCE_COUNT - BATCH_COUNT,
        "exactly one instance per batch must be removed from the {INSTANCE_COUNT} the fixture \
         spawns -- got {:?}",
        probe.raw
    );

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
            "batch {b}'s region base must be the gather's prefix sum -- the base is a property of \
             the SUBMITTED batch, not of the cull, so it does not move when instances are \
             rejected -- got {:?}",
            probe.raw
        );
        assert_eq!(
            *members,
            expected_region(b),
            "batch {b}'s survivor region must be its surviving GLOBAL ids in compaction order, \
             with the entry for local index {OFFSCREEN_LOCAL_INDEX} gone. The stored values are \
             ORIGINAL ring indices, never compacted slots (`vb_raster.vs.hlsl`'s INVARIANT \
             R2d-EXPORT-IS-GLOBAL): a region reading `base, base+1` would be the compacted SLOT \
             numbers, and one reading `base, base+1, base+2` would be rung R2d-5's identity run \
             -- got {:?}",
            probe.raw
        );
    }
    assert!(
        !probe.regions_are_identity(),
        "every survivor region is still the IDENTITY run. Because the culled instance is INTERIOR \
         to its batch (`vb_inst_cull_scene`'s const-asserted fixture property 2), a compacted \
         region CANNOT be the identity -- so this state means the raster is reading a survivor \
         list nobody compacted this frame: a stale list, or a cull that lowered the record word \
         without moving the entries -- got {:?}",
        probe.raw
    );

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
/// The GPU gate above needs a device, so in CI it never runs at all; this test is what holds the
/// fixture's premises in the meantime. It checks that the narrow framing rejects exactly the
/// interior instance of each batch, that it does so by a MARGIN rather than marginally, that the
/// on-screen instances are inside by their centres rather than by a corner, and that the per-BATCH
/// cull rejects NOTHING — the premise the whole rung rests on.
///
/// It is also the HOST ORACLE the GPU gate is compared against: it names the same instances, by
/// the same [`boyko_render::frustum::instance_visible_after_cull`] the armed shader mirrors. A
/// disagreement between the two is a SHADER bug, not a math bug — the planes are extracted once on
/// the host and pushed.
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

    // ---- the GPU gate's own expectation, against the HOST ORACLE -------------------------------
    // The gate above is `#[ignore]`d and needs a device, so without this the region expectation
    // could drift away from the fixture and nothing in CI would notice until a GPU run.
    let rejected = vb_inst_cull_scene::instance_rejections(&NARROW);
    for batch in 0..BATCH_COUNT {
        let kept: Vec<u32> = (0..BATCH_INSTANCES)
            .map(|local| vb_inst_cull_scene::global_instance_id(batch, local))
            .filter(|g| !rejected.contains(g))
            .collect();
        assert_eq!(
            expected_region(batch),
            kept,
            "batch {batch}'s expected survivor region must be exactly the instances the host \
             oracle KEEPS, in ring order — the GPU gate compares the device's region against this \
             list, so the two must be one statement"
        );
        assert_eq!(
            kept.len(),
            SURVIVORS_PER_BATCH,
            "batch {batch} keeps {} instances but the record expectation says \
             {SURVIVORS_PER_BATCH}",
            kept.len()
        );
    }

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
