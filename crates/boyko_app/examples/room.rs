//! The R3 "30-line scene" milestone (static form): a floor plane, four cubes,
//! and a perspective camera, assembled ONLY through ECS spawns +
//! `EnginePlugins`, rendered through the production G-buffer path (lit by the
//! engine's default sun + sky; Escape or closing the window exits).
//!
//! Run: `cargo run -p boyko-app --example room`

use boyko_app::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko room", 800, 600));
    app.add_startup_system(setup);
    app.run();
}

/// Spawns the room: startup runs WITH the device present, so meshes register
/// straight through the world-resident `GpuDevice` + `MeshRegistry`.
fn setup(mut commands: Commands, mut meshes: NonSendResMut<MeshRegistry>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);

    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    for (x, z) in [(-2.0, -1.0), (0.0, -2.5), (1.8, -0.6), (0.9, 1.2)] {
        commands.spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))));
    }

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
