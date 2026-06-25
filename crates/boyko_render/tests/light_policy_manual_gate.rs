//! P1 gate (Manual is the 0%-gate): `select_lighting_cull` updates the live point/spot
//! count but, in the default `ClusterSelectMode::Manual`, NEVER touches
//! `LightingConfig::clusters_enabled` — it stays exactly whatever the owner set, even
//! when the count is well above `CLUSTER_HI`. This is the no-behavior-change anchor for
//! every pre-P1 world.
//!
//! SINGLE-TEST BINARY: this file's one `#[test]` adds `LightingPlugin` (which registers
//! the process-global eviction hooks first) and then spawns lights — see the isolation
//! note in `le_support/common.rs`.

#[path = "le_support/common.rs"]
mod common;

use boyko_render::light_policy::CLUSTER_HI;

#[test]
fn manual_mode_never_changes_clusters_enabled_but_counts() {
    let mut app = common::lighting_app();

    // Owner pins clusters OFF (the default, made explicit). Mode stays Manual (the default).
    common::set_clusters_enabled(&mut app, false);

    // Spawn MORE point lights than CLUSTER_HI so an Auto policy WOULD switch ON — proving
    // Manual's silence is not just "the count happened to stay in band".
    let n = (CLUSTER_HI + 3) as usize;
    for i in 0..n {
        common::spawn_point_light(app.world_mut(), [i as f32, 0.0, 0.0]);
    }

    // Settle past the spawn `Added` window (mirrors the toggle gate's methodology): the
    // seed enables every light in-pass on the first frame, later frames drain `Added`.
    app.finish();
    app.update();
    app.update();
    app.update();

    // The policy DID update the count (its one always-on duty).
    assert_eq!(
        common::policy_point_spot_count(&app),
        n as u32,
        "select_lighting_cull counts all seeded point lights even in Manual mode"
    );
    // But `clusters_enabled` is untouched — owner-controlled, byte-identical to pre-P1.
    assert!(
        !common::clusters_enabled(&app),
        "Manual mode must NOT drive clusters_enabled (the 0%-gate), despite count > CLUSTER_HI"
    );

    // Owner flips it ON by hand — the policy must STILL not fight it back off, even though
    // (count > HI) and (count < ... ) are both irrelevant in Manual.
    common::set_clusters_enabled(&mut app, true);
    // Re-touch a light so collect/select run a fresh frame.
    common::spawn_point_light(app.world_mut(), [99.0, 0.0, 0.0]);
    app.update();
    app.update();
    assert!(
        common::clusters_enabled(&app),
        "Manual mode leaves an owner-set clusters_enabled = true untouched"
    );
}
