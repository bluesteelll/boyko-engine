//! **VB lab** — a comprehensive, fly-around Visibility-Buffer test bed. ONE scene that exercises
//! the whole render stack under `RenderPath::VisibilityBuffer`, inspectable from every angle:
//!
//! * varied MESH geometry — a row of spheres + a stack of cubes carrying a spread of PBR
//!   materials (matte dielectric, gold/blue/copper metal, a rough white, an EMISSIVE panel);
//! * a live SDF leg — two analytic spheres + a smooth-union blob, so `GeometryLegs::{Both,Sdf}`
//!   both show something and the mesh/SDF composite is visible;
//! * a floor receiver + a back wall (bounce + shadow catcher);
//! * a 3-cascade CSM sun with REAL cast shadows, a shadow-casting SPOT (punctual atlas), a
//!   point accent, and a sky/hemisphere fill;
//! * the full post stack — SSAO, DDGI global illumination, and TAA + RCAS sharpen — all armed by
//!   default and each toggleable by env WITHOUT a rebuild.
//!
//! # Run
//! ```text
//! scripts\run-vb-lab.ps1                       # VB, everything on, interactive fly-around
//! scripts\run-vb-lab.ps1 -Aa off -Gi off       # toggle features live (no rebuild)
//! scripts\run-scene.ps1 -Scene vb_lab -Path deferred   # the SAME scene in another paradigm
//! ```
//!
//! # Controls (the engine's `FlyCameraPlugin`)
//! `W`/`S`/`A`/`D` — fly · `Space`/`E` up · `Left Ctrl`/`Q` down · mouse — look · `Esc` — quit.
//!
//! # Feature env (all default ON for a comprehensive test; the render path is `BOYKO_RENDER_PATH`,
//! set by the launcher — default `vb`)
//! * `BOYKO_AA`          = `off` | `fxaa` | `smaa` | `taa`   (default `taa`)
//! * `BOYKO_TAA_SHARPEN` = `none` | `rcas`                   (default `rcas`)
//! * `BOYKO_SSAO`        = `off` | `on`                      (default `on`)
//! * `BOYKO_GI`          = `off` | `on`                      (default `on`)
//! * `BOYKO_CSM_OFF`     = `1`                               (disarm the cascade sun shadows)
//!
//! Meshes register through the PLAIN `register_mesh`/`cube`/`plane` — the boot-time
//! `boyko_render::backfill_vb_geometry_slots` claims their VB geometry-table slots, so the scene
//! renders under `RenderPath::VisibilityBuffer` with no special registration (the same reason
//! `paradigm_lab` does). `BOYKO_HOST_DUMP=<path.bmp>` captures one settled frame and exits.

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::mesh::Vertex;
use boyko_render::{
    AaConfig, AaMode, DdgiConfig, LightingConfig, Material, SharpenMode, SsaoConfig, SsaoQuality,
    TaaConfig, generate_tangents,
};

/// The sun direction TO the light.
const SUN_DIR: [f32; 3] = [-0.42, 0.80, 0.42];

/// A UV sphere (outward normals + UVs + a generated tangent basis) — same helper `paradigm_lab`
/// uses; the engine's `plane`/`cube` cover the flat geometry.
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

/// `true` when a feature env var is anything but the literal `off` (default ON).
fn env_on(key: &str) -> bool {
    std::env::var(key).ok().as_deref() != Some("off")
}

/// A quaternion rotating `angle` radians about +Y (`boyko_math::Quat` has no `from_rotation_y`).
fn rot_y(angle: f32) -> Quat {
    let (s, c) = (angle * 0.5).sin_cos();
    Quat::new(0.0, s, 0.0, c)
}

