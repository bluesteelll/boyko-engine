//! Rung VB-P1b — the VisibilityBuffer FUSED-`vb_resolve` FROXEL light-cull equality golden
//! (`docs/VB-PERFORMANCE-TRACK.md`'s VB-P1). [`vb_mesh.rs`](../vb_mesh.rs)'s five-sphere
//! `grand_showcase_2mat` scene, verbatim, PLUS many point/spot lights (comfortably below the
//! [`MAX_LIGHTS_PER_CLUSTER`]/[`INDEX_LIST_CAP`] caps) — some carrying
//! [`CastsPunctualShadow`] (VB-P1-0's regression target: the froxel cull must mask a light's
//! kind word before the POINT/SPOT test, or a shadow-flagged/atlas-slotted light silently
//! vanishes under clustering while the flat resolve keeps it) — rendered through
//! `RenderPath::VisibilityBuffer × GeometryLegs::Mesh` with `LightingConfig::clusters_enabled`
//! armed (VB-P1b: `boyko_app::runner` now threads this real toggle into
//! `ResolvedRenderPath::froxel_light_cull` instead of VB-P1a's hardcoded `false`).
//!
//! # The equality contract
//!
//! Below [`MAX_LIGHTS_PER_CLUSTER`] (256) and [`INDEX_LIST_CAP`] (16384), the froxel-culled
//! `LightIndexList` a pixel's cluster walks is, for every froxel this scene's lights actually
//! reach, exactly the SAME point/spot lights (in the SAME ascending table index order) the
//! flat `[l0a_count, light_count)` scan would visit for that pixel — `vb_resolve.comp.hlsl`'s
//! loop BODY (range test, falloff, spot cone, punctual atlas shadow, BSDF accumulate) is
//! TOKEN-FOR-TOKEN identical between the two arms; only the index-list SOURCE differs. So the
//! froxel-ON render of this scene MUST be BYTE-IDENTICAL to the SAME scene rendered
//! clusters-OFF (flat) — the SAME per-light FP accumulation, same order, same terms. This is
//! the O3 shared-frame-oracle equality proof `[vb_both]`/`[vb_sdf_only]` already use against
//! `[vb_mesh]`, applied to the froxel arm instead of a different geometry leg.
//!
//! The shadow-flagged/atlas-slotted lights are deliberately present so this equality would
//! FAIL (not vacuously pass) if the VB-P1-0 kind-word-masking bug (`cluster_cull.hlsl` testing
//! the RAW kind word — bit 16 `casts_sdf_shadow` + bits 17..21 the atlas slot — instead of the
//! masked `light_kind(L)`) were ever reintroduced: pre-VB-P1-0, the cull silently dropped such
//! a light under clustering while the flat resolve kept it, so the two legs would diverge.
//!
//! # The clusters on/off knob
//!
//! `BOYKO_VB_FROXEL_FORCE_OFF` (any value; presence is the trigger) forces
//! `LightingConfig::clusters_enabled = false` on this SAME scene — the flat baseline leg. Unset
//! (the default) arms clustering. One dump fn + an env toggle renders BOTH legs of the
//! identical scene without a near-duplicate second test file (mirrors `vb_mesh_tex.rs`'s own
//! `BOYKO_PATH` knob for its Deferred-vs-VB parity dump).
//!
//! Windowed-test conventions (mirrors `vb_mesh.rs`): `#[ignore]` (needs a real windowed GPU
//! device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`. `BOYKO_HOST_DUMP=
//! <path.bmp>` arms the `boyko_app::host_dump` screenshot capture; see `goldens/PINS.toml`'s
//! `[vb_mesh_froxel]` pin (UNBLESSED — the orchestrator renders BOTH legs, confirms byte
//! equality, then blesses to the shared hash).

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;
use boyko_render::{
    ClusterConfig, GeometryLegs, LightingConfig, MeshAssetsVbExt, MeshGeometryTableSlot,
    RenderPath, RenderPathConfig,
};

/// The sun direction TO the light (byte-identical to `grand_showcase_2mat.rs`'s / `vb_mesh.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// Verbatim copy of `vb_mesh.rs::uv_sphere` (itself a verbatim copy of
/// `grand_showcase_2mat.rs::uv_sphere`) — see that file's NOTE for why this is a local copy
/// rather than a shared `tests/common` helper (a pinned-golden scene keeps its exact mesh
/// generation frozen).
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

