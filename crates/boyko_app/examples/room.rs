//! The "30-line scene" milestone (host plan R3 + R4 lighting): a floor plane,
//! four shadow-casting cubes, a perspective camera, and ECS-owned lighting — an
//! angled sun (`DirectionalLightObject` + CSM cascades), a sky ambient fill,
//! and a warm point accent — assembled ONLY through ECS spawns +
//! `EnginePlugins`, rendered through the production G-buffer path (Escape or
//! closing the window exits).
//!
//! Lighting is structural (capability = presence): the cubes carry
//! [`ShadowCaster`] and stamp into the sun's cascades; the floor omits the
//! marker and only RECEIVES. Remove the `DirectionalLightObject` spawn and the
//! depth pass simply never records (no flags to flip).
//!
//! Run: `cargo run -p boyko-app --example room`

use boyko_app::prelude::*;

/// The sun direction TO the light (the engine's familiar showcase sun) — also
/// the `-Z` forward the sun entity's transform is oriented along, so the
/// transform-driven `light_reconcile` derives the same direction it was
/// authored with.
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko room", 800, 600));
    // Enable CSM (the owner-set knob; `CsmPlugin`'s default is DISABLED — the
    // 0%-gate). Three cascades over the room's 30-unit shadow range; the
    // default 2048 resolution matches the host cascade texture.
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.add_startup_system(setup);
    app.run();
}

/// Spawns the room: startup runs WITH the device present, so meshes register
/// straight through the world-resident `GpuDevice` + `Assets<MeshGpu>`.
fn setup(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
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
