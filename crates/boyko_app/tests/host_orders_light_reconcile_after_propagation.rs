//! **The shipped host orders the light-pose reconcile after transform propagation.** A
//! device-free gate on the `EnginePlugins` line
//! `b.configure_set(LightReconcileSet).after(CameraSet::Resolve)` in `src/plugins.rs`.
//!
//! # Why this file exists (R4b-open-edges)
//!
//! `light_reconcile` derives each light's `direction` / `position` from its `GlobalTransform`
//! (`boyko_render/src/light_reconcile.rs`: the three `&GlobalTransform` queries in the system's
//! signature, read through `to_light_dir(g)` and `g.translation()` in its three loops), and
//! `propagate_transforms` (a `CameraSet::Resolve` member) writes it. A reconcile that runs first
//! derives last frame's pose. `LightingPlugin`'s doc had left the pair to add-order, with the
//! `Changed<GlobalTransform>` gate as the argument that a wrong order self-corrects — it does not
//! on the frame it matters: measured on the shipped host, the frame-0 light table carried the
//! identity pose's direction for a sun spawned with a posed `Transform` and an identity
//! `GlobalTransform` (`docs/OPEN-QUESTIONS.md`, 2026-09-19 (b)). Whether the wrong order held on
//! every frame was not measured; wherever it does, a moving light's pose trails its transform by
//! one frame for as long as it moves, and the `Changed` gate re-fires each frame without ever
//! catching up.
//!
//! # How it catches the line
//!
//! `finish()` rejects an ordering cycle with `boyko-B9001`. The test builds the shipped composition,
//! declares the REVERSE edge `CameraSet::Resolve.after(LightReconcileSet)`, and expects that cycle.
//! Nothing else orders the reconcile after either `CameraSet::Resolve` member (the reconcile's
//! other edges all point forward, to the light-table fold, the CSM fit and the punctual
//! resolve, and none of those reaches propagation or the camera), so without the line the reverse
//! edge alone is a legal order, `finish()` returns, and the test fails with "finish() accepted
//! the reverse edge".
//!
//! The cycle must name exactly `light_reconcile` and the two `CameraSet::Resolve` members
//! (compared as a set — see `host_order_cycle/mod.rs`), so the panic can only be this edge's. It
//! is the same on both feature legs.
//!
//! # Why its own binary
//!
//! `EnginePlugins` can be built only once per process (see `particle_host_reachable.rs`), so this
//! file holds one `#[test]`. It needs no device: `finish()` builds the schedules and runs the
//! startup systems, and no frame runs.

mod host_order_cycle;

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_render::LightReconcileSet;
use boyko_scene::CameraSet;

use host_order_cycle::assert_finish_rejects_cycle;

/// RED: delete `b.configure_set(LightReconcileSet).after(CameraSet::Resolve);` from
/// `src/plugins.rs` and this test fails with "finish() accepted the reverse edge".
#[test]
fn the_shipped_host_orders_the_light_reconcile_after_transform_propagation() {
    // `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run`
    // installs the windowed runner, which this never calls.
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("reconcile-propagation-edge-gate", 64, 64));
    app.add_systems_cfg(|b| {
        b.configure_set(CameraSet::Resolve).after(LightReconcileSet);
    });
    assert_finish_rejects_cycle(
        &mut app,
        &[
            "boyko_scene::propagation::propagate_transforms [in: CameraSet::Resolve]",
            "boyko_scene::camera::resolve_active_camera [in: CameraSet::Resolve]",
            "boyko_render::light_reconcile::light_reconcile \
             [in: boyko_render::light_reconcile::LightReconcileSet]",
        ],
    );
}
