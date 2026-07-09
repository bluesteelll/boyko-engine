//! The R7 SDF-instance milestone: the `room.rs` scene (a floor plane, four
//! shadow-casting cubes, a perspective camera, and ECS-owned lighting) PLUS one
//! LIVE SDF sphere authored ECS-natively as an [`SdfPrimitive`] entity — direct-
//! marched by the analytic marcher and composited into the SAME G-buffer as the
//! raster cubes (min-combine), lit by the same sun with an analytic soft shadow.
//!
//! The SDF sphere is authored with a single spawn:
//! `commands.spawn(SdfPrimitive(SdfEdit::sphere([x, y, z], r, sdf_op::UNION, 0.0)))`.
//! No brick bake, no shader change — v1 is boot-static: the startup gather folds
//! every `SdfPrimitive` into the marcher's edit list, and the host writes it once
//! on the first frame. Material 0 (the engine default dielectric) — the sphere
//! SHAPE among the cubes + its ROUND sun soft-shadow is the composite proof.
//!
//! Run: `cargo run -p boyko-app --example sdf_room`
//!
//! Set `BOYKO_HOST_DUMP=<path>` to capture one frame to a BMP (the host's
//! diagnostic / owner-eval channel) instead of running interactively — the round
//! SDF sphere composited among the cubes, lit + sun-shadowed, is the visual oracle.

use boyko_app::prelude::*;

/// The sun direction TO the light (the engine's familiar showcase sun) — also
/// the `-Z` forward the sun entity's transform is oriented along, so the
/// transform-driven `light_reconcile` derives the same direction it was
/// authored with.
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko sdf room", 800, 600));
    // Enable CSM (the owner-set knob; `CsmPlugin`'s default is DISABLED — the
    // 0%-gate). Three cascades over the room's 30-unit shadow range; the
    // default 2048 resolution matches the host cascade texture.
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.add_startup_system(setup);
    app.run();
}

/// Spawns the room + one SDF sphere: startup runs WITH the device present, so
/// meshes register straight through the world-resident `GpuDevice` +
/// `MeshRegistry`, and the `SdfPrimitive` lands in the World before the startup
/// `collect_sdf_edits` gather folds it (drained in `finish()`).
fn setup(mut commands: Commands, mut meshes: NonSendResMut<MeshRegistry>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);

    // The floor is a RECEIVER only (no `ShadowCaster`): it never stamps itself
    // into the cascades, so it cannot cast a spurious whole-plane shadow.
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    // The cubes cast: the structural `ShadowCaster` marker routes them into the
    // cascade depth pass through the production `gather_shadow_casters`.
    for (x, z) in [(-2.0, -1.0), (0.0, -2.5), (1.8, -0.6), (0.9, 1.2)] {
        commands
            .spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))))
            .insert(ShadowCaster);
    }

    // The LIVE SDF sphere (host plan R7): an `SdfPrimitive` carrying a world-space
    // `SdfEdit::sphere` — a UNION-op sphere (hard combine, `smoothness == 0.0`) at
    // radius 0.7, placed among the cubes at (-0.9, 0.7, 0.4). Material 0 (no
    // `.with_material`) — the engine default dielectric. The marcher direct-marches
    // it into the shared G-buffer (min-combine with the raster depth), lit by the
    // same sun with the analytic `sdf_soft_shadow` (a round soft shadow).
    commands.spawn(SdfPrimitive(SdfEdit::sphere([-0.9, 0.7, 0.4], 0.7, sdf_op::UNION, 0.0)));

    // The sun: an angled directional light. Its transform is oriented so local
    // `-Z` points TO the light (`light_reconcile` derives the direction from
    // the propagated `GlobalTransform` — author the ROTATION, and keep the
    // `DirectionalLight` field consistent for the pre-propagation frame).
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
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], 2.8),
    });

    // The sky ambient fill (the hemisphere term — cool sky over a warm ground).
    commands.spawn(SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]));

    // A warm point accent between the cubes (UNSHADOWED — the punctual shadow
    // atlas is a later host rung; the light itself is fully ECS-owned).
    commands.spawn(PointLightObject {
        transform: Transform::from_translation(Vec3::new(0.6, 1.6, -0.8)),
        global: GlobalTransform::IDENTITY,
        light: PointLight::new([0.6, 1.6, -0.8], [1.0, 0.72, 0.45], 220.0, 7.0),
    });

    // Camera at (0, 1.7, 6) looking at the origin; aspect = the boot 800×600.
    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 1.7, 6.0), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform {
            translation: pose.translation,
            rotation: Quat::from_mat3(pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: core::f32::consts::FRAC_PI_3,
            aspect: 800.0 / 600.0,
            near: 0.1,
            far: 100.0,
        },
    });
}
