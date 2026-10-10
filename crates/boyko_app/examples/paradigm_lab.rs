//! **Paradigm lab** — one FLY-AROUND scene (WASD + mouse) that renders correctly in EVERY cell of
//! the `RenderPath × GeometryLegs` matrix. Pick the paradigm with a parameter (no rebuild):
//!
//! ```text
//! scripts\run-scene.ps1 -Scene paradigm_lab -Path vb -Legs both
//! scripts\run-scene.ps1 -Scene paradigm_lab -Path deferred        # legs default = both
//! ```
//!
//! `run-scene.ps1` just sets `BOYKO_RENDER_PATH` / `BOYKO_GEOMETRY_LEGS`, which `EnginePlugins::build`
//! reads (crates/boyko_app/src/plugins.rs). You can also select the paradigm IN CODE — insert your
//! own `RenderPathConfig` after `add_plugins` (it wins over the env default), e.g.
//! `app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Both });`.
//!
//! # Controls (the engine's `FlyCameraPlugin`)
//!
//! * `W`/`S`/`A`/`D` — fly; `Space`/`E` up, `Left Ctrl`/`Q` down; mouse — look; `Esc` — quit.
//!
//! # Scene: shadows + materials, all four paradigms
//!
//! A floor + a row of raster spheres carrying distinct PBR materials (matte red, gold metal, blue
//! metal) under a 3-cascade CSM sun (real cast shadows), plus one live analytic [`SdfPrimitive`]
//! sphere. Meshes are registered through the PLAIN
//! [`MeshAssetsExt::register_mesh`](boyko_render::MeshAssetsExt)/`plane` — NOT the VB-aware
//! `register_mesh_vb` — precisely to show that the engine now back-fills VB geometry-table slots at
//! boot (`boyko_render::backfill_vb_geometry_slots`), so ANY scene renders under
//! `RenderPath::VisibilityBuffer` without special registration. The mesh (`Mesh`) + SDF (`Sdf`)
//! legs both carry geometry, so `GeometryLegs::{Both, Mesh, Sdf}` each show something.
//!
//! `BOYKO_HOST_DUMP=<path.bmp>` captures one settled frame instead of running interactively.

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::mesh::Vertex;
use boyko_render::{Material, generate_tangents};

/// The sun direction TO the light.
const SUN_DIR: [f32; 3] = [-0.42, 0.80, 0.42];

/// A UV sphere (outward normals + UVs + a generated tangent basis).
fn uv_sphere(radius: f32, stacks: u32, slices: u32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let pi = core::f32::consts::PI;
    let mut verts = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let phi = (i as f32 / stacks as f32) * pi;
        let (sp, cp) = phi.sin_cos();
        let v = i as f32 / stacks as f32;
        for j in 0..=slices {
            let theta = (j as f32 / slices as f32) * (2.0 * pi);
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st];
            let u = j as f32 / slices as f32;
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

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko paradigm lab", 960, 640));
    // The interactive WASD fly-camera stack (InputPlugin<FlyAction> + the controller + ECS quit).
    app.add_plugin(FlyCameraPlugin);
    // CSM on (3 cascades) — real cast shadows in every paradigm, including the VB x Sdf
    // shadow-vocab march.
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
    app.add_startup_system(setup);
    app.run();
}

/// Spawns the scene. Meshes register through the PLAIN `register_mesh`/`plane` (no VB-aware
/// threading) — the boot-time `backfill_vb_geometry_slots` claims their VB slots under a VB boot.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
) {
    let floor = meshes.plane(dev.get(), 16.0);
    let (sphere_v, sphere_i) = uv_sphere(0.62, 28, 40, [0.75, 0.75, 0.78, 1.0]);
    let sphere = meshes.register_mesh(dev.get(), &sphere_v, &sphere_i);

    // Floor: a RECEIVER only (no `ShadowCaster`) so it never casts a whole-plane shadow.
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));

    // A row of raster spheres with distinct materials (the `Mesh` leg). All share ONE MeshHandle.
    let red = materials.add(Material::new([0.72, 0.05, 0.05, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let gold = materials.add(Material::new([1.0, 0.71, 0.29, 1.0], 1.0, 0.16, 0.5, [0.0; 3], 0));
    let blue = materials.add(Material::new([0.18, 0.36, 0.92, 1.0], 1.0, 0.42, 0.5, [0.0; 3], 0));
    let row: [Option<u16>; 4] =
        [None, Some(red.index() as u16), Some(gold.index() as u16), Some(blue.index() as u16)];
    for (i, mat) in row.iter().enumerate() {
        let x = (i as f32 - 1.5) * 1.6;
        let e = commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(x, 0.62, 0.0))))
            .insert(ShadowCaster)
            .id();
        if let Some(id) = mat {
            commands.entity(e).insert(MaterialHandle(*id));
        }
    }

    // The `Sdf` leg: one live analytic SDF sphere in FRONT of the raster row, so under `Both` the
    // mesh/SDF composite is visible and under `Sdf` it is the only geometry.
    commands.spawn(SdfPrimitive(SdfEdit::sphere([0.0, 0.75, 2.2], 0.75, sdf_op::UNION, 0.0)));

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
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.97, 0.92], 3.0),
    });

    commands.spawn(SkyLight::new([0.30, 0.38, 0.50], [0.16, 0.15, 0.14]));

    // A warm point accent (unshadowed) to give the materials a second highlight to move around.
    commands.spawn(PointLightObject {
        transform: Transform::from_translation(Vec3::new(1.4, 1.8, 1.4)),
        global: GlobalTransform::IDENTITY,
        light: PointLight::new([1.4, 1.8, 1.4], [1.0, 0.74, 0.48], 200.0, 8.0),
    });

    // The FLY camera at a start pose looking at the sphere row. `fly_camera_system` overwrites the
    // rotation from yaw/pitch on the first frame, so only the translation seeds the eye.
    commands.spawn(FlyCameraBundle {
        transform: Transform::from_translation(Vec3::new(0.0, 1.7, 7.4)),
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: 52.0 * core::f32::consts::PI / 180.0,
            aspect: 960.0 / 640.0,
            near: 0.1,
            far: 100.0,
        },
        fly: FlyCamera::default(),
    });
}
