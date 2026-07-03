//! The [`ShadowAtlasPlugin`] (Shadow Inc-1) — inserts the owner-set [`ShadowConfig`] + its
//! derived [`ResolvedShadowAtlas`] companion and registers the cold [`resolve_shadow_atlas`]
//! policy, symmetric with [`CsmPlugin`](crate::csm_plugin::CsmPlugin) /
//! [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin).

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::shadow_atlas::{
    PunctualResolveSet, ResolvedShadowAtlas, ShadowConfig, resolve_shadow_atlas,
};

/// Registers the sparse-shadow config substrate: inserts [`ShadowConfig`] (default DISABLED —
/// `enabled == false`, the 0%-gate) and its derived [`ResolvedShadowAtlas`] companion, and
/// schedules the cold [`resolve_shadow_atlas`](crate::shadow_atlas::resolve_shadow_atlas) (the
/// SINGLE writer of `ResolvedShadowAtlas`).
///
/// # Mirror of [`CsmPlugin`](crate::csm_plugin::CsmPlugin)
///
/// The CSM plugin inserts the owner-set [`CsmConfig`](crate::csm_config::CsmConfig) plus the
/// derived [`ResolvedCsm`](crate::csm_config::ResolvedCsm) and registers the cold
/// `resolve_csm_cascades`. This plugin is the spot/point analogue: owner-set `ShadowConfig`
/// plus derived `ResolvedShadowAtlas` plus the cold `resolve_shadow_atlas`.
///
/// # Add-order contract (cross-plugin ordering vs. camera + light)
///
/// `resolve_shadow_atlas` reads the engine-derived
/// [`ViewUniform`](boyko_scene::ViewUniform) (written by `resolve_active_camera` in
/// [`CameraPlugin`](boyko_scene::CameraPlugin)) and the live
/// [`SpotLight`](crate::light::SpotLight) + [`GlobalTransform`](boyko_scene::GlobalTransform)
/// poses (reconciled by `light_reconcile` in
/// [`LightingPlugin`](crate::light_plugin::LightingPlugin)), so it should run AFTER both. Those
/// ordering edges CANNOT be expressed here: a `.after(key)` edge needs the target system's
/// `SystemKey`, obtainable only at the `add_system` call site inside the OWNING plugin's
/// closure (and `add_system` does NOT dedup — re-registering those systems here would
/// double-run them). This is the SAME add-order discipline
/// [`CsmPlugin`](crate::csm_plugin::CsmPlugin) documents.
///
/// **Add `ShadowAtlasPlugin` together with [`CameraPlugin`](boyko_scene::CameraPlugin) and
/// [`LightingPlugin`](crate::light_plugin::LightingPlugin)** so the host schedule resolves the
/// camera + reconciles the spot poses before the atlas fit. The fit is recomputed every frame
/// from cold owner state, so a loose one-frame stagger (a fit off a one-frame-stale view /
/// pose) is self-correcting — and the default config is DISABLED, so until the owner enables
/// shadows the policy writes the all-zero selection regardless of order.
///
/// When the Inc-1-GPU depth pass + resolve land, the consumer that READS
/// [`ResolvedShadowAtlas`] (and the light-table assembly that packs each spot's slot via
/// [`pack_atlas_slot`](crate::shadow_atlas::pack_atlas_slot)) should be co-registered with
/// `resolve_shadow_atlas` in one closure so the `.before(depth_pass)` edge is expressible at
/// the call site — exactly as the CSM / SSAO / lighting plugins co-register their policy
/// `.before` their consumer.
#[derive(Default)]
pub struct ShadowAtlasPlugin;

impl Plugin for ShadowAtlasPlugin {
    fn build(&self, app: &mut App) {
        // The owner-set cold config (default DISABLED — the 0%-gate) + its derived carrier.
        // `resolve_shadow_atlas` is the single writer of `ResolvedShadowAtlas`; the default
        // `ResolvedShadowAtlas` already reads the disabled selection, so the world is correct
        // even before the first policy run.
        app.insert_resource(ShadowConfig::default());
        app.insert_resource(ResolvedShadowAtlas::default());
        // NOTE: the `PunctualSlotAssignment` resolve → light-table handoff resource is inserted by
        // `LightingPlugin` (the plugin that owns its READER, `collect_lights`), NOT here — so a
        // lighting-only world (LE gate tests) has the empty handoff even without this plugin. This
        // plugin's `resolve_shadow_atlas` is only its WRITER.

        // `resolve_shadow_atlas` joins `PunctualResolveSet` — the by-name ordering seam that pins
        // it BEFORE the light-table fold (`collect_lights` runs `.after_set(PunctualResolveSet)` in
        // `LightingPlugin`). Set-to-set ordering is add-order-independent and cross-schedule-safe
        // (both systems land in `CoreSchedule::Main`), so the resolve → publish → collect chain
        // completes within ONE frame — correct even on a moving camera, where the priority ranking
        // (`range²/dist²`) can reorder which light wins base 0 (the R6 `CameraSet` precedent).
        app.add_systems_cfg(|b| {
            b.add_system(resolve_shadow_atlas).in_set(PunctualResolveSet);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::ShadowAtlasPlugin"
    }
}
