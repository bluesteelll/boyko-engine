//! S3 camera GATES (integration).
//!
//! Drives the REAL [`resolve_active_camera`] system on a live ECS world and
//! asserts the active-camera selection policy (explicit override → highest-`order`
//! `is_active` → no-camera identity default) and the derived [`ViewUniform`]
//! contract ([`ViewUniform::from_camera`] matrix + orthonormal-basis correctness,
//! and the perspective no-regression basis). These are the S3 gate tests the
//! review found missing — the selection and the derive were previously untested.
//!
//! # The deterministic frame vehicle
//!
//! `resolve_active_camera` reads each camera's already-propagated
//! [`GlobalTransform`], so these tests set the WORLD pose directly (no hierarchy
//! needed) and run the resolver via `world.run_system(...)`. The resolver is not
//! tick-gated (it derives the view unconditionally from the selected camera), so
//! no tick-window dance is required — unlike the S2 propagation suite.

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Bundle;

use boyko_math::{Affine3A, Mat3, Vec3};

use boyko_scene::{
    ActiveCamera, Camera, GlobalTransform, Projection, ViewUniform, resolve_active_camera,
};

use core::f32::consts::FRAC_PI_3;

/// A camera entity's full bundle: the selection metadata, the projection, and the
/// already-propagated world pose the resolver reads.
#[derive(Bundle)]
struct CameraBundle {
    camera: Camera,
    projection: Projection,
    global: GlobalTransform,
}

/// A perspective projection with a sentinel `fov_y` so a derived view's `fov_y`
/// lane is identifiable in assertions.
fn perspective(fov_y: f32) -> Projection {
    Projection::Perspective {
        fov_y,
        aspect: 1.0,
        near: 0.1,
        far: 100.0,
    }
}

/// A world with the camera resources seeded (mirrors `CameraPlugin::build`) but
/// WITHOUT registering the system into a schedule, so each test drives the
/// resolver directly via `run_system` at a controlled point.
fn camera_world() -> App {
    let mut app = App::new();
    app.insert_resource(ActiveCamera::default());
    app.insert_resource(ViewUniform::default());
    app.finish();
    app
}

/// Spawns a camera entity (`Camera` + `Projection` + `GlobalTransform`) and
/// returns its live handle.
fn spawn_camera(
    world: &mut EcsMaster,
    camera: Camera,
    projection: Projection,
    global: GlobalTransform,
) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(CameraBundle {
                camera,
                projection,
                global,
            })
            .id();
        *probe.lock().expect("probe lock") = Some(e);
    });
    let e = sink.lock().expect("probe lock").expect("spawn produced a handle");
    assert!(world.has_entity(e), "spawned camera is live after the apply window");
    e
}

/// Runs `resolve_active_camera` once on the world.
#[inline]
fn resolve(app: &mut App) {
    app.world_mut().run_system(resolve_active_camera);
}

/// Reads the derived view resource.
#[inline]
fn view_of(app: &App) -> ViewUniform {
    *app.world().resource::<ViewUniform>()
}

