//! The [`SsaoPlugin`] (Render P7-Q2 TASK 3) — inserts the owner-set [`SsaoConfig`] +
//! its derived [`ResolvedSsao`] companion and registers the cold
//! [`resolve_ssao_policy`] at the gather/setup boundary, symmetric with
//! [`LightingPlugin`](crate::light_plugin::LightingPlugin) inserting
//! [`LightStats`](crate::light_policy::LightStats) + registering `select_lighting_cull`.

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::ssao_config::{ResolvedSsao, SsaoConfig, resolve_ssao_policy};

/// Registers the SSAO config substrate: inserts [`SsaoConfig`] (default
/// [`Off`](crate::ssao_config::SsaoQuality::Off) — the 0%-gate) and its derived
/// [`ResolvedSsao`] companion, and schedules the cold
/// [`resolve_ssao_policy`](crate::ssao_config::resolve_ssao_policy) (the SINGLE writer of
/// `ResolvedSsao`).
///
/// # Mirror of [`LightingPlugin`](crate::light_plugin::LightingPlugin)
///
/// The lighting plugin inserts the owner-set [`LightingConfig`](crate::light::LightingConfig)
/// plus the derived [`LightStats`](crate::light_policy::LightStats) and registers
/// `select_lighting_cull` `.before` `collect_lights`. This plugin is the SSAO analogue:
/// owner-set `SsaoConfig` plus derived `ResolvedSsao` plus the cold `resolve_ssao_policy`.
///
/// # Ordering vs. the render point (the named follow-up)
///
/// `resolve_ssao_policy` must run BEFORE the deferred-render ECS system that READS
/// [`ResolvedSsao`] to pick the variant pipeline + set the resolve's `with_ssao_mode`.
/// That consumer is the explicit LARGER follow-up (the deferred pipeline is test-driven
/// today, not a `boyko_render` system), so there is no `SystemKey` to `.before` here yet.
/// When that system lands it should be co-registered with `resolve_ssao_policy` in one
/// closure (so the `.before(render_point)` edge is expressible at the call site), exactly
/// as [`LightingPlugin`](crate::light_plugin::LightingPlugin) co-registers the policy
/// `.before(collect)`. The selection is recomputed every frame, so a loose one-frame
/// stagger before the consumer lands is self-correcting (the config is cold owner state).
#[derive(Default)]
pub struct SsaoPlugin;

impl Plugin for SsaoPlugin {
    fn build(&self, app: &mut App) {
        // The owner-set cold config (default Off — the 0%-gate) + its derived carrier.
        // `resolve_ssao_policy` is the single writer of `ResolvedSsao` (the one-producer
        // write discipline). The default `ResolvedSsao` already reads the no-pass selection,
        // so the world is correct even before the first policy run.
        app.insert_resource(SsaoConfig::default());
        app.insert_resource(ResolvedSsao::default());

        app.add_systems_cfg(|b| {
            b.add_system(resolve_ssao_policy);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::SsaoPlugin"
    }
}
