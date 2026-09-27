//! SDFDDGI host-hook gate (a2): is `sync_ddgi_light_gate` registered in the PRODUCTION `Main`
//! registration, with the edges that make the header bit land in the SAME frame?
//!
//! # Why this is an integration test and not a `#[cfg(test)] mod` in `src/plugins.rs`
//!
//! It was one, and two of the source-walking gates rejected it on the merged tree, both by
//! design: `tests/host_ecs_entry_points_are_guarded.rs` (P10) walks every `*.rs` under this
//! crate's `src/` as TEXT and forbids `app.update()` anywhere in it — it strips comments, not
//! `cfg(test)`, because a text census may only ever produce a false red — and the root
//! `tests/production_reachability_census.rs`'s sensitivity controls need `add_plugin(CsmPlugin)`
//! to occur EXACTLY ONCE in `plugins.rs`'s raw text, which a second composition in the same file
//! breaks (a control that injects nothing passes). A composition that runs frames therefore
//! belongs here, and `register_main_frame_systems` is `pub` for this file's sake.
//!
//! # Why the production registration is run in a subset world
//!
//! `app.update()` on a bare `EnginePlugins` panics: the runner, not `build`, inserts the world
//! residents (`tests/log_host_shipping_min.rs`). The ECS exposes no system-name introspection
//! either (`SystemBox.name` is `pub(crate)`). So the only device-free way to prove "the gate
//! is in the production closure" is to run THAT closure — `register_main_frame_systems`,
//! the verbatim code motion — in a world composed of the same render plugins minus the
//! window/runner (the `tests/camera_resolve.rs` subset precedent). Residents the frame
//! systems need and the runner normally inserts: `LightTableStaging`, `LightingConfig`,
//! `ClusterConfig` (seeded by `EnginePlugins::build` itself), `MeshRenderScratch`,
//! `CsmCasterScratch`, the NonSend `Assets<MeshGpu>` (`gather_shadow_casters`,
//! `gather_mesh_draws`) and `Assets<Material>` (`gather_mesh_draws`); on the `hwrt` leg
//! `gather_mesh_draws` also reads `ShadowDenoiseConfig`, so `ShadowDenoisePlugin` is composed
//! as `EnginePlugins::build` composes it (unconditionally, default `None` — inert).
//!
//! # What each assertion catches (the mutations)
//!
//! * Remove the `sync_ddgi_light_gate` registration line: `LightingConfig::ddgi_indirect`
//!   never becomes `true` in phase 2 — the bit is never set at all. This is the mutation
//!   the test exists for (the D2 defect: the gate was registered by no host).
//! * Drop the R9c freeze fold from `resolve_ddgi_grid_gated`: phase 1 goes red (the carrier
//!   resolves ENABLED under a frozen-OFF non-Deferred boot).
//! * The staging-header assertion (word 7 bit 4 of `LightTableStaging` after ONE update)
//!   catches a bit that reaches `LightingConfig` but not the GPU header in the same frame —
//!   a whole-frame miss, NOT the `.before_set(LightCollectSet)` edge specifically.
//!
//! # What this test does NOT gate — the two ordering edges (MEASURED, and it was overclaimed)
//!
//! An earlier revision of this doc claimed that removing `.before_set(LightCollectSet)` makes
//! the staging assertion "fail whenever `collect_lights` happened to run first, i.e.
//! nondeterministically", and that removing `.after_set(DdgiResolveSet)` makes the same-frame
//! assertions fail the same way. Both claims are FALSE, and the appeal to nondeterminism is
//! false twice over:
//!
//! * The kernel scheduler is DETERMINISTIC. `kahn_topological_sort`
//!   (`boyko_ecs::…::schedule_builder`) pops a FIFO ready queue, so unordered systems run in
//!   `add_system` insertion order — "stable for fixed input", its own doc. There is no dice
//!   roll for a flaky assertion to catch.
//! * Measured on this tree: with `.before_set(LightCollectSet)` removed the test passes 3/3,
//!   and with `.after_set(DdgiResolveSet)` removed (the other edge kept) it passes 3/3. Each
//!   required order is ALREADY implied by insertion order plus the surrounding plugins' own
//!   edges, so the explicit edge changes no run order in THIS composition and no assertion
//!   here can observe its absence.
//!
//! Both edges stay, and are not decoration — they are the contract that survives a future
//! reordering (a plugin added earlier, an edge dropped from a neighbour), which is exactly
//! what "already implied by the surrounding graph" is vulnerable to; the
//! `sync_cluster_light_gate` / `sync_sv0_light_gate` precedent carries the same edge for the
//! same reason. But a reader must not count them as GATED: proving an ordering edge needs a
//! run-order probe the ECS does not expose today (`SystemBox.name` is `pub(crate)`, and a
//! probe system registered from the test inherits the same tie-break, so it observes the
//! implied order rather than the edge).
//!
//! # ONE test, three phases — not three tests, and its own binary
//!
//! `LightingPlugin` installs `DirectionalLight`'s component hooks in a PROCESS-GLOBAL
//! registry, so a second `App` that composes it panics in `register_component_hooks`
//! (measured: two `#[test]`s here, the second one always red, and WHICH one is second is
//! thread-scheduling nondeterministic). The three phases therefore share one `App` and one
//! composition — which also makes them a stronger statement: the same world walks
//! frozen-clamped ⇒ enabled ⇒ disabled and the header follows in-frame each time. The same
//! registry is why this file holds exactly one `#[test]` and shares its binary with nothing
//! (the `ddgi_plugin_composed.rs` / `log_host_reachable.rs` rule).