/// One point light's placement + intensity — a plain data row so [`setup`]'s spawn loop stays
/// short (14 point/spot lights would otherwise be 14 near-identical `commands.spawn` blocks).
struct PointRow {
    position: [f32; 3],
    color: [f32; 3],
    power: f32,
    range: f32,
    /// `true` ⇒ carries [`CastsPunctualShadow`] (an atlas-slot candidate).
    shadow: bool,
}

/// One spot light's placement + intensity + cone.
struct SpotRow {
    position: [f32; 3],
    color: [f32; 3],
    power: f32,
    range: f32,
    inner_deg: f32,
    outer_deg: f32,
    /// `true` ⇒ carries [`CastsPunctualShadow`] (an atlas-slot candidate).
    shadow: bool,
}

/// Ten point lights around the five-sphere row — comfortably below [`MAX_LIGHTS_PER_CLUSTER`]
/// (256) and [`INDEX_LIST_CAP`] (16384), with modest ranges (2.5-4.0) so distinct froxels see
/// distinct subsets (clustering actually culls, rather than every froxel holding every light).
/// Row 0 is the shadow-flagged reference point VB-P1-0 protects.
const POINTS: [PointRow; 10] = [
    PointRow { position: [-3.5, 1.5, 1.0], color: [1.0, 0.75, 0.5], power: 80.0, range: 3.5, shadow: true },
    PointRow { position: [-2.0, 1.8, -1.0], color: [0.6, 0.8, 1.0], power: 60.0, range: 3.0, shadow: false },
    PointRow { position: [-0.8, 2.0, 1.5], color: [1.0, 0.6, 0.6], power: 70.0, range: 3.5, shadow: false },
    PointRow { position: [0.0, 2.2, -1.2], color: [0.7, 1.0, 0.7], power: 65.0, range: 3.0, shadow: false },
    PointRow { position: [0.8, 2.0, 1.3], color: [0.8, 0.8, 1.0], power: 70.0, range: 3.5, shadow: false },
    PointRow { position: [2.0, 1.8, -1.0], color: [1.0, 0.9, 0.6], power: 60.0, range: 3.0, shadow: false },
    PointRow { position: [3.5, 1.5, 1.0], color: [0.6, 1.0, 0.9], power: 80.0, range: 3.5, shadow: false },
    PointRow { position: [-1.5, 0.3, 2.5], color: [1.0, 0.5, 0.8], power: 50.0, range: 2.5, shadow: false },
    PointRow { position: [1.5, 0.3, 2.5], color: [0.5, 0.9, 1.0], power: 50.0, range: 2.5, shadow: false },
    PointRow { position: [0.0, 3.0, 0.0], color: [1.0, 1.0, 0.85], power: 90.0, range: 4.0, shadow: false },
];

/// Four spot lights bracketing the row from above. Row 0 is the atlas-slotted reference spot
/// VB-P1-0 protects (deliberately ALSO shadow-flagged — `pack_atlas_slot` couples the two bits
/// in production: a real atlas base always sets `casts_sdf_shadow` too, see
/// `shadow_atlas::pack_atlas_slot`'s own doc).
const SPOTS: [SpotRow; 4] = [
    SpotRow {
        position: [-2.5, 3.0, 3.0],
        color: [1.0, 0.85, 0.7],
        power: 200.0,
        range: 6.0,
        inner_deg: 15.0,
        outer_deg: 30.0,
        shadow: true,
    },
    SpotRow {
        position: [2.5, 3.0, 3.0],
        color: [0.75, 0.85, 1.0],
        power: 180.0,
        range: 6.0,
        inner_deg: 15.0,
        outer_deg: 28.0,
        shadow: false,
    },
    SpotRow {
        position: [0.0, 3.5, 4.0],
        color: [0.9, 1.0, 0.9],
        power: 220.0,
        range: 6.5,
        inner_deg: 12.0,
        outer_deg: 25.0,
        shadow: false,
    },
    SpotRow {
        position: [-3.5, 2.0, -2.0],
        color: [1.0, 0.7, 0.7],
        power: 150.0,
        range: 5.0,
        inner_deg: 18.0,
        outer_deg: 32.0,
        shadow: false,
    },
];

