//! **VG rung R2d-5 — the WIDE framing: the narrow gate's CONTROL.**
//!
//! The SAME fixture as `vb_inst_cull_narrow.rs` — same meshes, same six instances, same materials,
//! same extent — on a framing that CONTAINS all six. The two `Framing` constants differ in exactly
//! one field (`eye`), so a difference between the two runs' probe lines can only be the framing.
//!
//! # Why a control is required rather than nice
//!
//! At rung R2d-6 the narrow run's `inst` becomes `[2, 2]` and its survivor regions lose an entry.
//! Without this run, that could equally be "the arming rung culls one instance per batch" or "the
//! arming rung breaks one instance per batch everywhere". This run answers it: the SAME armed
//! build, on a framing where nothing is off-screen, must still report `[3, 3]` and the identity
//! regions. A cull that rejects visible geometry reds HERE while the narrow run looks correct.
//!
//! # What this rung asserts
//!
//! `inst = [3, 3]`, both regions the identity, `visible = 2`, `batches = 2` — and every one of
//! those numbers is UNCHANGED at rung R2d-6, which is precisely what makes it a control rather
//! than a second experiment.
//!
//! `#[ignore]`: needs a real windowed GPU device. `BOYKO_DISABLE_VALIDATION=1`, `--test-threads=1`.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::{GeometryLegs, Material, MeshGeometryTableSlot, RenderPath, RenderPathConfig};

mod vb_inst_cull_scene;

use vb_inst_cull_scene::{BATCH_COUNT, BATCH_INSTANCES, EXTENT, INSTANCE_COUNT, WIDE};

fn setup(
    commands: Commands,
    meshes: NonSendResMut<Assets<MeshGpu>>,
    materials: ResMut<Assets<Material>>,
    geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    vb_inst_cull_scene::fixture_setup_system(commands, meshes, materials, geo_table, dev, &WIDE);
}

/// **The control.** Every instance is on screen, so nothing may be removed at either rung.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it as the narrow gate's control"]
fn vb_inst_cull_wide_keeps_every_instance() {
    let probe = vb_inst_cull_scene::probe_in_process("wide", || {
        let mut app = App::new();
        app.add_plugins(EnginePlugins::window(
            "boyko_engine vb inst cull wide",
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

    assert_eq!(probe.batches, BATCH_COUNT, "got {:?}", probe.raw);
    assert_eq!(probe.visible as usize, BATCH_COUNT, "got {:?}", probe.raw);
    assert_eq!(
        probe.inst.as_slice(),
        [BATCH_INSTANCES as u32; BATCH_COUNT].as_slice(),
        "the wide framing contains all {INSTANCE_COUNT} instances, so every record must keep its \
         full {BATCH_INSTANCES}. This assertion is UNCHANGED at rung R2d-6 -- a drop here on the \
         armed build means the per-instance cull rejects visible geometry -- got {:?}",
        probe.raw
    );
    assert_eq!(probe.drawn_instances() as usize, INSTANCE_COUNT, "got {:?}", probe.raw);
    assert_eq!(probe.vis.len(), BATCH_COUNT, "got {:?}", probe.raw);
    for (b, (base, members)) in probe.vis.iter().enumerate() {
        assert_eq!(*base as usize, b * BATCH_INSTANCES, "got {:?}", probe.raw);
        assert_eq!(members.len(), BATCH_INSTANCES, "got {:?}", probe.raw);
    }
    assert!(
        probe.regions_are_identity(),
        "with nothing culled, every survivor region is the identity run at BOTH rungs -- got {:?}",
        probe.raw
    );
}

/// The control's own premise, on the CPU — NOT `#[ignore]`d.
///
/// A "control" that did not actually contain the scene would agree with the gate for the wrong
/// reason, and the GPU assertion above cannot tell the difference.
#[test]
fn the_wide_framing_contains_every_instance() {
    vb_inst_cull_scene::assert_fixture_invariants();
    assert!(
        vb_inst_cull_scene::instance_rejections(&WIDE).is_empty(),
        "the wide framing rejects {:?}; a control must contain the whole fixture",
        vb_inst_cull_scene::instance_rejections(&WIDE)
    );
    assert!(
        vb_inst_cull_scene::batch_rejections(&WIDE).is_empty(),
        "the per-BATCH cull must also reject nothing at the wide framing"
    );
}
