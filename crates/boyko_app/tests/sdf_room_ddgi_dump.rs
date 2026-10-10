//! SDFDDGI host-hook DEVICE gate: the GI-ON production dump differs from its OWN GI-OFF
//! control, by more than the frame-pacing floor, on the pixels the GI term can reach.
//!
//! # Why this binary, and not `engine_grand_showcase_512_ddgi_screenshot_dump`
//!
//! The RHI-level DDGI dump (`boyko_rhi_vulkan/tests/window_present_gbuffer.rs`) hand-writes the
//! b18 grid UBO and drives its own header bit at the RHI layer — it never touches
//! `boyko_app`, so it was GREEN for the whole time the host hook was broken (no `DdgiPlugin`
//! composed, `sync_ddgi_light_gate` registered nowhere, the b18 buffer never written after its
//! zero seed, the b6 update UBO packed from a LOCAL default config). A gate that cannot see the
//! defect is not a gate for it. This binary drives the PRODUCTION runner
//! (`EnginePlugins` → `boyko_app::runner` → `gpu_scene::GpuSceneBundles`) with a GI-enabled
//! config and dumps the presented frame.
//!
//! # The scene and the grid
//!
//! The `sdf_room_smoke.rs` scene verbatim: Deferred, the casters, ONE `SdfPrimitive` sphere, and
//! the sun / sky / point lights. GI applies only to `is_sdf_lit` pixels — and that name has
//! misled this gate once already, so it is spelled out here: `deferred_pbr.hlsl:786-788` reads
//! `bool is_sdf_lit = material_texel.b > 0.5`, the SDF-LIGHTING MASK (SDF soft shadow in `.r`,
//! SDF AO in `.g`, the mask in `.b`), and `sdf_gbuffer_composite.hlsl:1884` writes that mask as
//! `1.0` for the RASTERISED geometry too. So the receivers are the sphere AND the cubes AND the
//! floor — everything the raster lit — and only the background carries `0`. The earlier reading
//! ("so the SDF sphere is the receiver") is FALSE and produced a mis-specified gate clause; see
//! the A/B below. The grid is NON-default
//! (`origin [-6, -0.5, -6]`, `spacing 0.75`, `dims 16x8x16` — a 11.25 x 5.25 x 11.25 box around
//! the room): a resurrected local-default b6 pack (the old `DdgiConfig { ddgi_indirect: true,
//! ..Default }` at the arm site) would update probes over the 32 x 16 x 32 default box while
//! the resolve samples this one, which renders spatially wrong GI — visible, not byte-identical.
//!
//! # Two arms, ONE binary, selected by `BOYKO_DDGI_GATE`
//!
//! `BOYKO_DDGI_GATE=off` selects the GI-OFF control; anything else (including the variable being
//! unset) selects the GI-ON arm. The control is deliberately NOT a second `#[test]` and NOT a
//! second binary:
//!
//! - not a second `#[test]` in this file, because `EnginePlugins` composes `LightingPlugin`
//!   whose light-eviction hooks are PROCESS-GLOBAL — the same reason stated at the bottom of
//!   this doc, which is why this stays a single-test binary;
//! - not a second binary (e.g. reusing `sdf_room_smoke`), because a different binary is a
//!   different scene identity: a control has to differ from the treatment in the treatment and
//!   in nothing else, and `sdf_room_smoke` also differs in its frame budget — which is exactly
//!   what made the previous protocol uncomputable (F1 below).
//!
//! The two arms therefore differ in EXACTLY ONE expression, the `DdgiConfig` inserted after
//! `add_plugins`: [`GI_GRID`] on the ON arm, `DdgiConfig::default()` (the DISABLED default) on
//! the OFF arm. Same [`BUDGET`], same scene, same camera, same `CsmConfig`, same window size,
//! same window title.
//!
//! The OFF arm asserts `ResolvedDdgi::ddgi_mode_word == 0` and
//! `LightingConfig::ddgi_indirect == false`, so a control that silently ARMED cannot masquerade
//! as a control: without those two assertions an ON-vs-"OFF" byte-identity would be read as
//! "the GI term does not reach the screen" when the truth was "both arms were GI-ON".
//!
//! # Why the previous protocol was replaced — both findings MEASURED on the device, 2026-09-10
//!
//! The protocol this doc used to carry said: dump this test, dump `sdf_room_smoke` with the same
//! env as the GI-OFF control, and require the two sha256 to differ. Both halves of that are
//! broken, and the first one is broken on EVERY machine:
//!
//! **F1 — the old control produced no artifact at all, so the A/B was UNANSWERED (not passed,
//! not failed).** The readback needs roughly `SETTLE_FRAMES (30) + 1 + DRAIN_FRAMES (3)` ≈ 34
//! PRESENTED frames (`boyko_app::host_dump`), and the runner's step-3 `AppExit` check returns
//! from the frame loop BEFORE the present, unconditionally (`runner.rs`, "after the frame
//! completes, before the present"). `sdf_room_smoke` sets `BUDGET = 10`, so it presents ~9
//! frames, never reaches the request frame, exits green and writes NO BMP. This test only ever
//! produced a dump because it sets `BUDGET = 40`. That is why the control now lives here, on
//! this binary's budget, instead of being borrowed from another test.
//!
//! **F2 — a sha256 INEQUALITY is satisfied by frame pacing alone, with no GI term whatsoever.**
//! The same test, same env, run cold then warm: run 1 (cold, 28.22 s) and run 2 (warm, 3.91 s)
//! differ in 2220 of 76800 pixels (2.891 %), max per-channel delta 2, mean max-channel delta
//! 1.03, spread over a 288x72 band across the room rather than localised anywhere. Runs 2 and 3
//! (both warm) were BYTE-IDENTICAL. So the noise is first-run frame pacing, it is not confined
//! to the receiver, and "the hashes differ" is not evidence of anything. The protocol below
//! discards the cold capture and states a threshold ABOVE that measured floor.
//!
//! # Run protocol (run by the orchestrator on the GPU; NOT run in this lane)
//!
//! Every run carries `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`, and each capture
//! writes its own path. Each arm is run TWICE and the SECOND capture is the datum:
//!
//! 1. ON, cold:  `BOYKO_HOST_DUMP=D:\tmp\ddgi_on_1.bmp`  — DISCARDED (F2: the first run of a
//!    freshly built binary pays the shader/pipeline warm-up and its pacing differs).
//! 2. ON, warm:  `BOYKO_HOST_DUMP=D:\tmp\ddgi_on_2.bmp`  — the ON datum.
//! 3. OFF, cold: `BOYKO_DDGI_GATE=off BOYKO_HOST_DUMP=D:\tmp\ddgi_off_1.bmp` — DISCARDED.
//! 4. OFF, warm: `BOYKO_DDGI_GATE=off BOYKO_HOST_DUMP=D:\tmp\ddgi_off_2.bmp` — the OFF datum.
//!
//! Then, in order:
//!
//! - **Noise-floor control (must hold before the A/B is read at all).** A THIRD warm capture of
//!   either arm must be BYTE-IDENTICAL to that arm's warm datum. If warm-vs-warm is not
//!   byte-identical on this machine, the floor is not the one F2 measured and the threshold
//!   below is not calibrated for it — re-measure the floor before reporting an A/B.
//! - **The A/B**, three clauses, all measured on 2026-09-10 and all necessary. (1) MAGNITUDE:
//!   max per-channel delta STRICTLY GREATER than 2 — F2's cold-vs-warm floor was exactly 2, so
//!   `> 2` is the first value the pacing jitter cannot produce (measured: 37). (2) SIGN, and
//!   this is the clause that actually separates light from noise: EVERY differing pixel must be
//!   BRIGHTER on the ON arm, with zero exceptions. Pacing jitter is two-sided; an additive
//!   radiance term is not (measured: 12654 differing pixels, 12654 brighter, 0 darker; signed
//!   frame mean +0.198/+0.225/+0.227 with alpha untouched). (3) MASK BOUNDARY: the pixels where
//!   the mask is 0 must be delta 0 — the sky band (measured: 22720 sky pixels, 0 differing,
//!   max delta 0), and the SDF sphere's own disc must carry a non-zero all-brighter term
//!   (measured: 1038 of 2071 disc pixels differ, max 6).
//!
//!   ⚠ What this gate must NOT demand, and did in its first form: that the difference be
//!   CONCENTRATED on the sphere. Measured, 88.9 % of the differing mass and every pixel above
//!   delta 10 lie OUTSIDE the disc — the strongest blob (mean +9.0, max 37) is the CUBE face at
//!   `(-2, 0.5, -1)`, because the cubes carry `is_sdf_lit` too (above). That clause would have
//!   returned a RED against a working fix: the gate moved from "cannot fail for the right
//!   reason" to "can fail for a WRONG reason", which is the same family one level up.
//!
//! A run that prints a `SKIP` line, or `0 passed; 1 ignored`, is a failure to run, never a pass.
//!
//! # Where the receiver is (verify, do not trust)
//!
//! Projecting the sphere (centre `(-0.9, 0.7, 0.4)`, radius `0.7`) through this file's camera
//! (eye `(0, 1.7, 6)` looking at the origin, `fov_y` 60 deg, aspect `320/240`, viewport
//! 320x240) puts its centre at about `(127, 99)` with a radius of about 26 px — a ~53x53 disc,
//! `x` in `[101, 153]`, `y` in `[73, 125]`, **`y` measured DOWNWARD from the top of the
//! presented image**.
//!
//! `write_bmp` emits POSITIVE-height BITMAPINFOHEADER rows, i.e. BOTTOM-UP: a reader indexing
//! rows in file order sees this band at file rows `[114, 166]` (`239 - y`). Getting that
//! backwards mirrors the window about the image centre and lands it on the floor instead of the
//! sphere, which would read as "the difference is not on the receiver" — a wrong RED.
//!
//! These numbers are an analytic projection, not a measurement: recompute them (or locate the
//! sphere in the OFF dump directly) before using them to reject a difference. They were
//! re-derived and then CONFIRMED empirically on 2026-09-10 — an ASCII luminance render of the
//! OFF dump shows the disc's curved top edge first at `y = 72`, widening symmetrically about
//! `x = 127`. The box is used to CHECK clause 3 (the disc carries a term), never to reject a
//! difference for being elsewhere.
//!
//! # The 0%-gate half (separate from the A/B, and already GREEN)
//!
//! The pinned GI-OFF production goldens that bind the DDGI descriptors —
//! `[grand_showcase_2mat]`, `[vb_both_sdf]`, `[sdf_forward_only]`, `[vb_both]` in
//! `goldens/PINS.toml` — must stay byte-identical after `DdgiPlugin`'s unconditional
//! composition (the default carrier is the all-zero DISABLED image, the header bit stays 0,
//! `ddgi_update` stays `None`). Checked with `scripts/golden.ps1` in CHECK mode; measured green
//! 2026-09-10 with `PINS.toml` untouched.
//!
//! SINGLE-TEST BINARY: `EnginePlugins` composes `LightingPlugin`, whose light eviction hooks
//! are process-global — do not co-locate a second light-archetyping test here.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::light_system::LightTableGeneration;
use boyko_render::{DdgiConfig, LightingConfig, ResolvedDdgi};

