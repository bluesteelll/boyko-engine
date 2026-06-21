//! The [`LightingPlugin`] (standard-library Phase S4) — registers the per-frame
//! light-pose reconcile + the light-table collection in one builder closure so
//! the ordering edge between them is expressible.

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::light_reconcile::light_reconcile;
use crate::light_system::collect_lights;

/// Registers [`light_reconcile`](crate::light_reconcile::light_reconcile) BEFORE
/// [`collect_lights`](crate::light_system::collect_lights).
///
/// # Why one closure
///
/// Intra-schedule ordering edges are keyed by `SystemKey`, obtainable only at the
/// `add_system` call site, so the `.before` edge can be expressed only where BOTH
/// systems are registered. This plugin therefore co-registers `light_reconcile`
/// and `collect_lights` together, exactly as
/// [`CameraPlugin`](boyko_scene::CameraPlugin) co-registers `propagate_transforms`
/// + `resolve_active_camera`.
///
/// # Add-order contract (cross-schedule ordering vs. propagation)
///
/// `light_reconcile` reads the propagated `GlobalTransform`, so it must run AFTER
/// `propagate_transforms`. That edge cannot be expressed here (the propagation
/// system's key lives in `TransformPlugin` / `CameraPlugin`). **Add
/// `LightingPlugin` together with `TransformPlugin` or `CameraPlugin`** so the
/// host schedule runs propagation first. The `Changed<GlobalTransform>` gate on
/// `light_reconcile` makes a loose one-frame ordering stagger self-correcting (a
/// stale read re-fires next frame), but the intended order is propagate →
/// reconcile → collect.
#[derive(Default)]
pub struct LightingPlugin;

impl Plugin for LightingPlugin {
    fn build(&self, app: &mut App) {
        // Co-register so the `.before` ordering edge between the two keys is
        // expressible in a single closure (mirrors `CameraPlugin`).
        app.add_systems_cfg(|b| {
            let collect = b.add_system(collect_lights).key();
            b.add_system(light_reconcile).before(collect);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::LightingPlugin"
    }
}
