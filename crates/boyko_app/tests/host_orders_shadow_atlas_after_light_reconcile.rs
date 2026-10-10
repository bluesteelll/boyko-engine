//! **The shipped host orders the punctual shadow resolve after the light-pose reconcile.** A
//! device-free gate on the `EnginePlugins` line
//! `b.configure_set(PunctualResolveSet).after(LightReconcileSet)` in `src/plugins.rs`.
//!
//! # Why this file exists (R4b-open-edges)
//!
//! `resolve_shadow_atlas` ranks each spot and point light by `range² / dist²` from its `position`
//! (`boyko_render/src/shadow_atlas.rs`, `spot_priority`) and fits the winners from their
//! `position` / `direction` (`spot_input_from`), and `light_reconcile` writes those fields from
//! the light's `GlobalTransform` (`boyko_render/src/light_reconcile.rs`). A resolve that runs
//! first ranks a moving light from last frame's pose. The two are registered by different
//! plugins (`ShadowAtlasPlugin`, `LightingPlugin`) and had no ordering path before this edge:
//! `ShadowAtlasPlugin`'s doc recorded the pair as undeclared.
//!
//! # How it catches the line
//!
//! `finish()` rejects an ordering cycle with `boyko-B9001`. The test builds the shipped composition,
//! declares the REVERSE edge `LightReconcileSet.after(PunctualResolveSet)`, and expects that cycle.
//! Nothing else orders the resolve after `light_reconcile` (`collect_lights` is after both, and
//! is on no path between them), so without the line the reverse edge alone is a legal order,
//! `finish()` returns, and the test fails with "finish() accepted the reverse edge".
//!
//! The cycle must name exactly the two systems, `light_reconcile` and `resolve_shadow_atlas`
//! (compared as a set — see `host_order_cycle/mod.rs`), so the panic can only be this edge's. It is
//! the same on both feature legs.
//!
//! # Why its own binary
//!
//! `EnginePlugins` can be built only once per process (see `particle_host_reachable.rs`), so this
//! file holds one `#[test]`. It needs no device: `finish()` builds the schedules and runs the
//! startup systems, and no frame runs.

mod host_order_cycle;

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_render::{LightReconcileSet, PunctualResolveSet};

use host_order_cycle::assert_finish_rejects_cycle;

/// RED: delete `b.configure_set(PunctualResolveSet).after(LightReconcileSet);` from
/// `src/plugins.rs` and this test fails with "finish() accepted the reverse edge".
#[test]
fn the_shipped_host_orders_the_punctual_resolve_after_the_light_reconcile() {
    // `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run`
    // installs the windowed runner, which this never calls.
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("atlas-reconcile-edge-gate", 64, 64));
    app.add_systems_cfg(|b| {
        b.configure_set(LightReconcileSet).after(PunctualResolveSet);
    });
    assert_finish_rejects_cycle(
        &mut app,
        &[
            "boyko_render::light_reconcile::light_reconcile \
             [in: boyko_render::light_reconcile::LightReconcileSet]",
            "boyko_render::shadow_atlas::resolve_shadow_atlas \
             [in: boyko_render::shadow_atlas::PunctualResolveSet]",
        ],
    );
}
