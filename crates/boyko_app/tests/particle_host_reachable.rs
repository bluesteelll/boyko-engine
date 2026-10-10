//! **The particle subsystem is REACHABLE from a host** — and the latent frame-1 panic its absence
//! created is unreachable.
//!
//! # What was measured, and why this file exists
//!
//! `ParticlePlugin` was added NOWHERE outside tests. Its only three `add_plugin` calls live in
//! `boyko_render/tests/particle_containment.rs`; `EnginePlugins::build` composed sixteen plugins
//! and particles were not among them. None of the six resources the plugin inserts —
//! `ParticleConfig`, `ParticleClock`, `Assets<ParticleEffect>`, `ParticleEmitScratch`,
//! `ParticleEffectScratch`, `ParticleEffectRefs` — was provided anywhere else in production.
//!
//! That is the `ProfilerPlugin` shape one subsystem over, and `plugins.rs` records it at the
//! registration site: a complete subsystem, every one of its own gates green, one `add_plugin`
//! short of existing. `tests/profiling_host_reachable.rs` is the gate written for that one and the
//! model for this one — it builds the SHIPPED host composition rather than a world of its own,
//! because the hole is BETWEEN the subsystem's gates, in the composition none of them look at.
//!
//! # The part that made it urgent: a latent panic on the engine's own documented convention
//!
//! `plugins.rs` tells a host to override a 0%-gated config resource AFTER `add_plugins` — the
//! phrasing is at `CsmConfig` ("overwrite it AFTER `add_plugins` to enable sun shadows") and again
//! at `ShadowConfig`. A host that did exactly that for particles:
//!
//! ```ignore
//! app.add_plugins(EnginePlugins::window("game", 1280, 720));
//! app.insert_resource(ParticleConfig { mode: ParticleMode::GpuUnlit, ..Default::default() });
//! ```
//!
//! passed the boot gate in `runner.rs` (`try_resource::<ParticleConfig>()` + `enabled()`), built
//! the GPU bundle, reached the per-frame `particle_upload_slots(s).map(..)` with `Some(..)` — and
//! **panicked on frame 1** at `world.resource::<ParticleClock>()`, which panics when the resource
//! is absent. Nobody hit it only because the sole caller that arms particles is a test fixture
//! which also hand-inserted the other five.
//!
//! Clause 2 below is that precondition turned into an invariant: it arms the config the documented
//! way and then asserts the exact triple the per-frame block dereferences. It is what keeps a
//! future edit from reordering the inserts back into a panic.
//!
//! # What this gate cannot claim
//!
//! Nothing about a *windowed* run. `EnginePlugins::build` is exercised here without `App::run`, so
//! no device is booted, no bundle is built and no frame is presented — the arming in clause 2 is a
//! world fact, not a device one. It also claims nothing about particle SEMANTICS: that the emitters
//! fold, that the effect table bakes, that the GPU sim integrates. Those have their own gates
//! (`boyko_render/tests/particle_containment.rs`, and the two orchestrator-run GPU fixtures). What
//! it claims is composition and the panic precondition, which is exactly what was missing.
//!
//! # Why every clause is in ONE `#[test]`
//!
//! **`EnginePlugins` cannot be built twice in one process** — the second build panics in
//! `register_component_hooks::<boyko_render::light::DirectionalLight>`, because component hooks are
//! process-global and the derive's installation is not idempotent. That was measured while writing
//! `profiling_host_reachable.rs`, which pays for it by putting its second leg in a separate test
//! BINARY. Here the three clauses are three assertions about ONE host, so they cost one build and
//! need no second process; splitting them into three `#[test]` functions would reintroduce the
//! panic the moment the harness ran them without `--test-threads=1`.

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_ecs::ecs::core::asset::Assets;
use boyko_render::{
    LightingConfig, ParticleClock, ParticleConfig, ParticleEffect, ParticleEffectRefs,
    ParticleEffectScratch, ParticleEmitScratch, ParticleMode,
};