/// Frames left before the test requests exit. Decremented once per Main run.
#[derive(Resource)]
struct FrameBudget(u32);

/// Counts the budget down and requests exit on the last frame.
fn exit_after_budget(mut budget: ResMut<FrameBudget>, mut exit: ResMut<AppExit>) {
    if budget.0 > 0 {
        budget.0 -= 1;
        if budget.0 == 0 {
            exit.0 = true;
        }
    }
}

/// The sun direction TO the light — mirrors `examples/sdf_room.rs` / `tests/sdf_room_smoke.rs`.
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// The NON-default GI grid — see the module doc for why it must not be the default.
const GI_GRID: DdgiConfig = DdgiConfig {
    ddgi_indirect: true,
    origin: [-6.0, -0.5, -6.0],
    spacing: 0.75,
    dims: [16, 8, 16],
};

/// The environment variable selecting the arm — see the module doc's two-arms section.
const ARM_ENV: &str = "BOYKO_DDGI_GATE";

/// Whether the GI-OFF control arm is selected (`BOYKO_DDGI_GATE=off`, case-insensitive and
/// trimmed). Anything else — including the variable being absent — is the GI-ON arm, so the
/// default invocation is the treatment and the control is the one that must be asked for.
fn gi_off_arm() -> bool {
    std::env::var(ARM_ENV).is_ok_and(|v| v.trim().eq_ignore_ascii_case("off"))
}

