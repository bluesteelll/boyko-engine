//! Multi-paradigm render-path plan, rung R8 — the VisibilityBuffer v1 (FUSED `vb_resolve`)
//! mesh-only golden dump.
//!
//! Reuses [`grand_showcase_2mat`]'s EXACT five-sphere scene (same mesh, same five materials,
//! same sun/sky, same camera) verbatim — mirrors `forward_mesh.rs`'s own precedent: the dumped
//! BMP is a DIRECT visual comparator against the Deferred `f6147f90` / Forward `f93b5aad`
//! goldens (same geometry/lighting/shadows; only the render PATH differs — VB's id-raster +
//! compute-resolve re-fetch vs Forward's inline raster shade). Byte-identity is NOT expected
//! (the geometry re-fetch's analytic barycentric interpolation is a genuinely different
//! floating-point path than the rasterizer's own hardware interpolation) — the orchestrator
//! compares the two visually.
//!
//! # Decision 9 (VB1) 2-instance fixture
//!
//! [`grand_showcase_2mat`]'s five spheres all reference the SAME [`MeshHandle`] (one
//! `register_mesh_vb` call, five `MeshBundle::new(sphere, ...)` spawns) — `gather_mesh_draws`
//! therefore buckets them into ONE [`DrawBatch`] with `instance_count == 5`, so the SAME
//! `vkCmdDrawIndexed`'s `SV_InstanceID` ranges `0..5` against ONE shared index buffer. This is
//! EXACTLY the fixture Decision 9's `raw_prim_id % tri_count` normalization needs to prove
//! itself against instance > 0 (whichever `SV_PrimitiveID` per-instance semantics the driver
//! implements) — no separate fixture is needed; reusing the existing five-sphere scene already
//! exercises it.
//!
//! # Geometry-table slot claim (rung R8 register_mesh gap, closed)
//!
//! [`MeshAssetsExt::register_mesh`](boyko_render::MeshAssetsExt::register_mesh) never claims a
//! Decision-0 geometry-table slot (that fn's own doc — a host-authored mesh's `geometry_slot`
//! stays [`VB_GEOMETRY_RESERVED_SLOT`](boyko_render::mesh_geometry_table::VB_GEOMETRY_RESERVED_SLOT)
//! forever). This test uses the VB-aware sibling
//! [`MeshAssetsVbExt::register_mesh_vb`](boyko_render::MeshAssetsVbExt::register_mesh_vb)
//! instead, threading `NonSendResMut<MeshGeometryTableSlot>` — the World resource
//! `boyko_app::runner` constructs (`Some`-armed) BEFORE `app.finish()` drains this startup
//! system, on EVERY boot (`None` when the table isn't armed, e.g. a device lacking the
//! descriptor-indexing prerequisite — this test falls back to the plain, non-VB-aware
//! `register_mesh` in that case, which still renders correctly under the resolver's
//! `VbDeviceCapMissing` degrade to `Deferred`).
//!
//! Windowed-test conventions (mirrors `forward_mesh.rs`): `#[ignore]` (needs a real windowed GPU
//! device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.
//! `BOYKO_HOST_DUMP=<path.bmp>` arms the `boyko_app::host_dump` screenshot capture; see
//! `goldens/PINS.toml`'s `[vb_mesh]` pin (UNBLESSED — the orchestrator renders + blesses).

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;
use boyko_render::{GeometryLegs, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig};

