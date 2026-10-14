//! **The shipped host runs `check_route_markers` (RD2 / R0a, critique W3).**
//!
//! `boyko_render/tests/route_runtime_change.rs` runs the check by hand, so deleting its production
//! registration in `src/plugins.rs::register_main_frame_systems` leaves those tests green. This
//! file is the gate on the line that ships: the real `EnginePlugins` composition and its real
//! `Main` schedule, one `update`, over a startup scene whose one entity carries a `Translucent`
//! marker its opaque material does not route to. A debug build must panic with the check's
//! `reassign_route` remedy.
//!
//! RED: delete the `check_route_markers` registration from `src/plugins.rs` and this test fails
//! with "test did not panic as expected".
//!
//! The harness is `host_frame_zero_draws_every_mesh.rs`'s (no device: the window runner is never
//! installed, `Assets<MeshGpu>` holds one device-inert mesh). `EnginePlugins` can be built once
//! per process, so this file holds one `#[test]`. Release builds compile the check out, and this
//! file with it.
#![cfg(debug_assertions)]

use std::time::Duration;

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::system::Commands;
use boyko_math::Vec3;
use boyko_render::{Material, MeshBundle, MeshGpu, Translucent};
use boyko_rhi::enums::IndexType;
use boyko_rhi_vulkan::ffi::VkBuffer;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_scene::Transform;
use boyko_scene::render_caps::{MaterialHandle, MeshHandle};

/// A device-inert `MeshGpu` (see `host_frame_zero_draws_every_mesh.rs`).
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

/// One mesh on the opaque material at slot 1, marked `Translucent` — the mismatch.
fn spawn_scene(mut cmds: Commands) {
    let mut bundle = MeshBundle::new(MeshHandle(0), Transform::from_translation(Vec3::new(0.0, 0.0, -5.0)));
    bundle.material = MaterialHandle(1);
    cmds.spawn(bundle).insert(Translucent);
}

#[test]
#[should_panic(expected = "reassign_route")]
fn the_shipped_host_checks_the_route_markers() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("route-markers-check-gate", 64, 64));
    app.add_startup_system(spawn_scene);

    let mut meshes = Assets::<MeshGpu>::with_reserved(1);
    meshes.add(dummy_mesh_gpu());
    app.world_mut().insert_non_send_resource(meshes);
    let mut materials = Assets::<Material>::with_reserved(2);
    let default = materials.add(Material::default());
    materials.pin(default.index());
    materials.add(Material::new([0.5, 0.5, 0.5, 1.0], 0.0, 0.5, 0.5, [0.0; 3], 0));
    app.insert_resource(materials);

    app.finish();
    app.update_with_delta(Duration::from_millis(16));
}
