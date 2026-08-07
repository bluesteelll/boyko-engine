//! The [`OcclusionPlugin`] (VG R3 piece 4 rung P4-4) — inserts the owner-set [`OcclusionConfig`]
//! and registers NOTHING else, mirroring [`HzbPlugin`](crate::hzb_plugin::HzbPlugin)'s
//! system-less shape.

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::occlusion_config::OcclusionConfig;

/// Registers the occlusion-decision config substrate: inserts [`OcclusionConfig`] (default
/// [`Off`](crate::occlusion_config::OcclusionMode::Off) — the 0%-gate, no split and no late
/// passes).
///
/// # No system, and no derived carrier
///
/// [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin)/[`AaPlugin`](crate::aa_plugin::AaPlugin) also
/// insert a derived companion Resource and schedule the cold policy that is its single writer,
/// because quality → variant index is a real map. This map is the identity — the host reads
/// [`OcclusionConfig::enabled`] directly — so there is no carrier to own and therefore no policy
/// to run. This is [`HzbPlugin`](crate::hzb_plugin::HzbPlugin)'s shape: seed the config, add no
/// system.
///
/// # Why composing it unconditionally is safe
///
/// The default is `Off`, which is the 0%-gate: `path_vb_occlusion_split()` is false through its
/// first conjunct, so a host that composes this plugin and sets nothing renders byte-identically
/// to one that never heard of it. That is what lets `EnginePlugins` carry it beside `HzbPlugin`
/// without re-blessing a single golden.
///
/// # The DIAGNOSTIC axis is deliberately NOT composed here
///
/// `boyko_app::OcclusionForce` (defer nothing / defer everything) is a measurement instrument, not
/// an owner knob. It is inserted by the fixtures that need it and read through `try_resource`, so
/// an absent Resource IS its default — the same treatment an absent `HzbConfig` gets.
#[derive(Default)]
pub struct OcclusionPlugin;

impl Plugin for OcclusionPlugin {
    fn build(&self, app: &mut App) {
        // The owner-set cold config (default Off — the 0%-gate). The consumer is the host's frame
        // loop, which reads it live per frame beside `HzbConfig`.
        app.insert_resource(OcclusionConfig::default());
    }

    fn name(&self) -> &'static str {
        "boyko_render::OcclusionPlugin"
    }
}
