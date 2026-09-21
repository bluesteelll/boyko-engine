//! Asset-streaming plan "HARD PREREQ before async streaming" item (c) (branch
//! `fix/asset-validate-prereqs`) — the `validate_asset_refs → gather_*` scheduler
//! edge is a HARD, by-name contract, not an add-order accident.
//!
//! # Why a cycle probe, not an order observation
//!
//! Both `validate_asset_refs` and the gathers are `NonSend` (dispatcher-solo) systems,
//! so removing the edge never changes the OBSERVABLE run order in a two-system
//! schedule (lowest-index-first) — a behavioural "validate ran first" test cannot
//! go red when the edge is deleted. What CAN go red is the builder's cycle
//! detection: a probe system pinned `.after(gather).before_set(AssetValidateSet)`
//! closes a cycle IFF the gather really is `.after_set(AssetValidateSet)`. The
//! `B9001` panic from `App::finish` is therefore the direct proof the edge exists,
//! and deleting `.after_set(AssetValidateSet)` inside the registration helper turns
//! this test red.

use std::panic::{AssertUnwindSafe, catch_unwind};

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::asset::Assets;

use boyko_render::asset_refcount::AssetValidateSet;
use boyko_render::{
    AssetRefcountPlugin, CsmCasterScratch, Material, MeshGpu, MeshRenderScratch,
    add_gather_mesh_draws, add_gather_shadow_casters,
};

/// A bare `App` carrying exactly the resources the validate + gather systems resolve.
fn app_with_asset_pipeline() -> App {
    let mut app = App::with_threads(1);
    app.world_mut().insert_non_send_resource(Assets::<MeshGpu>::with_reserved(1));
    app.insert_resource(Assets::<Material>::with_reserved(1));
    app.insert_resource(MeshRenderScratch::default());
    app.insert_resource(CsmCasterScratch::default());
    #[cfg(feature = "hwrt")]
    app.insert_resource(boyko_render::ShadowDenoiseConfig::default());
    app.add_plugin(AssetRefcountPlugin);
    app
}

/// `finish()` must panic with the scheduler's `B9001` ordering-cycle text.
fn assert_finish_reports_a_cycle(mut app: App, what: &str) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        app.finish();
    }));
    let payload = match result {
        Ok(()) => panic!(
            "(c) {what}: App::finish succeeded, so no `AssetValidateSet → {what}` edge exists — \
             the probe `.after(gather).before_set(AssetValidateSet)` should have closed a cycle"
        ),
        Err(payload) => payload,
    };
    let text = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("<non-string panic payload>");
    assert!(
        text.contains("schedule contains a cycle"),
        "(c) {what}: finish panicked, but not with the B9001 ordering-cycle text: {text}"
    );
}

#[test]
fn gather_after_validate_is_a_hard_edge() {
    let mut app = app_with_asset_pipeline();
    app.add_systems_cfg(|b| {
        let gather = add_gather_mesh_draws(b).key();
        b.add_system(|| {}).after(gather).before_set(AssetValidateSet);
    });
    assert_finish_reports_a_cycle(app, "gather_mesh_draws");
}

#[test]
fn caster_gather_after_validate_is_a_hard_edge() {
    let mut app = app_with_asset_pipeline();
    app.add_systems_cfg(|b| {
        let casters = add_gather_shadow_casters(b).key();
        b.add_system(|| {}).after(casters).before_set(AssetValidateSet);
    });
    assert_finish_reports_a_cycle(app, "gather_shadow_casters");
}
