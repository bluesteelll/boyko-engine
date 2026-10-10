//! **G-RES (plan gate UG-20, record-only) and the id census (AL:M-A13 / MD:M-K3) for app (b)** —
//! the headless `EnginePlugins` + UI app of rung B2 (`b2_common::app_b`).
//!
//! One process, one reading of both records:
//!
//! * `ID-CENSUS app=b point=…` at M-K3's three points — the config phase (before `finish()`), right
//!   after `finish()`, after the first frame — and M-A13's steady state after 20 frames.
//! * `G-RES app=b point=after_boot …` after `finish()` and frame 0.
//!
//! Nothing here is pinned: both rows are recorded by hand into `docs/measurements/ug20-g-res.tsv`
//! and `docs/measurements/rk12-id-census.tsv` (tests never write into the tree). What IS asserted is
//! structural: the id scan agrees with a consuming mint, the counter is below every pinned slot, a
//! booted world has committed table bytes, and — because `UiPlugin` quietly spawns an EMPTY tree
//! when its path cannot be read — the UI fixture lowered all its nodes, so the (b) row can never be
//! a reading of an app without UI. See `b2_common/census_scan.rs` for what each figure is.

mod b2_common;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ui::components::{UiLayout, UiRoot};

use b2_common::census_scan::{g_res_line, id_cross_check, id_line};
use b2_common::{FRAME, MESHES, UI_FIXTURE, app_b, assert_fixture_parses_clean, canary, new_app};

/// The fixture's node count (`b2_common/app_b.ui`).
const FIXTURE_NODES: usize = 5;
/// M-A13's steady state.
const STEADY_FRAMES: usize = 20;

#[test]
fn g_res_id_census_app_b() {
    assert_fixture_parses_clean();
    // `BOYKO_B2_CANARY=g_res_no_ui` composes `UiPlugin` with no document: the red-first control of
    // the UI-node anti-vacuity below.
    let ui = if canary().as_deref() == Some("g_res_no_ui") {
        None
    } else {
        Some(UI_FIXTURE)
    };
    let mut app = new_app();
    app_b(&mut app, ui, MESHES);
    id_line("b", "config", 0);
    app.finish();
    id_line("b", "after_finish", 1);
    app.update_with_delta(FRAME);
    id_line("b", "after_frame_1", 2);

    let nodes = app
        .world()
        .query_entities(&[UiLayout::component_id()])
        .len();
    let roots = app.world().query_entities(&[UiRoot::component_id()]).len();
    println!("app b: UI nodes {nodes}, UiRoot {roots} after frame 0");
    assert!(
        nodes == FIXTURE_NODES && roots >= 1,
        "ANTI-VACUITY: app (b) holds {nodes} UI node(s) and {roots} UiRoot(s), the fixture lowers \
         {FIXTURE_NODES} — a mis-resolved fixture path spawns an empty tree without an error"
    );
    g_res_line("b", "after_boot", app.world());

    for _ in 1..STEADY_FRAMES {
        app.update_with_delta(FRAME);
    }
    let steady = id_line("b", "steady_20", 3);
    let minted = id_cross_check(&steady);
    println!(
        "ID-CENSUS app=b cross-check: the consuming probe minted slot {minted} = the scanned NEXT_ID"
    );
}
