//! **The shipped host orders the asset-ref validation after `visibility_sync`.** A device-free
//! gate on the `EnginePlugins` line
//! `b.configure_set(VisibilitySet::Validate).after(VisibilitySet::Sync)` in `src/plugins.rs`.
//!
//! # Why this file exists (R4b-open-edges), and what the edge carries today
//!
//! When R4b declared the edge, `validate_asset_refs` walked only the `Enabled<RenderEnabled>`
//! rows and cleared that bit on a stale mesh row, so a validation that ran before
//! `visibility_sync`'s apply window skipped a just-spawned row as not-yet-enabled, and a stale
//! mesh on it stayed enabled until `free_epoch` next advanced (the validation's cursor
//! early-out). The asset-validate prerequisites (A5.2, `fix/asset-validate-prereqs`) re-based
//! the validation on two bits of its own — `RenderStale` / `MaterialStale`
//! (`boyko_render/src/asset_refcount.rs`, written through `SetStaleCommand` on a transition
//! only). Its queries filter on NEITHER `RenderEnabled` nor `Visibility`, and it never writes
//! `RenderEnabled` — that bit belongs to `visibility_sync` and the user. So this edge carries NO
//! data today: the validation reads nothing the sync writes.
//!
//! It stays as the phase order R4b introduced — `Sync` → `Validate` → `Read` — with the
//! validation in its own phase because the readers must run after ITS write (the stale bits;
//! `tests/host_orders_render_enabled_readers_after_asset_validation.rs`) and a system cannot
//! join two sets that are ordered against each other (`boyko-B9004`). Whether the
//! `Validate.after(Sync)` edge is dropped is a ruling for the host's owner (the A5 merge record,
//! Open item 2); until then this gate pins the DECLARED order, not a data dependency, and the
//! comment on the `configure_set` line in `src/plugins.rs` says the same.
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
