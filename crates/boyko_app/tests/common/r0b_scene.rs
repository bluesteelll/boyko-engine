//! Render wave RD1 (defect R0b) — the device fixtures' shared scene, its frame-locked clock, the
//! env knobs the drivers set, the `BOYKO_TEST_FORCE` decode, and the screen-space geometry every
//! R0b gate reads its pixels at.
//!
//! Pulled into each R0b binary with `#[path = "common/r0b_scene.rs"] mod r0b_scene;`
//! (`common/mod.rs` is not involved — the OBS `graph_receipt.rs` idiom).
//!
//! # The scene
//!
//! An SDF sphere of radius [`SPHERE_R`] at [`SPHERE_C`], a camera on its axis at [`EYE`] looking at
//! it (60° vertical FOV, square [`WIN`]-px window), a floor plane below the frame's bottom rows
//! (so the mesh leg is real but never inside a probe window), a sun and a sky fill. Optional:
//!
//! - two particle emitters ([`ENV_EMIT`]): the OCCLUDED one 1 unit behind the sphere's centre on
//!   the view axis (it projects inside the sphere's silhouette), the CONTROL one in front of the
//!   background, off the sphere. Both emit with zero speed and a cone of zero width, so every
//!   particle sits exactly at its emitter — the image depends on the frame index alone;
//! - a mesh cube ([`ENV_MESH`]) in FRONT of the sphere, covering its centre: the mesh-won pixels the
//!   discriminator must leave untouched;
//! - the particle subsystem itself ([`ENV_PARTICLES`]) — armed, it is the boot's transparent-depth
//!   capability; with no emitter the draw still declares (zero instances), so the merge does too.
//!
//! # Determinism
//!
//! `Time` is PAUSED at config time and the particle clock is driven one fixed substep per frame,
//! after the fold (`particle_scene/mod.rs`'s module doc: the wall clock is removed from the loop,
//! not tamed). Spawns are bursts re-armed every frame. No TAA, no DDGI: two runs captured at the
//! same frame index render the same bytes, which is what lets a driver compare a run with the
//! particles against a run without them pixel for pixel.

#![allow(dead_code)] // each R0b binary uses a subset of this module

use boyko_app::prelude::*;
use boyko_app::test_force::{TestForce, parse};
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::ecs::core::time::Time;
use boyko_macros::Resource;
use boyko_render::{
    AaConfig, AaMode, EmitterActive, GeometryLegs, HzbConfig, HzbMode, PARTICLE_BLEND_ADDITIVE,
    PARTICLE_BLEND_ALPHA, PARTICLE_SHAPE_CONE, ParticleClock, ParticleConfig, ParticleEffect,
    ParticleEffectHandle, ParticleEffectsExt, ParticleEmitter, ParticleMode, ParticleTickSet,
    RenderPath, RenderPathConfig,
};

// ===============================================================================================
// The knobs
// ===============================================================================================

/// The render path: `deferred` (default), `forward`, `forwardplus`, `vb`.
pub const ENV_PATH: &str = "BOYKO_R0B_PATH";
/// The geometry legs: `both` (default), `sdf`, `mesh`.
pub const ENV_LEGS: &str = "BOYKO_R0B_LEGS";
/// `0` leaves the particle subsystem disarmed (default armed).
pub const ENV_PARTICLES: &str = "BOYKO_R0B_PARTICLES";
/// Which emitters spawn: `both` (default), `control`, `none`.
pub const ENV_EMIT: &str = "BOYKO_R0B_EMIT";
/// `1` adds the mesh cube in front of the sphere (default absent).
pub const ENV_MESH: &str = "BOYKO_R0B_MESH";
/// `1` arms TAA (default off — a jitter-free frame).
pub const ENV_TAA: &str = "BOYKO_R0B_TAA";
/// `1` arms the HZB pyramid build (`HzbMode::Build`; VisibilityBuffer only, default off).
pub const ENV_HZB: &str = "BOYKO_R0B_HZB";
/// The test-only force-key set (`boyko_app::test_force`), decoded HERE, by the fixture (PC-OBS-5).
pub const ENV_TEST_FORCE: &str = "BOYKO_TEST_FORCE";

/// The square window edge every R0b worker renders at (the WR probe overrides it).
pub const WIN: u32 = 256;

/// What one R0b worker boot arms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct R0bArming {
    pub path: RenderPath,
    pub legs: GeometryLegs,
    pub particles: bool,
    pub emit_occluded: bool,
    pub emit_control: bool,
    pub mesh_cube: bool,
    pub taa: bool,
    pub hzb: bool,
}

