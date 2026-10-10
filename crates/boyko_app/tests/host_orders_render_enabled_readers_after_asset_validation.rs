//! **The shipped host orders every plain `RenderEnabled` reader after the asset-ref validation.**
//! A device-free gate on the `EnginePlugins` line
//! `b.configure_set(VisibilitySet::Read).after(VisibilitySet::Validate)` in `src/plugins.rs`.
//!
//! # Why this file exists (R4b-open-edges)
//!
//! `validate_asset_refs` writes the two bits the gathers filter on beside `RenderEnabled`: on a
//! churn frame it marks every mesh row whose handle went stale `RenderStale` (a material row
//! `MaterialStale`), and clears the bit again once the handle is valid, through a deferred
//! command (`boyko_render/src/asset_refcount.rs`, `SetStaleCommand`, emitted on a transition
//! only). It never touches `RenderEnabled` itself — that bit is `visibility_sync`'s and the
//! user's (asset-validate prerequisites (a)/(b), A5.2). The mesh and shadow-caster gathers
//! filter on `Enabled<RenderEnabled>` AND `Disabled<RenderStale>`, so a gather that runs before
//! the validation's apply window draws a row the validation marked stale THIS frame for one more
//! frame. `AssetRefcountPlugin`'s doc recorded this edge as inexpressible from the plugin by
//! `SystemKey`, and the fence-gate proof in `retire_deferred_frees`' doc assumes it ("after
//! `apply`, before any gather"). Add-order is not a pin (the executor packs unordered systems
//! by wave), so R4b declares it by name: the validation joins `VisibilitySet::Validate`, the
//! readers stay in `VisibilitySet::Read`, and this line orders the set after the phase. The
//! consumer-side pin of the same order — the gather helpers' `.after_set(AssetValidateSet)`
//! (prereq (c)) — is independent and does not reach the instance packs; this set edge does.
//! Unlike its sibling `Validate.after(Sync)` (`host_orders_asset_validation_after_visibility_sync.rs`),
//! this edge carries data today.
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
    "boyko_render::asset_refcount::validate_asset_refs \
     [in: VisibilitySet::Validate, boyko_render::asset_refcount::AssetValidateSet]",
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
