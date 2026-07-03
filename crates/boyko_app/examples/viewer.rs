//! The R6 INTERACTIVE viewer: the `room.rs` scene made fly-able. A floor plane,
//! four shadow-casting cubes, ECS-owned lighting (an angled sun + CSM cascades, a
//! sky ambient fill, a warm point accent), and a first-person FLY camera driven
//! by OS input through the ECS — assembled ONLY through ECS spawns +
//! `EnginePlugins` + `FlyCameraPlugin`, rendered through the production G-buffer
//! path.
//!
//! Run: `cargo run -p boyko-app --example viewer`
//!
//! # Controls
//!
//! * `W` / `S` / `A` / `D` — fly forward / back / left / right (planar,
//!   diagonal-normalized ~3.5 u/s).
//! * `Space` / `E` — rise; `Left Ctrl` / `Q` — descend.
//! * mouse — look (sensitivity 0.0035, pitch clamped to ±1.5533).
//! * `P` — print the current camera pose once per press (`[pose] eye=[..]
//!   yaw=.. pitch=..`) — the shadow-flip / parity triage probe.
//! * `Esc` — quit (the rebindable `FlyAction::Quit`, ECS-native via
//!   `quit_on_action`); closing the window also exits.
//!
//! # Diagnostics
//!
//! `BOYKO_HOST_DUMP` is inherited from the runner with no code here: set it to
//! capture the same settled-frame G-buffer dump the other host examples expose
//! (see `boyko_app::host_dump`).

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::{Local, Res};
use boyko_input::PhysicalInput;

/// The sun direction TO the light (mirrors `room.rs`) — also the `-Z` forward the
/// sun entity's transform is oriented along, so `light_reconcile` derives the
/// same direction it was authored with.
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// The interactive viewer's start pose, reproduced EXACTLY from the reference
/// `run_interactive_viewer` (`boyko_rhi_vulkan`): eye `ROOM_CAM_EYE`, `yaw == 0`,
/// `pitch == VIEWER_INITIAL_PITCH` (`FlyCamera::default().pitch`).
const VIEWER_EYE: Vec3 = Vec3::new(0.0, 4.128478, 6.821193);

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko viewer", 800, 600));
    // The interactive input + fly-camera stack (InputPlugin<FlyAction> + the
    // controller in CameraSet::Control + the ECS-native quit).
    app.add_plugin(FlyCameraPlugin);
    // Enable CSM (owner-set knob; `CsmPlugin`'s default is DISABLED). Three
    // cascades over the room's shadow range — matching `room.rs`.
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.add_startup_system(setup);
    // The example-local pose probe (P prints the pose once per press).
    app.add_systems(pose_probe_system);
    app.run();
}

/// Spawns the room + the fly camera: startup runs WITH the device present, so
/// meshes register straight through the world-resident `GpuDevice` +
/// `MeshRegistry`.
fn setup(mut commands: Commands, mut meshes: NonSendResMut<MeshRegistry>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);

    // The floor is a RECEIVER only (no `ShadowCaster`).
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    // The cubes cast: the structural `ShadowCaster` marker routes them into the
    // cascade depth pass.
    for (x, z) in [(-2.0, -1.0), (0.0, -2.5), (1.8, -0.6), (0.9, 1.2)] {
        commands
            .spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))))
            .insert(ShadowCaster);
    }

    // The sun: an angled directional light (see `room.rs` for the orientation
    // rationale).
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

    // The sky ambient fill (cool sky over a warm ground).
    commands.spawn(SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]));

    // A warm point accent between the cubes (unshadowed — the punctual shadow
    // atlas is a later host rung).
    commands.spawn(PointLightObject {
        transform: Transform::from_translation(Vec3::new(0.6, 1.6, -0.8)),
        global: GlobalTransform::IDENTITY,
        light: PointLight::new([0.6, 1.6, -0.8], [1.0, 0.72, 0.45], 220.0, 7.0),
    });

    // The FLY camera at the reference viewer's start pose. `fly_camera_system`
    // overwrites the `rotation` from `yaw`/`pitch` on the first frame, so only
    // the `translation` (the eye) and the `FlyCamera` accumulators seed the view.
    commands.spawn(FlyCameraBundle {
        transform: Transform::from_translation(VIEWER_EYE),
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: 50.0 * core::f32::consts::PI / 180.0,
            aspect: 800.0 / 600.0,
            near: 0.1,
            far: 100.0,
        },
        // yaw 0, pitch VIEWER_INITIAL_PITCH (-0.3805) + default speed/sensitivity.
        fly: FlyCamera::default(),
    });
}

/// Prints the fly camera's pose ONCE per `P` press (the shadow-flip / parity
/// triage probe, host plan R6). Edge-triggered via a `Local<bool>` latch on the
/// level key bitset, in the EXACT reference-viewer format
/// (`[pose] eye=[x, y, z] yaw=.. pitch=..`).
///
/// Example-local (NOT a shipped engine system): the pose probe is a debug
/// convenience for the parity flight-check, so it lives here rather than in the
/// engine's public surface.
#[allow(clippy::needless_pass_by_value)]
fn pose_probe_system(
    input: Res<PhysicalInput>,
    cams: Query<(&FlyCamera, &Transform)>,
    mut latch: Local<bool>,
) {
    let p_now = KeyCode::KeyP
        .dense_index()
        .is_some_and(|i| input.keys_pressed.get(i));
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
