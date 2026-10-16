//! **RD6 / D-DS critique W2 — the programmatic registration path checks the winding.**
//!
//! `mesh_assets::assert_mesh_winding` (debug builds) must be WIRED into `register_mesh`: a mesh
//! fixture wound clockwise for its own normals renders inside-out once single-sided back faces
//! are culled, so code-authored geometry registered that way panics with `mesh_winding`. The
//! host unit test pins the check itself; this device test pins the wiring on the real
//! registration entry point (its `should_panic` fails the moment the call is dropped from
//! `register_mesh`). The control registers the same quad counter-clockwise.
#![cfg(debug_assertions)]

use boyko_ecs::ecs::core::asset::Assets;
use boyko_render::{MeshAssetsExt, MeshGpu, Vertex};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// A unit quad on `y = 0` with `+Y` normals, its two triangles counter-clockwise from above.
fn quad() -> (Vec<Vertex>, [u32; 6]) {
    let n = [0.0, 1.0, 0.0];
    let c = [1.0; 4];
    let v = vec![
        Vertex::new([-0.5, 0.0, -0.5], n, c),
        Vertex::new([-0.5, 0.0, 0.5], n, c),
        Vertex::new([0.5, 0.0, 0.5], n, c),
        Vertex::new([0.5, 0.0, -0.5], n, c),
    ];
    (v, [0, 1, 2, 0, 2, 3])
}

fn device() -> VulkanContext {
    VulkanContext::boot(InstanceConfig::default()).expect("gpu: this test needs a Vulkan device")
}

/// The registration entry point refuses a clockwise-for-its-normals mesh in a debug build.
#[test]
#[ignore = "gpu: boots a headless Vulkan device to call register_mesh; run on the device leg"]
#[should_panic(expected = "mesh_winding")]
fn register_mesh_refuses_a_clockwise_mesh() {
    let ctx = device();
    let mut meshes = Assets::<MeshGpu>::with_reserved(1);
    let (v, _) = quad();
    meshes.register_mesh(&ctx, &v, &[0, 2, 1, 0, 3, 2]);
}

/// The control: the same quad counter-clockwise registers.
#[test]
#[ignore = "gpu: boots a headless Vulkan device to call register_mesh; run on the device leg"]
fn register_mesh_accepts_the_counter_clockwise_quad() {
    let ctx = device();
    let mut meshes = Assets::<MeshGpu>::with_reserved(1);
    let (v, i) = quad();
    let h = meshes.register_mesh(&ctx, &v, &i);
    assert_eq!(meshes.mesh(h).index_count, 6);
    // SAFETY: `MeshAssetsExt::destroy`'s contract — the device is idle (no GPU work referencing
    // the mesh was ever recorded) and every mesh in the table was registered on `ctx`, which
    // outlives the call.
    unsafe { meshes.destroy(&ctx) };
}
