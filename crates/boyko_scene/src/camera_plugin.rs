//! The [`CameraPlugin`] (S3) — registers the per-frame camera-resolution system
//! and seeds the camera resources.

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::camera::{ActiveCamera, ViewUniform, resolve_active_camera};
use crate::propagation::{ensure_detach_observer, propagate_transforms};
use crate::visibility_sync::visibility_sync;

/// Registers [`resolve_active_camera`](crate::camera::resolve_active_camera) into
/// the App's per-frame (`Main`) schedule, ordered **after**
/// [`propagate_transforms`](crate::propagation::propagate_transforms), and seeds
/// the [`ActiveCamera`] / [`ViewUniform`] resources.
///
/// # Why this plugin owns the propagation registration too
///
/// The S2 schedule table requires `resolve_active_camera` to run after
/// `propagate_transforms` so the camera's `GlobalTransform` is the freshly
/// propagated world pose. Intra-schedule ordering edges are keyed by
/// [`SystemKey`](boyko_ecs::ecs::core::schedule::system_config::SystemConfig::key),
/// which is only obtainable at the `add_system` call site — so the `.after`
/// edge can only be expressed where BOTH systems are registered. This plugin
/// therefore registers `propagate_transforms` AND `resolve_active_camera`
/// together in one builder closure with the explicit edge.
///
/// **Add `CameraPlugin` INSTEAD of
/// [`TransformPlugin`](crate::plugin::TransformPlugin)** when you need the camera
/// view (it supersedes it — registering both double-registers
/// `propagate_transforms`). `TransformPlugin` remains the standalone choice for a
/// world that only needs propagation and no camera.
///
/// The `ChildOf` detach observer (F1) is installed eagerly here too (idempotent;
/// shared with the system's own lazy install), matching `TransformPlugin`.
#[derive(Default)]
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        // Seed the camera resources so the `Res<ActiveCamera>` / `ResMut<ViewUniform>`
        // params resolve on the very first frame (a missing resource is a panic).
        app.insert_resource(ActiveCamera::default());
        app.insert_resource(ViewUniform::default());

        // F1: install the detach observer eagerly (idempotent), exactly as
        // `TransformPlugin` does — a detach issued before the first propagate run
        // is still queued and re-rooted.
        ensure_detach_observer(app.world_mut());

        // Register propagation + camera resolution + the visibility bridge with
        // the ordering edges: the resolver runs AFTER propagation so it reads the
        // freshly-composed world pose, and `visibility_sync` (S4 follow-up) is
        // ordered AFTER propagation too so the durable `Visibility` →
        // `RenderEnabled` bridge sits in the documented per-frame chain (it must
        // run BEFORE the render pack — that cross-crate edge is contract-documented;
        // see `crate::visibility_sync::visibility_sync`). All keys are captured in
        // this single closure.
        app.add_systems_cfg(|b| {
            let propagate = b.add_system(propagate_transforms).key();
            b.add_system(resolve_active_camera).after(propagate);
            b.add_system(visibility_sync).after(propagate);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_scene::CameraPlugin"
    }
}
