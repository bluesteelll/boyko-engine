//! The [`Render3dPlugin`] (standard-library Phase S4) — registers the per-frame
//! 3D GPU-instance pack as a first-class scheduled system (Principle 0: a system
//! on the engine scheduler, not glue the host hand-rolls), symmetric with
//! [`LightingPlugin`](crate::light_plugin::LightingPlugin).

use boyko_ecs::ecs::core::app::{App, Plugin};
use boyko_scene::VisibilitySet;

use crate::gpu3d_system::sync_gpu_3d_instances;
use crate::instance_model::InstancePackSet;

/// Registers
/// [`sync_gpu_3d_instances`] — the
/// system that packs each visible entity's `GlobalTransform` into its
/// `Gpu3dInstance` column for upload.
///
/// # Ordering contract (vs. propagation and `visibility_sync`)
///
/// `sync_gpu_3d_instances` reads the propagated `GlobalTransform`, so it must run
/// AFTER `propagate_transforms`; it filters on `Enabled<RenderEnabled>`, so it must
/// run AFTER `visibility_sync`'s apply window. Neither edge can be expressed by key
/// here (both systems' `SystemKey`s live in `TransformPlugin` / `CameraPlugin`), so
/// the pack declares membership by name: it joins [`InstancePackSet`] and
/// [`VisibilitySet::Read`], and the composing host configures
/// `InstancePackSet.after(CameraSet::Resolve)` and `VisibilitySet::Read.after(VisibilitySet::Sync)`
/// (`boyko_app::EnginePlugins` does). A host composing this plugin by hand declares
/// the same two edges; without them the pack's position is whatever the executor's wave
/// packing makes it, and the pack is unconditional, so a wrong position is a permanent
/// one-frame lag, not a self-correcting stagger.
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
            b.add_system(sync_gpu_3d_instances)
                .in_set(InstancePackSet)
                .in_set(VisibilitySet::Read);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::Render3dPlugin"
    }
}
