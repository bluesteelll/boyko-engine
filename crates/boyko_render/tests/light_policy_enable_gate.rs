//! P1 gate (enabled-only count): `select_lighting_cull` counts ONLY lights whose
//! `IsEnabled<LightEnabled>` bit is set — exactly the enabled-test `collect_lights` folds
//! on. A `LightEnabled`-disabled point/spot light is excluded from
//! `LightStats.point_spot_count`.
//!
//! SINGLE-TEST BINARY (see `le_support/common.rs` isolation note).

#[path = "le_support/common.rs"]
mod common;

use boyko_render::light_system::set_light_enabled_now;

#[test]
fn disabled_point_spot_lights_are_not_counted() {
    let mut app = common::lighting_app();

    // Three point lights + one spot — all seeded enabled.
    let a = common::spawn_point_light(app.world_mut(), [0.0, 0.0, 0.0]);
    let _b = common::spawn_point_light(app.world_mut(), [1.0, 0.0, 0.0]);
    let _c = common::spawn_point_light(app.world_mut(), [2.0, 0.0, 0.0]);
    let s = common::spawn_spot_light(app.world_mut());

    // Settle past the spawn `Added` window so the seed has enabled every light and the
    // steady-state seed scan is empty (so a later disable is not re-enabled in-pass).
    app.finish();
    app.update();
    app.update();
    app.update();
    assert_eq!(
        common::policy_point_spot_count(&app),
        4,
        "all four seeded point/spot lights are counted while enabled"
    );

    // Disable one point + the spot via the immediate toggle surface.
    set_light_enabled_now(app.world_mut(), a, false);
    set_light_enabled_now(app.world_mut(), s, false);
    app.update();
    assert_eq!(
        common::policy_point_spot_count(&app),
        2,
        "LightEnabled-disabled point/spot lights are excluded from the count (4 -> 2)"
    );

    // Re-enable the point — it must be counted again.
    set_light_enabled_now(app.world_mut(), a, true);
    app.update();
    assert_eq!(
        common::policy_point_spot_count(&app),
        3,
        "re-enabling a light re-includes it in the count (2 -> 3)"
    );
}
