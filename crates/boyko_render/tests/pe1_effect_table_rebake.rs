//! PE-1 (render-wave RD5): the effect-table bake sees every change to `Assets<ParticleEffect>` —
//! an effect added or removed after the first bake included — and nothing else.
//!
//! `particle_pack_effects` bakes the effect table into `ParticleEffectScratch`, and the host
//! uploads it iff `rows_gen()` moved (`boyko_app::particle_gate`). Before RD5 its re-run key was
//! `(Assets::dirty_gen(), timestep)`, and `dirty_gen` moves only on `get_mut` / `fill`: an effect
//! minted by `register_effect` after the first bake kept its boot zero row on the device (zero
//! lifetime, so its particles retire on the first substep — an invisible effect), and a removed
//! row kept its old parameters.
//!
//! The tests drive a real `App` with `ParticlePlugin` (the `particle_containment.rs` fixture) and
//! edit the table between frames through the world's resource:
//!
//! 1. [`an_effect_registered_after_the_first_bake_is_baked`] — the `register_effect` add path;
//! 2. [`a_removed_unpinned_row_is_zeroed_by_the_next_bake`] — a row minted with `Assets::add`
//!    before the first frame (unpinned, never handed to a component), removed later;
//! 3. [`an_idle_frame_neither_rebakes_nor_bumps_rows_gen`] — the guard against a fix that
//!    re-bakes (and re-uploads) every frame;
//! 4. [`get_mut_still_rebakes`] — the edit path that worked before RD5 keeps working;
//! 6. [`a_timestep_change_rebakes`] — the key's second term (rung test 5 is the source census
//!    `tests/drain_edited_single_consumer_census.rs` in the root package).
//!
//! Host-only: no device, no window.

#![cfg(not(miri))]

use std::time::Duration;

use bytemuck::Zeroable;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::prelude::ThreadPoolBuilder;
use boyko_render::{
    EffectParamsGpu, ParticleClock, ParticleEffect, ParticleEffectScratch, ParticleEffectsExt,
    ParticlePlugin, pack_effect_params,
};

/// A frame delta far below one particle substep: the frames here bake, they do not simulate.
const TINY: Duration = Duration::from_micros(1);

/// A booted app with the particle plugin and nothing else.
fn app() -> App {
    let mut app = App::with_pool(ThreadPoolBuilder::new().num_threads(1).build());
    app.add_plugin(ParticlePlugin);
    app.finish();
    app
}

/// One rendered frame.
fn frame(app: &mut App) {
    app.update_with_delta(TINY);
}

/// The world's effect table, for an edit between frames.
fn effects(app: &mut App) -> &mut Assets<ParticleEffect> {
    app.world_mut().resource_mut::<Assets<ParticleEffect>>()
}

/// The baked effect table.
fn scratch(app: &App) -> &ParticleEffectScratch {
    app.world().resource::<ParticleEffectScratch>()
}

/// The clock's substep, the bake's second input.
fn timestep(app: &App) -> f32 {
    app.world().resource::<ParticleClock>().timestep()
}

/// A distinct, non-zero effect per `seed`: `drag` moves `damping`, the lifetimes move the row's
/// lifetime words, so two seeds never pack to the same row and no seed packs to the zero row.
fn effect(seed: f32) -> ParticleEffect {
    ParticleEffect {
        drag: 0.25 * seed,
        lifetime_min: 0.5 + seed,
        lifetime_max: 1.0 + seed,
        ..ParticleEffect::default()
    }
}

/// Test 1 (red before RD5): an effect minted through `register_effect` after the first bake is
/// re-baked on the next frame, at its own index, with its own parameters.
#[test]
fn an_effect_registered_after_the_first_bake_is_baked() {
    let mut app = app();
    effects(&mut app).register_effect(effect(1.0));
    frame(&mut app);
    let rows_before = scratch(&app).rows().len();
    let gen_before = scratch(&app).rows_gen();
    assert_eq!(rows_before, 1, "fixture: frame 1 baked the one Startup-time effect");

    let late = effect(2.0);
    let handle = effects(&mut app).register_effect(late);
    frame(&mut app);

    let rows = scratch(&app).rows();
    assert_eq!(
        rows.len(),
        rows_before + 1,
        "rows().len() stayed {rows_before}; the add path was not re-baked (PE-1)"
    );
    assert_eq!(scratch(&app).rows_gen(), gen_before + 1, "an actual rebuild bumps rows_gen once");
    let k = handle.index() as usize;
    assert_eq!(
        rows[k],
        pack_effect_params(&late, timestep(&app)),
        "row {k} carries the late effect's parameters"
    );
}

