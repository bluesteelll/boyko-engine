//! The [`DdgiPlugin`] (SDFDDGI I0) — inserts the owner-set [`DdgiConfig`] + its derived
//! [`ResolvedDdgi`] companion and registers the cold [`resolve_ddgi_grid`] policy under
//! [`DdgiResolveSet`], symmetric with [`CsmPlugin`](crate::csm_plugin::CsmPlugin) /
//! [`ShadowAtlasPlugin`](crate::shadow_plugin::ShadowAtlasPlugin).

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::ddgi_config::{DdgiConfig, DdgiResolveSet, ResolvedDdgi, resolve_ddgi_grid};

/// Registers the DDGI config substrate: inserts [`DdgiConfig`] (default DISABLED —
/// `ddgi_indirect == false`, the 0%-gate) and its derived [`ResolvedDdgi`] companion, and
/// schedules the cold [`resolve_ddgi_grid`](crate::ddgi_config::resolve_ddgi_grid) (the
/// SINGLE writer of `ResolvedDdgi`) under [`DdgiResolveSet`].
///
/// # Mirror of [`ShadowAtlasPlugin`](crate::shadow_plugin::ShadowAtlasPlugin)
///
/// The shadow-atlas plugin inserts the owner-set
/// [`ShadowConfig`](crate::shadow_atlas::ShadowConfig) plus the derived
/// [`ResolvedShadowAtlas`](crate::shadow_atlas::ResolvedShadowAtlas) and registers the
/// cold `resolve_shadow_atlas` in [`PunctualResolveSet`](crate::shadow_atlas::PunctualResolveSet).
/// This plugin is the GI analogue: owner-set `DdgiConfig` plus derived `ResolvedDdgi` plus
/// the cold `resolve_ddgi_grid` in `DdgiResolveSet`.
///
/// # Camera-independent (Decision D1) — no cross-plugin add-order dependency
///
/// Unlike the CSM / atlas resolves (which read the engine-derived
/// [`ViewUniform`](boyko_scene::ViewUniform) + reconciled light poses and so must run
/// AFTER the camera + light plugins), `resolve_ddgi_grid` reads ONLY the cold
/// [`DdgiConfig`] — the grid is WORLD-FIXED. So it has NO ordering dependency on the
/// camera or light reconcile; it may be added in any order. The default config is
/// DISABLED, so until the owner enables GI the policy writes the all-zero selection
/// regardless of order.
///
/// # The light-gate sync is app-wired (matches the CSM / punctual gates)
///
/// [`sync_ddgi_light_gate`](crate::ddgi_config::sync_ddgi_light_gate) (the SOLE writer of
/// the LightBuf word-7 bit-4 gate) is NOT registered here: it bridges this plugin's
/// [`DdgiConfig`] and the lighting plugin's `LightingConfig` / `LightTableDirty`, so only
/// the composing app (which adds BOTH) may register it — after `resolve_ddgi_grid`, in the
/// same builder closure as `sync_csm_light_gate` / `sync_punctual_light_gate`.
#[derive(Default)]
pub struct DdgiPlugin;

impl Plugin for DdgiPlugin {
    fn build(&self, app: &mut App) {
        // The owner-set cold config (default DISABLED — the 0%-gate) + its derived carrier.
        // `resolve_ddgi_grid` is the single writer of `ResolvedDdgi`; the default
        // `ResolvedDdgi` already reads the disabled selection, so the world is correct even
        // before the first policy run.
        app.insert_resource(DdgiConfig::default());
        app.insert_resource(ResolvedDdgi::default());

        // `resolve_ddgi_grid` joins `DdgiResolveSet` — the by-name ordering seam a consumer
        // pins BEFORE (via `.after_set(DdgiResolveSet)`). Set-to-set ordering is
        // add-order-independent; the grid is camera-independent so there is no camera/light
        // edge to express (see the doc above).
        app.add_systems_cfg(|b| {
            b.add_system(resolve_ddgi_grid).in_set(DdgiResolveSet);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::DdgiPlugin"
    }
}
