//! The [`ParticlePlugin`] — the particle subsystem's whole ECS-side composition, and the D17
//! CONTAINMENT contract it is gated against.
//!
//! # D17 — subsystem containment (the whole point of this file)
//!
//! `build` may insert **only**: this subsystem's own Resources, this subsystem's own components'
//! hooks, and this subsystem's own systems into `CoreSchedule::Main`. It may **not** touch
//! `CoreSchedule::Fixed`, `event_policy_cfg`, `Time`, `FixedTime`, or any schedule label another
//! subsystem observes.
//!
//! That is not tidiness. `App`'s `fixed_builder` is created LAZILY on the first
//! `*_in(CoreSchedule::Fixed, …)` call, and `event_policy_cfg: None` auto-resolves at `finish` to
//! "`WaitForFixed` **iff a Fixed schedule was configured**", with `fixed_steps_since_swap` then
//! holding the event swap across 0-substep frames. So a RENDERING plugin that registered anything
//! on `Fixed` would flip **every event type in the process** from `EveryFrame` to `WaitForFixed`
//! — at 200 fps against a 64 Hz step, two frames in three — silently changing input, UI and
//! collision event delivery in a game that merely installed a particle system. Patching the policy
//! back would be worse: it would override the resolution a user's OWN later Fixed schedule should
//! have produced, and it would make plugin order load-bearing.
//!
//! The subsystem therefore owns its clock ([`ParticleClock`], advanced from `Time::delta_secs()`
//! inside a Main system) and its refcount queue ([`ParticleEffectRefs`], drained by a Main system
//! of its own). `tests/particle_containment.rs` is the gate: it builds two apps differing ONLY by
//! this plugin and asserts the observable event-swap behaviour, the Fixed-schedule absence and the
//! live schedule-label set are identical.
//!
//! # Why composing it unconditionally is safe
//!
//! [`ParticleConfig`]'s default is [`Off`](crate::particle_config::ParticleMode::Off) — the
//! 0%-gate: nothing is declared, no device buffer is built, and every committed image pin stays
//! byte-identical by construction. And [`ParticleEmitter`](crate::particle::ParticleEmitter) is
//! opt-IN, so the three Main systems match zero rows and drain empty queues on a world that does
//! not use particles.

use boyko_ecs::ecs::core::app::{App, Plugin};
use boyko_ecs::ecs::core::asset::Assets;

use crate::particle::ParticleEffectRefs;
use crate::particle_clock::ParticleClock;
use crate::particle_config::ParticleConfig;
use crate::particle_effect::ParticleEffect;
use crate::particle_system::{
    ParticleEffectScratch, ParticleEmitScratch, particle_apply_effect_refs, particle_pack_effects,
    particle_tick_emitters,
};

/// Composes the GPU particle subsystem's ECS half: the owner-set config, the subsystem-owned
/// clock, the effect asset table, the two device-staging lanes, the carrier refcount queue, and
/// the three `CoreSchedule::Main` systems that drive them.
///
/// See the module doc for the containment contract this plugin is written against and the gate
/// that enforces it.
#[derive(Default)]
pub struct ParticlePlugin;

impl Plugin for ParticlePlugin {
    fn build(&self, app: &mut App) {
        // The owner-set cold config (default Off — the 0%-gate) and the subsystem's OWN clock.
        // The clock is inserted here, and NOT derived from `FixedTime`, for the reason the module
        // doc gives at length: reaching for the engine's shared fixed clock would re-tune event
        // buffering for every unrelated consumer in the process.
        app.insert_resource(ParticleConfig::default());
        app.insert_resource(ParticleClock::default());

        // The effect table. A plain `Resource` (not a `NonSendResource`): `ParticleEffect` is POD
        // and owns no device handle, unlike `Assets<MeshGpu>`/`Assets<TextureGpu>` whose rows own
        // RHI buffers. An empty table is a valid state — a world that mints no effect simply has
        // nothing for `particle_pack_effects` to bake.
        app.insert_resource(Assets::<ParticleEffect>::default());

        // The two device-staging lanes and the carrier refcount queue. All three are
        // `ScratchColumn`-backed and start EMPTY, so an unarmed world holds three unbacked VA
        // reservations and zero committed pages.
        app.insert_resource(ParticleEmitScratch::default());
        app.insert_resource(ParticleEffectScratch::default());
        app.insert_resource(ParticleEffectRefs::default());

        app.add_systems_cfg(|b| {
            // A1: advance the clock, then fold every enabled emitter into this frame's request
            // table. Sequential by design (≤256 rows), zero atomics, zero allocations.
            b.add_system(particle_tick_emitters);

            // The carrier refcount fold, ordered BEFORE the effect bake: a `+1`/`-1` can move the
            // asset table's own dirty generation, and the bake's re-run gate reads it. Running the
            // bake first would defer that change by a frame — invisible on a static scene, and a
            // one-frame stale effect row on the frame an emitter rebinds.
            let refs = b.add_system(particle_apply_effect_refs).key();
            b.add_system(particle_pack_effects).after(refs);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::ParticlePlugin"
    }
}
