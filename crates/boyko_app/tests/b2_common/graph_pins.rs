//! G-GRAPH's pinned system-name multisets (plan gate UG-13, Tier A).
//!
//! **Rule: exact equality, re-derived with attribution.** A merge that adds or removes a system
//! re-derives these lists in its own commit and names, in that commit, the commit that added or
//! removed each name (UG-03's rule, "merges add systems", 02 Phase B). Names are normalised by
//! `normalize` (the including binary's crate name reads `$test`).
//!
//! Measured on `u/b2` @ `c1e9f1db` (msvc, debug, default features — `hwrt` off), 2026-09-23:
//! app (a) Main 35 / Fixed 1; app (b) Main 47 / Fixed 1. (b) − (a) is the 12 UI and input
//! systems: `clear_consumed_fixed_edges` and `update_action_state` (`InputPlugin`), the three
//! interaction systems, the two bind systems, the two bar systems and the three animation systems.
//!
//! Re-derived on `u/dm1` (DM1 commit C4, "stage_material_edits"): both Main lists gain
//! `boyko_render::material_upload::stage_material_edits`, the dynamic-materials stager
//! `register_main_frame_systems` now registers after `gather_mesh_draws` — app (a) Main 36,
//! app (b) Main 48 (37 / 49 with `hwrt`); Fixed unchanged. Nothing else moved.
//!
//! **Feature sets.** The consts below are the default build. `--features hwrt` adds exactly
//! [`MAIN_HWRT_EXTRA`] to Main in BOTH apps (measured the same day, same tree: app (a) Main 36,
//! app (b) Main 48, Fixed unchanged); [`main_a`] / [`main_b`] fold it in when the feature is on, so
//! both shipped feature sets are pinned rather than one of them being red by construction.

use super::owned;

/// What `--features hwrt` adds to Main, in both apps (`boyko_app/src/plugins.rs`, the
/// `#[cfg(feature = "hwrt")]` prev-instance-model sync).
pub const MAIN_HWRT_EXTRA: &[(&str, u32)] = &[(
    "boyko_render::instance_model::sync_prev_instance_model_cols",
    1,
)];

/// `base` plus [`MAIN_HWRT_EXTRA`] when this build has the `hwrt` feature, sorted like an
/// inventory.
fn with_features(base: &[(&str, u32)]) -> Vec<(String, u32)> {
    let mut v = owned(base);
    if cfg!(feature = "hwrt") {
        for (n, c) in MAIN_HWRT_EXTRA {
            match v.iter_mut().find(|(vn, _)| vn == n) {
                Some((_, vc)) => *vc += c,
                None => v.push(((*n).to_string(), *c)),
            }
        }
        v.sort();
    }
    v
}

/// App (a)'s pinned Main for this build's feature set.
pub fn main_a() -> Vec<(String, u32)> {
    with_features(MAIN_A)
}

/// App (b)'s pinned Main for this build's feature set.
pub fn main_b() -> Vec<(String, u32)> {
    with_features(MAIN_B)
}

