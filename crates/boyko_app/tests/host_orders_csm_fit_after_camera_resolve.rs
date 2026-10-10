//! **The shipped host orders the CSM fit after the camera resolve.** A device-free gate on the
//! ORDER that the `EnginePlugins` line `b.configure_set(CsmResolveSet).after(CameraSet::Resolve)`
//! in `src/plugins.rs` declares.
//!
//! # Why this file exists (R4-frame-order)
//!
//! `resolve_csm_cascades` fits the cascades to `ViewUniform`, which `resolve_active_camera` (a
//! `CameraSet::Resolve` member) writes. A fit that runs first fits last frame's view. Before R4
//! the two had no ordering path, and the fit ran after the camera in the golden host only because
//! it happened to land in a later wave.
//!
//! # How it catches a broken order
//!
//! `finish()` rejects an ordering cycle with `boyko-B9001`. The test builds the shipped composition,
//! declares the REVERSE edge `CameraSet::Resolve.after(CsmResolveSet)`, and checks the cycle's
//! members (as a set — see `host_order_cycle/mod.rs`). `resolve_active_camera` is in the cycle
//! exactly when some path orders the fit after it.
//!
//! # What this gate can and cannot see — the line under test is NOT isolated
//!
//! In the shipped graph the line is one of THREE paths from `resolve_active_camera` to the fit.
//! The second is `CameraSet::Resolve → InstancePackSet` (the pack edge, whose `CameraSet::Resolve`
//! also holds the camera) `→ sync_instance_model_cols → gather_shadow_casters →
//! reduce_caster_bounds` (`CsmFitSet`) `→ CsmResolveSet`. The third, since R4b-open-edges, is
//! `CameraSet::Resolve → LightReconcileSet` (the reconcile-after-propagation edge) `→
//! CsmResolveSet` (the fit-after-reconcile edge). So deleting the line ALONE leaves this test
//! green — MEASURED — and no test on the order can tell the difference, because the order is the
//! same. The line is kept as the declaration of the fit's own data dependency, so the order
//! survives a change to either unrelated chain (the pack edge narrowed to propagation, a fit that
//! stops reading caster bounds, or a reconcile that no longer needs propagation). This test goes
//! red when the order itself breaks: deleting this line, the `InstancePackSet` line AND one of the
//! two `LightReconcileSet` lines drops `resolve_active_camera` from the cycle — measured for both
//! choices. Deleting this line and the `InstancePackSet` line alone keeps it green — also measured
//! (before R4b that pair was red; the reconcile path is what changed).
//!
//! The `hwrt` leg's cycle also holds `sync_prev_instance_model_cols`.
//!
//! # Why its own binary
//!
//! `EnginePlugins` can be built only once per process (see `particle_host_reachable.rs`), so this
//! file holds one `#[test]`. It needs no device: `finish()` builds the schedules and runs the
//! startup systems, and no frame runs.

mod host_order_cycle;

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_render::CsmResolveSet;
use boyko_scene::CameraSet;

use host_order_cycle::assert_finish_rejects_cycle;

/// The cycle's members on both feature legs.
const CYCLE: &[&str] = &[
    "boyko_scene::propagation::propagate_transforms [in: CameraSet::Resolve]",
    "boyko_scene::camera::resolve_active_camera [in: CameraSet::Resolve]",
    "boyko_scene::visibility_sync::visibility_sync [in: VisibilitySet::Sync]",
    "boyko_render::asset_refcount::validate_asset_refs \
     [in: VisibilitySet::Validate, boyko_render::asset_refcount::AssetValidateSet]",
    "boyko_render::instance_model::sync_instance_model_cols \
     [in: VisibilitySet::Read, boyko_render::instance_model::InstancePackSet]",
    "boyko_render::csm_caster::gather_shadow_casters [in: VisibilitySet::Read]",
    "boyko_render::csm_caster::reduce_caster_bounds [in: boyko_render::csm_caster::CsmFitSet]",
    "boyko_render::light_reconcile::light_reconcile \
     [in: boyko_render::light_reconcile::LightReconcileSet]",
    "boyko_render::csm_config::resolve_csm_cascades [in: boyko_render::csm_config::CsmResolveSet]",
];

/// The member only the `hwrt` leg registers.
#[cfg(feature = "hwrt")]
const HWRT_EXTRA: &[&str] =
    &["boyko_render::instance_model::sync_prev_instance_model_cols [in: VisibilitySet::Read]"];
/// The member only the `hwrt` leg registers (none here).
#[cfg(not(feature = "hwrt"))]
const HWRT_EXTRA: &[&str] = &[];

/// RED: delete `b.configure_set(CsmResolveSet).after(CameraSet::Resolve);`,
/// `b.configure_set(InstancePackSet).after(CameraSet::Resolve);` AND either
/// `b.configure_set(LightReconcileSet).after(CameraSet::Resolve);` or
/// `b.configure_set(CsmResolveSet).after(LightReconcileSet);` from `src/plugins.rs` and the member
/// check fails (`resolve_active_camera` leaves the cycle). Any one of the three paths alone keeps
/// the order, so deleting fewer lines keeps this green (see the module doc).
#[test]
fn the_shipped_host_orders_the_csm_fit_after_the_camera_resolve() {
    // `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run`
    // installs the windowed runner, which this never calls.
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("csm-camera-edge-gate", 64, 64));
    app.add_systems_cfg(|b| {
        b.configure_set(CameraSet::Resolve).after(CsmResolveSet);
    });
    assert_finish_rejects_cycle(&mut app, &[CYCLE, HWRT_EXTRA].concat());
}
