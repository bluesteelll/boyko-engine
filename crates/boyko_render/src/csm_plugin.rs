//! The [`CsmPlugin`] (CSM Inc-1a) — inserts the owner-set [`CsmConfig`] + its derived
//! [`ResolvedCsm`] companion and registers the cold [`resolve_csm_cascades`] policy,
//! symmetric with [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin) /
//! [`LightingPlugin`](crate::light_plugin::LightingPlugin).

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::csm_config::{
    CsmCasterBounds, CsmConfig, CsmFitState, CsmResolveSet, ResolvedCsm, resolve_csm_cascades,
};
use crate::render_path_config::ResolvedRenderPath;

/// Registers the CSM config substrate: inserts [`CsmConfig`] (default DISABLED —
/// `cascade_count == 0`, the 0%-gate) and its derived [`ResolvedCsm`] companion, and
/// schedules the cold [`resolve_csm_cascades`]
/// (the SINGLE writer of `ResolvedCsm`).
///
/// # Mirror of [`SsaoPlugin`](crate::ssao_plugin::SsaoPlugin)
///
/// The SSAO plugin inserts the owner-set [`SsaoConfig`](crate::ssao_config::SsaoConfig)
/// plus the derived [`ResolvedSsao`](crate::ssao_config::ResolvedSsao) and registers the
/// cold `resolve_ssao_policy`. This plugin is the CSM analogue: owner-set `CsmConfig` plus
/// derived `ResolvedCsm` plus the cold `resolve_csm_cascades`.
///
/// # Ordering contract (cross-plugin ordering vs. camera + light)
///
/// `resolve_csm_cascades` reads the engine-derived [`ViewUniform`](boyko_scene::ViewUniform)
/// (written by `resolve_active_camera` in
/// [`CameraPlugin`](boyko_scene::CameraPlugin)) and the primary
/// [`DirectionalLight`](crate::light::DirectionalLight) direction (reconciled by
/// `light_reconcile` in [`LightingPlugin`](crate::light_plugin::LightingPlugin)), so it
/// must run AFTER both. Those edges CANNOT be expressed by key here: a `.after(key)` edge
/// needs the target system's `SystemKey`, which is obtainable only at the `add_system` call
/// site inside the OWNING plugin's closure (`SystemKey` is a per-builder descriptor index,
/// and `add_system` does NOT dedup — re-registering `resolve_active_camera` /
/// `light_reconcile` here would double-run them). They are pinned by name instead: the fit
/// joins [`CsmResolveSet`], and the composing app
/// configures `CsmResolveSet.after(CameraSet::Resolve)` and
/// `CsmResolveSet.after(LightReconcileSet)` ([`LightReconcileSet`](crate::light_reconcile::LightReconcileSet))
/// — `boyko_app::EnginePlugins` does. They live in the app, not here: each names a set this
/// plugin does not populate, and an edge naming a memberless set warns `boyko-W1501`.
///
/// **Add `CsmPlugin` together with [`CameraPlugin`](boyko_scene::CameraPlugin) and
/// [`LightingPlugin`](crate::light_plugin::LightingPlugin)**, and declare those two set edges
/// if you compose them by hand. The default config is DISABLED, so until the owner enables
/// CSM the policy writes the all-zero selection regardless of order.
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
        // Shadow gate SG1: `resolve_csm_cascades` reads the boot carrier to decide whether this
        // leg set owns the cascades at all, and this kernel has no `Option<Res<R>>`, so a
        // bare-`CsmPlugin` world needs one. Inserted only if ABSENT: a carrier the host (or
        // `RenderPathPlugin`) already put there is the real answer and must survive. The
        // default is `Deferred × Both`, whose `mesh_shadow_producers()` is `true`, so the new
        // arm is the identity in every world that never boots a path.
        if !app.world().contains_resource::<ResolvedRenderPath>() {
            app.insert_resource(ResolvedRenderPath::default());
        }

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

#[cfg(test)]
mod tests {
    use core::f32::consts::FRAC_PI_3;

    use boyko_ecs::ecs::core::app::App;
    use boyko_ecs::ecs::core::system::Commands;
    use boyko_math::{Affine3A, Vec3};
    use boyko_scene::{Projection, ViewUniform};

    use super::CsmPlugin;
    use crate::csm_config::{CsmConfig, CsmFit, ResolvedCsm, resolve_csm};
    use crate::light::DirectionalLight;
    use crate::render_path_config::{
        GeometryLegs, RenderPath, RenderPathConfig, RenderPathConsumers, RenderPathDeviceCaps,
        ResolvedRenderPath, resolve_render_path,
    };

