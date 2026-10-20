//! Render wave PX1a tests 6 and 7b (`plan/H4-PX1a-hdr-lit.md` Tests 6, 7b, rev 3 arch W7; `px1a/cut.md`
//! D-E1, D-E2, D-E3, C2, C8; ruling 67 open question 1): **one owner of the pre-exposure, ordered
//! before every reader.**
//!
//! - **6 — the policy.** A cut is a [`CameraCut`] request or a change of the active camera's FULL
//!   `Entity` — so a despawned camera whose index the next camera reuses still cuts (an
//!   index-only identity would miss it); `pre_ratio` is `1.0` on frame 0 and on a cut and
//!   `pre_N / pre_{N−1}` otherwise. Pinned on the pure step and on a composed world.
//! - **7b — composition.** A world with `LightingPlugin` (or `AaPlugin`) but no `PostPlugin` panics
//!   at its first update naming `ResolvedPost` — neither plugin inserts a silent default.
//! - **The order probe (rule (xx)).** `PostPolicySet.after(CameraSet::Resolve)` is declared inside
//!   `PostPlugin` (a set-to-set edge expands at build whatever the add-order — the accepted
//!   equivalence of ruling 67 OQ1) and pinned here by data flow: a camera controller in
//!   `CameraSet::Control` swaps the active camera every frame and the policy must see each swap
//!   the frame it happens.
//!
//! **No test in this binary builds `LightingPlugin`** (critique O4): its light hooks are
//! process-global and a second `LightingPlugin::build` in one process panics. The two
//! `LightingPlugin` cases live in their own single-test binaries:
//! `post_plugin_lighting_without_post.rs` (7b) and `post_plugin_order_probe.rs` (the
//! `collect_lights` / `resolve_taa_policy` edges).

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_render::light::{LightingConfig, Tonemapper};
use boyko_render::post::{CameraCut, PostPlugin, ResolvedPost};
use boyko_render::AaPlugin;
use boyko_scene::{Camera, CameraPlugin, CameraSet, GlobalTransform, Projection, Transform};

const PROJ: Projection = Projection::Perspective { fov_y: 1.0, aspect: 1.0, near: 0.1, far: 100.0 };

fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; its `size_of::<T>()` bytes are read through a shared borrow
    // that the returned slice keeps alive. Every `T` spawned here is a fixed-layout component POD.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Spawns a camera row the active-camera resolve reads (`Camera` + `Projection` + the pose pair).
fn spawn_camera(world: &mut EcsMaster, order: i32, is_active: bool) -> Entity {
    let arch = world.create_archetype(&[
        Camera::component_id(),
        Projection::component_id(),
        Transform::component_id(),
        GlobalTransform::component_id(),
    ]);
    let cam = Camera { order, is_active, viewport: None };
    world
        .create_entity(
            arch,
            &[
                (Camera::component_id(), as_bytes(&cam)),
                (Projection::component_id(), as_bytes(&PROJ)),
                (Transform::component_id(), as_bytes(&Transform::IDENTITY)),
                (GlobalTransform::component_id(), as_bytes(&GlobalTransform::IDENTITY)),
            ],
        )
        .expect("invariant: the camera archetype accepts its four columns")
}

fn post(app: &App) -> ResolvedPost {
    *app.world().resource::<ResolvedPost>()
}

fn post_app() -> App {
    let mut app = App::new();
    app.insert_resource(LightingConfig::default());
    app.add_plugins(CameraPlugin);
    app.add_plugins(PostPlugin);
    app.finish();
    app
}

// ---- 6: the pure policy step -----------------------------------------------------------------

