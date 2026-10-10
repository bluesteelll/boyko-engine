//! **The shipped host orders the CSM fit after the light-pose reconcile.** A device-free gate on
//! the `EnginePlugins` line `b.configure_set(CsmResolveSet).after(LightReconcileSet)` in
//! `src/plugins.rs`.
//!
//! # Why this file exists (R4-frame-order)
//!
//! `resolve_csm_cascades` fits the cascades along the sun's `DirectionalLight::direction`, which
//! `light_reconcile` writes from the sun's `GlobalTransform`. A fit that runs first fits last
//! frame's sun. The two are registered by different plugins (`CsmPlugin`, `LightingPlugin`) and had
//! no ordering path before R4.
//!
//! # How it catches the line
//!
//! `finish()` rejects an ordering cycle with `boyko-B9001`. The test builds the shipped composition,
//! declares the REVERSE edge `LightReconcileSet.after(CsmResolveSet)`, and expects that cycle.
//! Nothing else orders the fit after `light_reconcile`, so without the line the reverse edge alone
//! is a legal order, `finish()` returns, and the test fails with "finish() accepted the reverse
//! edge".
//!
//! The cycle must name exactly the two systems, `light_reconcile` and `resolve_csm_cascades`
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
use boyko_render::{CsmResolveSet, LightReconcileSet};

use host_order_cycle::assert_finish_rejects_cycle;

/// RED: delete `b.configure_set(CsmResolveSet).after(LightReconcileSet);` from `src/plugins.rs`
/// and this test fails with "finish() accepted the reverse edge".
#[test]
fn the_shipped_host_orders_the_csm_fit_after_the_light_reconcile() {
    // `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run`
    // installs the windowed runner, which this never calls.
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("csm-reconcile-edge-gate", 64, 64));
    app.add_systems_cfg(|b| {
        b.configure_set(LightReconcileSet).after(CsmResolveSet);
    });
    assert_finish_rejects_cycle(
        &mut app,
        &[
            "boyko_render::light_reconcile::light_reconcile \
             [in: boyko_render::light_reconcile::LightReconcileSet]",
            "boyko_render::csm_config::resolve_csm_cascades \
             [in: boyko_render::csm_config::CsmResolveSet]",
        ],
    );
}
