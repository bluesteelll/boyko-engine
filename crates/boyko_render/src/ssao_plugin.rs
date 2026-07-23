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
/// [`resolve_ssao_policy`] (the SINGLE writer of
/// `ResolvedSsao`).
///
/// # Mirror of [`LightingPlugin`](crate::light_plugin::LightingPlugin)
///
/// The lighting plugin inserts the owner-set [`LightingConfig`](crate::light::LightingConfig)
/// plus the derived [`LightStats`](crate::light_policy::LightStats) and registers
/// `select_lighting_cull` `.before` `collect_lights`. This plugin is the SSAO analogue:
/// owner-set `SsaoConfig` plus derived `ResolvedSsao` plus the cold `resolve_ssao_policy`.
///
/// # Ordering vs. the render point (the live consumer)
///
/// `resolve_ssao_policy` writes [`ResolvedSsao`] every frame; `boyko_app::runner` reads
/// it through `World::try_resource` (the SAME per-frame `World` read `ResolvedAa` uses)
/// to pick the variant pipeline and arm `GBufferScene::ssao` — NOT an ECS system (the RHI
/// pipeline objects are host-owned), so there is no `SystemKey` to `.before` here. The
/// selection is recomputed every frame, so a loose one-frame stagger is self-correcting
/// (the config is cold owner state).
///
/// The resolve's `ssao_mode` header gate is a SEPARATE seam armed by
/// [`sync_ssao_light_gate`](crate::ssao_config::sync_ssao_light_gate), registered by the
/// composing app (NOT by this plugin — see that system's doc for why), mirroring
/// [`sync_ddgi_light_gate`](crate::ddgi_config::sync_ddgi_light_gate).
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
        // Rung R9a: the inert boot-freeze snapshot (this kernel has no `Option<Res<R>>`
        // SystemParam, so the systems' `Res<RenderPathFrozenConsumers>` param needs a value in
        // EVERY world composing this plugin). `non_deferred == false` ⇒ the clamp is a no-op;
        // `boyko_app::runner` OVERWRITES it at boot with the real snapshot (the
        // `ResolvedRenderPath` insert-default-then-boot-override precedent).
        app.insert_resource(crate::render_path_config::RenderPathFrozenConsumers::default());

        app.add_systems_cfg(|b| {
            b.add_system(resolve_ssao_policy);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::SsaoPlugin"
    }
}
