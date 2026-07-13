//! Multi-paradigm render-path plan, rung R4b-b — the Forward v1 mesh-only golden dump.
//!
//! Code-review golden re-route: the original `#[ignore]` golden
//! (`engine_forward_mesh_512_screenshot_dump`,
//! `boyko_rhi_vulkan/tests/window_present_gbuffer.rs`) could never actually exercise
//! `RenderPath::Forward` — that test file's `run_showcase_body` harness independently
//! re-implements boot + scene assembly (it does NOT go through `boyko_app::gpu_scene`), so its
//! `GBufferScene` literals carried type-matching-but-semantically-wrong PLACEHOLDER values for
//! the 5 Forward-only fields. This test instead boots through the REAL, PRODUCTION
//! `boyko_app::runner`/`gpu_scene::GpuSceneBundles` path (the `grand_showcase_2mat` pattern),
//! which has genuine Forward wiring (`GpuSceneBundles::boot`'s unconditional
//! `forward_pipeline`/`forward_layout0`/`forward_layout1` creation).
//!
//! Reuses [`grand_showcase_2mat`]'s EXACT five-sphere scene (same mesh, same five materials,
//! same sun/sky, same camera) verbatim — so the dumped BMP is a DIRECT visual comparator against
//! the Deferred `f6147f90` golden: same geometry/lighting/shadows, only the render PATH differs
//! (Forward's inline all-lights shading vs Deferred's fat-gbuffer + compute resolve). The ONE
//! delta from that test is inserting [`boyko_render::RenderPathConfig`] (`Forward` × `Mesh`) as a
//! `World` resource before `app.run()` — `boyko_app::runner` reads it via `world.try_resource`
//! at boot (Decision 1, a one-time commitment) and resolves `ResolvedRenderPath` from it,
//! overriding `RenderPathPlugin`'s `Deferred`-default insertion (`app.add_plugins` runs first,
//! this `insert_resource` after — the SAME "owner override after plugins" ordering
//! `grand_showcase_2mat.rs`'s own AA/SSAO env-toggle inserts use).
//!
//! `GeometryLegs::Mesh` is requested directly (not `Both`) — the resolver would collapse
//! `Both`/`Sdf` to `Mesh` anyway pre-R-SDFFWD (`LegsCollapsedToMeshPreSdfForward`), and this
//! scene has no SDF geometry regardless, so requesting `Mesh` directly resolves CLEAN (no
//! degrade) for a straightforward v1 dump. Every pre-light consumer (SSAO/DDGI/shadow-denoise/
//! TAA) stays at its `AaPlugin`/`SsaoPlugin` default (`Off`) — Forward v1 has no producer for any
//! of them (`cap_forward_v1_consumers`), so leaving them off avoids the (harmless but noisy)
//! `ForwardPreLightConsumersNotYetImplemented`/`ForwardTaaNotYetImplemented` boot warns the
//! `grand_showcase_2mat` env-toggle knobs would otherwise trigger under Forward.
//!
//! Windowed-test conventions (mirrors `grand_showcase_2mat.rs`): `#[ignore]` (needs a real
//! windowed GPU device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.
//! `BOYKO_HOST_DUMP=<path.bmp>` arms the `boyko_app::host_dump` screenshot capture; see
//! `goldens/PINS.toml`'s `[forward_mesh]` pin.

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

/// Verbatim copy of `grand_showcase_2mat.rs::setup` — the SAME five-sphere scene, so the dumped
/// BMP is a direct Forward-vs-Deferred visual comparator against `f6147f90`.
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

/// **The Forward v1 mesh-only golden dump.** The SAME five-sphere `grand_showcase_2mat` scene,
/// rendered through `RenderPath::Forward × GeometryLegs::Mesh` instead of the Deferred default —
/// the owner's RTX visual sign-off gate for rung R4b-b (`goldens/PINS.toml`'s `[forward_mesh]`).
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the Forward v1 mesh-only screenshot"]
fn forward_mesh_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine forward mesh-only", 512, 512);
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Multi-paradigm render-path plan, rung R4b-b: request `Forward × Mesh` — inserted AFTER
    // `add_plugins` (which installs `RenderPathPlugin`'s `Deferred`-default) so this override
    // wins, mirroring `grand_showcase_2mat.rs`'s own post-plugins owner-override inserts.
    app.insert_resource(RenderPathConfig { path: RenderPath::Forward, legs: GeometryLegs::Mesh });
    app.run();
}
