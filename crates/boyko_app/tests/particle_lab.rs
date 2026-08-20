//! **Particles P0 — the live-fire image fixture** (`docs/PARTICLES-PLAN.md` Rev 4, gates #12 and
//! #16).
//!
//! One windowed dump of the ARMED particle pipeline: `ParticleConfig { mode: GpuUnlit }`, one
//! effect, one emitter, the whole kickoff → emit → sim → indirect-draw chain composited into
//! `lit`, captured at presented frame 30 through `BOYKO_HOST_DUMP`.
//!
//! The scene, the pinned clock and the image reader live in [`particle_scene`] — shared with
//! `particle_counters_readback.rs` so the gate that MEASURES the pool partition measures the same
//! scene the gate that PINS the image renders.
//!
//! # What this binary asserts, and what it deliberately leaves to the owner
//!
//! Asserted programmatically, after `app.run()` returns:
//!
//! * the dump exists, decodes, and is the requested extent;
//! * it is not a black frame (the failure mode a hash gate reports as a plain mismatch and a
//!   screenshot reviewer reports as "the light is off");
//! * the emitter's screen region holds at least [`MIN_PARTICLE_PIXELS`] near-white pixels — the
//!   additive billboards actually reached `lit`. An armed subsystem whose sim never publishes
//!   `instanceCount`, or whose draw is declared and never recorded, renders a scene that is
//!   otherwise perfectly fine, and no image hash can say WHICH of those two it was.
//!
//! Left to the owner (gate #12's own words): whether the occlusion is CORRECT. A wrong depth
//! compare op inverts it — particles draw THROUGH the wall instead of behind it — and both images
//! are equally "non-black with particles present". The fixture produces the four BMPs; the eye
//! decides.
//!
//! # Usage
//!
//! ```text
//! # Gate #16 — the particle_additive pin (no occluder, one emitter, frame 30):
//! BOYKO_DISABLE_VALIDATION=1 BOYKO_HOST_DUMP=D:\tmp\particle_additive.bmp \
//!   cargo test -p boyko-app --test particle_lab -- --ignored --test-threads=1 --nocapture
//!
//! # Gate #12 — the four per-path occlusion dumps:
//! for %P in (deferred forward forwardplus vb) do BOYKO_RENDER_PATH=%P ^
//!   BOYKO_PARTICLE_OCCLUDER=1 BOYKO_HOST_DUMP=D:\tmp\particle_occl_%P.bmp ^
//!   cargo test -p boyko-app --test particle_lab -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! `#[ignore]`: needs a real windowed GPU device; the orchestrator runs it. Run with
//! `--test-threads=1`, and with `BOYKO_DISABLE_VALIDATION=1` on this machine (its validation layer
//! is crash-prone).
//!
//! # SINGLE-TEST BINARY
//!
//! See [`particle_scene`]'s module doc: `EnginePlugins` composes process-global plugin state and
//! the device singleton boots once, so this file holds exactly one `app.run()`.

#![cfg(windows)]

mod particle_scene;

use particle_scene::{CAPTURE_FRAME, LabClockWitness};

/// Near-white floor for the "a particle landed here" test. The additive billboards carry
/// `color_keys[0] == 0xFFFFFFFF` and no texture, so their cores saturate `lit`'s 8 bits; the
/// sun-lit floor and the occluder quad sit well below this on all three channels at once.
const WHITE_FLOOR: u8 = 240;

/// The minimum count of near-white pixels inside the emitter's screen region for the run to count
/// as "the particles reached `lit`".
///
/// Derived, not guessed: at the fixture's default rate the frame-30 image holds 31 particles, each
/// a `2 · 0.05`-unit quad at ~148 px/unit ⇒ ~15 px across ⇒ ~200 px of core each. Even if half the
/// fan has left the frame and every remaining sprite is clipped to a quarter of its area, the
/// count stays an order of magnitude above this floor — so a trip here means "almost nothing was
/// drawn", never "the tuning drifted".
const MIN_PARTICLE_PIXELS: usize = 64;

/// Any-channel floor separating "the frame was rendered" from "the frame is black".
const NONBLACK_FLOOR: u8 = 12;

