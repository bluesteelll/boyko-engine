//! The [`CsmPlugin`] (CSM Inc-1a) — inserts the owner-set [`CsmConfig`] + its derived
//! [`ResolvedCsm`] companion and registers the cold [`resolve_csm_cascades`] policy,
//! symmetric with [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin) /
//! [`LightingPlugin`](crate::light_plugin::LightingPlugin).

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::csm_config::{
    CsmCasterBounds, CsmConfig, CsmFitState, CsmResolveSet, ResolvedCsm, resolve_csm_cascades,
};

/// Registers the CSM config substrate: inserts [`CsmConfig`] (default DISABLED —
/// `cascade_count == 0`, the 0%-gate) and its derived [`ResolvedCsm`] companion, and
/// schedules the cold [`resolve_csm_cascades`](crate::csm_config::resolve_csm_cascades)
/// (the SINGLE writer of `ResolvedCsm`).
///
/// # Mirror of [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin)
///
/// The SSAO plugin inserts the owner-set [`SsaoConfig`](crate::ssao_config::SsaoConfig)
/// plus the derived [`ResolvedSsao`](crate::ssao_config::ResolvedSsao) and registers the
/// cold `resolve_ssao_policy`. This plugin is the CSM analogue: owner-set `CsmConfig` plus
/// derived `ResolvedCsm` plus the cold `resolve_csm_cascades`.
///
/// # Add-order contract (cross-plugin ordering vs. camera + light)
///
/// `resolve_csm_cascades` reads the engine-derived [`ViewUniform`](boyko_scene::ViewUniform)
/// (written by `resolve_active_camera` in
/// [`CameraPlugin`](boyko_scene::CameraPlugin)) and the primary
/// [`DirectionalLight`](crate::light::DirectionalLight) direction (reconciled by
/// `light_reconcile` in [`LightingPlugin`](crate::light_plugin::LightingPlugin)), so it
/// should run AFTER both. Those ordering edges CANNOT be expressed here: a `.after(key)`
/// edge needs the target system's `SystemKey`, which is obtainable only at the `add_system`
/// call site inside the OWNING plugin's closure (`SystemKey` is a per-builder descriptor
/// index, and `add_system` does NOT dedup — re-registering `resolve_active_camera` /
/// `light_reconcile` here would double-run them). This is the SAME add-order discipline
/// [`LightingPlugin`](crate::light_plugin::LightingPlugin) documents for `light_reconcile`
/// (after propagation) and [`Render3dPlugin`](crate::render3d_plugin::Render3dPlugin) for
/// `sync_gpu_3d_instances`.
///
/// **Add `CsmPlugin` together with [`CameraPlugin`](boyko_scene::CameraPlugin) and
/// [`LightingPlugin`](crate::light_plugin::LightingPlugin)** so the host schedule resolves
/// the camera + reconciles the sun before the cascade fit. The fit is recomputed every
/// frame from cold owner state, so a loose one-frame stagger (a fit off a one-frame-stale
/// view / sun) is self-correcting — and the default config is DISABLED, so until the owner
/// enables CSM the policy writes the all-zero selection regardless of order.
///
/// When the Inc-1b depth pass + resolve land, the consumer that READS [`ResolvedCsm`]
/// should be co-registered with `resolve_csm_cascades` in one closure so the
/// `.before(depth_pass)` edge is expressible at the call site — exactly as the SSAO /
/// lighting plugins co-register their policy `.before` their consumer.
#[derive(Default)]
pub struct CsmPlugin;

impl Plugin for CsmPlugin {
    fn build(&self, app: &mut App) {
        // The owner-set cold config (default DISABLED — the 0%-gate) + its derived carrier.
        // `resolve_csm_cascades` is the single writer of `ResolvedCsm`; the default
        // `ResolvedCsm` already reads the disabled selection, so the world is correct even
        // before the first policy run.
        app.insert_resource(CsmConfig::default());
        app.insert_resource(ResolvedCsm::default());
        // CSM auto-fit plan (`docs/CSM-AUTOFIT-PLAN.md`) rung C2/D7: the fit's caster input.
        // Inserted here so a bare-`CsmPlugin` world never panics resolving it (`Res::
        // get_param`'s missing-resource panic) even before the owning app wires the
        // unwired `reduce_caster_bounds` reducer (rung C5). `EMPTY` never becomes
        // `is_usable()`, so an unregistered reducer silently keeps the fit at `Fixed`.
        app.insert_resource(CsmCasterBounds::EMPTY);
        // Rung C3: the anti-shimmer latch. MUST be `UNLATCHED`, not `CsmFitState::default()`
        // (which gives `far_k == 0`, a VALID grid cell) — see `CsmFitState`'s own doc.
        app.insert_resource(CsmFitState { far_k: CsmFitState::UNLATCHED });

        // `resolve_csm_cascades` joins `CsmResolveSet` — the by-name ordering seam a
        // future app wiring (rung C5) pins AFTER `CsmFitSet` (`reduce_caster_bounds`), so
        // the caster-bounds fold this frame is visible to the fit resolve this frame.
        app.add_systems_cfg(|b| {
            b.add_system(resolve_csm_cascades).in_set(CsmResolveSet);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::CsmPlugin"
    }
}