/// A quaternion rotating `angle` radians about +Z.
fn rot_z(angle: f32) -> Quat {
    let (s, c) = (angle * 0.5).sin_cos();
    Quat::new(0.0, 0.0, s, c)
}

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko VB lab", 1120, 720));
    // The interactive WASD fly-camera stack (input ingest + controller + ECS quit).
    app.add_plugin(FlyCameraPlugin);

    // --- Feature arming: everything ON by default for a comprehensive VB test; each knob is
    //     env-toggleable so the owner can A/B any single feature WITHOUT a rebuild. Every config
    //     defaults DISABLED (the 0%-gate), so inserting these is what turns the stack on. ---
    let csm_on = std::env::var("BOYKO_CSM_OFF").is_err();
    app.insert_resource(CsmConfig { cascade_count: if csm_on { 3 } else { 0 }, ..CsmConfig::default() });
    app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
    // VB-SV0 DP3b: the owner-eval arming channel for the SDF-on-mesh shadow + contact-AO terms.
    // Default OFF — the opposite polarity of `env_on`'s default-on knobs, deliberately: every
    // committed pin renders with SV0 dark, and a knob that armed a brand-new term by default
    // would re-bless four goldens as a side effect of adding it. `BOYKO_SDF_MESH=on` opts in.
    // These are the REQUEST bits; `sync_sv0_light_gate` clamps them against
    // `vb_sdf_mesh_armable()` per frame (monotone downward), so setting them on a non-VB or
    // hwrt-shadowed boot arms nothing.
    let sdf_mesh = std::env::var("BOYKO_SDF_MESH").unwrap_or_default();
    app.insert_resource(LightingConfig {
        csm_shadows: csm_on,
        // `on` arms both; `shadow`/`ao` arm one bit — the S1 (ii-a)/(ii-b) per-bit legs.
        vb_sdf_mesh_shadow: matches!(sdf_mesh.as_str(), "on" | "shadow"),
        vb_sdf_mesh_ao: matches!(sdf_mesh.as_str(), "on" | "ao"),
        ..LightingConfig::default()
    });

    // VB v1 caps SSAO / DDGI / TAA OFF (`cap_vb_v1_consumers`) — the fused `vb_resolve` does not
    // consume them yet (the deferred R9 geo/shade split would unlock them); FXAA / SMAA DO run
    // under VB. So the AA DEFAULT is PATH-AWARE: SMAA under VB, TAA (+ RCAS) under deferred/forward.
    // An EXPLICIT `BOYKO_AA=taa` under VB degrades cleanly to `Off` in `GpuSceneBundles::scene`
    // (TAA is Deferred-only) — no crash; use `-Path deferred` for TAA + RCAS.
    let is_vb = std::env::var("BOYKO_RENDER_PATH").ok().as_deref() == Some("vb");
    let aa_mode = match std::env::var("BOYKO_AA").ok().as_deref() {
        Some("off") => AaMode::Off,
        Some("fxaa") => AaMode::Fxaa,
        Some("smaa") => AaMode::Smaa,
        Some("taa") => AaMode::Taa,
        _ => {
            if is_vb {
                AaMode::Smaa
            } else {
                AaMode::Taa
            }
        }
    };
    app.insert_resource(AaConfig { mode: aa_mode });
    let sharpen = match std::env::var("BOYKO_TAA_SHARPEN").ok().as_deref() {
        Some("none") => SharpenMode::None,
        _ => SharpenMode::Rcas,
    };
    app.insert_resource(TaaConfig { sharpen, ..TaaConfig::default() });

    let ssao_quality = if env_on("BOYKO_SSAO") { SsaoQuality::High } else { SsaoQuality::Off };
    app.insert_resource(SsaoConfig { quality: ssao_quality, ..SsaoConfig::default() });

    app.insert_resource(DdgiConfig { ddgi_indirect: env_on("BOYKO_GI"), ..DdgiConfig::default() });

    println!(
        "[vb_lab] path={} aa={aa_mode:?} sharpen={sharpen:?} ssao={ssao_quality:?} gi={} csm={} \
         | fly: WASD/Space/Ctrl + mouse, Esc quits{}",
        if is_vb { "vb" } else { "raster/deferred" },
        env_on("BOYKO_GI"),
        csm_on,
        if is_vb {
            " | NOTE: VB v1 caps SSAO/DDGI/TAA off -- use -Path deferred for the full post stack"
        } else {
            ""
        }
    );

    app.add_startup_system(setup);
    app.run();
}