impl R0bArming {
    /// Reads the arming from the environment the driver set.
    pub fn from_env() -> Self {
        let on = |k: &str, default: bool| std::env::var(k).map_or(default, |v| v.trim() != "0");
        let path = match std::env::var(ENV_PATH).as_deref().map(str::trim) {
            Ok("forward") => RenderPath::Forward,
            Ok("forwardplus") => RenderPath::ForwardPlus,
            Ok("vb") => RenderPath::VisibilityBuffer,
            Ok("deferred") | Err(_) => RenderPath::Deferred,
            Ok(other) => panic!("{ENV_PATH}={other} is not deferred|forward|forwardplus|vb"),
        };
        let legs = match std::env::var(ENV_LEGS).as_deref().map(str::trim) {
            Ok("sdf") => GeometryLegs::Sdf,
            Ok("mesh") => GeometryLegs::Mesh,
            Ok("both") | Err(_) => GeometryLegs::Both,
            Ok(other) => panic!("{ENV_LEGS}={other} is not both|sdf|mesh"),
        };
        let (emit_occluded, emit_control) = match std::env::var(ENV_EMIT).as_deref().map(str::trim) {
            Ok("control") => (false, true),
            Ok("none") => (false, false),
            Ok("both") | Err(_) => (true, true),
            Ok(other) => panic!("{ENV_EMIT}={other} is not both|control|none"),
        };
        Self {
            path,
            legs,
            particles: on(ENV_PARTICLES, true),
            emit_occluded,
            emit_control,
            mesh_cube: on(ENV_MESH, false),
            taa: on(ENV_TAA, false),
            hzb: on(ENV_HZB, false),
        }
    }

    /// The env word of `path`.
    pub fn path_word(path: RenderPath) -> &'static str {
        match path {
            RenderPath::Forward => "forward",
            RenderPath::ForwardPlus => "forwardplus",
            RenderPath::VisibilityBuffer => "vb",
            _ => "deferred",
        }
    }

    /// The env word of `legs`.
    pub fn legs_word(legs: GeometryLegs) -> &'static str {
        match legs {
            GeometryLegs::Sdf => "sdf",
            GeometryLegs::Mesh => "mesh",
            _ => "both",
        }
    }
}

/// The `BOYKO_TEST_FORCE` decode: unset or empty ⇒ `None` (every production run's empty set); an
/// unknown key is this fixture's boot refusal, panicking with the parser's own named error
/// (`BOYKO_TEST_FORCE: unknown key '…'`).
pub fn test_force_from_env() -> Option<TestForce> {
    let value = std::env::var(ENV_TEST_FORCE).unwrap_or_default();
    match parse(&value) {
        Ok(force) if force.is_empty() => None,
        Ok(force) => Some(force),
        Err(e) => panic!("{e}"),
    }
}

// ===============================================================================================
// The geometry
// ===============================================================================================

/// The camera position (on the sphere's axis, looking down `-z`).
pub const EYE: Vec3 = Vec3::new(0.0, 1.0, 5.0);
/// The SDF sphere's centre (the camera's target).
pub const SPHERE_C: [f32; 3] = [0.0, 1.0, 0.0];
/// The SDF sphere's radius.
pub const SPHERE_R: f32 = 0.5;
/// The OCCLUDED emitter: 1 unit behind the sphere's centre, on the view axis.
pub const OCCLUDED_POS: Vec3 = Vec3::new(0.0, 1.0, -1.0);
/// The CONTROL emitter: in front of the background, right of the sphere.
pub const CONTROL_POS: Vec3 = Vec3::new(1.2, 1.0, 0.0);
/// The mesh cube's centre (between the camera and the sphere, on the axis) and edge.
pub const CUBE_POS: Vec3 = Vec3::new(0.0, 1.0, 1.5);
/// The mesh cube's edge.
pub const CUBE_EDGE: f32 = 0.3;
/// The floor's height — low enough that no floor texel reaches a probe window before `far`.
pub const FLOOR_Y: f32 = -1.0;
/// The vertical field of view.
pub const FOV_Y: f32 = core::f32::consts::FRAC_PI_3;
/// The near plane.
pub const NEAR: f32 = 0.1;
/// The far plane.
pub const FAR: f32 = 100.0;
/// The billboard half-extent (world units).
pub const BILLBOARD: f32 = 0.15;
/// The particle clock's rate; the fixture advances it one substep per frame.
pub const STEP_HZ: f32 = 1_000.0;

