//! The [`ShadowDenoisePlugin`] (HW-RT Rung 3a Step 1) — inserts the author-set
//! [`ShadowDenoiseConfig`] + its derived [`ResolvedShadowDenoise`] companion and registers
//! the cold [`resolve_shadow_denoise_policy`] single-writer, symmetric with
//! [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin) and
//! [`RayPlugin`](crate::ray_plugin::RayPlugin)'s ray-shadow substrate.

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::shadow_denoise_config::{
    ResolvedShadowDenoise, ShadowDenoiseConfig, resolve_shadow_denoise_policy,
};

/// Registers the shadow-denoise config substrate: inserts [`ShadowDenoiseConfig`] (default
/// [`None`](crate::shadow_denoise_config::ShadowDenoiseMode::None) — the 0%-gate) and its
/// derived [`ResolvedShadowDenoise`] companion, and schedules the cold
/// [`resolve_shadow_denoise_policy`](crate::shadow_denoise_config::resolve_shadow_denoise_policy)
/// (the SINGLE writer of `ResolvedShadowDenoise`).
///
/// # Mirror of [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin)
///
/// The SSAO plugin inserts the owner-set [`SsaoConfig`](crate::ssao_config::SsaoConfig) plus
/// the derived [`ResolvedSsao`](crate::ssao_config::ResolvedSsao) and registers the cold
/// [`resolve_ssao_policy`](crate::ssao_config::resolve_ssao_policy). This plugin is the
/// shadow-denoise analogue: author-set `ShadowDenoiseConfig` plus derived
/// `ResolvedShadowDenoise` plus the cold `resolve_shadow_denoise_policy`.
///
/// # Ordering vs. the a-trous consumer (the named follow-up)
///
/// `resolve_shadow_denoise_policy` must run BEFORE the a-trous filter pass that READS
/// [`ResolvedShadowDenoise`] to weight its edge-stops. That consumer is the explicit LARGER
/// follow-up (Rung 3a Steps 2-7 — the pass/shader/framegraph work), so there is no
/// `SystemKey` to `.before` here yet. When that system lands it should be co-registered with
/// `resolve_shadow_denoise_policy` in one closure (so the `.before(a_trous)` edge is
/// expressible at the call site), exactly as [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin)
/// documents. The scalars are recomputed every frame, so a loose one-frame stagger before
/// the consumer lands is self-correcting (the config is cold author state).
#[derive(Default)]
pub struct ShadowDenoisePlugin;

impl Plugin for ShadowDenoisePlugin {
    fn build(&self, app: &mut App) {
        // The author-set cold config (default None — the 0%-gate) + its derived carrier.
        // `resolve_shadow_denoise_policy` is the single writer of `ResolvedShadowDenoise`
        // (the one-producer write discipline). The default `ResolvedShadowDenoise` already
        // carries the edge-stop scalars, so the world is correct even before the first run.
        app.insert_resource(ShadowDenoiseConfig::default());
        app.insert_resource(ResolvedShadowDenoise::default());

        app.add_systems_cfg(|b| {
            b.add_system(resolve_shadow_denoise_policy);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::ShadowDenoisePlugin"
    }
}
