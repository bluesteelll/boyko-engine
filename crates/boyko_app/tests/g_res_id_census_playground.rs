//! **G-RES (plan gate UG-20, record-only) and the id census (AL:M-A13 / MD:M-K3) for the
//! playground COMPOSITION** — `EnginePlugins` + `FlyCameraPlugin` + `PhysicsPlugin::new()`, the
//! plugin set of `examples/playground.rs`'s `main`.
//!
//! **Composition, not scene.** The playground's scene is built by `setup`, a private fn of an
//! example binary, so it cannot be driven headless from here. This process composes the same three
//! plugins, adds rung B2's device-free residents and seven device-inert meshes (the runner's two
//! world residents, plus the `AppExit` it inserts, without which a frame panics), and records that — labelled
//! `app=playground_composition`.
//!
//! * `ID-CENSUS app=playground_composition point=…` at M-K3's three points (config, after
//!   `finish()`, after the first frame) and M-A13's steady state after 20 frames.
//! * `G-RES app=playground_composition point=after_boot …` after `finish()` and frame 0.
//!
//! **Structural RED: the physics pinned band is present at the config phase.** `boyko_physics` is
//! the only production `register_layout` caller, and `PhysicsPlugin::build` inserts the resources
//! whose constructors pin its scratch ids — so from the config point on (every point this process
//! reads) the scan must see pinned slots. That is the pinned scan's anti-vacuity: a scan that
//! cannot see physics' band would report every composition as band-free.

mod b2_common;

use boyko_app::FlyCameraPlugin;
use boyko_ecs::AppExit;
use boyko_physics::PhysicsPlugin;

use b2_common::census_scan::{IdScan, g_res_line, id_cross_check, id_line};
use b2_common::{FRAME, MESHES, app_a, new_app};

/// M-A13's steady state.
const STEADY_FRAMES: usize = 20;
const APP: &str = "playground_composition";

fn require_band(scan: &IdScan, point: &str) {
    assert!(
        scan.pinned > 0,
        "ID-CENSUS ANTI-VACUITY: app={APP} point={point} sees no pinned slot — PhysicsPlugin is \
         composed, and its scratch band must be visible from the config phase on"
    );
}

#[test]
fn g_res_id_census_playground() {
    let mut app = new_app();
    // `app_a` is `EnginePlugins` + the device-free residents + the meshes; the playground adds its
    // two plugins after `EnginePlugins`, as its `main` does.
    app_a(&mut app, MESHES);
    app.add_plugin(FlyCameraPlugin);
    app.add_plugin(PhysicsPlugin::new());
    // The third runner resident: `run_windowed` inserts `AppExit(false)` if absent before
    // `finish()`, and `FlyCameraPlugin`'s `quit_on_action` writes it every frame.
    app.insert_resource(AppExit(false));

    let s = id_line(APP, "config", 0);
    require_band(&s, "config");
    app.finish();
    let s = id_line(APP, "after_finish", 1);
    require_band(&s, "after_finish");
    app.update_with_delta(FRAME);
    let s = id_line(APP, "after_frame_1", 2);
    require_band(&s, "after_frame_1");
    g_res_line(APP, "after_boot", app.world());

    for _ in 1..STEADY_FRAMES {
        app.update_with_delta(FRAME);
    }
    let steady = id_line(APP, "steady_20", 3);
    require_band(&steady, "steady_20");
    let minted = id_cross_check(&steady);
    println!(
        "ID-CENSUS app={APP} cross-check: the consuming probe minted slot {minted} = the scanned NEXT_ID"
    );
}