/// The sdf_room scene verbatim (`tests/sdf_room_smoke.rs::setup`).
fn setup(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    for (x, z) in [(-2.0, -1.0), (0.0, -2.5), (1.8, -0.6), (0.9, 1.2)] {
        commands
            .spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))))
            .insert(ShadowCaster);
    }

    // The R7 SDF sphere among the cubes — the GI RECEIVER (GI applies to `is_sdf_lit` pixels).
    commands.spawn(SdfPrimitive(SdfEdit::sphere([-0.9, 0.7, 0.4], 0.7, sdf_op::UNION, 0.0)));

    let sun_pose = Affine3A::look_at_rh(
        Vec3::ZERO,
        Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]),
        Vec3::new(0.0, 1.0, 0.0),
    );
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], 2.8),
    });
    commands.spawn(SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]));
    commands.spawn(PointLightObject {
        transform: Transform::from_translation(Vec3::new(0.6, 1.6, -0.8)),
        global: GlobalTransform::IDENTITY,
        light: PointLight::new([0.6, 1.6, -0.8], [1.0, 0.72, 0.45], 220.0, 7.0),
    });

    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 1.7, 6.0), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform {
            translation: pose.translation,
            rotation: Quat::from_mat3(pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: core::f32::consts::FRAC_PI_3,
            aspect: 320.0 / 240.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

/// Enough frames for the round-robin probe update to cover every subset at least once before
/// the `BOYKO_HOST_DUMP` frame (the runner dumps around frame ~34 and exits). SHARED by both
/// arms: the control differs from the treatment in the config and in nothing else, and a
/// smaller budget on the control is precisely the F1 defect this protocol replaced.
const BUDGET: u32 = 40;

/// The window title — IDENTICAL on both arms, so the two runs differ only in the config.
const WINDOW_TITLE: &str = "boyko_app SDFDDGI host-hook A/B dump";

#[test]
#[ignore = "gpu-windowed: needs a real windowed GPU device; run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1 and BOYKO_HOST_DUMP, twice per arm (BOYKO_DDGI_GATE=off selects the control); the orchestrator runs it on the GPU for the SDFDDGI host-hook A/B"]
fn sdf_room_ddgi_screenshot_dump() {
    let gi_off = gi_off_arm();

    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.add_systems(exit_after_budget);
    app.add_plugins(EnginePlugins::window(WINDOW_TITLE, 320, 240));
    app.add_startup_system(setup);
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    // THE ONE THING THAT DIFFERS BETWEEN THE ARMS. Inserted AFTER `add_plugins`, replacing
    // `DdgiPlugin`'s DISABLED default (the same "overwrite after add_plugins" contract
    // `CsmConfig` uses) — on the ON arm with the NON-default grid, on the control with the
    // DISABLED default, which is byte-identical to what `DdgiPlugin` already inserted. The
    // control therefore re-states the plugin's own default rather than removing the insert, so
    // the two arms execute the same statement sequence.
    app.insert_resource(if gi_off { DdgiConfig::default() } else { GI_GRID });

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    // Boot-failure discrimination: on a windowless / GPU-less box the runner exits BEFORE the
    // frame loop, so the budget is untouched — SKIP (never a vacuous green: the dump itself is
    // the gate, and it does not exist on a skipped run).
    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == BUDGET {
        let arm = if gi_off { "GI-OFF" } else { "GI-ON" };
        eprintln!("SKIP sdf_room_ddgi_screenshot_dump ({arm}): windowed boot unavailable");
        return;
    }

    let resolved = *app.world().resource::<ResolvedDdgi>();
    let light_gi = app.world().resource::<LightingConfig>().ddgi_indirect;

    if gi_off {
        // The control asserts it IS a control. Without these two, an OFF arm that silently
        // armed would make the A/B read "GI never reaches the screen" out of two GI-ON dumps —
        // the same shape of unfalsifiable green this whole gate was rewritten to escape.
        assert_eq!(resolved.ddgi_mode_word, 0, "the GI-OFF control resolved the DISABLED carrier");
        assert!(!light_gi, "the GI-OFF control left the LightBuf header gate closed");
    } else if resolved.ddgi_mode_word != 0 {
        // The host hook's device-side claims, readable back from the World after the run:
        // (1) the single writer resolved the NON-default grid (not the owner-locked default) —
        //     on a no-storage device it resolves DISABLED instead, which is the plan-§3 degrade,
        //     so the assertion is conditional on the carrier being armed at all;
        assert_eq!(resolved.origin, [-6.0, -0.5, -6.0, 0.0], "the carrier is the test's grid");
        assert_eq!(resolved.inv_spacing_dims[0], 1.0 / 0.75, "the carrier's inv_spacing");
        assert_eq!(resolved.inv_spacing_dims[1].to_bits(), 16);
        assert_eq!(resolved.inv_spacing_dims[2].to_bits(), 8);
        assert_eq!(resolved.inv_spacing_dims[3].to_bits(), 16);
        // (2) the header bit followed the carrier (the SOLE production writer ran in the
        //     production Main — the registration the headless gate (a2) pins).
        assert!(
            light_gi,
            "sync_ddgi_light_gate set LightingConfig::ddgi_indirect from the armed carrier"
        );
    } else {
        eprintln!("NOTE sdf_room_ddgi_screenshot_dump: DdgiCaps clamped the carrier to DISABLED (no B10G11R11/RG16F storage) -- the ON arm is GI-OFF on this device, so the A/B below CANNOT be read as a GI result");
    }

    let generation = app.world().resource::<LightTableGeneration>().0;
    assert!(generation > 0, "LightTableGeneration advanced past boot (lights were collected)");
}