/// The pixel (column, row) a world point projects to under the fixture camera on a `win`-px square
/// window (row 0 at the top). The camera looks down `-z` from [`EYE`] with `+y` up.
pub fn project(p: Vec3, win: u32) -> (u32, u32) {
    let tan = (FOV_Y * 0.5).tan();
    let depth = EYE.z - p.z;
    let ndc_x = (p.x - EYE.x) / (depth * tan);
    let ndc_y = (p.y - EYE.y) / (depth * tan);
    let w = win as f32;
    (((ndc_x + 1.0) * 0.5 * w) as u32, ((1.0 - ndc_y) * 0.5 * w) as u32)
}

/// The radius in pixels a world-space radius `r` at view depth `depth` covers on a `win`-px window.
pub fn px_radius(r: f32, depth: f32, win: u32) -> f32 {
    r / (depth * (FOV_Y * 0.5).tan()) * win as f32 * 0.5
}

/// An inclusive pixel rectangle `[x0, x1] × [y0, y1]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl Rect {
    /// The `(2·half + 1)²` square centred on `c`.
    pub fn around(c: (u32, u32), half: u32) -> Self {
        Self { x0: c.0 - half, y0: c.1 - half, x1: c.0 + half, y1: c.1 + half }
    }

    /// Every pixel of the rectangle.
    pub fn pixels(self) -> impl Iterator<Item = (u32, u32)> {
        (self.y0..=self.y1).flat_map(move |y| (self.x0..=self.x1).map(move |x| (x, y)))
    }
}

/// The 7×7 probe window on the OCCLUDED particle (inside the sphere's silhouette).
pub fn occluded_window(win: u32) -> Rect {
    Rect::around(project(OCCLUDED_POS, win), 3)
}

/// The 7×7 probe window on the CONTROL particle (over the background).
pub fn control_window(win: u32) -> Rect {
    Rect::around(project(CONTROL_POS, win), 3)
}

/// A rectangle every pixel of which the mesh cube's front face covers (well inside its
/// silhouette, which is itself well inside the sphere's).
pub fn mesh_won_rect(win: u32) -> Rect {
    let front = EYE.z - (CUBE_POS.z + CUBE_EDGE * 0.5);
    let half = (px_radius(CUBE_EDGE * 0.5, front, win) * 0.6) as u32;
    Rect::around(project(CUBE_POS, win), half)
}

/// A rectangle every pixel of which the SDF sphere covers and the cube does not: a band right of
/// the cube's silhouette, inside the sphere's.
pub fn sdf_won_rect(win: u32) -> Rect {
    let c = project(Vec3::new(SPHERE_C[0], SPHERE_C[1], SPHERE_C[2]), win);
    let front = EYE.z - (CUBE_POS.z + CUBE_EDGE * 0.5);
    let cube_px = px_radius(CUBE_EDGE * 0.5, front, win);
    let sphere_px = px_radius(SPHERE_R, EYE.z - SPHERE_C[2], win);
    let x0 = c.0 + (cube_px + 3.0) as u32;
    let x1 = c.0 + (sphere_px * 0.8) as u32;
    assert!(x1 > x0, "invariant: the sphere's silhouette reaches past the cube's ({x0}..{x1})");
    Rect { x0, y0: c.1 - 2, x1, y1: c.1 + 2 }
}

// ===============================================================================================
// The scene
// ===============================================================================================

/// The arming, as a resource the startup system reads.
#[derive(Resource)]
pub struct R0bArmingRes(pub R0bArming);

const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// A zero-speed, zero-width-cone, effectively immortal effect: every particle sits exactly at its
/// emitter for the whole run.
fn still_effect(blend_class: u32, color: u32) -> ParticleEffect {
    ParticleEffect {
        gravity: [0.0; 3],
        drag: 0.0,
        rot_speed: 0.0,
        lifetime_min: 1.0e6,
        lifetime_max: 1.0e6,
        speed_min: 0.0,
        speed_max: 0.0,
        size_base: BILLBOARD,
        cone_cos: 1.0,
        collision_radius: 0.0,
        restitution: 0.0,
        friction: 0.0,
        color_keys: [color; 4],
        color_times: [0.0, 0.333_333_34, 0.666_666_7, 1.0],
        size_keys: [1.0; 4],
        tex_index: 0,
        blend_class,
        flags: 0,
        emitter_shape: PARTICLE_SHAPE_CONE,
    }
}

fn spawn_emitter(commands: &mut Commands, at: Vec3, effect: u32) {
    commands
        .spawn(ParticleEmitter { rate: 0.0, accumulator: 0.0, burst: 1, speed_scale: 1.0 })
        .insert(Transform::from_translation(at))
        .insert(ParticleEffectHandle(effect))
        .enable::<EmitterActive>();
}

