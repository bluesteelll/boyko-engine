//! The interpolation milestone (host plan R5): a cube bouncing on a floor, driven
//! by a FIXED-timestep gameplay system (`FixedSet::Gameplay`) at 64 Hz and drawn
//! interpolated at the render rate — the standard fixed-sim / interpolated-render
//! split. The cube carries the `GpuTransform3D` interpolation pair (the opt-in);
//! `pack_gpu_transforms` (in `FixedSet::Snapshot`, wired by `EnginePlugins`) shuffles
//! its prev/curr each substep, and the runner lerps at `overstep_fraction()` — so
//! the cube glides smoothly between the discrete 64 Hz poses.
//!
//! Teleport it (a discontinuous jump with no streak) with
//! `commands.entity(e).teleport_to(Transform::from_translation(..))` — the
//! `TeleportCommandsExt` sugar writes the pose AND snaps `prev = curr` for one frame.
//!
//! Run: `cargo run -p boyko-app --example bounce`

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::app::CoreSchedule;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::Res;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_macros::{Bundle, Component};

/// The sun direction TO the light — the engine's familiar showcase sun.
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// The bouncing cube's vertical velocity (units/s), integrated in Fixed gameplay.
#[derive(Component, Clone, Copy)]
struct Velocity(f32);

/// A one-field bundle attaching the `GpuTransform3D` interpolation pair (a dense
/// component's self-`Bundle` is suppressed; a named bundle is the attach path).
#[derive(Bundle)]
struct PairOnly {
    pair: GpuTransform3D,
}

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko bounce", 800, 600));
    // Enable CSM so the interpolated cube casts a moving sun shadow: its dynamic
    // (compute-written) instance is then read by BOTH the raster pass AND the CSM
    // depth pass from the one shared instance ring — the refined-B multi-reader
    // shared-ring path.
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.add_startup_system(setup);
    // The bounce integrator runs in Fixed gameplay (`FixedSet::Gameplay`), so the
    // engine's `pack_gpu_transforms` snapshot (`FixedSet::Snapshot`, wired
    // `.after(Gameplay)` by EnginePlugins) observes the post-integrate pose.
    app.add_systems_cfg_in(CoreSchedule::Fixed, |b| {
        b.add_system(bounce).in_set(FixedSet::Gameplay);
    });
    app.run();
}

/// Spawns the scene: a floor, a bouncing cube (mesh + interpolation pair +
/// velocity), a sun, and a camera. Startup runs WITH the device present.
fn setup(mut commands: Commands, mut meshes: NonSendResMut<MeshRegistry>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);

    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));

    // The cube: drawable + the interpolation pair (seeded prev == curr) + velocity.
    let start = Transform::from_translation(Vec3::new(0.0, 4.0, 0.0));
    commands
        .spawn(MeshBundle::new(cube, start))
        .insert(ShadowCaster)
        .insert(PairOnly { pair: GpuTransform3D::from_transform(&start) })
        .insert(Velocity(0.0));

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
    commands.spawn(SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]));

    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 2.5, 8.0), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
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

/// The Fixed-timestep bounce integrator (`FixedSet::Gameplay`): gravity pulls the
/// cube down; hitting the floor (`y <= 0.5`, the cube half-extent) reflects the
/// velocity. Writes `Transform` — the source of truth `pack_gpu_transforms` snaps
/// into `GpuTransform3D::curr` the same substep, so the render lerp is one substep
/// wide.
#[allow(clippy::needless_pass_by_value)]
fn bounce(time: Res<FixedTime>, mut q: Query<(&mut Transform, &mut Velocity)>) {
    let dt = time.delta_secs();
    const GRAVITY: f32 = -9.8;
    const FLOOR_Y: f32 = 0.5;
    for (transform, vel) in q.iter_mut() {
        let v = vel.0 + GRAVITY * dt;
        let y = transform.translation.y + v * dt;
        if y <= FLOOR_Y {
            transform.translation.y = FLOOR_Y;
            // Reflect with a little energy retained so the bounce sustains.
            vel.0 = v.abs() * 0.92 + 4.0;
        } else {
            transform.translation.y = y;
            vel.0 = v;
        }
    }
}