/// Spawns the comprehensive scene. Meshes register through the PLAIN `register_mesh`/`cube`/`plane`
/// (no VB-aware threading) — `backfill_vb_geometry_slots` claims their VB slots under a VB boot.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
) {
    let floor = meshes.plane(dev.get(), 22.0);
    let wall = meshes.cube(dev.get(), 1.0);
    let cube = meshes.cube(dev.get(), 1.0);
    let (sphere_v, sphere_i) = uv_sphere(0.62, 28, 40, [0.8, 0.8, 0.82, 1.0]);
    let sphere = meshes.register_mesh(dev.get(), &sphere_v, &sphere_i);

    // Floor: a RECEIVER only (no `ShadowCaster`) so it never casts a whole-plane shadow.
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));

    // A back wall (a flattened cube), well behind the props — a bounce surface for GI + a shadow
    // catcher. Receiver only.
    commands.spawn(MeshBundle::new(
        wall,
        Transform {
            translation: Vec3::new(0.0, 3.0, -4.5),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(14.0, 6.0, 0.4),
        },
    ));

    // --- The material spread (Material::new(base_rgba, metallic, roughness, reflectance,
    //     emissive[3], flags)) — matte / metals / rough / emissive, to show PBR under VB. ---
    let matte = materials.add(Material::new([0.72, 0.06, 0.06, 1.0], 0.0, 0.42, 0.5, [0.0; 3], 0));
    let gold = materials.add(Material::new([1.0, 0.72, 0.30, 1.0], 1.0, 0.14, 0.5, [0.0; 3], 0));
    let blue = materials.add(Material::new([0.16, 0.34, 0.92, 1.0], 1.0, 0.40, 0.5, [0.0; 3], 0));
    let copper = materials.add(Material::new([0.95, 0.55, 0.38, 1.0], 1.0, 0.28, 0.5, [0.0; 3], 0));
    let chalk = materials.add(Material::new([0.86, 0.86, 0.88, 1.0], 0.0, 0.85, 0.5, [0.0; 3], 0));
    let lamp = materials.add(Material::new([0.02, 0.02, 0.02, 1.0], 0.0, 0.6, 0.5, [1.6, 0.9, 0.3], 0));

    // A row of spheres, one material each — spread across X, all shadow casters, all one MeshHandle.
    let row = [matte, gold, blue, copper, chalk];
    for (i, mat) in row.iter().enumerate() {
        let x = (i as f32 - 2.0) * 1.65;
        commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(x, 0.62, 0.0))))
            .insert(ShadowCaster)
            .insert(MaterialHandle(mat.index() as u16));
    }

    // A stack of cubes behind the row — sharp edges + varied metals, to stress the VB visibility
    // resolve on non-smooth geometry (and give the shadows a taller caster).
    let cubes = [(gold, -2.4, 0.5), (blue, 0.0, 0.5), (copper, 2.4, 0.5), (blue, 0.0, 1.5)];
    for (mat, x, y) in cubes {
        commands
            .spawn(MeshBundle::new(
                cube,
                Transform {
                    translation: Vec3::new(x, y, -2.2),
                    rotation: rot_y(0.4),
                    scale: Vec3::new(0.9, 0.9, 0.9),
                },
            ))
            .insert(ShadowCaster)
            .insert(MaterialHandle(mat.index() as u16));
    }

    // An EMISSIVE panel (a thin glowing slab) — a bright bounce source the GI can pick up.
    commands
        .spawn(MeshBundle::new(
            cube,
            Transform {
                translation: Vec3::new(-4.4, 1.4, -1.0),
                rotation: rot_z(0.2),
                scale: Vec3::new(0.25, 1.6, 1.6),
            },
        ))
        .insert(MaterialHandle(lamp.index() as u16));

    // --- The SDF leg: two analytic spheres + a smooth-union blob (a box smoothly merged with a
    //     sphere). The marcher composites every `SdfPrimitive` in the scene, so overlapping UNION
    //     edits with a nonzero smoothness blend into one organic shape. ---
    commands.spawn(SdfPrimitive(SdfEdit::sphere([3.2, 0.85, 1.8], 0.85, sdf_op::UNION, 0.0)));
    commands.spawn(SdfPrimitive(SdfEdit::sphere([-3.4, 0.7, 1.9], 0.7, sdf_op::UNION, 0.0)));
    // The blob: a box + an overlapping sphere, smooth-unioned (smoothness 0.35).
    commands.spawn(SdfPrimitive(SdfEdit::box_shape([0.4, 0.75, 2.6], [0.55, 0.55, 0.55], sdf_op::UNION, 0.35)));
    commands.spawn(SdfPrimitive(SdfEdit::sphere([0.95, 0.95, 2.6], 0.55, sdf_op::UNION, 0.35)));

    // --- Lighting: a CSM directional sun (cast shadows), a sky/hemisphere fill, a shadow-casting
    //     spot (punctual atlas), and a warm point accent. ---
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
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.97, 0.92], 3.2),
    });

    commands.spawn(SkyLight::new([0.28, 0.36, 0.50], [0.15, 0.14, 0.13]));

    // A shadow-casting SPOT from the upper-right, aimed at the prop row (aim via the pose, per
    // `SpotLight`'s doc). `CastsPunctualShadow` opts it into the exact atlas shadow map.
    let spot_eye = Vec3::new(3.6, 4.2, 3.2);
    let spot_pose = Affine3A::look_at_rh(spot_eye, Vec3::new(0.0, 0.5, 0.0), Vec3::new(0.0, 1.0, 0.0));
    commands
        .spawn(SpotLightObject {
            transform: Transform {
                translation: spot_eye,
                rotation: Quat::from_mat3(spot_pose.matrix3),
                scale: Vec3::ONE,
            },
            global: GlobalTransform::IDENTITY,
            light: SpotLight::new(
                [spot_eye.x, spot_eye.y, spot_eye.z],
                [-0.6, -0.7, -0.5],
                [1.0, 0.85, 0.6],
                6000.0,
                14.0,
                16.0,
                26.0,
            ),
        })
        .insert(CastsPunctualShadow);

    // A cool point accent (unshadowed) to give the metals a second highlight to orbit.
    commands.spawn(PointLightObject {
        transform: Transform::from_translation(Vec3::new(-1.8, 2.2, 2.4)),
        global: GlobalTransform::IDENTITY,
        light: PointLight::new([-1.8, 2.2, 2.4], [0.5, 0.7, 1.0], 240.0, 9.0),
    });

    // The FLY camera, back and up, looking at the prop row. `fly_camera_system` overwrites the
    // rotation from yaw/pitch on the first frame, so only the translation seeds the eye.
    commands.spawn(FlyCameraBundle {
        transform: Transform::from_translation(Vec3::new(0.0, 2.1, 8.4)),
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: 52.0 * core::f32::consts::PI / 180.0,
            aspect: 1120.0 / 720.0,
            near: 0.1,
            far: 120.0,
        },
        fly: FlyCamera::default(),
    });
}