/// The sun direction TO the light (byte-identical to `grand_showcase_2mat.rs`'s /
/// `forward_mesh.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// Verbatim copy of `grand_showcase_2mat.rs::uv_sphere` (see that file's NOTE for why this is a
/// local copy rather than a shared `tests/common` helper — a pinned-golden scene keeps its exact
/// mesh generation frozen).
fn uv_sphere(radius: f32, stacks: u32, slices: u32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let pi = core::f32::consts::PI;
    let mut verts = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let phi = (i as f32 / stacks as f32) * pi; // 0..π, north pole to south
        let (sp, cp) = phi.sin_cos();
        let v = i as f32 / stacks as f32; // phi / π
        for j in 0..=slices {
            let theta = (j as f32 / slices as f32) * (2.0 * pi); // 0..2π
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st]; // unit outward normal
            let u = j as f32 / slices as f32; // theta / 2π
            let mut vertex = Vertex::new([n[0] * radius, n[1] * radius, n[2] * radius], n, color);
            vertex.uv = [u, v];
            verts.push(vertex);
        }
    }
    let stride = slices + 1;
    let mut idx = Vec::with_capacity((stacks * slices * 6) as usize);
    for i in 0..stacks {
        for j in 0..slices {
            let a = i * stride + j;
            let b = (i + 1) * stride + j;
            idx.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    generate_tangents(&mut verts, &idx);
    (verts, idx)
}

/// Verbatim copy of `grand_showcase_2mat.rs::setup` — the SAME five-sphere scene, so the dumped
/// BMP is a direct VB-vs-Deferred/Forward visual comparator — with ONE delta: the sphere mesh is
/// registered via [`MeshAssetsVbExt::register_mesh_vb`] (falling back to the plain
/// [`MeshAssetsExt::register_mesh`] when the geometry table is not armed) so it claims a
/// Decision-0 geometry-table slot, without which `vb_geom_fetch.hlsli` could never resolve this
/// mesh's `gMeshVerts[]`/`gMeshIndices[]` entries.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let (verts, idx) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    let sphere = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };

    let red = materials.add(Material::new([0.72, 0.04, 0.04, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let green = materials.add(Material::new([0.05, 0.46, 0.10, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let gold = materials.add(Material::new([1.0, 0.71, 0.29, 1.0], 1.0, 0.13, 0.5, [0.0; 3], 0));
    let blue = materials.add(Material::new([0.20, 0.38, 0.92, 1.0], 1.0, 0.42, 0.5, [0.0; 3], 0));

    let spacing = 1.55;
    let materials_row: [Option<u16>; 5] =
        [None, Some(red.index() as u16), Some(green.index() as u16), Some(gold.index() as u16), Some(blue.index() as u16)];
    for (i, mat) in materials_row.iter().enumerate() {
        let x = (i as f32 - 2.0) * spacing;
        let e = commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(x, 0.6, 0.0))))
            .id();
        if let Some(id) = mat {
            commands.entity(e).insert(MaterialHandle(*id));
        }
    }

    let sun_pose = Affine3A::look_at_rh(
        Vec3::ZERO,
        Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]),
        Vec3::new(0.0, 1.0, 0.0),
    );
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.97, 0.92], 3.1),
    });

    commands.spawn(SkyLight::new([0.38, 0.44, 0.55], [0.20, 0.20, 0.22]));

    let pose = Affine3A::look_at_rh(
        Vec3::new(0.0, 1.1, 7.8),
        Vec3::new(0.0, 0.55, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    );
    commands.spawn(CameraRig {
        transform: Transform {
            translation: pose.translation,
            rotation: Quat::from_mat3(pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: 52.0 * core::f32::consts::PI / 180.0,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

/// **The VisibilityBuffer v1 (fused) mesh-only golden dump.** The SAME five-sphere
/// `grand_showcase_2mat` scene, rendered through `RenderPath::VisibilityBuffer ×
/// GeometryLegs::Mesh` instead of the Deferred default — the owner's RTX visual sign-off gate
/// for rung R8 (`goldens/PINS.toml`'s `[vb_mesh]`).
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the VisibilityBuffer mesh-only screenshot"]
fn vb_mesh_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine vb mesh-only", 512, 512);
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Multi-paradigm render-path plan, rung R8: request `VisibilityBuffer × Mesh` — inserted
    // AFTER `add_plugins` (which installs `RenderPathPlugin`'s `Deferred`-default) so this
    // override wins, mirroring `forward_mesh.rs`'s own post-plugins owner-override insert.
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh });
    app.run();
}
