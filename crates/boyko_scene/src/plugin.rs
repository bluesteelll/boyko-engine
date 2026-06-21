//! The [`TransformPlugin`] — registers the spatial vocabulary's per-frame
//! propagation system (S2).

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::propagation::{ensure_detach_observer, propagate_transforms};

/// Registers [`propagate_transforms`](crate::propagation::propagate_transforms)
/// into the App's per-frame (`Main`) schedule.
///
/// Per the S2 schedule table, propagation runs once per frame after the fixed
/// (physics) schedule has fully advanced and before the camera / light /
/// GPU-upload readers. Cross-schedule ordering is enforced by registering it in
/// the `Main` schedule (this plugin); intra-frame `.before(...)` edges against
/// the readers are added by the consuming render/camera plugins (S3/S4), which
/// own those systems.
///
/// The scratch resource ([`TransformPropagationScratch`]) is lazily inserted by
/// the system on first run (no explicit `init_resource` is required), keeping
/// this plugin a single registration.
///
/// # F1 detach observer
///
/// `build` also EAGERLY installs the `ChildOf` `on_remove` observer (F1) via
/// [`ensure_detach_observer`], so a detach issued BEFORE the first
/// `propagate_transforms` run is still queued and re-rooted. The install is
/// idempotent and shared with the system's own lazy install (the system installs
/// it on first run when driven without the plugin, e.g. in the gates).
///
/// [`TransformPropagationScratch`]: crate::propagation::TransformPropagationScratch
/// [`ensure_detach_observer`]: crate::propagation::ensure_detach_observer
#[derive(Default)]
pub struct TransformPlugin;

impl Plugin for TransformPlugin {
    fn build(&self, app: &mut App) {
        ensure_detach_observer(app.world_mut());
        app.add_systems(propagate_transforms);
    }

    fn name(&self) -> &'static str {
        "boyko_scene::TransformPlugin"
    }
}
