//! **Paradigm lab** — one scene that renders correctly in EVERY cell of the
//! `RenderPath × GeometryLegs` matrix (Multi-paradigm render-path plan). Drive it with
//! `scripts/run-scene.ps1` (which sets `BOYKO_RENDER_PATH` / `BOYKO_GEOMETRY_LEGS`, honored by
//! `EnginePlugins::build`'s env seam) or by exporting those vars yourself, e.g.:
//!
//! ```text
//! scripts\run-scene.ps1 -Scene paradigm_lab -Path vb -Legs both
//! scripts\run-scene.ps1 -Scene paradigm_lab -Path deferred -Legs sdf
//! ```
//!
//! # Why a dedicated scene
//!
//! The other interactive examples register meshes via the host-authored `cube()`/`plane()`
//! helpers, which do NOT claim a Decision-0 VB geometry-table slot — so under
//! `RenderPath::VisibilityBuffer` their meshes cannot be re-fetched by `vb_resolve` and vanish.
//! This scene registers its meshes through [`MeshAssetsVbExt::register_mesh_vb`] (falling back to
//! the plain [`MeshAssetsExt::register_mesh`] when the geometry table is not armed — i.e. under a
//! non-VB boot), so the SAME scene renders in Deferred / Forward / ForwardPlus / VisibilityBuffer.
//!
//! It ALSO carries BOTH geometry legs: raster spheres + floor (the `Mesh` leg) AND one live
//! [`SdfPrimitive`] sphere (the `Sdf` leg, direct-marched under Deferred or forward-marched under
//! the non-Deferred paths). So `GeometryLegs::{Both, Mesh, Sdf}` each show something, and — with
//! CSM enabled — this is the FIRST interactive scene to exercise the non-empty `VisibilityBuffer ×
//! Both` `HAS_MESH` reverse-Z composite and the `VisibilityBuffer × Sdf` shadow-vocab march that
//! the empty-field goldens (`vb_both`/`vb_sdf_only`) structurally cannot.
//!
//! `#[allow]` nothing special — a normal windowed example. `BOYKO_HOST_DUMP=<path.bmp>` captures
//! one settled frame instead of running interactively (the host's owner-eval channel).

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::mesh::Vertex;
use boyko_render::{Material, MeshAssetsVbExt, MeshGeometryTableSlot, generate_tangents};

/// The sun direction TO the light.
const SUN_DIR: [f32; 3] = [-0.42, 0.80, 0.42];

/// A UV sphere (verbatim generation shape shared with `vb_mesh.rs` — outward normals, UVs).
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

/// A flat +Y-facing quad of half-extent `half`, centered at the origin (the floor receiver).
fn floor_quad(half: f32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let n = [0.0, 1.0, 0.0];
    let mut verts = vec![
        Vertex::new([-half, 0.0, -half], n, color),
        Vertex::new([half, 0.0, -half], n, color),
        Vertex::new([half, 0.0, half], n, color),
        Vertex::new([-half, 0.0, half], n, color),
    ];
    verts[0].uv = [0.0, 0.0];
    verts[1].uv = [1.0, 0.0];
    verts[2].uv = [1.0, 1.0];
    verts[3].uv = [0.0, 1.0];
    let idx = vec![0u32, 1, 2, 0, 2, 3];
    (verts, idx)
}

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko paradigm lab", 900, 640));
    // CSM on (3 cascades) — so every paradigm's shadow seam is exercised, including the
    // VisibilityBuffer x Sdf shadow-vocab march (the empty-field goldens can't reach it).
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
    app.add_startup_system(setup);
    app.run();
}

/// Spawns the matrix-exercising scene. Meshes are registered VB-aware (fallback to plain
/// `register_mesh` when the geometry table is not armed — the SAME `vb_mesh.rs` pattern), so the
/// scene renders in ALL four render paths. One `SdfPrimitive` sphere carries the `Sdf` leg.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    // Floor + one shared sphere mesh, both VB-aware-registered (table when armed, else plain).
    let (floor_v, floor_i) = floor_quad(7.0, [0.55, 0.55, 0.58, 1.0]);
    let (sphere_v, sphere_i) = uv_sphere(0.62, 28, 40, [0.75, 0.75, 0.78, 1.0]);
    let (floor, sphere) = match geo_table.0.as_mut() {
        Some(table) => (
            meshes.register_mesh_vb(dev.get(), &floor_v, &floor_i, table),
            meshes.register_mesh_vb(dev.get(), &sphere_v, &sphere_i, table),
        ),
        None => (
            meshes.register_mesh(dev.get(), &floor_v, &floor_i),
            meshes.register_mesh(dev.get(), &sphere_v, &sphere_i),
        ),
    };

    // Floor: a RECEIVER only (no `ShadowCaster`) so it never casts a whole-plane shadow.
    commands.spawn(MeshBundle::new(floor, Transform::from_translation(Vec3::new(0.0, 0.0, 0.0))));

    // A row of raster spheres with distinct materials (the `Mesh` leg). All share ONE MeshHandle
    // (Decision 9's instanced-draw fixture). Material 0 (index 0) stays the engine default.
    let red = materials.add(Material::new([0.72, 0.05, 0.05, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let gold = materials.add(Material::new([1.0, 0.71, 0.29, 1.0], 1.0, 0.16, 0.5, [0.0; 3], 0));
    let blue = materials.add(Material::new([0.18, 0.36, 0.92, 1.0], 1.0, 0.42, 0.5, [0.0; 3], 0));
    let row: [Option<u16>; 4] =
        [None, Some(red.index() as u16), Some(gold.index() as u16), Some(blue.index() as u16)];
    for (i, mat) in row.iter().enumerate() {
        let x = (i as f32 - 1.5) * 1.55;
        let e = commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(x, 0.62, 0.0))))
            .insert(ShadowCaster)
            .id();
        if let Some(id) = mat {
            commands.entity(e).insert(MaterialHandle(*id));
        }
    }

    // The `Sdf` leg: one live analytic SDF sphere, placed in FRONT of the raster row so under
    // `GeometryLegs::Both` the mesh/SDF composite (min-combine, or the non-Deferred forward-march
    // HAS_MESH depth bound) is visible, and under `GeometryLegs::Sdf` it is the only geometry.
    commands.spawn(SdfPrimitive(SdfEdit::sphere([0.0, 0.75, 2.1], 0.75, sdf_op::UNION, 0.0)));

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

    let pose = Affine3A::look_at_rh(
        Vec3::new(0.0, 1.6, 7.2),
        Vec3::new(0.0, 0.6, 0.4),
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
            aspect: 900.0 / 640.0,
            near: 0.1,
            far: 100.0,
        },
    });
}
