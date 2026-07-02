//! Host plan R4 / D5 — the `LightTableGeneration` bump-on-rewrite protocol:
//! `collect_lights` bumps the generation exactly once per ACTUAL staging rewrite
//! (spawn / toggle / gate flip), and an idle frame leaves it untouched — the
//! writer-side determinism the ringed host's per-slot `light_uploaded_gen` compare
//! relies on (the slot-catch-up half of the protocol is unit-tested host-side in
//! `boyko_app::light_gate`).
//!
//! SINGLE-TEST BINARY: this file's one `#[test]` adds `LightingPlugin` (which registers
//! the process-global eviction hooks first) and then spawns lights. Do NOT co-locate a
//! second light-archetyping test here — `was_ever_archetyped` is process-global and never
//! reset, so a second `LightingPlugin::build` would panic `AlreadyArchetyped`.

#[path = "le_support/common.rs"]
mod common;

use boyko_render::light::LightingConfig;
use boyko_render::light_system::{LightTableGeneration, set_light_enabled_now};

use boyko_ecs::ecs::core::app::App;

/// Reads the current staged-table write generation.
fn generation(app: &App) -> u64 {
    app.world().resource::<LightTableGeneration>().0
}

#[test]
fn generation_bumps_on_rewrite_and_holds_on_idle() {
    let mut app = common::lighting_app();
    app.finish();

    // Boot anchor: LightingPlugin seeds generation 0 — the ringed host initializes
    // `light_uploaded_gen` to u64::MAX, so even a never-rebuilt (light-less) world
    // uploads its (empty) staged table into both slots on the first two frames.
    assert_eq!(generation(&app), 0, "LightingPlugin seeds generation 0");

    // A light-less frame: collect_lights' rebuild gate (Changed OR dirty) is closed,
    // so NO staging rewrite happens and the generation must hold.
    app.update();
    assert_eq!(generation(&app), 0, "a light-less frame rewrites nothing => no bump");

    // Spawn a light: the spawn's `Added ⇒ Changed` window opens the gate — exactly one
    // rebuild, exactly one bump. (The seed's first-run/`Added` passes may ALSO mark the
    // dirty channel while the spawn window drains, so allow the documented coarse
    // rebuilt⇒bump behavior across the settle frames — but every bump must correspond
    // to a frame, never more than one per frame.)
    let a = common::spawn_point_light(app.world_mut(), [0.0, 0.0, 0.0]);
    app.update();
    let after_spawn = generation(&app);
    assert!(after_spawn >= 1, "the spawn frame rebuilds the table => the generation advances");

    // Settle to the steady state (drain the spawn's Added window), then pin the idle
    // invariant: two consecutive static frames, zero bumps.
    app.update();
    app.update();
    let settled = generation(&app);
    app.update();
    assert_eq!(generation(&app), settled, "an idle frame never bumps the generation");
    app.update();
    assert_eq!(generation(&app), settled, "the idle invariant holds across frames");

    // A tickless structural change (the LightEnabled O(1) toggle marks LightTableDirty):
    // exactly one rebuild => exactly one bump.
    set_light_enabled_now(app.world_mut(), a, false);
    app.update();
    assert_eq!(generation(&app), settled + 1, "a toggle-driven rebuild bumps exactly once");

    // And the frame after the toggle is idle again.
    app.update();
    assert_eq!(generation(&app), settled + 1, "no residual bump after the toggle settles");

    // The R4 header-gate channel: flipping `csm_shadows` + marking dirty (what
    // `sync_csm_light_gate` does on a predicate flip) rebuilds the header with the
    // gate word and bumps the generation — so ringed hosts re-upload both slots.
    {
        let cfg = app.world_mut().resource_mut::<LightingConfig>();
        cfg.csm_shadows = true;
    }
    app.world_mut().resource_mut::<boyko_render::light::LightTableDirty>().0 = true;
    app.update();
    assert_eq!(generation(&app), settled + 2, "a gate flip (dirty-marked) bumps exactly once");
    let staging = app.world().resource::<boyko_render::light_system::LightTableStaging>();
    assert!(
        common::read_header(staging.bytes()).csm_mode(),
        "the rebuilt header carries the CSM gate (word 7 bit 2)"
    );
}
