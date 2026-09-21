//! **The shipped host orders the instance packs after transform propagation.** A device-free gate
//! on the `EnginePlugins` line `b.configure_set(InstancePackSet).after(CameraSet::Resolve)` in
//! `src/plugins.rs`.
//!
//! # Why this file exists (R4-frame-order)
//!
//! `sync_instance_model_cols` and `sync_gpu_3d_instances` copy `GlobalTransform` into their
//! instance columns, and `propagate_transforms` (a `CameraSet::Resolve` member) writes it. The packs
//! are unconditional, so a pack that runs before propagation leaves every moving instance one frame
//! behind its transform, permanently. Before R4 nothing ordered them after it: in the golden host
//! both packs ran before propagation, on both feature legs.
//!
//! # How it catches the line
//!
//! `finish()` rejects an ordering cycle with `boyko-B9001`. The test builds the shipped composition,
//! declares the REVERSE edge `CameraSet::Resolve.after(InstancePackSet)`, and checks the cycle's
//! members (as a set — see `host_order_cycle/mod.rs`).
//!
//! # What the member list does and does not prove
//!
//! The packs are ALSO after `propagate_transforms` through a second path:
//! `propagate_transforms → visibility_sync` (`CameraPlugin`) `→ VisibilitySet::Read` (the packs are
//! members). So the reverse edge closes a cycle through `propagate_transforms` with or without the
//! line under test, and "some cycle" would prove nothing about it. What only the line contributes
//! is the packs' order after the OTHER `CameraSet::Resolve` member, `resolve_active_camera`: with
//! the line, `resolve_active_camera` is in the cycle; without it, it is not, and the member check
//! fails. That is what turns this red when the line is deleted. The propagation half of the line's
//! meaning is redundant with that second path today and cannot be isolated by any test on the
//! order; it is declared anyway, because the `visibility_sync → propagation` edge carries no data
//! and is not this dependency's declaration.
//!
//! Since R4b-open-edges that second path also runs through the asset-ref validation
//! (`visibility_sync → VisibilitySet::Validate → VisibilitySet::Read`), so `validate_asset_refs`
//! is a member of the cycle on both legs. The `hwrt` leg's cycle also holds
//! `sync_prev_instance_model_cols`, which runs after `visibility_sync` and before the pack.
//!
//! # Why its own binary
//!
//! `EnginePlugins` can be built only once per process (see `particle_host_reachable.rs`), so this
//! file holds one `#[test]`. It needs no device: `finish()` builds the schedules and runs the
//! startup systems, and no frame runs.

mod host_order_cycle;

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_render::InstancePackSet;
use boyko_scene::CameraSet;

use host_order_cycle::assert_finish_rejects_cycle;

/// The cycle's members on both feature legs.
const CYCLE: &[&str] = &[
    "boyko_scene::propagation::propagate_transforms [in: CameraSet::Resolve]",
    "boyko_scene::camera::resolve_active_camera [in: CameraSet::Resolve]",
    "boyko_scene::visibility_sync::visibility_sync [in: VisibilitySet::Sync]",
    "boyko_render::asset_refcount::validate_asset_refs \
     [in: VisibilitySet::Validate, boyko_render::asset_refcount::AssetValidateSet]",
    "boyko_render::gpu3d_system::sync_gpu_3d_instances \
     [in: VisibilitySet::Read, boyko_render::instance_model::InstancePackSet]",
    "boyko_render::instance_model::sync_instance_model_cols \
     [in: VisibilitySet::Read, boyko_render::instance_model::InstancePackSet]",
];

/// The member only the `hwrt` leg registers.
#[cfg(feature = "hwrt")]
const HWRT_EXTRA: &[&str] =
    &["boyko_render::instance_model::sync_prev_instance_model_cols [in: VisibilitySet::Read]"];
/// The member only the `hwrt` leg registers (none here).
#[cfg(not(feature = "hwrt"))]
const HWRT_EXTRA: &[&str] = &[];

/// RED: delete `b.configure_set(InstancePackSet).after(CameraSet::Resolve);` from
/// `src/plugins.rs` and the member check fails (`resolve_active_camera` leaves the cycle).
#[test]
fn the_shipped_host_orders_the_instance_packs_after_transform_propagation() {
    // `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run`
    // installs the windowed runner, which this never calls.
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("instance-pack-edge-gate", 64, 64));
    app.add_systems_cfg(|b| {
        b.configure_set(CameraSet::Resolve).after(InstancePackSet);
    });
    assert_finish_rejects_cycle(&mut app, &[CYCLE, HWRT_EXTRA].concat());
}