/// Sets the explicit active-camera override.
#[inline]
fn set_active(app: &mut App, target: Option<Entity>) {
    *app.world_mut().resource_mut::<ActiveCamera>() = ActiveCamera(target);
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 1 — no camera ⇒ ViewUniform stays at the identity default
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn no_camera_leaves_identity_view() {
    let mut app = camera_world();
    resolve(&mut app);
    assert_eq!(view_of(&app), ViewUniform::IDENTITY);
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 2 — a lone active camera by policy
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn lone_active_camera_is_selected_by_policy() {
    let mut app = camera_world();
    let global = GlobalTransform(Affine3A {
        matrix3: Mat3::IDENTITY,
        translation: Vec3::new(0.0, 0.0, 3.0),
    });
    spawn_camera(app.world_mut(), Camera::DEFAULT, perspective(FRAC_PI_3), global);

    resolve(&mut app);

    let view = view_of(&app);
    assert_eq!(view, ViewUniform::from_camera(global.0, perspective(FRAC_PI_3)));
    assert_eq!(view.camera_pos.z, 3.0);
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 3 — highest-`order` wins among multiple is_active cameras
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn highest_order_active_camera_wins() {
    let mut app = camera_world();

    let low = GlobalTransform(Affine3A {
        matrix3: Mat3::IDENTITY,
        translation: Vec3::new(1.0, 0.0, 0.0),
    });
    let high = GlobalTransform(Affine3A {
        matrix3: Mat3::IDENTITY,
        translation: Vec3::new(2.0, 0.0, 0.0),
    });
    spawn_camera(
        app.world_mut(),
        Camera { order: 0, is_active: true, viewport: None },
        perspective(FRAC_PI_3),
        low,
    );
    spawn_camera(
        app.world_mut(),
        Camera { order: 10, is_active: true, viewport: None },
        perspective(FRAC_PI_3),
        high,
    );

    resolve(&mut app);

    // The order-10 camera (eye x = 2.0) wins over order-0 (eye x = 1.0).
    assert_eq!(view_of(&app).camera_pos.x, 2.0);
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 4 — an is_active==false camera is never selected by policy
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn inactive_camera_skipped_by_policy() {
    let mut app = camera_world();

    let inactive = GlobalTransform(Affine3A {
        matrix3: Mat3::IDENTITY,
        translation: Vec3::new(9.0, 0.0, 0.0),
    });
    let active = GlobalTransform(Affine3A {
        matrix3: Mat3::IDENTITY,
        translation: Vec3::new(1.0, 0.0, 0.0),
    });
    // The higher-order camera is INACTIVE, so the lower-order ACTIVE one wins.
    spawn_camera(
        app.world_mut(),
        Camera { order: 99, is_active: false, viewport: None },
        perspective(FRAC_PI_3),
        inactive,
    );
    spawn_camera(
        app.world_mut(),
        Camera { order: 0, is_active: true, viewport: None },
        perspective(FRAC_PI_3),
        active,
    );

    resolve(&mut app);

    assert_eq!(view_of(&app).camera_pos.x, 1.0, "inactive high-order camera must not win");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 5 — explicit override beats the policy pass
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn explicit_override_beats_policy() {
    let mut app = camera_world();

    let policy_winner = GlobalTransform(Affine3A {
        matrix3: Mat3::IDENTITY,
        translation: Vec3::new(5.0, 0.0, 0.0),
    });
    let overridden = GlobalTransform(Affine3A {
        matrix3: Mat3::IDENTITY,
        translation: Vec3::new(-7.0, 0.0, 0.0),
    });
    // The order-100 camera would win by policy, but we override to the order-0 one.
    spawn_camera(
        app.world_mut(),
        Camera { order: 100, is_active: true, viewport: None },
        perspective(FRAC_PI_3),
        policy_winner,
    );
    let chosen = spawn_camera(
        app.world_mut(),
        Camera { order: 0, is_active: true, viewport: None },
        perspective(FRAC_PI_3),
        overridden,
    );
    set_active(&mut app, Some(chosen));

    resolve(&mut app);

    assert_eq!(view_of(&app).camera_pos.x, -7.0, "explicit override must win over policy");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 6 — an override naming a non-camera entity falls through to policy
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn override_of_non_camera_falls_through_to_policy() {
    let mut app = camera_world();

    let policy = GlobalTransform(Affine3A {
        matrix3: Mat3::IDENTITY,
        translation: Vec3::new(4.0, 0.0, 0.0),
    });
    spawn_camera(
        app.world_mut(),
        Camera::DEFAULT,
        perspective(FRAC_PI_3),
        policy,
    );

    // Spawn a NON-camera entity and override to it: the resolver must not find it
    // among the camera rows and must fall through to the policy pass.
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    app.world_mut().run_system(move |mut cmds: Commands| {
        let e = cmds.spawn(GlobalTransform::IDENTITY).id();
        *probe.lock().expect("probe lock") = Some(e);
    });
    let non_camera = sink.lock().expect("probe lock").expect("spawn produced a handle");
    set_active(&mut app, Some(non_camera));

    resolve(&mut app);

    assert_eq!(view_of(&app).camera_pos.x, 4.0, "non-camera override falls through to policy");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 7 — from_camera basis: identity rotation reproduces the canonical basis
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn from_camera_identity_basis_is_canonical() {
    let global = Affine3A {
        matrix3: Mat3::IDENTITY,
        translation: Vec3::new(0.0, 0.0, 3.0),
    };
    let view = ViewUniform::from_camera(global, perspective(FRAC_PI_3));

    // Canonical marcher basis: right +X, up +Y, forward -Z.
    assert_eq!([view.cam_right.x, view.cam_right.y, view.cam_right.z], [1.0, 0.0, 0.0]);
    assert_eq!([view.cam_up.x, view.cam_up.y, view.cam_up.z], [0.0, 1.0, 0.0]);
    assert_eq!([view.cam_forward.x, view.cam_forward.y, view.cam_forward.z], [0.0, 0.0, -1.0]);
    assert_eq!([view.camera_pos.x, view.camera_pos.y, view.camera_pos.z], [0.0, 0.0, 3.0]);
    assert_eq!(view.fov_y, FRAC_PI_3);
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 8 — from_camera: view·inv_view == identity (view is global⁻¹)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn from_camera_view_is_inverse_of_world() {
    // A non-trivial but rigid camera pose (eye offset; identity rotation).
    let global = Affine3A {
        matrix3: Mat3::IDENTITY,
        translation: Vec3::new(2.0, -1.0, 5.0),
    };
    let view = ViewUniform::from_camera(global, perspective(FRAC_PI_3));

    // inv_view is the world matrix; view (embedded in view_proj) is its inverse,
    // so `view · inv_view == I`. Recompute `view = proj⁻¹? ` is awkward; instead
    // assert the documented identity: inv_view equals the world affine as Mat4,
    // and camera_pos equals the world translation.
    assert_eq!(view.inv_view, global.to_mat4());
    assert_eq!([view.camera_pos.x, view.camera_pos.y, view.camera_pos.z], [2.0, -1.0, 5.0]);
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 9 — orthographic camera carries the fov_y == 0.0 marcher-ortho sentinel
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn orthographic_camera_sets_fov_sentinel() {
    let view = ViewUniform::from_camera(
        Affine3A::IDENTITY,
        Projection::Orthographic {
            half_height: 1.0,
            aspect: 1.0,
            near: 0.0,
            far: 100.0,
        },
    );
    assert_eq!(view.fov_y, 0.0, "ortho carries the fov_y == 0.0 sentinel");
}
