//! In-motion temporal-shadow-denoise evaluation harness (rayconfig rung 3b).
//!
//! A single mesh cube CASTER drifts back and forth above a large flat SDF FLOOR
//! receiver, lit by an angled sun. Built `--features hwrt` on a ray-query device
//! the caster is a TLAS instance, so the SDF floor pixels trace HARDWARE
//! mesh-shadow rays (`RayBackend::HardwareTri`) — and that shadow-vis buffer is
//! what the temporal denoiser (`BOYKO_SHADOW_DENOISE=temporal|both`) filters.
//!
//! The caster moves via the `GpuTransform3D` interpolation pair driven in
//! `FixedSet::Gameplay` (the bounce.rs pattern) — the ONLY per-frame mesh-motion
//! path that actually reaches the render: a plain `&mut Transform` mutation is
//! change-detection-untracked, so transform propagation skips it and the mesh
//! never moves on the GPU. Because the caster keeps moving, the `BOYKO_HOST_DUMP`
//! settle-frame capture is a genuine IN-MOTION frame.
//!
//! NOTE (per the analyst trace): a moving CASTER leaves the static floor's
//! screen-space motion vector at ZERO (the shadow value changes but the receiver
//! pixel does not move). The plan predicts the temporal filter's k=0.95 history
//! blend still yields a BOUNDED ghost at the sweeping shadow edge; an exact
//! `both`==`spatial` match under real motion would indicate the blend is not
//! taking effect.
//!
//! `BOYKO_EVAL_SPEED` = caster drift speed in units/sec (default 3.0; 0 = static).
//!
//! Run (interactive):
//!   `cargo run -p boyko-app --example shadow_denoise_eval --features hwrt`
//! Capture one in-motion frame per denoise mode:
//!   `$env:BOYKO_SHADOW_DENOISE="both"; $env:BOYKO_HOST_DUMP="D:\tmp\den_both.bmp"; cargo run -p boyko-app --example shadow_denoise_eval --features hwrt`

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::app::CoreSchedule;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::Res;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_macros::{Bundle, Component};

/// The sun direction TO the light (the engine's familiar showcase sun).
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// The half-span of the caster's X drift (world units); it reverses at ±this.
const DRIFT_HALF_SPAN: f32 = 2.0;

/// The caster's constant drift speed (units/s), reversed at the span bounds. Its
/// sign carries the current direction.
#[derive(Component, Clone, Copy)]
struct Velocity(f32);

/// Marks the drifting mesh cube (the caster).
#[derive(Component, Clone, Copy)]
struct Caster;

/// A one-field bundle attaching the `GpuTransform3D` interpolation pair (a dense
/// component's self-`Bundle` is suppressed; a named bundle is the attach path).
#[derive(Bundle)]
struct PairOnly {
    pair: GpuTransform3D,
}

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko shadow-denoise eval", 900, 640));
    // A primary directional + CSM is one of the temporal-denoise arming gates
    // (`has_primary_directional`); `hwrt` + a non-empty TLAS + the HardwareTri
    // backend supply the rest. `ShadowConfig` arms the SDF analytic-shadow path.
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
    app.add_startup_system(setup);
    // The drift integrator runs in Fixed gameplay so the engine's
    // `pack_gpu_transforms` snapshot (`FixedSet::Snapshot`, `.after(Gameplay)`)
    // observes the post-integrate pose — the interpolated instance the raster,
    // CSM, and TLAS passes all read from the shared ring.
    app.add_systems_cfg_in(CoreSchedule::Fixed, |b| {
        b.add_system(drift).in_set(FixedSet::Gameplay);
    });
    app.run();
}

fn setup(mut commands: Commands, mut meshes: NonSendResMut<MeshRegistry>, dev: NonSendRes<GpuDevice>) {
    let cube = meshes.cube(dev.get(), 1.0);

    // The RECEIVER: a large flat SDF slab (40×40, top surface at y = 0). SDF,
    // so it is NOT a TLAS instance — a pure shadow receiver the marcher direct-
    // marches into the shared G-buffer. The moving mesh shadow lands on it.
    commands.spawn(SdfPrimitive(SdfEdit::box_shape([0.0, -0.5, 0.0], [20.0, 0.5, 20.0], sdf_op::UNION, 0.0)));

    // The CASTER: one mesh cube hovering above the floor + the interpolation
    // pair + a drift velocity. As a mesh instance it is packed into the TLAS by
    // the M3 gather, so the SDF floor's hardware shadow-ray trace hits it.
    let speed: f32 = std::env::var("BOYKO_EVAL_SPEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3.0);
    eprintln!("[eval] caster drift speed = {speed} units/s");
    let start = Transform::from_translation(Vec3::new(-DRIFT_HALF_SPAN, 1.4, 0.0));
    commands
        .spawn(MeshBundle::new(cube, start))
        .insert(ShadowCaster)
        .insert(Caster)
        .insert(PairOnly { pair: GpuTransform3D::from_transform(&start) })
        .insert(Velocity(speed));

    // The sun: an angled directional light + CSM cascades (orient local -Z TO the
    // light so `light_reconcile` derives the authored direction).
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
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], 2.6),
    });

    // Sky ambient fill so the shadowed region is not pure black (the penumbra
    // detail the denoiser acts on stays legible).
    commands.spawn(SkyLight::new([0.30, 0.36, 0.46], [0.14, 0.14, 0.15]));

    // The camera looks down-front at the floor so the sweeping shadow spans the
    // frame — the region the visual oracle reads for noise vs ghost.
    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 4.6, 6.2), Vec3::new(0.0, 0.3, -0.4), Vec3::new(0.0, 1.0, 0.0));
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
            aspect: 900.0 / 640.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

/// The Fixed-timestep drift integrator (`FixedSet::Gameplay`): moves the caster
/// in X at a constant speed, reversing at ±[`DRIFT_HALF_SPAN`]. Writes
/// `Transform` — the source of truth `pack_gpu_transforms` snaps into
/// `GpuTransform3D::curr` the same substep, so the render lerp (and the TLAS) see
/// the moving caster.
#[allow(clippy::needless_pass_by_value)]
fn drift(time: Res<FixedTime>, mut q: Query<(&mut Transform, &mut Velocity)>) {
    let dt = time.delta_secs();
    for (transform, vel) in q.iter_mut() {
        let mut x = transform.translation.x + vel.0 * dt;
        if x > DRIFT_HALF_SPAN {
            x = DRIFT_HALF_SPAN;
            vel.0 = -vel.0.abs();
        } else if x < -DRIFT_HALF_SPAN {
            x = -DRIFT_HALF_SPAN;
            vel.0 = vel.0.abs();
        }
        transform.translation.x = x;
    }
}