/// **The armed particle dump.** See the module doc for what it asserts and what it hands to the
/// owner.
#[test]
#[ignore = "needs a real windowed GPU device; orchestrator-run particles-P0 live fire (gates #12/#16)"]
fn particle_lab_screenshot_dump() {
    particle_scene::print_config("particle_lab");
    let dump = std::env::var("BOYKO_HOST_DUMP").unwrap_or_default();
    assert!(
        !dump.is_empty(),
        "BOYKO_HOST_DUMP must name the destination .bmp — without it the host frame loop never \
         returns and this test hangs instead of failing"
    );

    let mut app = particle_scene::build_app("boyko_app particles P0 lab");
    app.run();

    // MEASURE EVERYTHING, PRINT EVERYTHING, THEN assert. A gate that panics before it has
    // reported the numbers makes the next run the only way to learn what the failing one saw.
    let witness = *app.world().resource::<LabClockWitness>();
    println!(
        "particle_lab: frames={} substeps={} unpaused_frames={} off_rate_frames={} \
         real_frame_ms=[min {:.4}, max {:.4}] (wall time does not reach the simulation)",
        witness.frames,
        witness.substeps,
        witness.unpaused_frames,
        witness.off_rate_frames,
        witness.min_real_ms(),
        witness.max_real_ms(),
    );

    let image = particle_scene::read_bmp(&dump);
    let win = particle_scene::window_size();
    let nonblack = image.count_at_least(NONBLACK_FLOOR);
    let white = image.count_white(WHITE_FLOOR);
    // The emitter sits on the view axis and the cone opens UPWARD, so the fan occupies the middle
    // horizontal third and everything above the emitter's own row. In top-down pixel coordinates
    // that is x in [1/6 w, 5/6 w) and y in [0, 3/4 h).
    let region = (win / 6, 0, win - win / 6, win * 3 / 4);
    let white_in_fan = image.count_white_in(region.0, region.1, region.2, region.3, WHITE_FLOOR);
    println!(
        "particle_lab: dump={dump} {}x{} nonblack={nonblack} white={white} \
         white_in_fan={white_in_fan} max_channel={} sha256={}",
        image.width,
        image.height,
        image.max_channel(),
        particle_scene::sha256_file(&dump),
    );

    assert!(
        witness.frames > CAPTURE_FRAME,
        "the capture frame ({CAPTURE_FRAME}) must have been presented; the loop ran only {} \
         frames",
        witness.frames
    );
    assert!(
        witness.is_deterministic(),
        "the simulated time was NOT a function of the frame index: {} frames, {} substeps, {} \
         frames with a non-zero virtual delta, {} frames off the one-substep rate. The image is \
         reproducible only when every frame drives exactly one substep and the engine clock \
         delivers none — otherwise the spawn FRAME INDICES differ between runs and, with them, \
         every particle's RNG-drawn direction. Do not bless this run as a golden.",
        witness.frames,
        witness.substeps,
        witness.unpaused_frames,
        witness.off_rate_frames,
    );
    assert_eq!(image.width, win, "dump width");
    assert_eq!(image.height, win, "dump height");

    assert!(
        nonblack > (image.bgra.len() / 100),
        "the dump is (near-)black: only {nonblack} of {} pixels reach {NONBLACK_FLOOR} — the \
         scene did not render at all, which is a host failure and not a particle one",
        image.bgra.len()
    );
    assert!(
        white_in_fan >= MIN_PARTICLE_PIXELS,
        "no particles reached `lit`: {white_in_fan} near-white pixels in the emitter's screen \
         region, floor {MIN_PARTICLE_PIXELS}. The subsystem was ARMED, so this is the armed \
         pipeline failing silently.{}",
        if particle_scene::on_known_particle_free_path() {
            "\n         THIS RUN IS ON THE DEFERRED PATH, where that is the KNOWN defect and not \
             a new one: the projection the runner hands the particle draw there has row2 == row3, \
             so every billboard vertex rasterizes at SV_Position.z == 1.0 against a depth buffer \
             holding euclidean length(cam_eye - P)/T_MAX under VK_COMPARE_OP_LESS, and every \
             fragment fails. See `particle_scene::on_known_particle_free_path` for the full \
             derivation and why no host-side matrix can fix it. Re-run with \
             BOYKO_RENDER_PATH=vb|forward|forwardplus to exercise the working arms."
        } else {
            " Check the sim's instanceCount publish and the draw's declare/record parity before \
             touching the tuning — the gate-#7 readback (`particle_counters_readback`) separates \
             those two: it reports what the compute half published."
        }
    );
}
