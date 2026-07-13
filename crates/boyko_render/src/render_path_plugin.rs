//! The [`RenderPathPlugin`] — inserts the owner-set [`RenderPathConfig`] + its derived
//! [`ResolvedRenderPath`] companion, mirroring [`AaPlugin`](crate::aa_plugin::AaPlugin) /
//! [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin). UNLIKE those two, it registers NO per-frame
//! policy system: Decision 1 (see [`crate::render_path_config`]'s module doc) commits the
//! resolved carrier exactly once, at `WindowHost::boot`, not every frame.

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::render_path_config::{RenderPathConfig, ResolvedRenderPath};

/// Registers the render-path config substrate: inserts [`RenderPathConfig`] (default
/// `Deferred + Both` — the byte-identity anchor) and its derived [`ResolvedRenderPath`]
/// companion (the resolve of the default config — Deferred + Both, no consumers armed, no
/// degrades).
///
/// # No per-frame policy system (unlike [`AaPlugin`](crate::aa_plugin::AaPlugin) / [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin))
///
/// `AaPlugin`/`SsaoPlugin` register a cold system (`resolve_aa_policy`/`resolve_ssao_policy`)
/// that re-derives their resolved carrier every frame from live config. `ResolvedRenderPath` is
/// DIFFERENT: Decision 1 forbids a per-frame path/leg re-derivation (it would re-allocate
/// fixed-size images/pipelines mid-stream — the `ssaa_armed` reason). `boyko_app::runner` calls
/// [`crate::render_path_config::resolve_render_path`] directly, exactly once at boot, and
/// OVERWRITES this plugin's default `ResolvedRenderPath` with the real boot-resolved value
/// (mirroring how the runner overrides `DdgiCaps`/`RayCaps` post-boot) — so this plugin's insert
/// is only the "correct before boot runs" placeholder a headless/offscreen world (which never
/// calls the windowed runner's boot sequence) keeps forever.
#[derive(Default)]
pub struct RenderPathPlugin;

impl Plugin for RenderPathPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RenderPathConfig::default());
        app.insert_resource(ResolvedRenderPath::default());
    }

    fn name(&self) -> &'static str {
        "boyko_render::RenderPathPlugin"
    }
}