/// Test 2 (red before RD5): a row removed from the table is zeroed by the next bake. The row is
/// minted with `Assets::add` BEFORE the first frame — unpinned and never handed to a component,
/// the one case `register_effect`'s pin rationale allows — so frame 1's bake contains it. The
/// test does not re-add into the freed slot.
#[test]
fn a_removed_unpinned_row_is_zeroed_by_the_next_bake() {
    let mut app = app();
    let doomed = effect(3.0);
    let handle = effects(&mut app).add(doomed);
    effects(&mut app).register_effect(effect(4.0));
    frame(&mut app);
    let k = handle.index() as usize;
    assert_eq!(
        scratch(&app).rows()[k],
        pack_effect_params(&doomed, timestep(&app)),
        "fixture: frame 1 baked the unpinned row"
    );

    assert!(effects(&mut app).remove(handle).is_some(), "fixture: the row was Loaded");
    frame(&mut app);

    let row = scratch(&app).rows()[k];
    assert_ne!(
        row,
        pack_effect_params(&doomed, timestep(&app)),
        "row {k} still holds the removed effect's parameters; the remove path was not re-baked \
         (PE-1)"
    );
    assert_eq!(row, EffectParamsGpu::zeroed(), "a removed row is baked as the zero row");
}

/// Test 3 (green before and after RD5): idle frames neither re-bake nor bump `rows_gen`, so they
/// upload nothing.
#[test]
fn an_idle_frame_neither_rebakes_nor_bumps_rows_gen() {
    let mut app = app();
    effects(&mut app).register_effect(effect(5.0));
    effects(&mut app).register_effect(effect(6.0));
    frame(&mut app);
    let gen_baked = scratch(&app).rows_gen();
    let rows_baked: Vec<EffectParamsGpu> = scratch(&app).rows().to_vec();
    assert_eq!(gen_baked, 1, "fixture: frame 1 is the one bake");

    for idle in 1..=3 {
        frame(&mut app);
        assert_eq!(
            scratch(&app).rows_gen(),
            gen_baked,
            "idle frame {idle} bumped rows_gen - something re-bakes (and re-uploads) every frame"
        );
        assert_eq!(scratch(&app).rows(), rows_baked.as_slice(), "idle frame {idle} moved the rows");
    }
}

/// Test 4 (green before and after RD5): a `get_mut` edit is re-baked on the next frame.
#[test]
fn get_mut_still_rebakes() {
    let mut app = app();
    let handle = effects(&mut app).register_effect(effect(7.0));
    frame(&mut app);
    let gen_before = scratch(&app).rows_gen();

    let edited = effect(8.0);
    *effects(&mut app).get_mut(handle).expect("fixture: the row is Loaded") = edited;
    frame(&mut app);

    assert_eq!(scratch(&app).rows_gen(), gen_before + 1, "a get_mut edit re-bakes once");
    let k = handle.index() as usize;
    assert_eq!(
        scratch(&app).rows()[k],
        pack_effect_params(&edited, timestep(&app)),
        "row {k} carries the edited parameters"
    );
}

/// Test 6 (green before and after RD5): a clock stepping at a different rate re-bakes every row
/// against the new substep — `damping` and the rotation multiplier are per-substep constants.
#[test]
fn a_timestep_change_rebakes() {
    let mut app = app();
    let a = effect(9.0);
    let b = effect(10.0);
    effects(&mut app).register_effect(a);
    effects(&mut app).register_effect(b);
    frame(&mut app);
    let gen_before = scratch(&app).rows_gen();
    let old_timestep = timestep(&app);

    *app.world_mut().resource_mut::<ParticleClock>() = ParticleClock::from_hz(30.0);
    frame(&mut app);

    let new_timestep = timestep(&app);
    assert_ne!(new_timestep.to_bits(), old_timestep.to_bits(), "fixture: the substep changed");
    assert_eq!(scratch(&app).rows_gen(), gen_before + 1, "a timestep change re-bakes once");
    assert_eq!(
        scratch(&app).rows(),
        [pack_effect_params(&a, new_timestep), pack_effect_params(&b, new_timestep)].as_slice(),
        "every row is re-baked against the new substep"
    );
}
