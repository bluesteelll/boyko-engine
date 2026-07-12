//! PBR material showcase — a PROPER scene (not spheres on a black void).
//!
//! The `grand_showcase_2mat` chart floats bare spheres on black: a metal there
//! has *nothing to reflect*, so it can never read as metal. This scene gives the
//! materials a real environment — a large ground plane, a bright/contrasty sky
//! (blue up, neutral ground), and a warm directional sun that casts grounding
//! shadows — so a metal body reflects a bright sky on its upper arc and the floor
//! on its lower arc (via the analytic sky/ground env in `deferred_pbr.hlsl`), the
//! contrast that makes it read as metal.
//!
//! Left to right: chrome (near-mirror), gold, copper, brushed steel (rough metal),
//! and a red dielectric for contrast. All drive their `base_color` + metallic +
//! roughness through the F8 `PER_INSTANCE_MATERIAL` pipeline (software leg, no
//! temporal ⇒ PM binds).
//!
//! Windowed-eval conventions (mirrors `grand_showcase_2mat.rs`): `#[ignore]` (needs
//! a real windowed GPU device), run with `BOYKO_DISABLE_VALIDATION=1` and
//! `--test-threads=1`; `BOYKO_HOST_DUMP=<path.bmp>` arms the screenshot capture.
//! This is a VISUAL-ORACLE eval scene — it is NOT byte-pinned in `goldens/PINS.toml`.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;

/// The sun direction TO the light (shared with the other showcase scenes).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

// NOTE: pinned-golden scene — keeps its local mesh copy verbatim (incl. the
// degenerate pole triangles); migrate to tests/common's fixed uv_sphere at the next
// golden re-bless.
/// Generates a UV-sphere (`stacks` × `slices`) of `radius`, centered at the origin,
/// with outward per-vertex normals, a uniform `color`, spherical UVs (`u =
/// theta/2π`, `v = phi/π`), and a generated tangent basis (feeds a future textured
/// scene, T6). The gbuffer raster is `CullMode::None`, so winding is not
/// load-bearing.
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

/// A large flat floor quad on the XZ plane at y=0, up-facing normal, uniform color,
/// a planar UV, and a generated tangent basis. Two triangles; `CullMode::None`
/// makes winding irrelevant.
fn floor_plane(half: f32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
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
    let idx = vec![0, 1, 2, 0, 2, 3];
    generate_tangents(&mut verts, &idx);
    (verts, idx)
}

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
) {
    // ── The environment: a large matte floor the metals reflect on their lower arc.
    let (fv, fi) = floor_plane(60.0, [0.5, 0.5, 0.52, 1.0]);
    let floor_mesh = meshes.register_mesh(dev.get(), &fv, &fi);
    let floor_mat = materials.add(Material::new([0.16, 0.16, 0.18, 1.0], 0.0, 0.5, 0.5, [0.0; 3], 0));
    let floor = commands
        .spawn(MeshBundle::new(floor_mesh, Transform::from_translation(Vec3::new(0.0, 0.0, 0.0))))
        .id();
    commands.entity(floor).insert(MaterialHandle(floor_mat.index() as u16));

    // ── One smooth high-res sphere mesh, reused for every material instance.
    let (sv, si) = uv_sphere(0.8, 48, 64, [0.7, 0.7, 0.72, 1.0]);
    let sphere = meshes.register_mesh(dev.get(), &sv, &si);

    // ── The materials (LINEAR base_color). Metals use the measured F0 base colors;
    //    roughness climbs left→right across the metals (mirror → brushed).
    let chrome = materials.add(Material::new([0.95, 0.95, 0.97, 1.0], 1.0, 0.05, 0.5, [0.0; 3], 0));
    let gold = materials.add(Material::new([1.0, 0.77, 0.34, 1.0], 1.0, 0.10, 0.5, [0.0; 3], 0));
    let copper = materials.add(Material::new([0.95, 0.64, 0.54, 1.0], 1.0, 0.15, 0.5, [0.0; 3], 0));
    let steel = materials.add(Material::new([0.70, 0.70, 0.73, 1.0], 1.0, 0.42, 0.5, [0.0; 3], 0));
    let red = materials.add(Material::new([0.72, 0.06, 0.06, 1.0], 0.0, 0.35, 0.5, [0.0; 3], 0));

    let row: [u16; 5] = [
        chrome.index() as u16,
        gold.index() as u16,
        copper.index() as u16,
        steel.index() as u16,
        red.index() as u16,
    ];
    let spacing = 2.1;
    for (i, mat) in row.iter().enumerate() {
        let x = (i as f32 - 2.0) * spacing;
        let e = commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(x, 0.8, 0.0))))
            .id();
        commands.entity(e).insert(MaterialHandle(*mat));
    }

    // ── A warm, bright sun at a grazing angle for strong glints + long grounding shadows.
    let sun_pose =
        Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.88], 5.0),
    });

    // ── A bright, CONTRASTY sky: a saturated blue upper hemisphere over a neutral
    //    ground that matches the floor — the high-contrast environment a metal body
    //    mirrors (bright blue on top, floor-grey underneath) that a flat void cannot.
    commands.spawn(SkyLight::new([0.85, 1.05, 1.7], [0.04, 0.04, 0.05]));

    // ── The camera: slightly elevated, looking down the row so the floor foreground
    //    and the sky background are both in frame.
    let pose = Affine3A::look_at_rh(
        Vec3::new(0.0, 2.6, 11.0),
        Vec3::new(0.0, 0.7, 0.0),
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
            fov_y: 50.0 * core::f32::consts::PI / 180.0,
            aspect: 1.0,
            near: 0.1,
            far: 200.0,
        },
    });
}

/// **The PBR showcase dump.** Five materials on a floor under a bright sky — the
/// scene for judging whether metal reads as metal in a real environment.
///
/// `#[ignore]`: needs a real windowed GPU device; the orchestrator runs it on the GPU.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the PBR showcase screenshot"]
fn pbr_showcase_screenshot_dump() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine PBR showcase", 640, 640));
    app.add_startup_system(setup);
    app.run();
}
