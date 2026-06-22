//! Gate 5b (`gate5_evict_ok`, the LOAD-BEARING gate): DESPAWNing a light entity evicts it
//! from the GPU table on the next `collect_lights`.
//!
//! A full despawn fires `on_remove` per component too (so the single `on_remove`
//! registration subsumes despawn), which marks `LightTableDirty`; the next collect rebuilds
//! with the despawned light gone.
//!
//! SINGLE-TEST BINARY (see light_enabled_toggle.rs for the process-global-isolation note).

#[path = "le_support/common.rs"]
mod common;

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;

#[test]
fn despawning_a_light_entity_evicts_it_from_the_table() {
    let mut app = common::lighting_app();

    let _dir = common::spawn_dir_light(app.world_mut(), [0.0, 0.0, -1.0]);
    let point = common::spawn_point_light(app.world_mut(), [2.0, 0.0, 0.0]);

    app.finish();
    app.update();
    assert_eq!(common::light_count(&app), 2, "both lights present after the first frame");

    // Despawn the WHOLE point-light entity. The deferred despawn applies under the apply
    // window and fires on_remove per component (including PointLight) → marks
    // LightTableDirty.
    let p: Entity = point;
    app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(p).despawn();
    });
    assert!(!app.world().has_entity(point), "the point-light entity is gone after despawn");

    // Next collect rebuilds — the despawned light is evicted.
    app.update();
    assert_eq!(
        common::light_count(&app),
        1,
        "gate-5b: despawning the light entity evicts it (2 -> 1)"
    );
    assert_eq!(common::point_spot_count(&app), 0, "the despawned point light is gone");
}
