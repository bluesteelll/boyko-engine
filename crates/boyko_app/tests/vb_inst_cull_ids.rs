//! **VG rung R2d-5 — the id-MAPPING gate: which instance ids the raster actually exported.**
//!
//! The narrow fixture rendered under the existing `BOYKO_VG_CENSUS` harness, reading the census
//! row's distinct-instance set (`boyko_render::vg_census::reduce`'s R2d-5 statistic). The raster
//! exports a GLOBAL instance-ring index (`vb_raster.vs.hlsl`: `output.instance_id = global`), so
//! that set answers exactly one question: **did the indirection preserve the mapping from a drawn
//! lane to the instance it is drawing?**
//!
//! # ⚠️ WHAT THIS GATE IS, AND WHAT IT IS NOT
//!
//! **It is a MAPPING gate.** Rung R2d-4 made the vertex shader read its instance index THROUGH the
//! survivor list. A shader that exported the compacted SLOT instead of the stored global id, or one
//! that dropped the `base_instance` term, produces a DIFFERENT id set for the same picture. That is
//! what this reads.
//!
//! **It is INVARIANT UNDER THE ARMING, BY CONSTRUCTION.** A culled instance is culled precisely
//! because it is off-screen, so it covers no texel whether or not it is drawn; and
//! `vg_census::reduce` admits only NON-SENTINEL texels, so an instance that covers nothing is
//! absent from the set either way. The expected set below is therefore the same at rung R2d-5 and
//! at rung R2d-6. That is a property of the instrument, not a weakness to be worked around: an id
//! set that CHANGED under the arming would mean the arming moved geometry.
//!
//! **There is NO commit-checkout control for it, and none can exist.** A control would have to be a
//! build in which this gate is red, and no committed rung of this ladder produces one — the
//! statement above says why. Its only real red state is a DELIBERATE SOURCE MUTATION (export the
//! compacted slot; drop the `base_instance` term; index the survivor list with a local id). Nothing
//! in this file claims a counterfactual that was not executed.
//!
//! # The set, and why it is not merely a count
//!
//! Four of the six instances are on screen at the narrow framing: local indices `0` and `2` of both
//! batches. With batch bases `0` and `BATCH_INSTANCES`, their global ids are `{0, 2, 3, 5}`. That
//! set also constrains the fixture's own "the culled instance is at
//! [`vb_inst_cull_scene::OFFSCREEN_LOCAL_INDEX`]" invariant — but only UP TO the permutations that
//! FIX that index. With `OFFSCREEN_LOCAL_INDEX = 1` of `BATCH_INSTANCES = 3`, local index 1 is the
//! fixed point of the reversal of {0,1,2}, so a ring iterating each batch backwards produces the
//! IDENTICAL set. The gate is fully sensitive only to orders that MOVE the off-screen instance.
//! Stated precisely because the stronger claim would be false.
//!
//! `#[ignore]`: needs a real windowed GPU device. `BOYKO_DISABLE_VALIDATION=1`, `--test-threads=1`.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::{GeometryLegs, Material, MeshGeometryTableSlot, RenderPath, RenderPathConfig};

mod vb_inst_cull_scene;
mod vg_thresholds;

