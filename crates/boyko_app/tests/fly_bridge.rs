//! R6 fly-camera host integration gates (`boyko_app`, host plan R6).
//!
//! Headless (no window, no device):
//!
//!  1. **Input-ingest bridge** — with `InputPlugin<FlyAction>` composed, pushing a
//!     `MouseMotion` + `Key{W,Pressed}` + `Key{Escape,Pressed}` into the World's
//!     `RawInputQueue` and running a frame folds them into `PhysicalInput`
//!     (mouse_delta + W level) and `ActionState<FlyAction>` (`Quit` pressed).
//!  3. **Ordering pin** — `fly_camera_system` (CameraSet::Control) runs BEFORE
//!     `propagate_transforms` (CameraSet::Resolve) REGARDLESS of whether
//!     `CameraPlugin` or `FlyCameraPlugin` is added first: a single frame with a
//!     held `W` recomposes the camera's `GlobalTransform` to the JUST-FLOWN pose
//!     the SAME frame (no one-frame lag), in both add-orders.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::AppExit;

use boyko_input::{ActionState, ButtonState, KeyCode, PhysicalInput, RawInputEvent, RawInputQueue};

use boyko_math::Vec3;

use boyko_scene::{
    CameraPlugin, Camera, FlyCamera, FlyCameraBundle, GlobalTransform, Projection, Transform,
};

use boyko_app::{FlyAction, FlyCameraPlugin};

const FIXED_DELTA: Duration = Duration::from_millis(16);
const EPS: f32 = 1.0e-4;

fn key(code: KeyCode, state: ButtonState) -> RawInputEvent {
    RawInputEvent::Key { code, state, repeat: false }
}

/// Pushes a raw event into the World's `RawInputQueue` (the runner's step-1
/// bridge does this from OS messages; here we inject synthetic events).
fn push(world: &mut EcsMaster, ev: RawInputEvent) {
    world.resource_mut::<RawInputQueue>().push_raw(ev);
}

// ════════════════════════════════════════════════════════════════════════════
// 1. Input-ingest bridge
// ════════════════════════════════════════════════════════════════════════════

/// `InputPlugin<FlyAction>` (composed by `FlyCameraPlugin`) folds queued raw
/// events into `PhysicalInput` + `ActionState<FlyAction>` on the frame's
/// `update_action_state`: the mouse delta sums, W's level holds, and the
/// Escape-bound `Quit` action reads pressed.
#[test]
fn input_plugin_folds_queued_events_into_snapshot_and_actions() {
    let mut app = App::new();
    // FlyCameraPlugin composes InputPlugin<FlyAction> (+ the controller + quit).
    // CameraPlugin supplies propagate + resolve so the schedule is well-formed.
    app.add_plugin(CameraPlugin);
    app.add_plugin(FlyCameraPlugin);
    // `quit_on_action` reads `ResMut<AppExit>`; the real runner inserts it before
    // the frame loop (`App::run` / `run_windowed`). This test drives frames
    // directly (bypassing `App::run`), so mirror that insert-if-absent.
    app.insert_resource(AppExit(false));
    app.finish();

    // Inject this frame's events (as the runner bridge would).
    push(app.world_mut(), RawInputEvent::MouseMotion { dx: 12.0, dy: -5.0 });
    push(app.world_mut(), key(KeyCode::KeyW, ButtonState::Pressed));
    push(app.world_mut(), key(KeyCode::Escape, ButtonState::Pressed));

    app.update_with_delta(FIXED_DELTA);

    let physical = app.world().resource::<PhysicalInput>();
    assert_eq!(physical.mouse_delta, [12.0, -5.0], "mouse delta summed from the queue");
    let w = KeyCode::KeyW.dense_index().unwrap();
    assert!(physical.keys_pressed.get(w), "W level held after the pressed event");

    let actions = app.world().resource::<ActionState<FlyAction>>();
    assert!(actions.pressed(FlyAction::Quit), "Escape → FlyAction::Quit is pressed");
}

// ════════════════════════════════════════════════════════════════════════════
// 3. Ordering pin (both plugin add-orders → no one-frame lag)
// ════════════════════════════════════════════════════════════════════════════

/// Spawns a fly camera at `eye`; returns its handle.
fn spawn_fly_camera(world: &mut EcsMaster, eye: Vec3) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(FlyCameraBundle {
                transform: Transform::from_translation(eye),
                global: GlobalTransform::IDENTITY,
                camera: Camera::DEFAULT,
                projection: Projection::Perspective { fov_y: core::f32::consts::FRAC_PI_2, aspect: 1.0, near: 0.1, far: 100.0 },
                fly: FlyCamera::new(0.0, 0.0),
            })
            .id();
        *probe.lock().expect("probe lock") = Some(e);
    });
    let e = sink.lock().expect("probe lock").expect("spawn produced a handle");
    assert!(world.has_entity(e), "spawned fly camera is live");
    e
}

/// Drives one frame with `W` held (via the input queue) on an app built with the
/// two plugins added in the given order, and asserts the camera's propagated
/// `GlobalTransform` matches the JUST-FLOWN local `Transform` — i.e. the fly write
/// (Control) preceded propagation (Resolve) in the SAME frame.
fn assert_no_lag_for_order(camera_first: bool) {
    let mut app = App::new();
    if camera_first {
        app.add_plugin(CameraPlugin);
        app.add_plugin(FlyCameraPlugin);
    } else {
        app.add_plugin(FlyCameraPlugin);
        app.add_plugin(CameraPlugin);
    }
    // `quit_on_action` reads `ResMut<AppExit>` — inserted by the real runner
    // before the loop; mirror that here (this test drives frames directly).
    app.insert_resource(AppExit(false));
    app.finish();

    let start_eye = Vec3::new(0.0, 1.0, 5.0);
    let e = spawn_fly_camera(app.world_mut(), start_eye);

    // Hold W: the ingest applies the level, the fly moves the eye forward.
    push(app.world_mut(), key(KeyCode::KeyW, ButtonState::Pressed));
    app.update_with_delta(FIXED_DELTA);

    let local = *app.world().get_component::<Transform>(e).expect("has Transform");
    let global = app
        .world()
        .get_component::<GlobalTransform>(e)
        .expect("has GlobalTransform")
        .affine();

    // The eye actually moved forward (-Z at yaw 0), so "no lag" is a real claim.
    assert!(
        (local.translation.z - start_eye.z).abs() > EPS,
        "the fly write moved the eye this frame (order camera_first={camera_first})"
    );
    // The propagated world translation equals the just-flown local translation:
    // propagation (Resolve) observed the fly write (Control) THIS frame.
    let order = if camera_first { "CameraPlugin first" } else { "FlyCameraPlugin first" };
    assert!(
        (global.translation.x - local.translation.x).abs() <= EPS
            && (global.translation.y - local.translation.y).abs() <= EPS
            && (global.translation.z - local.translation.z).abs() <= EPS,
        "{order}: GlobalTransform must track the fly write the SAME frame (no lag): \
         global={:?} local={:?}",
        global.translation,
        local.translation
    );
}

/// The `Control.before(Resolve)` set-to-set edge holds when `CameraPlugin` is
/// added FIRST.
#[test]
fn ordering_holds_camera_plugin_first() {
    assert_no_lag_for_order(true);
}

/// ...and when `FlyCameraPlugin` is added FIRST (the edge is add-order-independent
/// because it is pinned by NAME, set-to-set — not by a co-visible `SystemKey`).
#[test]
fn ordering_holds_fly_plugin_first() {
    assert_no_lag_for_order(false);
}
