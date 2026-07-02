//! The [`LightingPlugin`] (standard-library Phase S4) — registers the per-frame
//! light-pose reconcile + the light-table collection in one builder closure so
//! the ordering edge between them is expressible, plus the Axis-2 `LightEnabled`
//! runtime on/off machinery (seed system + eviction hooks).

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::light::{DirectionalLight, LightTableDirty, PointLight, SkyLight, SpotLight};
use crate::light_policy::{LightStats, select_lighting_cull};
use crate::light_reconcile::light_reconcile;
use crate::light_system::{LightTableGeneration, collect_lights, evict_light, light_seed_state};

/// Registers [`light_reconcile`](crate::light_reconcile::light_reconcile) BEFORE
/// [`collect_lights`](crate::light_system::collect_lights), plus the
/// [`LightEnabled`](crate::light::LightEnabled) runtime on/off machinery (the
/// [`LightSeedState`](crate::light_system::LightSeedState) exclusive seed and the
/// `on_remove` eviction hooks).
///
/// # Registration-first ordering invariant (hooks)
///
/// The eviction hooks ([`evict_light`](crate::light_system::evict_light)) are registered
/// as the **FIRST** action of `build`, BEFORE any system registration or resource insert.
/// `register_component_hooks` panics `AlreadyArchetyped` if the component was EVER placed
/// in any archetype of any world in the process (the gate is process-global and never
/// reset). The light components carry `#[require(Transform, GlobalTransform)]`, so the
/// first light spawn archetypes them immediately. Therefore: **no light component may be
/// archetyped before `LightingPlugin::build` runs — add `LightingPlugin` before any
/// light-spawning plugin/system in app setup.** A violation surfaces as the loud,
/// immediate `AlreadyArchetyped` panic at build time (fail-fast, not silent missing
/// eviction).
///
/// Only `on_remove` is registered (4 hooks, not 8): a full despawn fires `on_remove` per
/// component too, so `on_remove` alone catches both the component-remove and the
/// whole-entity-despawn classes.
///
/// # Why one closure
///
/// Intra-schedule ordering edges are keyed by `SystemKey`, obtainable only at the
/// `add_system` call site, so the `.before` edge can be expressed only where BOTH
/// systems are registered. This plugin therefore co-registers `light_reconcile`,
/// `collect_lights`, and the seed together, exactly as
/// [`CameraPlugin`](boyko_scene::CameraPlugin) co-registers `propagate_transforms`
/// + `resolve_active_camera`.
///
/// # Add-order contract (cross-schedule ordering vs. propagation)
///
/// `light_reconcile` reads the propagated `GlobalTransform`, so it must run AFTER
/// `propagate_transforms`. That edge cannot be expressed here (the propagation
/// system's key lives in `TransformPlugin` / `CameraPlugin`). **Add
/// `LightingPlugin` together with `TransformPlugin` or `CameraPlugin`** so the
/// host schedule runs propagation first. The `Changed<GlobalTransform>` gate on
/// `light_reconcile` makes a loose one-frame ordering stagger self-correcting (a
/// stale read re-fires next frame), but the intended order is propagate →
/// reconcile → seed → collect.
#[derive(Default)]
pub struct LightingPlugin;

impl Plugin for LightingPlugin {
    fn build(&self, app: &mut App) {
        // FIRST action — register the gate-5 eviction hooks before any light component can
        // be archetyped (see the registration-first invariant in the type docs). Only
        // `on_remove` (it subsumes despawn). The `AlreadyArchetyped` panic is the
        // fail-fast if a light was spawned before this plugin was added.
        let world = app.world_mut();
        world.register_component_hooks::<DirectionalLight>().on_remove(evict_light).finish();
        world.register_component_hooks::<SkyLight>().on_remove(evict_light).finish();
        world.register_component_hooks::<PointLight>().on_remove(evict_light).finish();
        world.register_component_hooks::<SpotLight>().on_remove(evict_light).finish();

        // The structural-change channel (Decision 2): catches tickless toggles and
        // removals/despawns that the `Changed` gate cannot see.
        app.insert_resource(LightTableDirty(false));

        // Host plan D5: the writer-side staging generation `collect_lights` bumps on
        // every actual rewrite; ringed hosts gate their per-slot staging writes on it.
        app.insert_resource(LightTableGeneration(0));

        // P1: the cold cost-model carrier for the lighting StrategyPolicy. Default starts
        // the band OFF (matching `LightingConfig::clusters_enabled`'s `false` default), so
        // a default-Manual world is byte-identical to pre-P1. `select_lighting_cull` is its
        // single writer (the Part 2.2 write discipline).
        app.insert_resource(LightStats::default());

        // Co-register so the `.before` ordering edges between the keys are expressible in a
        // single closure (mirrors `CameraPlugin`). The seed is an EXCLUSIVE
        // (`&mut EcsMaster`) system that flips `LightEnabled` bits immediately, so the bits
        // + the dirty mark are live in the SAME pass before `collect_lights` folds (W2).
        // The cross-frame seed state (the eight CACHED light-id systems + the first-run flag
        // + the reused scratch) is owned here, in the registering closure; capturing it once
        // is what makes the per-frame system `initialize` cost amortise to zero (W1).
        // `select_lighting_cull` (P1) also runs `.before(collect)` so this frame's banded
        // cluster decision feeds the header fold (no one-frame staleness).
        app.add_systems_cfg(|b| {
            let collect = b.add_system(collect_lights).key();
            b.add_system(light_reconcile).before(collect);
            b.add_system(select_lighting_cull).before(collect);
            let mut seed_state = light_seed_state();
            b.add_system(move |w: &mut boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster| {
                seed_state.seed(w);
            })
            .before(collect);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::LightingPlugin"
    }
}