use vb_inst_cull_scene::{
    BATCH_COUNT, BATCH_INSTANCES, EXTENT, INSTANCE_COUNT, NARROW, OFFSCREEN_LOCAL_INDEX,
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

/// The global ids the narrow framing puts on screen: every local index except
/// [`OFFSCREEN_LOCAL_INDEX`], in both batches, sorted ascending — the order
/// `vg_census::reduce` emits.
fn expected_onscreen_ids() -> Vec<u32> {
    let mut ids: Vec<u32> = (0..BATCH_COUNT)
        .flat_map(|b| {
            (0..BATCH_INSTANCES)
                .filter(|l| *l != OFFSCREEN_LOCAL_INDEX)
                .map(move |l| vb_inst_cull_scene::global_instance_id(b, l))
        })
        .collect();
    ids.sort_unstable();
    ids
}

/// **The gate.** Reads the census row's distinct-instance set off a real raster.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it to read the exported instance ids"]
fn vb_inst_cull_ids_are_the_global_ring_indices() {
    let out = std::env::temp_dir().join("boyko_vb_inst_cull_ids.toml");
    let _ = std::fs::remove_file(&out);
    // SAFETY: single-threaded test setup, before any engine thread exists. Windowed tests in this
    // crate run with `--test-threads=1` by convention, so no sibling test observes this write.
    // `BOYKO_VB_CULL_READBACK` is deliberately NOT set: the readback block returns from the frame
    // loop BEFORE the census driver runs, so arming both would leave this file unwritten.
    unsafe {
        std::env::set_var("BOYKO_VG_CENSUS", &out);
    }

    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine vb inst cull ids", EXTENT.0, EXTENT.1));
    app.add_startup_system(setup);
    app.insert_resource(RenderPathConfig {
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Mesh,
    });
    app.run();

    let text = std::fs::read_to_string(&out).unwrap_or_else(|e| {
        panic!(
            "the census wrote no row at {} ({e}). The run ended without reaching the census, so \
             nothing here is evidence about the exported ids.",
            out.display()
        )
    });
    let row = vg_thresholds::parse_row(&text);

    assert!(
        row.vb_mesh_leg,
        "the census frame must carry a VB mesh leg; without it the readback is sentinel-only and \
         every assertion below would be about an empty buffer"
    );
    assert_eq!(
        row.achieved, EXTENT,
        "the row must be the {EXTENT:?} frame this fixture spawns"
    );
    assert!(
        row.covered_pixels > 0,
        "a frame covering no mesh texel cannot say anything about instance ids"
    );

    // The cap is not in play at this scale, and saying so is what makes the set assertion a set
    // assertion rather than a prefix assertion.
    assert!(
        row.distinct_instance_count <= row.distinct_instance_cap,
        "the distinct-instance list was TRUNCATED ({} of {} kept); the set assertion below would \
         then be about a prefix",
        row.distinct_instance_cap,
        row.distinct_instance_count
    );

    let want = expected_onscreen_ids();
    assert_eq!(
        row.distinct_instance_count as usize,
        want.len(),
        "the narrow framing puts {} of {INSTANCE_COUNT} instances on screen; the raster exported \
         {} distinct ids",
        want.len(),
        row.distinct_instance_count
    );
    assert_eq!(
        row.distinct_instances, want,
        "the exported ids must be the GLOBAL instance-ring indices {want:?}. A set with the right \
         SIZE but different members means one of: the VS exported a compacted slot instead of the \
         stored global id; the `base_instance` term was dropped, collapsing both batches onto the \
         same ids; or the instance ring's order MOVED the off-screen instance off local index \
         {OFFSCREEN_LOCAL_INDEX} (an order that merely reverses each batch leaves index 1 of 3 \
         fixed and is NOT caught here)"
    );
}

/// The expectation this file pins is derived from the fixture's own constants, not typed in — so a
/// fixture edit moves both together. NOT `#[ignore]`d.
#[test]
fn the_expected_id_set_follows_the_fixture_constants() {
    vb_inst_cull_scene::assert_fixture_invariants();
    let want = expected_onscreen_ids();
    assert_eq!(
        want.len(),
        INSTANCE_COUNT - BATCH_COUNT,
        "one instance per batch is off screen at the narrow framing"
    );
    // The CPU oracle agrees about WHICH ones, so the GPU assertion is not the only statement of it.
    let rejected = vb_inst_cull_scene::instance_rejections(&NARROW);
    let mut kept: Vec<u32> = (0..INSTANCE_COUNT as u32).filter(|g| !rejected.contains(g)).collect();
    kept.sort_unstable();
    assert_eq!(kept, want, "the host oracle and the id expectation must name the same instances");
}