#[allow(clippy::needless_pass_by_value)]
fn r0b_setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut effects: ResMut<Assets<ParticleEffect>>,
    dev: NonSendRes<GpuDevice>,
    arming: Res<R0bArmingRes>,
) {
    let arming = arming.0;
    let floor = meshes.plane(dev.get(), 40.0);
    commands.spawn(MeshBundle::new(floor, Transform::from_translation(Vec3::new(0.0, FLOOR_Y, 0.0))));
    if arming.mesh_cube {
        let cube = meshes.cube(dev.get(), CUBE_EDGE);
        commands.spawn(MeshBundle::new(cube, Transform::from_translation(CUBE_POS)));
    }
    commands.spawn(SdfPrimitive(SdfEdit::sphere(SPHERE_C, SPHERE_R, sdf_op::UNION, 0.0)));

    if arming.particles {
        // The occluded particle is the ALPHA class (the rung's G18 consumer), the control one the
        // additive class: both draws of `particle_draw` are exercised.
        if arming.emit_occluded {
            let e = effects.register_effect(still_effect(PARTICLE_BLEND_ALPHA, 0xFFFF_FFFF));
            spawn_emitter(&mut commands, OCCLUDED_POS, e.index());
        }
        if arming.emit_control {
            let e = effects.register_effect(still_effect(PARTICLE_BLEND_ADDITIVE, 0xFFFF_FFFF));
            spawn_emitter(&mut commands, CONTROL_POS, e.index());
        }
    }

    let sun_pose = Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(sun_pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], 2.8),
    });
    commands.spawn(SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]));
    let target = Vec3::new(SPHERE_C[0], SPHERE_C[1], SPHERE_C[2]);
    let pose = Affine3A::look_at_rh(EYE, target, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform { translation: pose.translation, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective { fov_y: FOV_Y, aspect: 1.0, near: NEAR, far: FAR },
    });
}

/// Re-arms one burst per emitter per frame, ordered BEFORE the particle fold.
#[allow(clippy::needless_pass_by_value)]
fn r0b_arm_burst(mut emitters: Query<&mut ParticleEmitter>) {
    for e in emitters.iter_mut() {
        e.burst = 1;
    }
}

/// Advances the particle clock by exactly one substep, ordered AFTER the fold (whose own
/// `advance(0.0)` on the paused clock would otherwise leave the sim no step).
#[allow(clippy::needless_pass_by_value)]
fn r0b_drive_clock(mut clock: ResMut<ParticleClock>) {
    clock.advance(1.0 / STEP_HZ);
}

/// Builds the R0b app for `arming` on a `win_w`×`win_h` window (square [`WIN`] for every gate but
/// the WR probe).
pub fn build_r0b_app(title: &'static str, arming: R0bArming, win_w: u32, win_h: u32) -> App {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window(title, win_w, win_h));
    app.insert_resource(R0bArmingRes(arming));
    app.add_startup_system(r0b_setup);
    // Configuration AFTER `add_plugins`, which seeds every default.
    app.insert_resource(RenderPathConfig { path: arming.path, legs: arming.legs });
    if arming.taa {
        app.insert_resource(AaConfig { mode: AaMode::Taa });
    }
    if arming.hzb {
        app.insert_resource(HzbConfig { mode: HzbMode::Build });
    }
    if arming.particles {
        app.insert_resource(ParticleConfig { mode: ParticleMode::GpuUnlit, capacity: 4096, ..ParticleConfig::default() });
        app.insert_resource(ParticleClock::from_hz(STEP_HZ));
        let mut time = Time::default();
        time.pause();
        app.insert_resource(time);
        app.add_systems_cfg(|b| {
            b.add_system(r0b_arm_burst).before_set(ParticleTickSet);
            b.add_system(r0b_drive_clock).after_set(ParticleTickSet);
        });
    }
    if let Some(force) = test_force_from_env() {
        app.insert_resource(force);
    }
    app
}

/// Asserts the boot resolved the asked path and legs and ran frames — a degraded boot proves
/// nothing about the path it was asked for.
pub fn assert_r0b_resolved(app: &App, arming: R0bArming) {
    let frames = app.world().resource::<HostFrameStats>().frames;
    assert!(frames > 0, "the frame loop never ran (no windowed device?) — RED, never a skip");
    let resolved = *app.world().resource::<boyko_render::ResolvedRenderPath>();
    assert_eq!(resolved.path, arming.path, "PREMISE: asked for {:?}, the boot resolved {:?}", arming.path, resolved.path);
    assert_eq!(
        resolved.sdf_leg,
        matches!(arming.legs, GeometryLegs::Both | GeometryLegs::Sdf),
        "PREMISE: the SDF leg of {:?} did not resolve as asked ({resolved:?})",
        arming.legs
    );
}
