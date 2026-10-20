//! Render wave PX1a — the order probe of the pre-exposure's readers (`plan/H4-PX1a-hdr-lit.md`
//! Tests 7b, rule (xx); `px1a/cut.md` D-E2, C2; ruling 67 OQ1): **`collect_lights` and
//! `resolve_taa_policy` read THIS frame's `ResolvedPost`.**
//!
//! Their `.after_set(PostPolicySet)` edges are declared inside `LightingPlugin` / `AaPlugin`
//! (set-to-set, expanded at build — the accepted equivalence of rule (xx)'s "in the composing app").
//! The plugins are added in the ADVERSE order (the readers before the writer) and the readers
//! conflict with the writer on `ResolvedPost`, so without the edges the scheduler would run them
//! first and they would read last frame's value: the staged header and the TAA ratio would lag by
//! one frame.
//!
//! SINGLE-TEST BINARY: `LightingPlugin::build` installs process-global light hooks (critique O4).

use boyko_ecs::ecs::core::app::App;
use boyko_render::AaPlugin;
use boyko_render::aa_config::ResolvedTaa;
use boyko_render::light::LightingConfig;
use boyko_render::light_plugin::LightingPlugin;
use boyko_render::light_system::LightTableStaging;
use boyko_render::post::{PostPlugin, ResolvedPost};
use boyko_scene::CameraPlugin;

fn post(app: &App) -> ResolvedPost {
    *app.world().resource::<ResolvedPost>()
}

/// The readers' plugins FIRST, the writer's last: without the set edges the conflicting
/// `ResolvedPost` access would run the readers before the writer.
fn adverse_order_app() -> App {
    let mut app = App::new();
    app.insert_resource(LightTableStaging::default());
    app.insert_resource(LightingConfig::default());
    app.add_plugins(LightingPlugin);
    app.add_plugins(AaPlugin);
    app.add_plugins(PostPlugin);
    app.add_plugins(CameraPlugin);
    app.finish();
    app
}

#[test]
fn collect_lights_and_resolve_taa_policy_read_this_frames_resolved_post() {
    let mut app = adverse_order_app();
    app.update();
    for (k, exposure) in [2.0f32, 0.5, 8.0, 1.0].into_iter().enumerate() {
        let before = post(&app).pre_exposure;
        app.world_mut().resource_mut::<LightingConfig>().exposure = exposure;
        app.update();
        assert_eq!(post(&app).pre_exposure, exposure, "frame {k}: resolve_post_policy never ran");
        let staged = app.world().resource::<LightTableStaging>().bytes();
        let word1 = f32::from_le_bytes(staged[4..8].try_into().expect("invariant: the header's word 1"));
        assert_eq!(word1, exposure, "frame {k}: collect_lights packed last frame's pre-exposure");
        // `ResolvedTaa`'s offset-12 word (named `pre_ratio_delta` at the switch commit).
        let delta = app.world().resource::<ResolvedTaa>().pre_ratio_delta;
        assert_eq!(delta, exposure / before - 1.0, "frame {k}: resolve_taa_policy read last frame's ratio");
    }
}
