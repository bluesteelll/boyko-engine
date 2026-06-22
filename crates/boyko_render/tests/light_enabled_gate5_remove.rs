//! Gate 5a (`gate5_evict_ok`, the LOAD-BEARING gate): removing a light COMPONENT from a
//! live entity evicts that light from the GPU table on the next `collect_lights`.
//!
//! The entity survives the remove, so no surviving row's `Changed` tick advances — the
//! `Changed` gate alone could NEVER see it. The `on_remove` hook (registered first in
//! `LightingPlugin::build`) marks `LightTableDirty`, so the next collect rebuilds with the
//! removed light gone. This is the class that failed 1/11 in the prior attempt.
//!
//! SINGLE-TEST BINARY (see light_enabled_toggle.rs for the process-global-isolation note).

#[path = "le_support/common.rs"]
mod common;

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_render::light::PointLight;

#[test]
fn removing_a_light_component_evicts_it_from_the_table() {
    let mut app = common::lighting_app();

    // One directional + one point light (seed enables both).
    let _dir = common::spawn_dir_light(app.world_mut(), [0.0, 0.0, -1.0]);
    let point = common::spawn_point_light(app.world_mut(), [2.0, 0.0, 0.0]);

    app.finish();
    app.update();
    assert_eq!(common::light_count(&app), 2, "both lights present after the first frame");
    assert_eq!(common::point_spot_count(&app), 1, "one point light in the L0b block");

    // Remove the PointLight COMPONENT (the entity survives — it keeps Transform /
    // GlobalTransform). The deferred remove applies under the apply window, firing the
    // on_remove eviction hook which marks LightTableDirty. `run_system` drains the hook
    // queue at its apply window (verified: phase14a runtime on_remove fires via the same
    // Commands path).
    let p: Entity = point;
    app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(p).remove::<PointLight>();
    });

    // Next collect rebuilds (LightTableDirty set by the hook) — the point light is gone.
    app.update();
    assert_eq!(
        common::light_count(&app),
        1,
        "gate-5a: removing the PointLight component evicts it (2 -> 1)"
    );
    assert_eq!(
        common::point_spot_count(&app),
        0,
        "the L0b block is now empty (the only point light was removed)"
    );
}
