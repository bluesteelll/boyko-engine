//! The punctual-shadow showcase (the punctual host rung): the `room` scene with the
//! sparse spot/point shadow atlas ENABLED, so the point AND spot lights carrying
//! [`CastsPunctualShadow`] cast REAL (mapped) shadows through the production
//! `boyko_render` shadow-atlas machinery, driven from the windowed host runner —
//! mirroring R4's CSM lift. Assembled ONLY through ECS spawns + `EnginePlugins`.
//!
//! Shadowing is structural (capability = presence): the cubes carry [`ShadowCaster`]
//! and stamp into BOTH the sun's cascades AND the punctual atlas; the floor omits the
//! marker and only RECEIVES. A LIGHT carrying [`CastsPunctualShadow`] owns an atlas map
//! (six cube faces for a point, one perspective map for a spot); remove the marker and
//! that light falls back to the analytic (unshadowed) term — no flags to flip.
//!
//! This is a NEW example, NOT a modification of `room.rs` (which stays the pristine
//! 0%-gate byte-identity reference + book hero). The structural deltas from `room.rs`
//! are the enabled [`ShadowConfig`], the [`CastsPunctualShadow`] marker on the point
//! light, and an added spot light (also carrying the marker) — so both the POINT (cube)
//! and SPOT (perspective) atlas paths render.
//!
//! Run: `cargo run -p boyko-app --example punctual_room`
//! `BOYKO_HOST_DUMP=<path.bmp>` captures one settled frame + prints the atlas selection.

use boyko_app::prelude::*;

/// The sun direction TO the light (the engine's familiar showcase sun).
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko punctual room", 800, 600));
    // Enable CSM (a DIM sun for fill; the punctual shadows are the feature). Both
    // `CsmConfig` and `ShadowConfig` default DISABLED — the 0%-gate — so these two
    // inserts are the owner-set knobs that arm the sun cascades and the punctual atlas.
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
    app.add_startup_system(setup);
    app.run();
}

fn setup(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 14.0);
    let cube = meshes.cube(dev.get(), 1.0);

    // Floor RECEIVES only.
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    // Cubes CAST (into both the cascades AND the punctual atlas, one gather feeds both).
    for (x, z) in [(-2.0, -1.0), (0.0, -2.4), (1.9, -0.5), (0.8, 1.1)] {
        commands
            .spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))))
            .insert(ShadowCaster);
    }

    // A DIM angled sun (fill + a soft cascaded shadow), so the punctual shadows read as
    // the feature rather than competing with a bright directional.
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
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], 0.7),
    });

    // A cool sky ambient fill.
    commands.spawn(SkyLight::new([0.20, 0.25, 0.34], [0.10, 0.10, 0.11]));

    // A WARM POINT light high on the RIGHT, casting the cubes' omnidirectional shadows
    // down-and-left across the floor. `CastsPunctualShadow` gives it six cube-face maps.
    commands
        .spawn(PointLightObject {
            transform: Transform::from_translation(Vec3::new(2.8, 3.4, 1.6)),
            global: GlobalTransform::IDENTITY,
            light: PointLight::new([2.8, 3.4, 1.6], [1.0, 0.78, 0.52], 130.0, 11.0),
        })
        .insert(CastsPunctualShadow);

    // A COOL SPOT high on the LEFT, AIMED at the cubes. The spot direction is derived from
    // the transform's -Z (the SpotLight::new direction is only a pre-propagation seed), so
    // the transform MUST be oriented with look_at, exactly like the sun.
    let spot_pos = Vec3::new(-3.0, 3.8, 1.8);
    let spot_target = Vec3::new(0.0, 0.5, -1.0);
    let spot_rot = Quat::from_mat3(Affine3A::look_at_rh(spot_pos, spot_target, Vec3::new(0.0, 1.0, 0.0)).matrix3);
    commands
        .spawn(SpotLightObject {
            transform: Transform { translation: spot_pos, rotation: spot_rot, scale: Vec3::ONE },
            global: GlobalTransform::IDENTITY,
            light: SpotLight::new(
                [-3.0, 3.8, 1.8],
                [-0.57, 0.63, 0.53],
                [0.55, 0.72, 1.0],
                85.0,
                12.0,
                20.0,
                34.0,
            ),
        })
        .insert(CastsPunctualShadow);

    // Camera at (0, 1.9, 6.2) looking at the cubes.
    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 1.9, 6.2), Vec3::new(0.0, 0.4, -0.5), Vec3::new(0.0, 1.0, 0.0));
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
