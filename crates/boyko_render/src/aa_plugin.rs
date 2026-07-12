//! The [`AaPlugin`] — inserts the owner-set [`AaConfig`] + its derived [`ResolvedAa`]
//! companion and registers the cold [`resolve_aa_policy`] at the gather/setup boundary,
//! symmetric with [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin). Also inserts the TAA (Stage 4)
//! substrate Resources — [`JitterState`] and [`TaaState`] — so `boyko_app::runner`'s per-frame
//! reads/writes never panic on a missing Resource, mirroring how `AaConfig`/`ResolvedAa` are
//! seeded here rather than left to the runner to insert-if-absent.

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::aa_config::{AaConfig, ResolvedAa, resolve_aa_policy};
use crate::taa_jitter::JitterState;
use crate::taa_state::TaaState;

/// Registers the anti-aliasing config substrate: inserts [`AaConfig`] (default
/// [`Off`](crate::aa_config::AaMode::Off) — the 0%-gate) and its derived [`ResolvedAa`]
/// companion, and schedules the cold [`resolve_aa_policy`](crate::aa_config::resolve_aa_policy)
/// (the SINGLE writer of `ResolvedAa`).
///
/// # Mirror of [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin)
///
/// The SSAO plugin inserts the owner-set `SsaoConfig` plus the derived `ResolvedSsao` and
/// registers `resolve_ssao_policy`. This plugin is the AA analogue: owner-set `AaConfig` plus
/// derived `ResolvedAa` plus the cold `resolve_aa_policy`.
///
/// # Ordering vs. the render point
///
/// `resolve_aa_policy` must run BEFORE the render driver reads [`ResolvedAa`] to build the AA
/// activation. The selection is recomputed every frame, so a loose one-frame stagger before
/// the consumer runs is self-correcting (the config is cold owner state). Unlike SSAO — whose
/// live consumer is still a follow-up — the AA consumer is wired in the host frame loop
/// (mirroring how the shadow-denoise mode is threaded into the backend scene).
#[derive(Default)]
pub struct AaPlugin;

impl Plugin for AaPlugin {
    fn build(&self, app: &mut App) {
        // The owner-set cold config (default Off — the 0%-gate) + its derived carrier.
        // `resolve_aa_policy` is the single writer of `ResolvedAa` (the one-producer write
        // discipline). The default `ResolvedAa` already reads the no-pass selection, so the
        // world is correct even before the first policy run.
        app.insert_resource(AaConfig::default());
        app.insert_resource(ResolvedAa::default());
        // Anti-aliasing Stage 4 (TAA): the jitter-phase + history-reset substrate Resources.
        // Both default to the 0%-gate shape (`JitterState { phase: 0, armed: false }`,
        // `TaaState { reset: false, .. }`) — a world that never selects `AaMode::Taa` never
        // observes a nonzero phase or a forced reset.
        app.insert_resource(JitterState::default());
        app.insert_resource(TaaState::default());

        app.add_systems_cfg(|b| {
            b.add_system(resolve_aa_policy);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::AaPlugin"
    }
}
