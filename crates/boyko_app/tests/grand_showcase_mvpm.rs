//! F8-mv — the combined MOTION_VECTORS + PER_INSTANCE_MATERIAL gbuffer variant, visual-oracle dump.
//!
//! A near-copy of `grand_showcase_2mat.rs`: the SAME static five-material UV-sphere row (one
//! material each, left to right: pinned default / red / green / gold-metal / blue-metal), the
//! SAME sun + sky + camera. There is NO drifting object and NO `Update` system — the scene is
//! entirely static, so `Δuv == 0` everywhere and the mvpm pipeline's motion-vector output is not
//! itself under test here (that is `shadow_denoise_eval`'s job). The point of THIS dump is the
//! `PER_INSTANCE_MATERIAL` half of the combined pipeline: run it on the hwrt+temporal leg
//! (`--features hwrt`, `BOYKO_SHADOW_DENOISE=temporal` or `both`) so `mesh_mvpm_active()` is true
//! and the five spheres draw through `gbuffer_mrt_mvpm` instead of `gbuffer_mrt_mv` — before F8-mv,
//! a material-bearing scene under temporal denoise silently rendered DEFAULT materials (MV took
//! priority over PM, hardcoding material id 0); this dump is the owner's visual sign-off that the
//! combined pipeline now renders the five distinct materials correctly under temporal denoise.
//!
//! This is a VISUAL-ORACLE-only dump: the orchestrator eyeballs the screenshot under temporal
//! denoise (which is not bit-reproducible across settle-frame counts) — it is NOT byte-pinned in
//! `goldens/PINS.toml`.
//!
//! `#[ignore]`: needs a real windowed GPU device with ray-query + RG16 storage support. Run with
//! `--features hwrt`, `BOYKO_SHADOW_DENOISE=temporal` (or `both`), `BOYKO_DISABLE_VALIDATION=1`;
//! the orchestrator runs it on the GPU to dump the screenshot.

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
    // neutral: every instance here carries a material, so the PM half of the mvpm
    // pipeline sources the albedo from `Materials[id].base_color`, not this vertex color.
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

/// **The F8-mv combined-pipeline visual-oracle dump.** The SAME five-sphere / five-material
/// static scene as `grand_showcase_2mat`, at the native 512×512 composite extent, run on the
/// hwrt+temporal leg so `mesh_mvpm_active()` selects `gbuffer_mrt_mvpm` instead of the MV-only
/// pipeline — the owner's RTX visual sign-off that materials survive under temporal denoise.
///
/// `#[ignore]`: needs a real windowed GPU device with ray-query + RG16 storage. Run with
/// `--features hwrt`, `BOYKO_SHADOW_DENOISE=temporal` (or `both`), `BOYKO_DISABLE_VALIDATION=1`;
/// the orchestrator runs it on the GPU to dump the screenshot (not byte-pinned — see module doc).
#[test]
#[ignore = "needs a real windowed hwrt+temporal-capable GPU device; the orchestrator runs it to dump the F8-mv screenshot"]
fn grand_showcase_mvpm_screenshot_dump() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine grand showcase mvpm", 512, 512));
    app.add_startup_system(setup);
    app.run();
}
