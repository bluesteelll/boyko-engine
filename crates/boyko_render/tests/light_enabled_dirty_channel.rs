//! Gate 6 (`dirty_channel_ok`): a `LightEnabled` bit flip bumps NO `Changed` tick, yet it
//! still triggers a `collect_lights` rebuild — via the `LightTableDirty` structural-change
//! channel. Also pins the dirty-consume invariant (W2): after every rebuild
//! `LightTableDirty.0 == false`, so a set bit is never stranded.
//!
//! SINGLE-TEST BINARY (see light_enabled_toggle.rs for the process-global-isolation note).

#[path = "le_support/common.rs"]
mod common;

use boyko_render::light::LightTableDirty;
use boyko_render::light_system::set_light_enabled_now;

#[test]
fn bit_flip_rebuilds_via_dirty_channel_without_a_changed_tick() {
    let mut app = common::lighting_app();

    let a = common::spawn_point_light(app.world_mut(), [0.0, 0.0, 0.0]);
    let _b = common::spawn_point_light(app.world_mut(), [1.0, 0.0, 0.0]);

    // Settle: first frame seeds + folds, a couple more drain the Added/Changed windows so
    // the scene is genuinely STATIC (no Changed tick will fire from here).
    app.finish();
    app.update();
    app.update();
    app.update();
    assert_eq!(common::point_spot_count(&app), 2, "two static point lights folded");

    // Sanity: a purely static frame leaves the table at 2 and consumes no dirty (the
    // dirty channel is clean before the flip).
    assert!(
        !app.world().resource::<LightTableDirty>().0,
        "dirty channel is clear on a static scene before the toggle"
    );

    // Flip a's bit OFF. This bumps NO Changed tick (tickless O(1) bitset op). The ONLY
    // signal collect_lights can see is the LightTableDirty mark the toggle surface sets.
    set_light_enabled_now(app.world_mut(), a, false);
    assert!(
        app.world().resource::<LightTableDirty>().0,
        "the toggle marked LightTableDirty (the bit flip is tickless, so this is the only channel)"
    );

    app.update();
    // Rebuilt purely because of the dirty channel (no Changed tick existed).
    assert_eq!(
        common::point_spot_count(&app),
        1,
        "dirty-channel rebuild without a Changed tick excluded the disabled light (2 -> 1)"
    );
    // Dirty-consume invariant (W2): the rebuild always consumes the bit.
    assert!(
        !app.world().resource::<LightTableDirty>().0,
        "LightTableDirty is consumed (false) after the rebuild — no stranded bit (W2)"
    );
}
