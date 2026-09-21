//! **The shipped host orders the asset-ref validation after `visibility_sync`.** A device-free
//! gate on the `EnginePlugins` line
//! `b.configure_set(VisibilitySet::Validate).after(VisibilitySet::Sync)` in `src/plugins.rs`.
//!
//! # Why this file exists (R4b-open-edges)
//!
//! `validate_asset_refs` walks only the `Enabled<RenderEnabled>` rows, and `visibility_sync`
//! enables a spawned row's bit through a deferred command. A validation that runs before that
//! apply window skips the row as not-yet-enabled; the row is enabled right after, and a stale
//! mesh on it stays enabled until `free_epoch` next advances (the validation's cursor early-out,
//! `boyko_render/src/asset_refcount.rs`). R4 put the validation in `VisibilitySet::Read` for
//! this edge; R4b moves it to its own phase, `VisibilitySet::Validate`, because it also WRITES the
//! bit (it disables a stale row) and the plain readers must run after that write too
//! (`tests/host_orders_render_enabled_readers_after_asset_validation.rs`). A system cannot join
//! two sets that are ordered against each other (`boyko-B9004`), so the phase is its own.
//!
//! # How it catches the line
//!
//! `finish()` rejects an ordering cycle with `boyko-B9001`. The test builds the shipped composition,
//! declares the REVERSE edge `VisibilitySet::Sync.after(VisibilitySet::Validate)`, and expects that
//! cycle. Nothing else orders the validation after `visibility_sync` (the `Read` edges order the
//! readers after both and lie on no path between them), so without the line the reverse edge
//! alone is a legal order, `finish()` returns, and the test fails with "finish() accepted the
//! reverse edge".
//!
//! The cycle must name exactly the two systems, `visibility_sync` and `validate_asset_refs`
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
use boyko_scene::VisibilitySet;

use host_order_cycle::assert_finish_rejects_cycle;

/// RED: delete `b.configure_set(VisibilitySet::Validate).after(VisibilitySet::Sync);` from
/// `src/plugins.rs` and this test fails with "finish() accepted the reverse edge".
#[test]
fn the_shipped_host_orders_the_asset_validation_after_visibility_sync() {
    // `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run`
    // installs the windowed runner, which this never calls.
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("validate-sync-edge-gate", 64, 64));
    app.add_systems_cfg(|b| {
        b.configure_set(VisibilitySet::Sync).after(VisibilitySet::Validate);
    });
    assert_finish_rejects_cycle(
        &mut app,
        &[
            "boyko_scene::visibility_sync::visibility_sync [in: VisibilitySet::Sync]",
            "boyko_render::asset_refcount::validate_asset_refs \
     [in: VisibilitySet::Validate, boyko_render::asset_refcount::AssetValidateSet]",
        ],
    );
}
