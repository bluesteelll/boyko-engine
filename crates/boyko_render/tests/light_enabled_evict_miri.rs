//! Miri-TB coverage for the gate-5 `on_remove` eviction hook (`evict_light`), WITHOUT the
//! full scheduler.
//!
//! The functional gate-5 tests (`light_enabled_gate5_remove` / `_despawn`) drive the hook
//! through `app.update()` (the whole collect_lights schedule), which is glacial under
//! Miri-TB. This minimal test fires the SAME path — a deferred component remove applies
//! under a single `run_system` apply window, triggering `on_remove` -> `evict_light` ->
//! `resource_mut(LightTableDirty)` — so Miri can interpret it to completion fast. It is the
//! exact (signature-only-`unsafe fn`, safe-body) hook surface the soundness review cares
//! about; no `app.update()` / no GPU.
//!
//! SINGLE-TEST BINARY (see light_enabled_toggle.rs for the process-global-isolation note).

#[path = "le_support/common.rs"]
mod common;

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::{Commands, ResMut};
use boyko_render::light::{LightTableDirty, PointLight};

#[test]
fn on_remove_eviction_hook_marks_dirty_miri() {
    let mut app = common::lighting_app();
    let point = common::spawn_point_light(app.world_mut(), [2.0, 0.0, 0.0]);
    app.finish();

    // Reset the structural-change channel so the assertion isolates the on_remove mark
    // (plugin/seed setup may leave it set). No `app.update()` — the scheduler is exactly
    // what makes the full gate-5 test glacial under Miri, and we do not need it here.
    app.world_mut().run_system(|mut d: ResMut<LightTableDirty>| d.0 = false);
    assert!(
        !app.world().resource::<LightTableDirty>().0,
        "precondition: dirty channel reset before the remove"
    );

    // Remove the PointLight component. The deferred remove applies under `run_system`'s
    // apply window, firing the `on_remove` eviction hook (`evict_light` -> resource_mut
    // -> `.0 = true`). The entity survives (keeps Transform / GlobalTransform), so NO
    // surviving row's Changed tick advances — the dirty mark is the only signal.
    let p: Entity = point;
    app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(p).remove::<PointLight>();
    });

    assert!(
        app.world().resource::<LightTableDirty>().0,
        "gate-5 hook: on_remove(PointLight) -> evict_light marked LightTableDirty \
         (no Changed tick, no scheduler)"
    );
}
