//! Render wave PX1a test 7b, the lighting half (`plan/H4-PX1a-hdr-lit.md` Tests 7b, rev 3 arch W7;
//! `px1a/cut.md` D-E2, C2): **a world with `LightingPlugin` but no `PostPlugin` panics at its first
//! update naming `ResolvedPost`** — `LightingPlugin` inserts no silent default, which would stop a
//! lighting-only app's exposure from reaching the GPU (the defect PX1a fixes).
//!
//! SINGLE-TEST BINARY: `LightingPlugin::build` installs process-global light hooks, so a second
//! build in one process panics for a reason that is not this test's (critique O4).

use boyko_ecs::ecs::core::app::App;
use boyko_render::light::LightingConfig;
use boyko_render::light_plugin::LightingPlugin;
use boyko_render::light_system::LightTableStaging;

#[test]
#[should_panic(expected = "ResolvedPost")]
fn a_lighting_world_without_post_plugin_panics_naming_resolved_post() {
    let mut app = App::new();
    app.insert_resource(LightTableStaging::default());
    app.insert_resource(LightingConfig::default());
    app.add_plugins(LightingPlugin);
    app.finish();
    app.update();
}
