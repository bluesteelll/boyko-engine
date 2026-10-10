//! **Frame 0 of the shipped host gathers every mesh the startup spawned.** A device-free gate on the
//! frame-0 consequence of the `EnginePlugins` edge `VisibilitySet::Read.after(VisibilitySet::Sync)`
//! in `src/plugins.rs`.
//!
//! # Why this file exists (R4-frame-order)
//!
//! A `MeshBundle` carries no `RenderEnabled` bit: `visibility_sync` enables it through a deferred
//! command on the entity's first frame, and the mesh and shadow-caster gathers filter on
//! `Enabled<RenderEnabled>`. With the gathers ordered before `visibility_sync`, frame 0 gathered
//! zero meshes (measured on the golden host: 0 draw rows on frame 0, 7 on every later frame), and
//! every TAA pin carried that meshless frame in its history.
//! `tests/host_orders_render_enabled_readers_after_visibility_sync.rs` pins the edge; this file pins
//! what the edge is for, in rows, on the frame it matters.
//!
//! # What runs
//!
//! The real `EnginePlugins` composition and its real `Main` schedule, one `update`. The window
//! runner is never installed, so the test inserts the two world residents the runner would insert
//! before `finish()` and the gathers read — `Assets<MeshGpu>` and `Assets<Material>` — with a
//! device-inert `MeshGpu` (`VkBuffer::NULL` handles; the gathers read only `index_count` /
//! `index_type`, the idiom of `boyko_render/tests/asset_streaming_f8_material_gather.rs`). A startup
//! system spawns the scene, so the spawn drains in `finish()` exactly as a host scene's does, and
//! the first `update` is frame 0.
//!
//! The observable is each gather's own output: `MeshRenderScratch::instance_count()` for the mesh
//! gather and `CsmCasterScratch`'s for the caster gather — the rows the runner uploads and draws.
//!
//! # Why the two camera-controller systems
//!
//! Without the edge, which of the gathers and `visibility_sync` runs first is decided by the
//! executor's wave packing, a function of the whole schedule. MEASURED: `EnginePlugins` alone
//! packs `visibility_sync` first even with every set edge of the R4 block deleted (measured before
//! R4b's four edges existed, i.e. on the composition with none of the nine), so a gate on that
//! composition passes with or without them and proves nothing. The golden host
//! (`taa_jitter_eval.rs`) adds two `CameraSet::Control` systems, and that composition packs the
//! gathers first. This file adds two systems of the same shape — the same set, the same declared
//! access. A camera controller in `CameraSet::Control` is the ordinary shipped case
//! (`FlyCameraPlugin`'s `fly_camera_system` joins the same set).
//!
//! # What turns it red, measured — and what does not
//!
//! * The composition before R4 and R4b (all NINE set edges of the R4 block deleted from
//!   `src/plugins.rs` — R4's five and R4b's four): frame 0 gathers 0 of 7 meshes on the software
//!   leg. On the `hwrt` leg the result also depends on one lighting edge, the light seed's
//!   `.before(cull)` in `boyko_render/src/light_plugin.rs`: without it, 0 of 7 as well; with it,
//!   that leg's packing happens to put `visibility_sync` first and gathers 7 — the same split the
//!   golden host showed.
//! * Deleting only R4's five edges does NOT turn this red (measured GREEN on both legs): R4b's two
//!   `VisibilitySet::Validate` edges, `Sync → Validate → Read`, carry the same order to every
//!   reader on their own. (Until 2026-09-21 this doc said the five sufficed; that was measured
//!   before R4b existed and is stale since.)
//! * With every R4 and R4b edge in place, both legs gather all 7, with or without that lighting
//!   edge.
//! * Deleting ONLY `VisibilitySet::Read.after(VisibilitySet::Sync)` does NOT turn this red: the
//!   other edges repack the waves so that `visibility_sync` happens to run first here. That
//!   line is pinned by the cycle gate named above together with the two `VisibilitySet::Validate`
//!   gates (R4b-open-edges: `Sync → Validate → Read` is a second path to every reader, so the
//!   cycle gate goes red only when that path is cut too); cycle gates cannot drift with the
//!   packing. This file pins the outcome, and its red half depends on a packing a future scheduler
//!   change can move.
//!
//! # Why its own binary
//!
//! `EnginePlugins` can be built only once per process (see `particle_host_reachable.rs`), so this
//! file holds one `#[test]`. It needs no device.

use std::time::Duration;

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::data::Mut;
use boyko_ecs::ecs::core::iters::query::filter::With;
use boyko_ecs::ecs::core::system::{Commands, Res};
use boyko_macros::{Component, Resource};
use boyko_math::Vec3;
use boyko_render::{
    CsmCasterScratch, Material, MeshBundle, MeshGpu, MeshRenderScratch, RenderEpoch, ShadowCaster,
};
use boyko_rhi::enums::IndexType;
use boyko_rhi_vulkan::ffi::VkBuffer;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_scene::render_caps::MeshHandle;
use boyko_scene::{Camera, CameraSet, Transform};

