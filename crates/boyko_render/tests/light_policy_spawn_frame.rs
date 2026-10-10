//! P1 gate (spawn frame): `select_lighting_cull` counts a light on the frame it is added, so
//! `LightStats::point_spot_count` agrees with the light table `collect_lights` folds that frame.
//!
//! # The defect this pins (R2b-edge, G2)
//!
//! The count reads `IsEnabled<LightEnabled>`, and a newly added light's `LightEnabled` bit is set by
//! the exclusive light seed. `LightingPlugin` registered `select_lighting_cull` `.before(collect)`
//! and not after the seed, so on the frame lights were added it ran first and saw them as disabled.
//! Three point lights spawned before the first update read `point_spot_count == 0` on that frame
//! while the table already held all three; the next frame both read 3. The sibling gates
//! (`light_policy_enable_gate.rs`, `light_policy_auto_band.rs`) settle for three frames before
//! their first assertion, so none of them could see it. Under `ClusterSelectMode::Auto` the same
//! stale count also decides `clusters_enabled` for that frame.
//!
//! RED: delete the seed's `.before(cull)` edge in `light_plugin.rs` and the count assertion below
//! fails with `left: 0, right: 3`.
//!
//! # Why the table is asserted first
//!
//! `collect_lights` folds only lights whose `LightEnabled` bit is set, and it is ordered after the
//! seed. If the table holds all three, the seed ran this frame and enabled them, so a count of 0
//! can only mean the policy ran before the seed. Without that premise, a seed that enabled nothing
//! would also produce 0 and be reported as an ordering defect.
//!
//! SINGLE-TEST BINARY (see `le_support/common.rs` isolation note).

#[path = "le_support/common.rs"]
mod common;

#[test]
fn the_policy_counts_lights_on_the_frame_the_seed_enables_them() {
    let mut app = common::lighting_app();

    // Spawned before the first update: the seed's first pass is its full scan, which enables
    // all three in the same frame's pass.
    for i in 0..3 {
        common::spawn_point_light(app.world_mut(), [i as f32, 1.0, 0.0]);
    }

    app.finish();
    app.update();

    assert_eq!(
        common::point_spot_count(&app),
        3,
        "premise: the spawn-frame light table must already hold the three point lights. \
         `collect_lights` folds only ENABLED lights and runs after the seed, so a table short of 3 \
         means the seed did not enable them this frame -- a seed defect, not the ordering this \
         gate pins"
    );
    assert_eq!(
        common::policy_point_spot_count(&app),
        3,
        "spawn frame: the light table holds 3 enabled point lights, but `select_lighting_cull` \
         counted fewer. It read `LightEnabled` before the seed enabled them: `LightingPlugin` must \
         order the seed before `select_lighting_cull` (the seed's `.before(cull)` edge)"
    );
}