/// App (a), Main.
pub const MAIN_A: &[(&str, u32)] = &[
    (
        "<boyko_render::light_plugin::LightingPlugin as boyko_ecs::ecs::core::app::plugin::Plugin>::build::{{closure}}::{{closure}}",
        1,
    ),
    ("boyko_ecs::ecs::core::log::plugin::log_drain_system", 1),
    ("boyko_render::aa_config::resolve_aa_policy", 1),
    ("boyko_render::asset_refcount::apply_refcount_deltas", 1),
    ("boyko_render::asset_refcount::validate_asset_refs", 1),
    ("boyko_render::csm_caster::gather_shadow_casters", 1),
    ("boyko_render::csm_caster::reduce_caster_bounds", 1),
    ("boyko_render::csm_caster::sync_csm_light_gate", 1),
    ("boyko_render::csm_config::resolve_csm_cascades", 1),
    ("boyko_render::ddgi_config::sync_ddgi_light_gate", 1),
    ("boyko_render::ddgi_update::resolve_ddgi_grid_gated", 1),
    ("boyko_render::gpu3d_system::sync_gpu_3d_instances", 1),
    ("boyko_render::instance_model::sync_instance_model_cols", 1),
    ("boyko_render::light::sync_cluster_light_gate", 1),
    ("boyko_render::light::sync_sv0_light_gate", 1),
    ("boyko_render::light_policy::select_lighting_cull", 1),
    ("boyko_render::light_reconcile::light_reconcile", 1),
    ("boyko_render::light_system::collect_lights", 1),
    ("boyko_render::material_upload::stage_material_edits", 1),
    ("boyko_render::mesh_draw::gather_mesh_draws", 1),
    (
        "boyko_render::particle_system::particle_apply_effect_refs",
        1,
    ),
    ("boyko_render::particle_system::particle_pack_effects", 1),
    ("boyko_render::particle_system::particle_tick_emitters", 1),
    ("boyko_render::ray_backend::resolve_ray_backend_system", 1),
    (
        "boyko_render::ray_shadow_config::resolve_ray_shadow_system",
        1,
    ),
    ("boyko_render::shadow_atlas::resolve_shadow_atlas", 1),
    ("boyko_render::shadow_atlas::sync_punctual_light_gate", 1),
    (
        "boyko_render::shadow_denoise_config::resolve_shadow_denoise_policy",
        1,
    ),
    (
        "boyko_render::shadow_denoise_config::resolve_temporal_shadow_policy",
        1,
    ),
    ("boyko_render::snap_interpolation::snap_apply", 1),
    ("boyko_render::ssao_config::resolve_ssao_policy", 1),
    ("boyko_render::ssao_config::sync_ssao_light_gate", 1),
    ("boyko_render::taa_config::resolve_taa_policy", 1),
    ("boyko_scene::camera::resolve_active_camera", 1),
    ("boyko_scene::propagation::propagate_transforms", 1),
    ("boyko_scene::visibility_sync::visibility_sync", 1),
];
/// App (a), Fixed.
pub const FIXED_A: &[(&str, u32)] = &[("boyko_render::gpu_transform_pack::pack_gpu_transforms", 1)];
/// App (b), Main.
pub const MAIN_B: &[(&str, u32)] = &[
    (
        "<boyko_render::light_plugin::LightingPlugin as boyko_ecs::ecs::core::app::plugin::Plugin>::build::{{closure}}::{{closure}}",
        1,
    ),
    ("boyko_ecs::ecs::core::log::plugin::log_drain_system", 1),
    (
        "boyko_input::action::process::clear_consumed_fixed_edges<$test::b2_common::TestAction>",
        1,
    ),
    (
        "boyko_input::action::process::update_action_state<$test::b2_common::TestAction>",
        1,
    ),
    ("boyko_render::aa_config::resolve_aa_policy", 1),
    ("boyko_render::asset_refcount::apply_refcount_deltas", 1),
    ("boyko_render::asset_refcount::validate_asset_refs", 1),
    ("boyko_render::csm_caster::gather_shadow_casters", 1),
    ("boyko_render::csm_caster::reduce_caster_bounds", 1),
    ("boyko_render::csm_caster::sync_csm_light_gate", 1),
    ("boyko_render::csm_config::resolve_csm_cascades", 1),
    ("boyko_render::ddgi_config::sync_ddgi_light_gate", 1),
    ("boyko_render::ddgi_update::resolve_ddgi_grid_gated", 1),
    ("boyko_render::gpu3d_system::sync_gpu_3d_instances", 1),
    ("boyko_render::instance_model::sync_instance_model_cols", 1),
    ("boyko_render::light::sync_cluster_light_gate", 1),
    ("boyko_render::light::sync_sv0_light_gate", 1),
    ("boyko_render::light_policy::select_lighting_cull", 1),
    ("boyko_render::light_reconcile::light_reconcile", 1),
    ("boyko_render::light_system::collect_lights", 1),
    ("boyko_render::material_upload::stage_material_edits", 1),
    ("boyko_render::mesh_draw::gather_mesh_draws", 1),
    (
        "boyko_render::particle_system::particle_apply_effect_refs",
        1,
    ),
    ("boyko_render::particle_system::particle_pack_effects", 1),
    ("boyko_render::particle_system::particle_tick_emitters", 1),
    ("boyko_render::ray_backend::resolve_ray_backend_system", 1),
    (
        "boyko_render::ray_shadow_config::resolve_ray_shadow_system",
        1,
    ),
    ("boyko_render::shadow_atlas::resolve_shadow_atlas", 1),
    ("boyko_render::shadow_atlas::sync_punctual_light_gate", 1),
    (
        "boyko_render::shadow_denoise_config::resolve_shadow_denoise_policy",
        1,
    ),
    (
        "boyko_render::shadow_denoise_config::resolve_temporal_shadow_policy",
        1,
    ),
    ("boyko_render::snap_interpolation::snap_apply", 1),
    ("boyko_render::ssao_config::resolve_ssao_policy", 1),
    ("boyko_render::ssao_config::sync_ssao_light_gate", 1),
    ("boyko_render::taa_config::resolve_taa_policy", 1),
    ("boyko_scene::camera::resolve_active_camera", 1),
    ("boyko_scene::propagation::propagate_transforms", 1),
    ("boyko_scene::visibility_sync::visibility_sync", 1),
    ("boyko_ui::animation::ui_clock_tick", 1),
    ("boyko_ui::animation::ui_tween_reap", 1),
    ("boyko_ui::animation::ui_visual_tick", 1),
    ("boyko_ui::binding::bind_system::ui_bind_apply", 1),
    ("boyko_ui::binding::bind_system::ui_bind_discovery", 1),
    (
        "boyko_ui::interaction::dispatch::ui_dispatch_system<$test::b2_common::TestAction>",
        1,
    ),
    (
        "boyko_ui::interaction::dispatch::ui_refreeze_fixed_snapshot<$test::b2_common::TestAction>",
        1,
    ),
    ("boyko_ui::interaction::focus::ui_focus_system", 1),
    ("boyko_ui::widgets::ui_bar_apply", 1),
    ("boyko_ui::widgets::ui_bar_discovery", 1),
];
/// App (b), Fixed.
pub const FIXED_B: &[(&str, u32)] = &[("boyko_render::gpu_transform_pack::pack_gpu_transforms", 1)];