use boyko_app::plugins::register_main_frame_systems;
use boyko_ecs::App;
use boyko_ecs::ecs::core::asset::Assets;
use boyko_render::light::DDGI_MODE_BIT;
use boyko_render::light_system::LightTableStaging;
use boyko_render::{
    ClusterConfig, CsmCasterScratch, CsmPlugin, DdgiConfig, DdgiPlugin, LightingConfig,
    LightingPlugin, Material, MaterialUploadStaging, MeshGpu, MeshRenderScratch, RenderPathFrozenConsumers,
    RenderPathPlugin, ResolvedDdgi, ShadowAtlasPlugin, ShadowDenoisePlugin, SsaoConfig,
    SsaoPlugin,
};
use boyko_scene::CameraPlugin;

/// Light-header word 7 (`sky_diffuse.w`, bytes 28..32 LE) of the staged table — the word
/// the resolve's `load_ddgi_mode` reads bit 4 from.
fn header_word7(app: &App) -> u32 {
    let bytes = app.world().resource::<LightTableStaging>().bytes();
    assert!(bytes.len() >= 32, "the staging holds at least the light header");
    u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]])
}

/// The production render-plugin subset + the runner-inserted residents, with the
/// PRODUCTION `Main` registration.
fn production_subset_app() -> App {
    let mut app = App::new();
    app.insert_resource(LightTableStaging::default());
    app.insert_resource(LightingConfig::default());
    app.insert_resource(ClusterConfig::default());
    app.add_plugin(LightingPlugin);
    app.add_plugin(SsaoPlugin);
    app.add_plugin(CsmPlugin);
    app.add_plugin(ShadowAtlasPlugin);
    // Composed unconditionally by `EnginePlugins::build` too; on the `hwrt` leg
    // `gather_mesh_draws` reads `Res<ShadowDenoiseConfig>`, which only this plugin inserts.
    // Without it the first `app.update()` panics on that leg (measured while moving the test
    // out of `src/`: the `#[cfg(test)]` original was never run under `--features hwrt`).
    app.add_plugin(ShadowDenoisePlugin);
    app.add_plugin(DdgiPlugin);
    app.add_plugin(RenderPathPlugin);
    app.add_plugin(CameraPlugin);
    app.insert_resource(MeshRenderScratch::default());
    app.insert_resource(CsmCasterScratch::default());
    // `EnginePlugins::build` inserts it beside the two scratches above; the production
    // registration's `stage_material_edits` (DM1) reads it every frame.
    app.insert_resource(MaterialUploadStaging::default());
    app.insert_resource(Assets::<Material>::default());
    app.world_mut().insert_non_send_resource(Assets::<MeshGpu>::default());
    app.add_systems_cfg(register_main_frame_systems);
    app
}

#[test]
fn production_main_registration_packs_the_ddgi_header_bit_in_the_same_frame() {
    let mut app = production_subset_app();
    // Enable AFTER composition (the owner's contract), replacing the plugin's DISABLED seed.
    app.insert_resource(DdgiConfig { ddgi_indirect: true, ..DdgiConfig::default() });

    // ---- phase 1: the rung-R9c freeze clamp still holds through the single writer -------
    // A frozen-OFF, non-Deferred boot keeps the carrier DISABLED even with the config ON,
    // so the header bit stays 0 — the frozen-consumers gate (c) at the PRODUCTION seam.
    app.insert_resource(RenderPathFrozenConsumers::new(SsaoConfig::default(), false, true));
    app.update();
    assert_eq!(
        *app.world().resource::<ResolvedDdgi>(),
        ResolvedDdgi::DISABLED,
        "the single writer folds the freeze: frozen OFF => DISABLED carrier"
    );
    assert!(
        !app.world().resource::<LightingConfig>().ddgi_indirect,
        "frozen OFF => the header bit stays 0 with the config ON"
    );
    assert_eq!((header_word7(&app) >> DDGI_MODE_BIT) & 1, 0, "staged header: bit 4 clear");

    // ---- phase 2: an inert freeze (the Deferred path) => the bit lands the SAME frame ---
    app.insert_resource(RenderPathFrozenConsumers::default());
    app.update();
    assert_ne!(
        app.world().resource::<ResolvedDdgi>().ddgi_mode_word,
        0,
        "an inert freeze lets the live ENABLED config through to the carrier"
    );
    let cfg = *app.world().resource::<LightingConfig>();
    assert!(
        cfg.ddgi_indirect,
        "the registered sync_ddgi_light_gate read THIS frame's ENABLED carrier"
    );
    assert_eq!((cfg.shadow_gate_word() >> DDGI_MODE_BIT) & 1, 1, "word-7 bit 4 from the config");
    assert_eq!(
        (header_word7(&app) >> DDGI_MODE_BIT) & 1,
        1,
        "staged header bit 4 after ONE update: needs after_set(DdgiResolveSet)+before_set(LightCollectSet)"
    );

    // ---- phase 3: the DISABLE drops the bit in the same frame it flips ------------------
    app.insert_resource(DdgiConfig::default());
    app.update();
    assert_eq!(*app.world().resource::<ResolvedDdgi>(), ResolvedDdgi::DISABLED);
    assert!(
        !app.world().resource::<LightingConfig>().ddgi_indirect,
        "a DISABLED carrier drops the bit in the same frame"
    );
    assert_eq!((header_word7(&app) >> DDGI_MODE_BIT) & 1, 0, "the staged header dropped bit 4");
}
