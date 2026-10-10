//! **The shipped host orders the CSM fit after the light seed.** A device-free gate on the
//! `EnginePlugins` line `b.configure_set(CsmResolveSet).after(LightSeedSet)` in `src/plugins.rs`.
//!
//! # Why this file exists (R2b-edge, G1)
//!
//! `resolve_csm_cascades` takes the first ENABLED sun, and the exclusive light seed is what enables
//! a light added this frame. `boyko_render/tests/csm_primary_sun_agreement.rs` gates the defect
//! the edge prevents, but it declares its own copy of the edge in its own world. The line in
//! `EnginePlugins` is the only one that orders the fit after the seed in a shipped host, and
//! nothing gated it: the W3 tester deleted it and `cargo test -p boyko-app --all-targets` stayed
//! green (266 passed, 0 failed), while the golden pins rendered the same bytes with and without it.
//!
//! # How it catches the line
//!
//! `Schedule` exposes no accessor for its resolved order, so the test cannot read the order
//! directly. What the build does do is reject a cycle: `finish()` expands the set edges and panics
//! `boyko-B9001` on an ordering cycle. The test builds the shipped composition, declares the
//! REVERSE edge `LightSeedSet.after(CsmResolveSet)` on top of it, and expects that panic. With the
//! shipped edge present the two edges form a cycle through the seed and the fit. Without it the
//! reverse edge alone is a legal order, `finish()` returns, and the test fails with "test did not
//! panic as expected".
//!
//! The same red covers both memberships the edge depends on. If `LightingPlugin` dropped the
//! seed's `.in_set(LightSeedSet)`, or `CsmPlugin` dropped the fit's `.in_set(CsmResolveSet)`, the
//! edges would expand to no pairs (a `boyko-W1501` warning, not a panic), and there would be no
//! cycle.
//!
//! # What keeps the panic from being some other cycle's
//!
//! `expected = "boyko-B9001"` matches any ordering cycle. The control is
//! `particle_host_reachable.rs`: it builds the same `EnginePlugins::window` composition and calls
//! `finish()` WITHOUT the reverse edge, so a cycle of the composition's own would panic there with
//! the same code. It has to live in another binary (next section), so it runs in the same
//! `cargo test -p boyko-app` invocation but not in this process.
//!
//! # Why its own binary
//!
//! `EnginePlugins` can be built only once per process: a second build panics in
//! `register_component_hooks::<DirectionalLight>` (recorded in `particle_host_reachable.rs`). This
//! file therefore holds one `#[test]`.
//!
//! It needs no device: `finish()` builds the schedules and runs the startup systems, and no frame
//! runs.

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_render::{CsmResolveSet, LightSeedSet};

/// RED: replace the shipped `b.configure_set(CsmResolveSet).after(LightSeedSet);` in
/// `src/plugins.rs` with a no-op and this test fails with "test did not panic as expected".
#[test]
#[should_panic(expected = "boyko-B9001")]
fn the_shipped_host_orders_the_csm_fit_after_the_light_seed() {
    // `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run`
    // installs the windowed runner, which this never calls.
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("csm-seed-edge-gate", 64, 64));
    app.add_systems_cfg(|b| {
        b.configure_set(LightSeedSet).after(CsmResolveSet);
    });
    app.finish();
}
