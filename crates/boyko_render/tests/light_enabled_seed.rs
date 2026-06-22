//! Gate 4 (`seed_ok`): a light spawned WITHOUT explicitly setting `LightEnabled` is enabled
//! by default (the seed turns its bit on) and appears in the table — AND it appears in the
//! SAME pass as the first seed run (proves the W2 in-pass immediate seeding, not a one-frame
//! lag). This is the back-compat guarantee: existing lighting code never touches
//! `LightEnabled`, so its lights must stay visible.
//!
//! SINGLE-TEST BINARY (see light_enabled_toggle.rs for the process-global-isolation note).

#[path = "le_support/common.rs"]
mod common;

#[test]
fn lights_spawned_without_light_enabled_are_seeded_in_the_same_pass() {
    let mut app = common::lighting_app();

    // One of each kind, none touching LightEnabled.
    common::spawn_dir_light(app.world_mut(), [0.0, 0.0, -1.0]);
    common::spawn_sky_light(app.world_mut());
    common::spawn_point_light(app.world_mut(), [1.0, 0.0, 0.0]);
    common::spawn_spot_light(app.world_mut());

    // A SINGLE pass: the exclusive seed (first run) enables every light's bit IMMEDIATELY,
    // before collect_lights folds in the same pass — so all four appear after ONE update,
    // not the next frame.
    app.finish();
    app.update();

    assert_eq!(
        common::light_count(&app),
        4,
        "all four un-tagged lights are seeded-enabled and appear in the SAME pass (back-compat + W2)"
    );
    assert_eq!(common::l0a_count(&app), 2, "directional + sky in the no-P front block");
    assert_eq!(common::point_spot_count(&app), 2, "point + spot in the L0b block");
}
