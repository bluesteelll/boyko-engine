//! **The shipped host orders every `RenderEnabled` reader after `visibility_sync`.** A device-free
//! gate on the `EnginePlugins` line `b.configure_set(VisibilitySet::Read).after(VisibilitySet::Sync)`
//! in `src/plugins.rs`.
//!
//! # Why this file exists (R4-frame-order)
//!
//! `visibility_sync` sets the `RenderEnabled` bit through a deferred command, and every system in
//! `VisibilitySet::Read` filters on `Enabled<RenderEnabled>`. Before the edge the pair had no
//! ordering path, so the executor's wave packing decided it: on the golden host the mesh gather ran
//! first, drew 0 of 7 meshes on frame 0, and the TAA pins carried that meshless frame. One edge added
//! between two lighting systems flipped the pair on the `hwrt` leg and moved six pins, which is how
//! it was found. `tests/host_frame_zero_draws_every_mesh.rs` pins the frame-0 consequence.
//!
//! # How it catches the line
//!
//! `Schedule` exposes no accessor for its resolved order, but `finish()` rejects an ordering cycle
//! with `boyko-B9001`. The test builds the shipped composition, declares the REVERSE edge
//! `VisibilitySet::Sync.after(VisibilitySet::Read)` on top of it, and expects that cycle.
//!
//! The cycle must name exactly `visibility_sync`, the asset-ref validation and the set's members
//! (compared as a set — see `host_order_cycle/mod.rs` for why not as a string), so the panic can
//! only be this seam's, and a reader leaving `VisibilitySet::Read` turns this red too. The `hwrt`
//! leg has one more member, `sync_prev_instance_model_cols`, which `EnginePlugins` registers only
//! under that feature.
//!
//! # What this gate can and cannot see — the line under test is NOT isolated (since R4b)
//!
//! R4b-open-edges split the seam into three phases: `validate_asset_refs` moved from `Read` to
//! its own `VisibilitySet::Validate` (it also WRITES the bit), with `Validate.after(Sync)` and
//! `Read.after(Validate)`. Those two lines are a second path from `visibility_sync` to every
//! reader, so deleting the line under test ALONE leaves this test green, and no test on the order
//! can tell the difference, because the order is the same. The line is kept as the readers' own
//! declaration of their writer, so the order survives a change to that unrelated path (the
//! validation leaving the seam, or its edges being narrowed). This test goes red when the order
//! itself breaks: deleting this line AND either `Validate` line makes the reverse edge a legal
//! order, `finish()` returns, and the test fails with "finish() accepted the reverse edge". The
//! `Validate` lines have gates of their own
//! (`tests/host_orders_asset_validation_after_visibility_sync.rs`,
//! `tests/host_orders_render_enabled_readers_after_asset_validation.rs`).
//!
//! # Why its own binary
//!
//! `EnginePlugins` can be built only once per process: a second build panics in
//! `register_component_hooks::<DirectionalLight>` (recorded in `particle_host_reachable.rs`). This
//! file therefore holds one `#[test]`.
//!
//! It needs no device: `finish()` builds the schedules and runs the startup systems, and no frame
//! runs.

mod host_order_cycle;

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_scene::VisibilitySet;

use host_order_cycle::assert_finish_rejects_cycle;

/// The cycle's members on both feature legs: `visibility_sync`, the validation (on the path
/// `Sync → Validate → Read`) and every `VisibilitySet::Read` member `EnginePlugins` registers on
/// both.
const CYCLE: &[&str] = &[
    "boyko_scene::visibility_sync::visibility_sync [in: VisibilitySet::Sync]",
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

/// RED: delete `b.configure_set(VisibilitySet::Read).after(VisibilitySet::Sync);` AND either
/// `Validate` line from `src/plugins.rs` and this test fails with "finish() accepted the reverse
/// edge". The line alone keeps the order through the `Validate` phase, so deleting it alone keeps
/// this green (see the module doc).
#[test]
fn the_shipped_host_orders_every_render_enabled_reader_after_visibility_sync() {
    // `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run`
    // installs the windowed runner, which this never calls.
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("visibility-sync-edge-gate", 64, 64));
    app.add_systems_cfg(|b| {
        b.configure_set(VisibilitySet::Sync).after(VisibilitySet::Read);
    });
    assert_finish_rejects_cycle(&mut app, &[CYCLE, HWRT_EXTRA].concat());
}
