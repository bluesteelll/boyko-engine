//! Multi-paradigm render-path plan, rung R-SDFFWD — the Forward x Both golden dump.
//!
//! Verbatim clone of `forward_mesh.rs`'s five-sphere scene and production boot path (the SAME
//! `grand_showcase_2mat` mesh/materials/sun/sky/camera, booted through the REAL
//! `boyko_app::runner`/`gpu_scene::GpuSceneBundles` path), with ONE delta: `RenderPathConfig{path:
//! Forward, legs: Both}` instead of `{path: Forward, legs: Mesh}`.
//!
//! `SDF_FORWARD_IMPLEMENTED` is now `true` (`boyko_render::render_path_config`), so `Both` no
//! longer collapses to `Mesh` under `Forward` (`LegsCollapsedToMeshPreSdfForward` is unreachable
//! for this combo) — the resolver honors `sdf_leg == true`, arming `sdf_forward_marched` and the
//! `sdf_forward_march` compute pass (`declare_forward_graph`'s `sdf_forward_march` arm +
//! `record_forward`'s HAS_MESH dispatch, since this scene's mesh leg is ALSO present).
//!
//! # Expected: byte-identical to `[forward_mesh]`
//!
//! This scene's SDF edit-list is boot-seeded EMPTY (`GpuSceneBundles::boot`'s doc: `count == 0`,
//! the marcher/march no-ops the field cleanly) — no test in this repo's `boyko_app`-level ECS API
//! populates real SDF edits, so `sdf_forward_march`'s dispatch marches every pixel against an
//! EMPTY field, finds NO hit anywhere, and (per the pass's own "a miss writes nothing" contract —
//! `shaders/sdf_forward_march.comp.hlsl`'s header doc) stores NOTHING into `gLit`. `forward_opaque`'s
//! mesh-raster color therefore stands untouched for every pixel, so the dumped BMP is expected to
//! be byte-identical to `[forward_mesh]`'s `f93b5aad9f799626e6d4abf0dd06d8596d7c6f4c6e9a758a3a6d202d22de71ad`
//! (`goldens/PINS.toml`'s `[forward_both]` pin cross-references it).
//!
//! Windowed-test conventions (mirrors `forward_mesh.rs`): `#[ignore]` (needs a real windowed GPU
//! device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`. `BOYKO_HOST_DUMP=<path.bmp>`
//! arms the `boyko_app::host_dump` screenshot capture; see `goldens/PINS.toml`'s `[forward_both]` pin.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;
use boyko_render::{GeometryLegs, RenderPath, RenderPathConfig};

/// The sun direction TO the light (byte-identical to `grand_showcase_2mat.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// Verbatim copy of `grand_showcase_2mat.rs::uv_sphere` — see that file's NOTE for why this is a
/// local copy rather than a shared `tests/common` helper (a pinned-golden scene keeps its exact
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

/// Verbatim copy of `grand_showcase_2mat.rs::setup` (via `forward_mesh.rs`) — the SAME
/// five-sphere scene, so the dumped BMP is a direct comparator against `[forward_mesh]`.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
) {
    let (verts, idx) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    let sphere = meshes.register_mesh(dev.get(), &verts, &idx);

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

/// **The Forward x Both golden dump.** The SAME five-sphere `grand_showcase_2mat` scene, rendered
/// through `RenderPath::Forward x GeometryLegs::Both` — exercises `sdf_forward_march`'s dispatch
/// (armed by `sdf_leg == true`) against an EMPTY SDF field, expected to be a visual no-op (see
/// this file's own doc for the byte-identity chain to `[forward_mesh]`).
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the Forward x Both screenshot"]
fn forward_both_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine forward both", 512, 512);
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Multi-paradigm render-path plan, rung R-SDFFWD: request `Forward x Both` — inserted AFTER
    // `add_plugins` (which installs `RenderPathPlugin`'s `Deferred`-default) so this override
    // wins, mirroring `forward_mesh.rs`'s own post-plugins owner-override insert.
    app.insert_resource(RenderPathConfig { path: RenderPath::Forward, legs: GeometryLegs::Both });
    app.run();
}