/// **Clauses 3, 1 and 2**, asserted in that order on one host.
///
/// RED 1 (clause 1): delete `app.add_plugin(ParticlePlugin)` from `EnginePlugins::build` ⇒ the
/// first particle assertion fails and the subsystem is back to being unreachable — the exact state
/// this gate was written to end. Verified by running it against the unwired tree.
///
/// RED 2 (clause 2): move the plugin's `insert_resource(ParticleClock::default())` to depend on the
/// config, or drop it ⇒ the armed-host assertion fails. That is the failure that matters: it is the
/// frame-1 panic at `runner.rs`'s `world.resource::<ParticleClock>()`, caught in a device-free
/// process instead of in a shipped run.
///
/// RED 3 (clause 3): if `try_resource` ever stopped finding anything, `LightingConfig` — inserted
/// by `EnginePlugins::build` beside the plugins and untouched by this campaign — would fail FIRST,
/// so a broken accessor reports itself as a broken accessor rather than as a dead subsystem. It is
/// asserted before the particle clauses for exactly that reason.
#[test]
fn a_host_composes_the_particle_subsystem_and_can_arm_it_without_a_frame_one_panic() {
    // `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run`
    // installs the windowed runner, which this never calls.
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("particle-host-gate", 64, 64));

    // ── Clause 3 — the vacuity control, FIRST ────────────────────────────────────────────────
    assert!(
        app.world().try_resource::<LightingConfig>().is_some(),
        "the control resource is missing. `LightingConfig` is inserted by `EnginePlugins::build` \
         itself and has nothing to do with particles, so its absence means this harness cannot see \
         a host's resources AT ALL -- every clause below would then be reporting the accessor, not \
         the subsystem."
    );

    // ── Clause 1 — substrate reachability: all six resources the plugin owns ─────────────────
    //
    // Named one at a time rather than folded into a loop: `try_resource` is generic over the
    // resource type, so there is no value to iterate, and a per-resource message is what a reader
    // of the failure needs.
    assert!(
        app.world().try_resource::<ParticleConfig>().is_some(),
        "EnginePlugins did not compose ParticlePlugin: `ParticleConfig` is absent. It is the \
         subsystem's arming knob AND its boot gate -- `runner.rs` reads it once, before the frame \
         loop, to decide whether the GPU bundle is built at all. Without the plugin no host can \
         arm particles by setting a field; it must know to insert the whole resource."
    );
    assert!(
        app.world().try_resource::<ParticleClock>().is_some(),
        "`ParticleClock` is absent. The subsystem owns its own clock precisely so it never touches \
         `CoreSchedule::Fixed` (D17), and the per-frame upload block dereferences it with the \
         PANICKING `world.resource::<ParticleClock>()`."
    );
    assert!(
        app.world().try_resource::<Assets<ParticleEffect>>().is_some(),
        "`Assets<ParticleEffect>` is absent. An empty table is a valid state; a MISSING one means \
         `particle_pack_effects` has no table to bake from and no host can mint an effect."
    );
    assert!(
        app.world().try_resource::<ParticleEmitScratch>().is_some(),
        "`ParticleEmitScratch` is absent -- the emit-request staging lane `particle_tick_emitters` \
         fills and `upload_particle_emit_requests` reads."
    );
    assert!(
        app.world().try_resource::<ParticleEffectScratch>().is_some(),
        "`ParticleEffectScratch` is absent -- the effect-params staging lane \
         `particle_pack_effects` bakes and `upload_particle_effects` reads."
    );
    assert!(
        app.world().try_resource::<ParticleEffectRefs>().is_some(),
        "`ParticleEffectRefs` is absent -- the carrier refcount queue the effect handle's \
         component hooks push into and `particle_apply_effect_refs` drains."
    );

    // ── Clause 2 — the panic precondition, as an invariant ───────────────────────────────────
    //
    // The insert is AFTER `add_plugins`, which is the arming convention `plugins.rs` documents for
    // every other 0%-gated config in the composition. Under the unwired tree this is the exact
    // sequence that boots a GPU bundle and then panics on frame 1.
    app.insert_resource(ParticleConfig { mode: ParticleMode::GpuUnlit, ..Default::default() });

    let armed = app
        .world()
        .try_resource::<ParticleConfig>()
        .expect("invariant: the resource just inserted is present");
    assert!(
        armed.enabled(),
        "the arming insert did not arm: `ParticleConfig::enabled()` is the structural predicate \
         `mode != Off` that `runner.rs` boots the GPU bundle on, and a false here would make the \
         rest of this clause vacuous -- it would assert the disarmed path"
    );

    // The exact triple the per-frame `particle_upload_slots(s).map(..)` closure dereferences with
    // the panicking `world.resource::<T>()`, in the order it dereferences them. On an armed host
    // that closure runs, so each of these three is a frame-1 panic if absent.
    assert!(
        app.world().try_resource::<ParticleClock>().is_some(),
        "an ARMED host has no `ParticleClock`. This is the frame-1 panic: the boot gate builds the \
         particle bundle from `ParticleConfig` alone, so `particle_upload_slots` returns \
         `Some(..)`, and the first thing the closure does is `world.resource::<ParticleClock>()`, \
         which panics when the resource is absent. Arming must never be able to outrun the \
         substrate -- keep every one of the six inserts unconditional and in the plugin."
    );
    assert!(
        app.world().try_resource::<ParticleEmitScratch>().is_some(),
        "an ARMED host has no `ParticleEmitScratch` -- the second dereference in the same \
         per-frame closure, and the same frame-1 panic"
    );
    assert!(
        app.world().try_resource::<ParticleEffectScratch>().is_some(),
        "an ARMED host has no `ParticleEffectScratch` -- the third dereference in the same \
         per-frame closure, and the same frame-1 panic"
    );

    // ── Clause 4 — the ordering edge is BUILT, not merely written ────────────────────────────
    //
    // `finish()` consumes the schedule builders and runs `expand_set_edges`, which is where
    // `configure_set(ParticleTickSet).after(CameraSet::Resolve)` — the edge that keeps the emitter
    // fold off a one-frame-stale `GlobalTransform` — turns into real `member → member` pairs. A
    // schedule build error (a cycle, or `SetsOrderedButIntersect`) PANICS out of `build`, so the
    // assertion here IS the absence of that panic; there is no `Result` to inspect.
    //
    // Without this clause the `configure_set` line executes in NO device-free test in the tree:
    // every other host that builds this composition needs a GPU and is `#[ignore]`d, so a broken
    // edge would surface for the first time on a windowed run. That is the same "complete and
    // unexercised" shape this file exists to end, one level down.
    //
    // ⚠️ What it does NOT catch, stated rather than left to be discovered: that the edge has any
    // EFFECT. If `ParticlePlugin` dropped the fold's `.in_set(ParticleTickSet)`, the edge would
    // reference a memberless set, `expand_set_edges` would emit `boyko-W1501` and produce zero
    // pairs — a warning, not a panic — and this clause would stay green while the fold went back
    // to being unordered against propagation. Closing that needs either a public accessor for the
    // resolved order (`Schedule` exposes none) or an assertion over the emitted diagnostic; both
    // reach into `boyko_ecs`'s schedule module, which this change does not touch.
    //
    // `finish()` is device-free here and stays that way by construction: it resolves the event
    // policy, seeds `Time`/`FixedTime` if absent, builds the `Main` and `Fixed` schedules, and runs
    // the startup systems -- of which the composed plugins register NONE. It does NOT run a frame,
    // deliberately: several composed systems read `Assets<MeshGpu>`/`Assets<Material>`, which
    // `runner::run_windowed` inserts after this point, so a frame would panic on the host wiring
    // rather than on anything this gate is about.
    app.finish();
    assert!(app.is_finished(), "invariant: finish() completed the build it was asked for");
}
