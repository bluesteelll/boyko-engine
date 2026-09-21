//! **The shipped host orders the punctual shadow resolve after the camera resolve.** A device-free
//! gate on the `EnginePlugins` line `b.configure_set(PunctualResolveSet).after(CameraSet::Resolve)`
//! in `src/plugins.rs`.
//!
//! # Why this file exists (R4-frame-order)
//!
//! `resolve_shadow_atlas` ranks the punctual lights by `range² / dist²` to
//! `ViewUniform::camera_pos`, which `resolve_active_camera` (a `CameraSet::Resolve` member) writes.
//! A resolve that runs first ranks against last frame's camera, so on a moving camera the atlas can
//! hand base 0 to the previous frame's winner. Before R4 nothing ordered the pair: in the golden
//! host the resolve ran before the camera on both feature legs.
//!
//! # How it catches a broken order
//!
//! `finish()` rejects an ordering cycle with `boyko-B9001`. The test builds the shipped composition,
//! declares the REVERSE edge `CameraSet::Resolve.after(PunctualResolveSet)`, and expects that cycle.
//! When no path orders the resolve after a `CameraSet::Resolve` member, the reverse edge alone is a
//! legal order, `finish()` returns, and the test fails with "finish() accepted the reverse edge".
//!
//! The cycle must name exactly `resolve_shadow_atlas`, the two `CameraSet::Resolve` members and
//! `light_reconcile` (compared as a set — see `host_order_cycle/mod.rs`). It is the same on both
//! feature legs.
//!
//! # What this gate can and cannot see — the line under test is NOT isolated (since R4b)
//!
//! R4b-open-edges added `LightReconcileSet.after(CameraSet::Resolve)` (the light pose is derived
//! from the propagated transform) and `PunctualResolveSet.after(LightReconcileSet)` (the ranking
//! reads that pose). Together they are a second path from the camera to the resolve, which is why
//! `light_reconcile` is in the cycle, and why deleting the line under test ALONE leaves this test
//! green — MEASURED — with no test on the order able to tell the difference, because the order is
//! the same. The line is kept as the resolve's own declaration of its `ViewUniform` dependency, so
//! the order survives a change to that unrelated chain. This test goes red when the order itself
//! breaks: deleting this line AND either `LightReconcileSet` line makes the reverse edge a legal
//! order — measured for both choices. Those two lines have gates of their own
//! (`tests/host_orders_light_reconcile_after_propagation.rs`,
//! `tests/host_orders_shadow_atlas_after_light_reconcile.rs`).
//!
//! # Why its own binary
//!
//! `EnginePlugins` can be built only once per process (see `particle_host_reachable.rs`), so this
//! file holds one `#[test]`. It needs no device: `finish()` builds the schedules and runs the
//! startup systems, and no frame runs.

mod host_order_cycle;

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_render::PunctualResolveSet;
use boyko_scene::CameraSet;

use host_order_cycle::assert_finish_rejects_cycle;

/// RED: delete `b.configure_set(PunctualResolveSet).after(CameraSet::Resolve);` AND either
/// `b.configure_set(LightReconcileSet).after(CameraSet::Resolve);` or
/// `b.configure_set(PunctualResolveSet).after(LightReconcileSet);` from `src/plugins.rs` and this
/// test fails with "finish() accepted the reverse edge". The line alone keeps the order through
/// the reconcile, so deleting it alone keeps this green (see the module doc).
#[test]
fn the_shipped_host_orders_the_punctual_resolve_after_the_camera_resolve() {
    // `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run`
    // installs the windowed runner, which this never calls.
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("atlas-camera-edge-gate", 64, 64));
    app.add_systems_cfg(|b| {
        b.configure_set(CameraSet::Resolve).after(PunctualResolveSet);
    });
    assert_finish_rejects_cycle(
        &mut app,
        &[
            "boyko_scene::propagation::propagate_transforms [in: CameraSet::Resolve]",
            "boyko_scene::camera::resolve_active_camera [in: CameraSet::Resolve]",
            "boyko_render::light_reconcile::light_reconcile \
             [in: boyko_render::light_reconcile::LightReconcileSet]",
            "boyko_render::shadow_atlas::resolve_shadow_atlas \
             [in: boyko_render::shadow_atlas::PunctualResolveSet]",
        ],
    );
}