#[test]
fn the_policy_step_cuts_on_a_camera_change_and_on_a_request_and_carries_the_ratio() {
    let cfg = LightingConfig::default();
    let boot = ResolvedPost::from_config(&cfg);
    assert_eq!(
        boot,
        ResolvedPost {
            pre_exposure: 1.0,
            pre_ratio: 1.0,
            prev_pre_exposure: 1.0,
            tonemapper: 0,
            cut: 0,
            active_camera: None
        }
    );

    let a = Entity::new(EntityId(3), 0);
    let a_reused = Entity::new(EntityId(3), 1);
    let b = Entity::new(EntityId(4), 0);

    let f0 = boot.next(&cfg, Some(a), false);
    assert_eq!((f0.cut, f0.pre_ratio, f0.active_camera), (1, 1.0, Some(a)), "frame 0 acquires a camera: a cut");
    let f1 = f0.next(&cfg, Some(a), false);
    assert_eq!((f1.cut, f1.pre_ratio), (0, 1.0), "a steady frame");

    let bright = LightingConfig { exposure: 2.0, ..cfg };
    let f2 = f1.next(&bright, Some(a), false);
    assert_eq!((f2.pre_exposure, f2.pre_ratio, f2.prev_pre_exposure, f2.cut), (2.0, 2.0, 1.0, 0), "the ratio");
    let f3 = f2.next(&bright, Some(a), false);
    assert_eq!((f3.pre_ratio, f3.prev_pre_exposure), (1.0, 2.0), "a steady exposure: ratio 1");

    let f4 = f3.next(&bright, Some(a_reused), false);
    assert_eq!((f4.cut, f4.active_camera), (1, Some(a_reused)), "the reused index with a new generation is a new camera");
    let f5 = f4.next(&bright, Some(a_reused), true);
    assert_eq!((f5.cut, f5.pre_ratio), (1, 1.0), "a CameraCut request");
    let f6 = f5.next(&cfg, Some(a_reused), true);
    assert_eq!((f6.cut, f6.pre_ratio, f6.pre_exposure, f6.prev_pre_exposure), (1, 1.0, 1.0, 2.0), "a cut drops the ratio");
    let f7 = f6.next(&cfg, Some(b), false);
    assert_eq!(f7.cut, 1, "another camera");
    let f8 = f7.next(&cfg, None, false);
    assert_eq!((f8.cut, f8.active_camera), (1, None), "losing the camera");
    let f9 = f8.next(&cfg, None, false);
    assert_eq!(f9.cut, 0, "no camera, steady");

    let neutral = LightingConfig { tonemapper: Tonemapper::Neutral, ..cfg };
    assert_eq!(f9.next(&neutral, None, false).tonemapper, Tonemapper::Neutral as u32);
}

// ---- 6: the composed world -------------------------------------------------------------------

#[test]
fn a_despawned_camera_whose_index_is_reused_cuts_and_a_camera_cut_is_read_next_frame() {
    let mut app = post_app();
    let a = spawn_camera(app.world_mut(), 0, true);
    app.update();
    assert_eq!((post(&app).cut, post(&app).active_camera), (1, Some(a)), "frame 0 acquires the camera");
    app.update();
    assert_eq!(post(&app).cut, 0);

    assert!(app.world_mut().despawn_without_children(a));
    let b = spawn_camera(app.world_mut(), 0, true);
    assert_eq!(b.id(), a.id(), "precondition: the next camera reuses the despawned index");
    assert_ne!(b.generation(), a.generation(), "precondition: under a new generation");
    app.update();
    assert_eq!((post(&app).cut, post(&app).active_camera), (1, Some(b)), "the reused index did not cut");
    app.update();
    assert_eq!(post(&app).cut, 0);

    app.world().send_event(0, CameraCut::request()).expect("invariant: CameraCut is registered by PostPlugin");
    app.update();
    assert_eq!(post(&app).cut, 1, "a CameraCut sent before this frame is read this frame");
    app.update();
    assert_eq!(post(&app).cut, 0, "one request, one cut");

    app.world_mut().resource_mut::<LightingConfig>().exposure = 4.0;
    app.update();
    assert_eq!((post(&app).pre_exposure, post(&app).pre_ratio), (4.0, 4.0));
}

// ---- 7b: composition -------------------------------------------------------------------------

#[test]
#[should_panic(expected = "ResolvedPost")]
fn an_aa_world_without_post_plugin_panics_naming_resolved_post() {
    let mut app = App::new();
    app.add_plugins(AaPlugin);
    app.finish();
    app.update();
}

// ---- the order probes (rule (xx)) ------------------------------------------------------------

#[test]
fn resolve_post_policy_sees_a_camera_switch_made_in_camera_set_control_the_same_frame() {
    let mut app = App::new();
    app.insert_resource(LightingConfig::default());
    app.add_plugins(PostPlugin);
    app.add_plugins(CameraPlugin);
    // A controller in `CameraSet::Control` that swaps which camera is active, every frame.
    app.add_systems_cfg(|b| {
        b.add_system(|mut cams: Query<&mut Camera>| {
            for cam in cams.iter_mut() {
                cam.is_active = !cam.is_active;
            }
        })
        .in_set(CameraSet::Control);
    });
    app.finish();
    let a = spawn_camera(app.world_mut(), 0, true);
    let b = spawn_camera(app.world_mut(), 1, false);
    for frame in 0..6 {
        app.update();
        // The controller ran first this frame: after an odd number of swaps `b` is active.
        let expect = if frame % 2 == 0 { b } else { a };
        assert_eq!(post(&app).active_camera, Some(expect), "frame {frame}: the policy read last frame's camera");
        assert_eq!(post(&app).cut, 1, "frame {frame}: every swap is a cut");
    }
}
