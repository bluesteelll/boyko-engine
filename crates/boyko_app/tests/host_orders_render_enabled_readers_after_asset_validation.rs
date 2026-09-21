//! **The shipped host orders every plain `RenderEnabled` reader after the asset-ref validation.**
//! A device-free gate on the `EnginePlugins` line
//! `b.configure_set(VisibilitySet::Read).after(VisibilitySet::Validate)` in `src/plugins.rs`.
//!
//! # Why this file exists (R4b-open-edges)
//!
//! `validate_asset_refs` is not only a reader of the `RenderEnabled` bit: on a churn frame it
//! DISABLES every mesh row whose handle went stale, through a deferred command
//! (`boyko_render/src/asset_refcount.rs`, `DisableStaleMeshCommand`). The instance packs and the
//! mesh and shadow-caster gathers filter on that bit, so a gather that runs before the
//! validation's apply window draws the stale row for one more frame. `AssetRefcountPlugin`'s doc
//! recorded this edge as inexpressible from the plugin and left it to add-order, and the
//! fence-gate proof in `retire_deferred_frees`' doc assumes it ("after `apply`, before any
//! gather"). Add-order is not a pin (the executor packs unordered systems by wave), so R4b
//! declares it by name: the validation joins `VisibilitySet::Validate`, the readers stay in
//! `VisibilitySet::Read`, and this line orders the set after the phase.
//!
//! # How it catches the line
//!
//! `finish()` rejects an ordering cycle with `boyko-B9001`. The test builds the shipped composition,
//! declares the REVERSE edge `VisibilitySet::Validate.after(VisibilitySet::Read)`, and expects that
//! cycle. Nothing else orders a reader after the validation, so without the line the reverse edge
//! alone is a legal order, `finish()` returns, and the test fails with "finish() accepted the
//! reverse edge".
//!
//! The cycle must name exactly `validate_asset_refs` and the `Read` members (compared as a set —
//! see `host_order_cycle/mod.rs`), so the panic can only be this edge's, and a reader leaving
//! `VisibilitySet::Read` turns this red too. `visibility_sync` is NOT in it: it is before both
//! sides and on no path from a reader back to the validation. The `hwrt` leg has one more member,
//! `sync_prev_instance_model_cols`, which `EnginePlugins` registers only under that feature.
//!
//! # Why its own binary
//!
//! `EnginePlugins` can be built only once per process (see `particle_host_reachable.rs`), so this
//! file holds one `#[test]`. It needs no device: `finish()` builds the schedules and runs the
//! startup systems, and no frame runs.

mod host_order_cycle;

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_scene::VisibilitySet;

use host_order_cycle::assert_finish_rejects_cycle;

/// The cycle's members on both feature legs: the validation and every `VisibilitySet::Read`
/// member `EnginePlugins` registers on both.
const CYCLE: &[&str] = &[
    "boyko_render::asset_refcount::validate_asset_refs [in: VisibilitySet::Validate]",
    "boyko_render::gpu3d_system::sync_gpu_3d_instances \
     [in: VisibilitySet::Read, boyko_render::instance_model::InstancePackSet]",
    "boyko_render::instance_model::sync_instance_model_cols \
     [in: VisibilitySet::Read, boyko_render::instance_model::InstancePackSet]",
    "boyko_render::csm_caster::gather_shadow_casters [in: VisibilitySet::Read]",
    "boyko_render::mesh_draw::gather_mesh_draws [in: VisibilitySet::Read]",
];

/// The member only the `hwrt` leg registers.
#[cfg(feature = "hwrt")]
const HWRT_EXTRA: &[&str] =
    &["boyko_render::instance_model::sync_prev_instance_model_cols [in: VisibilitySet::Read]"];
/// The member only the `hwrt` leg registers (none here).
#[cfg(not(feature = "hwrt"))]
const HWRT_EXTRA: &[&str] = &[];

/// RED: delete `b.configure_set(VisibilitySet::Read).after(VisibilitySet::Validate);` from
/// `src/plugins.rs` and this test fails with "finish() accepted the reverse edge".
#[test]
fn the_shipped_host_orders_every_render_enabled_reader_after_the_asset_validation() {
    // `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run`
    // installs the windowed runner, which this never calls.
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("read-validate-edge-gate", 64, 64));
    app.add_systems_cfg(|b| {
        b.configure_set(VisibilitySet::Validate).after(VisibilitySet::Read);
    });
    assert_finish_rejects_cycle(&mut app, &[CYCLE, HWRT_EXTRA].concat());
}
