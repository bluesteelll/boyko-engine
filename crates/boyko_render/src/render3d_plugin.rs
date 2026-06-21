//! The [`Render3dPlugin`] (standard-library Phase S4) — registers the per-frame
//! 3D GPU-instance pack as a first-class scheduled system (Principle 0: a system
//! on the engine scheduler, not glue the host hand-rolls), symmetric with
//! [`LightingPlugin`](crate::light_plugin::LightingPlugin).

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::gpu3d_system::sync_gpu_3d_instances;

/// Registers
/// [`sync_gpu_3d_instances`](crate::gpu3d_system::sync_gpu_3d_instances) — the
/// system that packs each visible entity's `GlobalTransform` into its
/// `Gpu3dInstance` column for upload.
///
/// # Add-order contract (cross-schedule ordering vs. propagation)
///
/// `sync_gpu_3d_instances` reads the propagated `GlobalTransform`, so it must run
/// AFTER `propagate_transforms`. That ordering edge cannot be expressed here (the
/// propagation system's `SystemKey` lives in `TransformPlugin` / `CameraPlugin`),
/// so **add `Render3dPlugin` together with `TransformPlugin` or `CameraPlugin`** so
/// the host schedule runs propagation first — the same add-order discipline
/// [`LightingPlugin`](crate::light_plugin::LightingPlugin) documents for
/// `light_reconcile`. The system's `Changed`-driven inputs make a loose one-frame
/// stagger self-correcting (a stale read re-packs next frame).
///
/// # Scope
///
/// This wires the PACK (`GlobalTransform` → `Gpu3dInstance` column). The DRAW /
/// column → GPU upload (`bytemuck::cast_slice`) remains the consuming renderer's
/// responsibility (the demo's `upload_instances` pattern) — S4 owns the pack + the
/// column, not the draw-count / cull policy.
#[derive(Default)]
pub struct Render3dPlugin;

impl Plugin for Render3dPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems_cfg(|b| {
            b.add_system(sync_gpu_3d_instances);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::Render3dPlugin"
    }
}