    /// CSM auto-fit plan (`docs/CSM-AUTOFIT-PLAN.md`) rung C5, test T15 (disposition
    /// finding I): a world that adds ONLY `CsmPlugin` — no `CsmCasterScratch`, and none
    /// of `reduce_caster_bounds` / `CsmFitSet` / `CsmResolveSet`'s app wiring (that
    /// lives in `boyko_app::plugins`, rung C5, not here) — must NOT panic in
    /// `Res::get_param` fetching `CsmCasterBounds` / `CsmFitState` (the OLD design read
    /// `CsmCasterScratch` directly and did panic there), and must degrade to exactly the
    /// fit a direct `resolve_csm(cfg, view, sun, CsmFit::NONE)` call produces — a silent
    /// no-op, never a crash (D7).
    #[test]
    fn bare_csm_plugin_world_runs_without_the_reducer() {
        let mut app = App::new();
        app.add_plugin(CsmPlugin);

        // Scene-realistic `CsmConfig` — the same `cascade_count: 3` idiom every in-tree
        // scene uses (e.g. `examples/room.rs:29`) — so the fit is actually EXERCISED
        // rather than short-circuited by the plugin's own disabled (`cascade_count == 0`)
        // seed. `fit_mode` is left untouched at the plugin default `Fixed`, which is
        // exactly the "reducer not registered" scenario this test pins.
        let cfg = CsmConfig { cascade_count: 3, ..CsmConfig::default() };
        app.insert_resource(cfg);

        let eye = Vec3::new(0.0, 2.0, 0.0);
        let forward = Vec3::new(0.0, 0.0, -1.0);
        let world_xf = Affine3A::look_at_rh(eye, eye + forward, Vec3::new(0.0, 1.0, 0.0));
        let proj = Projection::Perspective { fov_y: FRAC_PI_3, aspect: 16.0 / 9.0, near: 0.1, far: 1000.0 };
        let view = ViewUniform::from_camera(world_xf, proj);
        app.insert_resource(view);

        let sun_dir = [0.3_f32, -1.0, 0.2];
        app.world_mut().run_system(move |mut cmds: Commands| {
            cmds.spawn(DirectionalLight { direction: sun_dir, color: [1.0; 3], illuminance: 10_000.0 });
        });

        // 2 frames — the "silent no-op" must hold every frame, not just frame 0.
        app.run_n(2);

        let got = *app.world().resource::<ResolvedCsm>();
        let want = resolve_csm(&cfg, &view, sun_dir, CsmFit::NONE);
        assert_eq!(
            got, want,
            "a bare CsmPlugin world (no reducer wired) must degrade to today's Fixed fit, \
             byte-identical to a direct CsmFit::NONE call"
        );
    }

    /// Shadow gate SG1, the plugin half. A boot carrier inserted BEFORE `add_plugin` survives
    /// it (the plugin inserts its default only when the resource is absent), and a mesh-less
    /// carrier drives `resolve_csm_cascades` to `DISABLED` in the very world
    /// [`bare_csm_plugin_world_runs_without_the_reducer`] shows producing a live fit: CSM
    /// enabled, a sun, a perspective camera. The live-fit control is re-derived here, so the
    /// `DISABLED` result cannot come from inputs that would not have fitted anyway.
    #[test]
    fn a_pre_inserted_mesh_less_carrier_survives_and_disables_the_cascades() {
        let sdf_only = resolve_render_path(
            &RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Sdf },
            RenderPathConsumers::default(),
            RenderPathDeviceCaps::new(true),
        )
        .0;
        assert!(!sdf_only.mesh_shadow_producers(), "the fixture must be a mesh-less leg set");
        assert_ne!(sdf_only, ResolvedRenderPath::default());

        let mut app = App::new();
        app.insert_resource(sdf_only);
        app.add_plugin(CsmPlugin);
        assert_eq!(
            *app.world().resource::<ResolvedRenderPath>(),
            sdf_only,
            "CsmPlugin overwrote a carrier that was already present"
        );

        let cfg = CsmConfig { cascade_count: 3, ..CsmConfig::default() };
        app.insert_resource(cfg);
        let eye = Vec3::new(0.0, 2.0, 0.0);
        let world_xf =
            Affine3A::look_at_rh(eye, eye + Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.0, 1.0, 0.0));
        let proj = Projection::Perspective { fov_y: FRAC_PI_3, aspect: 16.0 / 9.0, near: 0.1, far: 1000.0 };
        let view = ViewUniform::from_camera(world_xf, proj);
        app.insert_resource(view);
        let sun_dir = [0.3_f32, -1.0, 0.2];
        app.world_mut().run_system(move |mut cmds: Commands| {
            cmds.spawn(DirectionalLight { direction: sun_dir, color: [1.0; 3], illuminance: 10_000.0 });
        });
        app.run_n(2);

        assert_eq!(
            resolve_csm(&cfg, &view, sun_dir, CsmFit::NONE).csm_mode_word,
            1,
            "control: the same inputs fit live cascades under a mesh-carrying leg set"
        );
        assert_eq!(
            *app.world().resource::<ResolvedCsm>(),
            ResolvedCsm::DISABLED,
            "a leg set without mesh-shadow producers must publish the DISABLED selection"
        );
    }
}
