//! Multi-paradigm render-path plan, rung R10 — the VisibilityBuffer x Sdf (mesh-less) golden dump.
//!
//! The FIRST scene to exercise `GeometryLegs::Sdf` under the VisibilityBuffer path: NO mesh
//! entities are spawned (`GeometryLegs::Sdf`'s "zero mesh cost" contract), booted through the REAL
//! `boyko_app::runner`/`gpu_scene::GpuSceneBundles` path with
//! `RenderPathConfig{path: VisibilityBuffer, legs: Sdf}`. Clones `sdf_forward_only.rs`'s scene
//! (same sun/sky/camera framing) verbatim — only the render PATH differs.
//!
//! `VB_SDF_IMPLEMENTED` is `true` (rung R10), so `VisibilityBuffer x Sdf` resolves CLEAN —
//! `sdf_leg == true` arms `sdf_forward_marched`, `mesh_leg == false` gates `vb_raster`/`vb_resolve`
//! OFF entirely (they need the Decision-0 geometry table, which carries no slot with no mesh leg),
//! and `vb_geometry_table` stays `false`. `vb_sky` (the sky background) + the mesh-less
//! `sdf_forward_march_sdfonly_pipeline` variant are the sole `lit` producers (`declare_vb_graph`'s
//! `mesh_leg` gate + `sdf_forward_march` arm; `record_vb` matches).
//!
//! # Expected: sky-only (compared against `sdf_forward_only`'s `a1256bde`)
//!
//! No mesh entities exist and this scene's SDF edit-list is boot-seeded EMPTY (`count == 0`), so
//! `sdf_forward_march`'s dispatch marches every pixel against an EMPTY field, finds NO hit
//! anywhere, and (per the pass's own "a miss writes nothing" contract) stores NOTHING into `gLit`.
//! The frame is therefore expected to be the `forward_sky.{vs,fs}.hlsl` analytic sky/ground
//! gradient + sun disc ONLY — the SAME background `sdf_forward_only.rs` (Forward x Sdf) produces.
//! `vb_sky` REUSES the compiled `forward_sky` SPIR-V verbatim (`VbPassPlan::vb_sky`'s doc), so with
//! the same camera/sun uniforms the sky pixels are EXPECTED byte-identical to `sdf_forward_only`'s
//! `a1256bde` — the orchestrator confirms the equality on the GPU (blessing a distinct pin if the
//! separate `vb_sky` pipeline object diverges by any ULP).
//!
//! Windowed-test conventions (mirrors `sdf_forward_only.rs`): `#[ignore]` (needs a real windowed
//! GPU device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.
//! `BOYKO_HOST_DUMP=<path.bmp>` arms the `boyko_app::host_dump` screenshot capture; see
//! `goldens/PINS.toml`'s `[vb_sdf_only]` pin.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_render::{GeometryLegs, RenderPath, RenderPathConfig};

/// The sun direction TO the light (byte-identical to `grand_showcase_2mat.rs`'s / `sdf_forward_only.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// NO mesh entities — only the sun/sky/camera (`GeometryLegs::Sdf`'s "zero mesh cost" contract).
/// Verbatim camera/light framing from `sdf_forward_only.rs::setup`.
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

/// **The VisibilityBuffer x Sdf (mesh-less) golden dump.** No mesh geometry, an empty SDF field —
/// exercises the `mesh_leg`-gated-off `vb_raster`/`vb_resolve` pair (skipped) + the mesh-less
/// `sdf_forward_march_sdfonly_pipeline` dispatch over `vb_sky` (see this file's own doc for the
/// sky-only expectation).
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the VisibilityBuffer x Sdf screenshot"]
fn vb_sdf_only_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine vb sdf only", 512, 512);
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Multi-paradigm render-path plan, rung R10: request `VisibilityBuffer x Sdf` — inserted AFTER
    // `add_plugins` (which installs `RenderPathPlugin`'s `Deferred`-default) so this override wins,
    // mirroring `sdf_forward_only.rs`'s own post-plugins owner-override insert.
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Sdf });
    app.run();
}
