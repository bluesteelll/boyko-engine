//! P1 gate (Auto + hysteresis): in `ClusterSelectMode::Auto`, `select_lighting_cull`
//! drives `clusters_enabled` from the banded live point/spot count — ON at `>= CLUSTER_HI`,
//! OFF at `<= CLUSTER_LO`, and HOLDS the previous side strictly inside `(LO, HI)`.
//!
//! The hysteresis is tested in BOTH directions: with the band seeded ON, an in-band count
//! stays ON; with the band seeded OFF, the same in-band count stays OFF. A single
//! threshold (no band) would force one of those to flip — the band is what prevents
//! boundary thrash.
//!
//! SINGLE-TEST BINARY: `LightingPlugin::build` registers process-global eviction hooks and
//! may run ONLY ONCE per process, so this whole test uses ONE `App` and drives the live
//! count via `LightEnabled` toggles on a fixed light pool (no second `lighting_app()`).

#[path = "le_support/common.rs"]
mod common;

use boyko_ecs::ecs::core::entity::entity::Entity;

use boyko_render::light::ClusterSelectMode;
use boyko_render::light_policy::{CLUSTER_HI, CLUSTER_LO};
use boyko_render::light_system::set_light_enabled_now;

/// Enables exactly the first `on` lights of `pool`, disables the rest, then runs a frame so
/// `select_lighting_cull` re-counts and re-bands. Past the spawn `Added` window the seed no
/// longer re-enables a just-disabled light, so the live count equals `on` exactly.
fn set_live_count(app: &mut boyko_ecs::ecs::core::app::App, pool: &[Entity], on: u32) {
    for (i, &e) in pool.iter().enumerate() {
        set_light_enabled_now(app.world_mut(), e, (i as u32) < on);
    }
    app.update();
    assert_eq!(
        common::policy_point_spot_count(app),
        on,
        "fixture: the live enabled count must equal the requested `on`"
    );
}

#[test]
fn auto_mode_bands_and_holds_hysteresis() {
    // The in-band count used for the hold tests must lie strictly inside (LO, HI).
    const IN_BAND: u32 = CLUSTER_LO + 1;
    const _: () = assert!(IN_BAND < CLUSTER_HI, "test fixture: IN_BAND must lie inside the band");

    let mut app = common::lighting_app();
    common::set_cluster_select(&mut app, ClusterSelectMode::Auto);

    // A fixed pool of CLUSTER_HI point lights — the live count is then any value in
    // [0, CLUSTER_HI] via per-light `LightEnabled` toggles (no despawn, one plugin build).
    let pool: Vec<Entity> =
        (0..CLUSTER_HI).map(|i| common::spawn_point_light(app.world_mut(), [i as f32, 0.0, 0.0])).collect();

    // Settle past the spawn `Added` window so the seed has enabled all and its steady-state
    // scan is empty (a later disable then stays disabled).
    app.finish();
    app.update();
    app.update();
    app.update();

    // --- 1) count >= CLUSTER_HI -> ON --------------------------------------------------
    set_live_count(&mut app, &pool, CLUSTER_HI);
    assert!(common::clusters_enabled(&app), "count >= CLUSTER_HI selects clusters ON");
    assert!(common::policy_cluster_band(&app), "the band side is recorded ON");

    // --- 2) hysteresis, was-ON: an in-band count HOLDS ON ------------------------------
    // (the band is currently ON from step 1; drop the count into the band).
    set_live_count(&mut app, &pool, IN_BAND);
    assert!(
        common::clusters_enabled(&app),
        "in-band count holds the previous ON side (hysteresis, was-on stays on)"
    );
    assert!(common::policy_cluster_band(&app));

    // --- 3) count <= CLUSTER_LO -> OFF even though seeded ON ---------------------------
    set_live_count(&mut app, &pool, CLUSTER_LO);
    assert!(
        !common::clusters_enabled(&app),
        "count <= CLUSTER_LO forces clusters OFF regardless of the previous side"
    );
    assert!(!common::policy_cluster_band(&app), "the band side is recorded OFF");

    // --- 4) hysteresis, was-OFF: the same in-band count HOLDS OFF ----------------------
    // (the band is now OFF from step 3; raise the count back into the band).
    set_live_count(&mut app, &pool, IN_BAND);
    assert!(
        !common::clusters_enabled(&app),
        "the same in-band count holds the previous OFF side (hysteresis, was-off stays off)"
    );
    assert!(!common::policy_cluster_band(&app));
}
