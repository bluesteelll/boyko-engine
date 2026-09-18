//! The [`ShadowAtlasPlugin`] (Shadow Inc-1) — inserts the owner-set [`ShadowConfig`] + its
//! derived [`ResolvedShadowAtlas`] companion and registers the cold [`resolve_shadow_atlas`]
//! policy, symmetric with [`CsmPlugin`](crate::csm_plugin::CsmPlugin) /
//! [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin).

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::render_path_config::ResolvedRenderPath;
use crate::shadow_atlas::{
    PunctualResolveSet, ResolvedShadowAtlas, ShadowConfig, resolve_shadow_atlas,
};

/// Registers the sparse-shadow config substrate: inserts [`ShadowConfig`] (default DISABLED —
/// `enabled == false`, the 0%-gate) and its derived [`ResolvedShadowAtlas`] companion, and
/// schedules the cold [`resolve_shadow_atlas`] (the
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
        // Shadow gate SG1: `resolve_shadow_atlas` reads the boot carrier to decide whether this
        // leg set owns the atlas at all; inserted only if ABSENT, exactly as `CsmPlugin` does
        // (a present carrier is the real answer; the `Deferred × Both` default is the identity).
        if !app.world().contains_resource::<ResolvedRenderPath>() {
            app.insert_resource(ResolvedRenderPath::default());
        }
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

#[cfg(test)]
mod tests {
    use core::f32::consts::FRAC_PI_3;

    use boyko_ecs::ecs::core::app::App;
    use boyko_ecs::ecs::core::system::Commands;
    use boyko_math::{Affine3A, Vec3};
    use boyko_scene::{GlobalTransform, Projection, ViewUniform};

    use super::ShadowAtlasPlugin;
    use crate::light::{LightTableDirty, SpotLight};
    use crate::render_path_config::{
        GeometryLegs, RenderPath, RenderPathConfig, RenderPathConsumers, RenderPathDeviceCaps,
        ResolvedRenderPath, resolve_render_path,
    };
    use crate::shadow_atlas::{PunctualSlotAssignment, ResolvedShadowAtlas, ShadowConfig};
    use crate::shadow_marker::CastsPunctualShadow;

    /// One `ShadowAtlasPlugin` world: an enabled `ShadowConfig`, a perspective camera and one
    /// `CastsPunctualShadow` spot, run for two frames under `carrier` (`None` = the plugin's own
    /// default). Returns the published atlas and slot handoff. `LightTableDirty` and
    /// `PunctualSlotAssignment` are `LightingPlugin`'s resources; they are inserted by hand so
    /// this world needs no lighting fold.
    fn run_atlas_world(carrier: Option<ResolvedRenderPath>) -> (ResolvedShadowAtlas, PunctualSlotAssignment) {
        let mut app = App::new();
        if let Some(c) = carrier {
            app.insert_resource(c);
        }
        app.insert_resource(LightTableDirty(false));
        app.insert_resource(PunctualSlotAssignment::default());
        app.add_plugin(ShadowAtlasPlugin);
        if let Some(c) = carrier {
            assert_eq!(
                *app.world().resource::<ResolvedRenderPath>(),
                c,
                "ShadowAtlasPlugin overwrote a carrier that was already present"
            );
        }
        app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
        let eye = Vec3::new(0.0, 2.0, 6.0);
        let world_xf = Affine3A::look_at_rh(eye, Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
        let proj = Projection::Perspective { fov_y: FRAC_PI_3, aspect: 16.0 / 9.0, near: 0.1, far: 1000.0 };
        app.insert_resource(ViewUniform::from_camera(world_xf, proj));
        app.world_mut().run_system(|mut cmds: Commands| {
            cmds.spawn(SpotLight::new([0.0, 3.0, 0.0], [0.0, -1.0, 0.0], [1.0; 3], 200.0, 8.0, 15.0, 30.0))
                .insert(GlobalTransform::IDENTITY)
                .insert(CastsPunctualShadow);
        });
        app.run_n(2);
        (
            *app.world().resource::<ResolvedShadowAtlas>(),
            *app.world().resource::<PunctualSlotAssignment>(),
        )
    }

    /// Shadow gate SG1 for the atlas: a pre-inserted mesh-less carrier survives `add_plugin`, and
    /// under it `resolve_shadow_atlas` takes the config-disabled arm — `DISABLED` AND the EMPTY
    /// slot handoff, the half that closes the light-table route. The control is the same world
    /// under the plugin's default carrier, which must fit a live atlas and slot the spot;
    /// without it the mesh-less result could come from a spot that never qualified.
    #[test]
    fn a_mesh_less_carrier_publishes_the_disabled_atlas_and_no_slot() {
        let sdf_only = resolve_render_path(
            &RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Sdf },
            RenderPathConsumers::default(),
            RenderPathDeviceCaps::new(true),
        )
        .0;
        assert!(!sdf_only.mesh_shadow_producers(), "the fixture must be a mesh-less leg set");

        let (live, live_slots) = run_atlas_world(None);
        assert_eq!(live.mode_word, 1, "control: the default carrier fits the spot");
        assert_ne!(live_slots, PunctualSlotAssignment::EMPTY, "control: the spot won a slot");

        let (off, off_slots) = run_atlas_world(Some(sdf_only));
        assert_eq!(off, ResolvedShadowAtlas::DISABLED);
        assert_eq!(off_slots, PunctualSlotAssignment::EMPTY);
    }
}
