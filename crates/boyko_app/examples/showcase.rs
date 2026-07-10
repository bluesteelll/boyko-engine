//! The INTERACTIVE mixed showcase: fly a first-person camera around a scene that
//! exercises EVERY host render feature at once — raster mesh cubes, a live SDF
//! sphere composited into the same G-buffer, an angled SUN with cascaded (CSM)
//! shadows, and BOTH punctual shadow kinds (a POINT and a SPOT carrying
//! `CastsPunctualShadow`). Assembled ONLY through ECS spawns + `EnginePlugins` +
//! `FlyCameraPlugin`. This is the host's golden-INDEPENDENT visual regression:
//! move around and confirm every shadow type is correct, in motion, from any angle.
//!
//! Run: `cargo run -p boyko-app --example showcase`
//!
//! # Controls
//! * `W`/`S`/`A`/`D` — fly; `Space`/`E` — rise; `Left Ctrl`/`Q` — descend.
//! * mouse — look. `P` — print the camera pose once per press. `Esc` — quit.
//!
//! `BOYKO_HOST_DUMP=<path.bmp>` captures one settled frame (inherited from the runner).

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::{Local, Res};
use boyko_input::PhysicalInput;

/// The sun direction TO the light (mirrors `room.rs`/`viewer.rs`).
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko showcase", 900, 640));
    // Interactive input + fly camera.
    app.add_plugin(FlyCameraPlugin);
    // Arm the two shadow systems (both default DISABLED — the 0%-gate).
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
    app.add_startup_system(setup);
    app.add_systems(pose_probe_system);
    app.run();
}

fn setup(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 16.0);
    let cube = meshes.cube(dev.get(), 1.0);

    // Floor — receiver only.
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    // Raster cubes — casters (into BOTH the sun cascades AND the punctual atlas).
    for (x, z) in [(-2.2, -1.0), (0.2, -2.6), (2.1, -0.4), (1.0, 1.3)] {
        commands
            .spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))))
            .insert(ShadowCaster);
    }

    // A live SDF sphere composited into the same G-buffer as the cubes (boot-static;
    // its PRESENCE routes it into the marcher's edit list). It min-combines with the
    // mesh depth and is lit + sun-shadowed alongside the cubes.
    commands.spawn(SdfPrimitive(SdfEdit::sphere([-0.9, 0.7, 0.6], 0.75, sdf_op::UNION, 0.0)));

    // The SUN: an angled directional light + CSM cascades (orient the transform so
    // local -Z points TO the light; the reconcile derives the direction from it).
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
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], 2.2),
    });

    // Sky ambient fill.
    commands.spawn(SkyLight::new([0.24, 0.30, 0.40], [0.11, 0.11, 0.12]));

    // A WARM POINT light casting an omnidirectional (cube-map) shadow. Marker on ⇒ mapped.
    commands
        .spawn(PointLightObject {
            transform: Transform::from_translation(Vec3::new(2.6, 3.2, 1.4)),
            global: GlobalTransform::IDENTITY,
            light: PointLight::new([2.6, 3.2, 1.4], [1.0, 0.78, 0.52], 130.0, 11.0),
        })
        .insert(CastsPunctualShadow);

    // A COOL SPOT, AIMED at the cubes (orient the transform via look_at — the spot
    // direction is transform-driven; an unaimed spot shines -Z). Casts a mapped cone
    // shadow.
    let spot_pos = Vec3::new(-3.2, 4.0, 2.2);
    let spot_target = Vec3::new(0.0, 0.5, -0.6);
    let spot_rot = Quat::from_mat3(Affine3A::look_at_rh(spot_pos, spot_target, Vec3::new(0.0, 1.0, 0.0)).matrix3);
    commands
        .spawn(SpotLightObject {
            transform: Transform { translation: spot_pos, rotation: spot_rot, scale: Vec3::ONE },
            global: GlobalTransform::IDENTITY,
            light: SpotLight::new(
                [-3.2, 4.0, 2.2],
                [-0.62, 0.60, 0.50],
                [0.55, 0.72, 1.0],
                95.0,
                12.0,
                20.0,
                34.0,
            ),
        })
        .insert(CastsPunctualShadow);

    // The FLY camera at a vantage that frames the cubes + the SDF sphere.
    commands.spawn(FlyCameraBundle {
        transform: Transform::from_translation(Vec3::new(0.0, 3.6, 7.2)),
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: 52.0 * core::f32::consts::PI / 180.0,
            aspect: 900.0 / 640.0,
            near: 0.1,
            far: 100.0,
        },
        fly: FlyCamera::default(),
    });
}

/// Prints the fly camera's pose once per `P` press (edge-latched) — the parity/flight
/// triage probe, example-local (not a shipped engine system).
#[allow(clippy::needless_pass_by_value)]
fn pose_probe_system(
    input: Res<PhysicalInput>,
    cams: Query<(&FlyCamera, &Transform)>,
    mut latch: Local<bool>,
) {
    let p_now = KeyCode::KeyP.dense_index().is_some_and(|i| input.keys_pressed.get(i));
    if p_now && !*latch {
        for (fly, transform) in cams.iter() {
            let e = transform.translation;
            eprintln!(
                "[pose] eye=[{:.3}, {:.3}, {:.3}] yaw={:.4} pitch={:.4}",
                e.x, e.y, e.z, fly.yaw, fly.pitch
            );
        }
    }
    *latch = p_now;
}
