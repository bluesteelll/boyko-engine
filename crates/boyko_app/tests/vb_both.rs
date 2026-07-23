//! Multi-paradigm render-path plan, rung R10 — the VisibilityBuffer x Both golden dump.
//!
//! Clones [`vb_mesh`](vb_mesh.rs)'s EXACT five-sphere `grand_showcase_2mat` scene (same mesh,
//! same five materials, same sun/sky, same camera) verbatim, with ONE delta:
//! `RenderPathConfig{path: VisibilityBuffer, legs: Both}` instead of `{VisibilityBuffer, Mesh}`.
//! Mirrors `forward_both.rs`'s own precedent against `forward_mesh.rs`.
//!
//! # Expected: byte-identical to [`vb_mesh`]'s `f4719cbf`
//!
//! `VB_SDF_IMPLEMENTED` is now `true` (rung R10), so `VisibilityBuffer x Both` resolves CLEAN —
//! the legs stay `Both`, arming `sdf_forward_marched` and the `HAS_MESH` `sdf_forward_march`
//! COMPUTE dispatch declared/recorded AFTER `vb_resolve` (`declare_vb_graph`'s `sdf_forward_march`
//! arm + `record_vb`). This scene's SDF edit-list is boot-seeded EMPTY (`count == 0`,
//! `GpuSceneBundles::boot`'s doc) — the march finds NO hit anywhere and (per the pass's own "a
//! miss writes nothing" contract) stores NOTHING into `gLit`, leaving the five spheres
//! `vb_resolve` painted untouched. The frame is therefore expected byte-for-byte identical to the
//! mesh-only VB frame `[vb_mesh]` (`f4719cbf`), EXACTLY as `forward_both` == `forward_mesh`
//! (`f93b5aad`) proves for the Forward family. The orchestrator confirms the equality on the GPU.
//!
//! Windowed-test conventions (mirrors `vb_mesh.rs`): `#[ignore]` (needs a real windowed GPU
//! device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.
//! `BOYKO_HOST_DUMP=<path.bmp>` arms the `boyko_app::host_dump` screenshot capture; see
//! `goldens/PINS.toml`'s `[vb_both]` pin.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;
use boyko_render::{GeometryLegs, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig};

/// The sun direction TO the light (byte-identical to `grand_showcase_2mat.rs`'s / `vb_mesh.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// Verbatim copy of `vb_mesh.rs::uv_sphere` (a pinned-golden scene keeps its exact mesh generation
/// frozen — see that file's NOTE for why this is a local copy rather than a shared helper).
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

/// Verbatim copy of `vb_mesh.rs::setup` — the SAME five-sphere scene (registered via
/// [`MeshAssetsVbExt::register_mesh_vb`], falling back to the plain
/// [`MeshAssetsExt::register_mesh`] when the geometry table is not armed). Only the requested
/// `RenderPathConfig` legs differ (`Both` here vs `vb_mesh`'s `Mesh`).
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

/// **The VisibilityBuffer x Both golden dump.** The SAME five-sphere `grand_showcase_2mat` scene
/// as `[vb_mesh]`, rendered through `RenderPath::VisibilityBuffer x GeometryLegs::Both` — the mesh
/// leg via `vb_raster`/`vb_resolve`, the (empty) SDF leg composited via `sdf_forward_march`. See
/// this file's own doc for the byte-identity-to-`[vb_mesh]` expectation.
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the VisibilityBuffer x Both screenshot"]
fn vb_both_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine vb both", 512, 512);
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Multi-paradigm render-path plan, rung R10: request `VisibilityBuffer x Both` — inserted
    // AFTER `add_plugins` (which installs `RenderPathPlugin`'s `Deferred`-default) so this
    // override wins, mirroring `vb_mesh.rs`'s own post-plugins owner-override insert.
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Both });
    app.run();
}

/// **Rung R9c: the VB×Both + DDGI eval dump** (owner-facing eval + the `-ValidationOn`-class
/// coverage vehicle, NOT a byte pin — the probe update is a round-robin accumulator, and this
/// scene's SDF edit-list is empty, so the GI term is a near-uniform probe-miss ambient; its
/// value is exercising the FULL R9c chain on real hardware: `ddgi_on` survives the cap on
/// `Both`, the split arms, `ddgi_update` runs under the VB graph (seeded SRO→GENERAL layered
/// transitions), and `vb_shade_split` samples the atlases through its conditional reads).
///
/// `#[ignore]`: needs a real windowed GPU device.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU for the R9c DDGI-under-VB validation/eval dump"]
fn vb_both_ddgi_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine vb both ddgi", 512, 512);
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Both });
    // Rung R9c: DDGI ON — the boot resolver sees `ddgi_on = true` on a Both leg set and
    // commits the split (then frozen by `RenderPathFrozenConsumers`).
    app.insert_resource(boyko_render::DdgiConfig { ddgi_indirect: true, ..Default::default() });
    app.run();
}
