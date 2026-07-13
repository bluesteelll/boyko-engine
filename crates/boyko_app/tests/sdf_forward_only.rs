//! Multi-paradigm render-path plan, rung R-SDFFWD — the Forward x Sdf (mesh-less) golden dump.
//!
//! The FIRST scene to exercise `GeometryLegs::Sdf` under a Forward-family path: NO mesh entities
//! are spawned (`GeometryLegs::Sdf` is a "zero mesh cost" leg toggle — the app simply does not
//! populate `StaticMesh`/`MeshBundle` content for a leg it does not use, the SAME
//! "capability = component presence" discipline every other leg-toggle scene in this repo
//! follows), booted through the REAL `boyko_app::runner`/`gpu_scene::GpuSceneBundles` path (the
//! `forward_mesh.rs`/`forward_both.rs` pattern) with `RenderPathConfig{path: Forward, legs: Sdf}`.
//!
//! `SDF_FORWARD_IMPLEMENTED` is `true` (`boyko_render::render_path_config`), so `Forward x Sdf`
//! resolves CLEAN — `sdf_leg == true` arms `sdf_forward_marched` and the mesh-less
//! `sdf_forward_march` compute pipeline variant (`declare_forward_graph`'s `sdf_forward_march`
//! arm, `mesh_leg == false` so `record_forward` selects
//! `GBufferScene::sdf_forward_march_sdfonly_pipeline` and skips the `HAS_MESH` `forward_depth`
//! sample). `needs_depth_prepass` also stays OFF under `ForwardPlus x Sdf` (rung R-SDFFWD's
//! `resolve_rules` mesh_leg gate — nothing for the prepass to cull with no mesh leg), but this
//! scene requests plain `Forward`, which has no prepass regardless.
//!
//! # Expected: sky-only
//!
//! No mesh entities exist (`forward_opaque`'s raster loop draws nothing but the `forward_sky`
//! background), and this scene's SDF edit-list is boot-seeded EMPTY (`GpuSceneBundles::boot`'s
//! doc: `count == 0` — no test in this repo's `boyko_app`-level ECS API populates real SDF edits),
//! so `sdf_forward_march`'s dispatch marches every pixel against an EMPTY field, finds NO hit
//! anywhere, and (per the pass's own "a miss writes nothing" contract —
//! `shaders/sdf_forward_march.comp.hlsl`'s header doc) stores NOTHING into `gLit`. The frame is
//! therefore expected to be the `forward_sky.{vs,fs}.hlsl` analytic sky/ground gradient + sun disc
//! ONLY — the SAME background `[forward_mesh]`'s five spheres sit in front of, with no geometry
//! drawn over it (`goldens/PINS.toml`'s `[sdf_forward_only]` pin).
//!
//! Windowed-test conventions (mirrors `forward_mesh.rs`): `#[ignore]` (needs a real windowed GPU
//! device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`. `BOYKO_HOST_DUMP=<path.bmp>`
//! arms the `boyko_app::host_dump` screenshot capture; see `goldens/PINS.toml`'s
//! `[sdf_forward_only]` pin.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_render::{GeometryLegs, RenderPath, RenderPathConfig};

/// The sun direction TO the light (byte-identical to `grand_showcase_2mat.rs`'s / `forward_mesh.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// NO mesh entities — only the sun/sky/camera (`GeometryLegs::Sdf`'s "zero mesh cost" contract:
/// an app using this leg set spawns no `StaticMesh` content). Verbatim camera/light framing from
/// `forward_mesh.rs::setup` (minus the five spheres + their materials).
fn setup(mut commands: Commands) {
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

/// **The Forward x Sdf (mesh-less) golden dump.** No mesh geometry, an empty SDF field —
/// exercises the mesh-less `sdf_forward_march_sdfonly_pipeline` dispatch (see this file's own doc
/// for the sky-only expectation).
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the Forward x Sdf screenshot"]
fn sdf_forward_only_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine sdf forward only", 512, 512);
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Multi-paradigm render-path plan, rung R-SDFFWD: request `Forward x Sdf` — inserted AFTER
    // `add_plugins` (which installs `RenderPathPlugin`'s `Deferred`-default) so this override
    // wins, mirroring `forward_mesh.rs`'s own post-plugins owner-override insert.
    app.insert_resource(RenderPathConfig { path: RenderPath::Forward, legs: GeometryLegs::Sdf });
    app.run();
}
