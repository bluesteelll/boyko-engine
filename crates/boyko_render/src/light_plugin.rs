//! The [`LightingPlugin`] (standard-library Phase S4) — registers the per-frame
//! light-pose reconcile + the light-table collection in one builder closure so
//! the ordering edge between them is expressible, plus the Axis-2 `LightEnabled`
//! runtime on/off machinery (seed system + eviction hooks).

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::light::{DirectionalLight, LightTableDirty, PointLight, SkyLight, SpotLight};
use crate::light_policy::{LightStats, select_lighting_cull};
use crate::light_reconcile::{LightReconcileSet, light_reconcile};
use crate::light_system::{
    LightCollectSet, LightSeedSet, LightTableGeneration, collect_lights, evict_light,
    light_seed_state,
};
use crate::shadow_atlas::{PunctualResolveSet, PunctualSlotAssignment};

/// Registers [`light_reconcile`] BEFORE
/// [`collect_lights`], plus the
/// [`LightEnabled`](crate::light::LightEnabled) runtime on/off machinery (the
/// [`LightSeedState`](crate::light_system::LightSeedState) exclusive seed and the
/// `on_remove` eviction hooks).
///
/// # Registration-first ordering invariant (hooks)
///
/// The eviction hooks ([`evict_light`]) are registered
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
/// # Ordering contract (vs. propagation, and the pose's readers outside this plugin)
///
/// `light_reconcile` reads the propagated `GlobalTransform`, so it must run AFTER
/// `propagate_transforms`. That edge cannot be expressed by key here (the propagation
/// system's key lives in `TransformPlugin` / `CameraPlugin`), so it is pinned by name:
/// `light_reconcile` joins [`LightReconcileSet`], and the composing host configures
/// `LightReconcileSet.after(CameraSet::Resolve)` (`boyko_app::EnginePlugins` does; a host
/// composing this plugin by hand declares the same edge). Add-order is NOT a pin: unordered,
/// the pair is decided by the executor's wave packing, and the `Changed<GlobalTransform>`
/// gate does not make a wrong order self-correcting — measured, the frame-0 light table
/// carried the identity pose's direction for a sun spawned with a posed `Transform` and an
/// identity `GlobalTransform` (`docs/OPEN-QUESTIONS.md`, 2026-09-19 (b)); wherever the wrong
/// order holds on a later frame, a moving light's pose trails its transform by one frame for
/// as long as it moves. The same set orders the pose's readers outside this plugin after the
/// reconcile (the CSM fit and the punctual atlas resolve; see [`LightReconcileSet`]). Within
/// this plugin the order is reconcile → seed → collect, with `select_lighting_cull` between
/// the seed and collect; `select_lighting_cull` and `light_reconcile` share no data (the cull
/// counts `IsEnabled<LightEnabled>` rows under `With<PointLight>` / `With<SpotLight>` and
/// reads no light field; the reconcile writes only `position` / `direction`), so that pair
/// is left unordered.
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

        // The entity-keyed punctual atlas-base handoff (Inc-1-GPU): its READER is
        // `collect_lights` (owned here), so this plugin guarantees the resource exists even in a
        // lighting-only world (LE gate tests, no `ShadowAtlasPlugin`). Default EMPTY — the 0%-gate,
        // so `collect_lights` reads `SLOT_NONE` for every light and packs NOTHING until the shadow
        // resolve (in `ShadowAtlasPlugin`) publishes winners. The single WRITER is
        // `resolve_shadow_atlas`.
        app.insert_resource(PunctualSlotAssignment::default());

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
        // cluster decision feeds the header fold (no one-frame staleness), and after the seed so
        // the lights it counts are this frame's too.
        app.add_systems_cfg(|b| {
            // `collect_lights` runs `.after_set(PunctualResolveSet)` — the by-name cross-plugin
            // edge that guarantees the punctual shadow resolve (in `ShadowAtlasPlugin`) has
            // published `PunctualSlotAssignment` BEFORE this fold reads it to pack each light's
            // atlas base. Set-to-set ordering is add-order-independent (holds even though
            // `ShadowAtlasPlugin` is added after this plugin) and cross-schedule-safe (both land in
            // `CoreSchedule::Main`), so the assignment is never one frame stale — correct on a
            // moving camera, where the priority ranking can reorder which light wins base 0.
            //
            // `.in_set(LightCollectSet)` (VB-P1b-0 C1) makes this fold visible to a cross-plugin
            // `.before_set(LightCollectSet)` edge — `sync_cluster_light_gate` (composing-app-wired,
            // `boyko_app::plugins`) needs one so the header's cluster lane can never carry
            // `clusters_enabled=1` with stale/zero dims (an out-of-bounds `ClusterGrid` index on
            // the GPU — see `LightCollectSet`'s own doc).
            let collect =
                b.add_system(collect_lights).after_set(PunctualResolveSet).in_set(LightCollectSet).key();
            // `.in_set(LightReconcileSet)` is membership only, for the edges this plugin cannot
            // express by key: after propagation (the pose's writer, in `CameraPlugin`), and before
            // the pose's readers OUTSIDE this plugin (the CSM fit, the punctual atlas resolve).
            // The composing app declares those set edges. See the set's doc.
            b.add_system(light_reconcile).before(collect).in_set(LightReconcileSet);
            let cull = b.add_system(select_lighting_cull).before(collect).key();
            // Every system this plugin registers that reads `LightEnabled` runs after the seed,
            // ordered here by key: `collect_lights` and `select_lighting_cull`. A reader that is
            // not ordered after the seed sees a light added this frame as disabled while the fold
            // already writes it. For `select_lighting_cull` that was measured: on the frame three
            // point lights were added it counted 0 of them, in 30 of 30 runs
            // (`tests/light_policy_spawn_frame.rs`). Its edge is declared here, on the seed, so the
            // registration order stays as it was.
            //
            // `.in_set(LightSeedSet)` is membership only, for readers OUTSIDE this plugin: their
            // composing app declares the set edge (`CsmResolveSet.after(LightSeedSet)`). See
            // `LightSeedSet`'s doc.
            let mut seed_state = light_seed_state();
            b.add_system(move |w: &mut boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster| {
                seed_state.seed(w);
            })
            .before(collect)
            .before(cull)
            .in_set(LightSeedSet);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::LightingPlugin"
    }
}
