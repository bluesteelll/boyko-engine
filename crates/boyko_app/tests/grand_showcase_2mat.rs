//! Asset-streaming plan F8 — the material-showcase golden dump.
//!
//! A row of five UV-spheres, one material each, exercising the
//! `PER_INSTANCE_MATERIAL` raster pipeline (F8) — the FIRST scene where a
//! per-instance material drives BOTH the mesh COLOR (F8 base_color→gAlbedo
//! extension) AND its metallic/roughness. Left to right:
//!   0. the pinned default material (`MaterialHandle(0)`, mid-gray dielectric),
//!   1. a matte RED dielectric,
//!   2. a matte GREEN dielectric,
//!   3. a GOLD metal, low roughness (sharp specular highlight — the metalness cue),
//!   4. a BLUE metal, higher roughness (broad, soft highlight).
//!
//! Any non-default material flips `MeshRenderScratch::any_non_default_material`,
//! arming `raster_pipeline_pm`; the resolve already samples `Materials[mat_id]`
//! for metallic/roughness, and the PM gbuffer now also writes the material's
//! `base_color` into `gNormal`'s albedo target — so the five spheres read as
//! five visibly distinct surfaces (previously every mesh pixel hardcoded
//! `DEFAULT_MESH_MATERIAL_ID = 0`, the confirmed bug F8 closes). Runs on the
//! SOFTWARE leg (no temporal denoise ⇒ `mv_active` false ⇒ PM binds — MV would
//! otherwise take priority, F8 §2.3).
//!
//! The gbuffer raster is `CullMode::None` (gpu_scene/mod.rs), so the generated
//! sphere's triangle winding is irrelevant to visibility; only the outward
//! per-vertex normals (set below) matter, for lighting.
//!
//! Windowed-test conventions (mirrors `windowed_smoke.rs`): `#[ignore]` (needs a
//! real windowed GPU device), run with `BOYKO_DISABLE_VALIDATION=1` and
//! `--test-threads=1`. `BOYKO_HOST_DUMP=<path.bmp>` arms the `boyko_app::host_dump`
//! screenshot capture; see `goldens/PINS.toml`'s `[grand_showcase_2mat]` pin.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;

/// The sun direction TO the light (mirrors `shadow_denoise_eval`'s showcase sun).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

// NOTE: pinned-golden scene — keeps its local mesh copy verbatim (incl. the
// degenerate pole triangles); migrate to tests/common's fixed uv_sphere at the next
// golden re-bless.
/// Generates a UV-sphere (`stacks` latitude bands × `slices` longitude segments)
/// of the given `radius`, centered at the model-space origin, with outward
/// per-vertex normals, a uniform `color`, spherical UVs (`u = theta/2π`, `v =
/// phi/π`), and a generated tangent basis. Winding is CCW-ish but the gbuffer
/// raster is `CullMode::None`, so it is not load-bearing. Vertex count stays well
/// under the `Uint16` index limit for the sizes used here.
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

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
) {
    // One smooth sphere mesh, reused for all five instances. The vertex color is
    // neutral: every instance here carries a material, so the PM pipeline sources
    // the albedo from `Materials[id].base_color`, not this vertex color.
    let (verts, idx) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    let sphere = meshes.register_mesh(dev.get(), &verts, &idx);

    // Five materials (LINEAR base_color). Dielectrics (metallic 0) show their
    // base_color as bright diffuse; metals (metallic 1) show base_color as a
    // tinted specular reflection + a highlight whose tightness reveals roughness.
    let red = materials.add(Material::new([0.72, 0.04, 0.04, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let green = materials.add(Material::new([0.05, 0.46, 0.10, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let gold = materials.add(Material::new([1.0, 0.71, 0.29, 1.0], 1.0, 0.13, 0.5, [0.0; 3], 0));
    let blue = materials.add(Material::new([0.20, 0.38, 0.92, 1.0], 1.0, 0.42, 0.5, [0.0; 3], 0));

    // The row: default(0) · red · green · gold-metal · blue-metal, left to right.
    let spacing = 1.55;
    let materials_row: [Option<u16>; 5] =
        [None, Some(red.index() as u16), Some(green.index() as u16), Some(gold.index() as u16), Some(blue.index() as u16)];
    for (i, mat) in materials_row.iter().enumerate() {
        let x = (i as f32 - 2.0) * spacing;
        let e = commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(x, 0.6, 0.0))))
            .id();
        // Object 0 keeps the bundle's default `MaterialHandle(0)`; the rest override
        // it (fires the F2 refcount hooks — `on_replace(-1)` for the pinned slot 0,
        // then `on_insert(+1)` for the real material).
        if let Some(id) = mat {
            commands.entity(e).insert(MaterialHandle(*id));
        }
    }

    // A bright angled sun so the metals catch a clear specular highlight.
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

    // A brighter sky fill than the 2-cube scene: metals reflect the environment,
    // so a dark sky leaves them near-black. This lifts the metal bodies enough to
    // read their tint without washing out the matte dielectrics.
    commands.spawn(SkyLight::new([0.38, 0.44, 0.55], [0.20, 0.20, 0.22]));

    // The camera frames the whole five-sphere row head-on.
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

/// **The material-showcase golden dump.** Five spheres, five distinct materials,
/// at the native 512×512 composite extent — the owner's RTX visual sign-off gate
/// for the F8 `PER_INSTANCE_MATERIAL` pipeline + the base_color→albedo extension
/// (`goldens/PINS.toml`'s `[grand_showcase_2mat]`).
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`;
/// the orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the material-showcase screenshot"]
fn grand_showcase_2mat_screenshot_dump() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine grand showcase materials", 512, 512));
    app.add_startup_system(setup);
    app.run();
}