/// Meshes the startup spawns — the golden `taa_armed` scene's count.
const MESHES: usize = 7;
/// How many of them also carry `ShadowCaster` (the rest are receiver-only, like a floor).
const CASTERS: usize = 4;

/// The controllers' pose input — the golden host's `EvalMotion` shape (a test-owned resource the
/// controllers read and nothing writes).
#[derive(Resource, Default)]
struct ControllerPose {
    /// The pose written to every driven entity.
    pose: Transform,
}

/// Marks the entity the second controller drives — the golden host's `MovingCaster` shape.
#[derive(Component)]
struct DrivenCaster;

/// A device-inert `MeshGpu`: null buffers, a unit cube's index count. Sound here because no
/// device exists in this process and the gathers read only `index_count` / `index_type`.
fn dummy_mesh_gpu() -> MeshGpu {
    let dummy_buf = || BoundBuffer { buffer: VkBuffer::NULL, offset: 0, size: 0, mapped: None, block: 0 };
    MeshGpu {
        vertex_buffer: dummy_buf(),
        index_buffer: dummy_buf(),
        index_count: 36,
        index_type: IndexType::Uint16,
        vertex_count: 8,
        #[cfg(feature = "hwrt")]
        blas: None,
        geometry_slot: 0,
        local_min: [-0.5; 3],
        local_max: [0.5; 3],
    }
}

/// The scene, spawned the way a host scene is: from a startup system, through `Commands`, with no
/// `RenderEnabled` bit set by hand.
fn spawn_scene(mut cmds: Commands) {
    for i in 0..MESHES {
        let pose = Transform::from_translation(Vec3::new(i as f32 * 2.0, 0.0, -5.0));
        let mut mesh = cmds.spawn(MeshBundle::new(MeshHandle(0), pose));
        if i < CASTERS {
            mesh.insert(ShadowCaster);
        }
    }
}

/// A camera controller with `drive_camera_motion`'s declared access. No camera is spawned, so its
/// body writes nothing; only its place in the schedule matters here.
// SystemParams are consumed by value by the SystemParam contract.
#[allow(clippy::needless_pass_by_value)]
fn drive_camera(
    _epoch: Res<RenderEpoch>,
    input: Res<ControllerPose>,
    mut cameras: Query<Mut<Transform>, With<Camera>>,
) {
    for mut transform in cameras.iter_mut() {
        transform.set_if_neq(input.pose);
    }
}

/// A second controller with `drive_moving_caster_motion`'s declared access.
// SystemParams are consumed by value by the SystemParam contract.
#[allow(clippy::needless_pass_by_value)]
fn drive_caster(
    _epoch: Res<RenderEpoch>,
    input: Res<ControllerPose>,
    mut casters: Query<Mut<Transform>, With<DrivenCaster>>,
) {
    for mut transform in casters.iter_mut() {
        transform.set_if_neq(input.pose);
    }
}

/// RED: delete all nine set edges of the R4 block from `src/plugins.rs` — R4's five and R4b's four
/// (and, for the `hwrt` leg, the light seed's `.before(cull)`) — and frame 0 gathers 0 of 7 meshes:
/// the gathers run before `visibility_sync`'s apply window. Deleting only R4's five leaves it
/// green: R4b's `Sync → Validate → Read` edges carry the order. See the module doc for what else
/// does not turn it red.
#[test]
fn frame_zero_gathers_every_mesh_the_startup_spawned() {
    // `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run`
    // installs the windowed runner, which this never calls.
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("frame-zero-mesh-gate", 64, 64));
    app.add_startup_system(spawn_scene);
    app.insert_resource(ControllerPose::default());
    app.add_systems_cfg(|b| {
        b.add_system(drive_camera).in_set(CameraSet::Control);
        b.add_system(drive_caster).in_set(CameraSet::Control);
    });

    let mut meshes = Assets::<MeshGpu>::with_reserved(1);
    let cube = meshes.add(dummy_mesh_gpu());
    assert_eq!(cube.index(), 0, "precondition: the scene's `MeshHandle(0)` names the one mesh added");
    app.world_mut().insert_non_send_resource(meshes);
    let mut materials = Assets::<Material>::with_reserved(1);
    materials.add(Material::default());
    app.insert_resource(materials);

    app.finish();

    // Frame 0.
    app.update_with_delta(Duration::from_millis(16));

    let drawn = app.world().resource::<MeshRenderScratch>().instance_count();
    let cast = app.world().resource::<CsmCasterScratch>().0.instance_count();
    assert_eq!(
        drawn, MESHES,
        "frame 0 must gather every mesh the startup spawned; a short count means the mesh gather \
         ran before `visibility_sync` enabled the new rows' `RenderEnabled` bit"
    );
    assert_eq!(
        cast, CASTERS,
        "frame 0 must gather every shadow caster the startup spawned; a short count means the \
         caster gather ran before `visibility_sync` enabled the new rows' `RenderEnabled` bit"
    );
}
