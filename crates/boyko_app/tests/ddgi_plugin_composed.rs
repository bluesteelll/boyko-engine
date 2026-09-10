//! SDFDDGI host-hook gate (a1): does the PRODUCTION host compose `DdgiPlugin` at all?
//!
//! # Why this gate exists
//!
//! Measured at HEAD `ed0bed45`: `boyko_app::plugins` composed `LightingPlugin`, `SsaoPlugin`,
//! `CsmPlugin`, `ShadowAtlasPlugin`, `RayPlugin`, ... and NOT `DdgiPlugin` — the plugin was
//! reachable only from its own unit tests. The production world therefore carried no
//! `ResolvedDdgi` and no `DdgiCaps` (the runner's boot `insert_resource(DdgiCaps)` landed in a
//! world where nothing read it), `resolve_ddgi_grid_gated` never ran, and every GI-enabling test
//! (`vb_both_ddgi_screenshot_dump`, `vb_lab`) got only the runner's ad-hoc read of a bare
//! `DdgiConfig`. Every rung I0..I7 of `docs/RENDER-SDFDDGI-PLAN.md` was green over that hole,
//! the same shape as `log_host_reachable.rs` ("fifteen green rungs passed over `ProfilerPlugin`
//! never being added").
//!
//! # What it asserts
//!
//! After `add_plugin(EnginePlugins::window(..))` the world holds the two resources ONLY
//! `DdgiPlugin::build` inserts: `ResolvedDdgi` (the single carrier the header gate, the b18
//! upload and the update arming all read) and `DdgiCaps` (the runner's boot override target).
//! Mutation: delete `app.add_plugin(DdgiPlugin)` from `EnginePlugins::build` — red.
//!
//! Own binary: `EnginePlugins` cannot be built twice in one process (the second build panics in
//! `register_component_hooks::<DirectionalLight>`, see `log_host_reachable.rs`).

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_render::{DdgiCaps, DdgiConfig, ResolvedDdgi};

#[test]
fn engine_plugins_composes_the_ddgi_plugin() {
    let mut app = App::new();
    app.add_plugin(EnginePlugins::window("ddgi composed", 320, 240));

    assert!(
        app.world().try_resource::<DdgiConfig>().is_some(),
        "DdgiPlugin::build inserts the owner-set DdgiConfig (default DISABLED)"
    );
    assert!(
        app.world().try_resource::<ResolvedDdgi>().is_some(),
        "DdgiPlugin::build inserts the ResolvedDdgi carrier -- without it the production world \
         has no single truth for the header bit, the b18 grid bytes and the update arming"
    );
    assert!(
        app.world().try_resource::<DdgiCaps>().is_some(),
        "DdgiPlugin::build inserts DdgiCaps -- the runner's boot `ddgi_storage_ok()` override \
         needs a plugin-composed reader, else it is a dead datum"
    );
    // The 0%-gate anchor for Decision 4 (unconditional composition): the default carrier is
    // the all-zero DISABLED image, so composing the plugin changes no byte on a GI-OFF world.
    assert_eq!(
        *app.world().resource::<ResolvedDdgi>(),
        ResolvedDdgi::DISABLED,
        "a default world composes to the DISABLED carrier (GI-OFF byte-identity)"
    );
}
