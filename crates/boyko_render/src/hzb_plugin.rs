//! The [`HzbPlugin`] (VG R3 piece 1 step P1-1) — inserts the owner-set [`HzbConfig`] and
//! registers NOTHING else, mirroring
//! [`RenderPathPlugin`](crate::render_path_plugin::RenderPathPlugin)'s system-less shape
//! rather than [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin)'s policy-registering one.

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::hzb_config::HzbConfig;

/// Registers the depth-pyramid config substrate: inserts [`HzbConfig`] (default
/// [`Off`](crate::hzb_config::HzbMode::Off) — the 0%-gate, no image and no build passes).
///
/// # No system, and no derived carrier (unlike [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin))
///
/// `SsaoPlugin`/[`AaPlugin`](crate::aa_plugin::AaPlugin) also insert a derived companion
/// Resource and schedule the cold policy that is its single writer, because quality → variant
/// index is a real map. The HZB map is the identity — the render driver reads
/// [`HzbConfig::enabled`] directly (see [`crate::hzb_config`]'s module doc) — so there is no
/// carrier to own and therefore no policy to run. This plugin is the
/// [`RenderPathPlugin`](crate::render_path_plugin::RenderPathPlugin) shape: seed the config,
/// add no system.
///
/// # No `RenderPathFrozenConsumers` insert either
///
/// `SsaoPlugin` must insert the inert
/// [`RenderPathFrozenConsumers`](crate::render_path_config::RenderPathFrozenConsumers)
/// snapshot because its systems take it as a `Res` param and this kernel has no
/// `Option<Res<R>>`. This plugin registers no system, and the pyramid's arming is captured
/// onto `GBufferTargets` at create time rather than read live per frame, so it neither needs
/// the snapshot nor participates in it.
#[derive(Default)]
pub struct HzbPlugin;

impl Plugin for HzbPlugin {
    fn build(&self, app: &mut App) {
        // The owner-set cold config (default Off — the 0%-gate). Nothing reads it in piece 1;
        // the arming consumer arrives with the image and the build passes.
        app.insert_resource(HzbConfig::default());
    }

    fn name(&self) -> &'static str {
        "boyko_render::HzbPlugin"
    }
}
