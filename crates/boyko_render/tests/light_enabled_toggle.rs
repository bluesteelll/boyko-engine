//! Gate 2 (`enable_toggle_ok`): a `LightEnabled`-OFF light is excluded from the table;
//! toggling ON (via the immediate `set_light_enabled_now`, which marks dirty) re-adds it
//! on the next collect; OFF again removes it.
//!
//! SINGLE-TEST BINARY: this file's one `#[test]` adds `LightingPlugin` (which registers
//! the process-global eviction hooks first) and then spawns lights. Do NOT co-locate a
//! second light-archetyping test here — `was_ever_archetyped` is process-global and never
//! reset, so a second `LightingPlugin::build` would panic `AlreadyArchetyped`.

#[path = "le_support/common.rs"]
mod common;

use boyko_render::light_system::set_light_enabled_now;

#[test]
fn toggle_off_excludes_and_on_re_adds_a_light() {
    let mut app = common::lighting_app();

    // Spawn two point lights (no LightEnabled touched — the seed enables them).
    let a = common::spawn_point_light(app.world_mut(), [0.0, 0.0, 0.0]);
    let _b = common::spawn_point_light(app.world_mut(), [1.0, 0.0, 0.0]);

    // Settle into the steady state: the first frame's seed first-run enables both bits
    // (in-pass) and collect_lights folds both; the extra frames drain the spawn's `Added`
    // window so the seed's steady-state `Added<*Light>` scan is empty. This matters because
    // the seed RE-ENABLES any light still reported as `Added` — toggling a light OFF while
    // it is still inside its spawn `Added` window would be undone by the same-pass seed.
    // Steady-state runtime toggling (the gate's scenario) is past that window.
    app.finish();
    app.update();
    app.update();
    app.update();
    assert_eq!(
        common::point_spot_count(&app),
        2,
        "both seeded point lights appear in the table in the steady state"
    );

    // Disable light `a` via the immediate toggle surface (marks LightTableDirty so the
    // tickless bit flip is observed) — then run a frame.
    set_light_enabled_now(app.world_mut(), a, false);
    app.update();
    assert_eq!(
        common::point_spot_count(&app),
        1,
        "a LightEnabled-OFF light is excluded from the table (2 -> 1)"
    );

    // Re-enable `a` — it must come back next collect.
    set_light_enabled_now(app.world_mut(), a, true);
    app.update();
    assert_eq!(
        common::point_spot_count(&app),
        2,
        "toggling ON re-adds the light to the table (1 -> 2)"
    );

    // And OFF again removes it (mirror, proves the toggle is symmetric).
    set_light_enabled_now(app.world_mut(), a, false);
    app.update();
    assert_eq!(
        common::point_spot_count(&app),
        1,
        "toggling OFF again removes the light (2 -> 1)"
    );
}