/// Verbatim copy of `vb_mesh.rs::setup` — the SAME five-sphere scene — PLUS [`POINTS`]/[`SPOTS`]
/// (14 point/spot lights total, 2 of them `CastsPunctualShadow`-flagged) and
/// `ShadowConfig{enabled:true}` (so `resolve_shadow_atlas` actually assigns the flagged lights a
/// real atlas base, which is what sets their `casts_sdf_shadow` bit — see
/// `shadow_atlas::pack_atlas_slot`'s doc: a real slot always sets that bit too).
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

    // The froxel-cull exercise: 10 points + 4 spots, 2 of them CastsPunctualShadow-flagged
    // (VB-P1-0's regression target — see this file's module doc).
    for row in &POINTS {
        let p = Vec3::new(row.position[0], row.position[1], row.position[2]);
        let e = commands
            .spawn(PointLightObject {
                transform: Transform::from_translation(p),
                global: GlobalTransform::IDENTITY,
                light: PointLight::new(row.position, row.color, row.power, row.range),
            })
            .id();
        if row.shadow {
            commands.entity(e).insert(CastsPunctualShadow);
        }
    }
    for row in &SPOTS {
        let p = Vec3::new(row.position[0], row.position[1], row.position[2]);
        let aim = Vec3::new(0.0, 0.6, 0.0); // aim down at the sphere row
        let pose = Affine3A::look_at_rh(p, aim, Vec3::new(0.0, 1.0, 0.0));
        let e = commands
            .spawn(SpotLightObject {
                transform: Transform {
                    translation: p,
                    rotation: Quat::from_mat3(pose.matrix3),
                    scale: Vec3::ONE,
                },
                global: GlobalTransform::IDENTITY,
                light: SpotLight::new(
                    row.position,
                    [0.0, -1.0, 0.0],
                    row.color,
                    row.power,
                    row.range,
                    row.inner_deg,
                    row.outer_deg,
                ),
            })
            .id();
        if row.shadow {
            commands.entity(e).insert(CastsPunctualShadow);
        }
    }

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

/// **The VisibilityBuffer FROXEL light-cull equality golden dump (rung VB-P1b).** The
/// [`POINTS`]/[`SPOTS`]-augmented `[vb_mesh]` scene, rendered through
/// `RenderPath::VisibilityBuffer × GeometryLegs::Mesh` with `LightingConfig::clusters_enabled`
/// armed by default — `BOYKO_VB_FROXEL_FORCE_OFF` forces the flat (clusters-off) baseline leg
/// of the SAME scene instead (see this file's module doc for the equality contract both legs
/// must satisfy). The owner's RTX visual sign-off gate for rung VB-P1b
/// (`goldens/PINS.toml`'s `[vb_mesh_froxel]`).
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it TWICE on the GPU (once with `BOYKO_VB_FROXEL_FORCE_OFF` unset, once
/// with it set) to dump both legs and confirm byte equality before blessing.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator renders both the froxel-ON and \
            clusters-OFF legs to confirm byte equality before blessing"]
fn vb_mesh_froxel_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine vb mesh froxel", 512, 512);
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Rung VB-P1b: request `VisibilityBuffer × Mesh` — inserted AFTER `add_plugins` (which
    // installs `RenderPathPlugin`'s `Deferred`-default), mirroring `vb_mesh.rs`'s own
    // post-plugins owner-override insert.
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh });
    // Arms the shadow atlas so `resolve_shadow_atlas` assigns the two `CastsPunctualShadow`
    // rows ([`POINTS`]`[0]`/[`SPOTS`]`[0]`) a real atlas base — the SAME production mechanism
    // that sets their `casts_sdf_shadow` kind-word bit (`shadow_atlas::pack_atlas_slot`'s doc).
    app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
    // The clusters on/off knob (this file's module doc): unset arms clustering (the froxel
    // leg), `BOYKO_VB_FROXEL_FORCE_OFF` forces it off (the flat baseline leg) on the
    // IDENTICAL scene. `ClusterConfig::default()` (16x9x24) is pinned explicitly so a future
    // change to `EnginePlugins`' own default cannot silently redefine this golden's grid.
    let clusters_enabled = std::env::var("BOYKO_VB_FROXEL_FORCE_OFF").is_err();
    app.insert_resource(LightingConfig { clusters_enabled, ..LightingConfig::default() });
    app.insert_resource(ClusterConfig::default());
    app.run();
}
