//! **Particles P0 — the pool-partition readback gate** (`docs/PARTICLES-PLAN.md` Rev 4, gates
//! **#7** and **#9**).
//!
//! Renders the SAME scene `particle_lab.rs` pins (shared via [`particle_scene`]), reads
//! `p_counters` and `p_draw_args` back from the device after a chosen presented frame, and asserts
//! the plan's arithmetic on them:
//!
//! * **B3, the two-term equality** — `alive_count_next + dead_count == CAP`. Every slot of the pool
//!   is on exactly one of the two lists. A leak makes the sum small, a double-count makes it large,
//!   and both are invisible in every image the engine can produce.
//! * **The M2 class split** — `additive.instanceCount + alpha.instanceCount == alive_count_next`.
//!   At P0 the alpha term is structurally zero, so this reads "every survivor got a render slot";
//!   it is stated as the sum because that is the form that catches rung P2's alpha leak.
//! * **R9 by construction** — `additive.instanceCount` is the live count, NOT the capacity. A pool
//!   of 65 536 that draws 65 536 instances is the pitfall this number refutes.
//! * **F5b** — `firstInstance == 0` in both slots, read back rather than trusted.
//! * **Gate #9, frame 0** — with `BOYKO_PARTICLE_READBACK_FRAME=1`: `alive_count_cur ==
//!   real_emit_count` and `emit_append_base == 0`, i.e. kickoff's `A = alive_count_next + E`
//!   reduced to `A = E` because nothing was alive before the first frame.
//!
//! # The capture frame is an ENV, and the test reads it
//!
//! `BOYKO_PARTICLE_READBACK_FRAME=<n>` arms the runner's probe (see
//! [`boyko_app::particle_readback`]). This binary sets it ITSELF when it is unset, so the gate is
//! self-driving; an explicit value wins, which is how the same binary serves both the frame-0 case
//! (`n = 1`) and the settled case (`n = 30`).
//!
//! # Usage
//!
//! ```text
//! # Gate #7 — the settled partition (30 presented frames):
//! BOYKO_DISABLE_VALIDATION=1 BOYKO_RENDER_PATH=vb BOYKO_PARTICLE_READBACK_FRAME=30 \
//!   cargo test -p boyko-app --test particle_counters_readback -- --ignored --test-threads=1 --nocapture
//!
//! # Gate #9 — frame 0:
//! BOYKO_DISABLE_VALIDATION=1 BOYKO_RENDER_PATH=vb BOYKO_PARTICLE_READBACK_FRAME=1 \
//!   cargo test -p boyko-app --test particle_counters_readback -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! `#[ignore]`: needs a real windowed GPU device. Run with `--test-threads=1`.
//!
//! # SINGLE-TEST BINARY
//!
//! See [`particle_scene`]'s module doc.

#![cfg(windows)]

mod particle_scene;

use boyko_app::particle_readback::ParticleCountersReadback;
use particle_scene::LabClockWitness;

/// The capture frame this gate arms when the env does not name one — the same settled window the
/// image pin uses, so the counters describe the instant the picture does.
const DEFAULT_READBACK_FRAME: &str = "30";

/// **The pool-partition readback.** See the module doc for the five properties it asserts.
#[test]
#[ignore = "needs a real windowed GPU device; orchestrator-run particles-P0 partition readback (gates #7/#9)"]
fn particle_counters_partition_readback() {
    particle_scene::print_config("particle_counters_readback");

    // Self-driving: arm the runner's probe when the caller did not. An explicit value wins, which
    // is what lets `BOYKO_PARTICLE_READBACK_FRAME=1` turn this same binary into gate #9.
    let requested = std::env::var("BOYKO_PARTICLE_READBACK_FRAME").ok();
    if requested.is_none() {
        // SAFETY: `std::env::set_var` is `unsafe` in Rust 2024 because a concurrent reader in
        // another thread would race it. There is no such reader here: this runs at the top of a
        // `--test-threads=1` binary, before `App::new` — so before any engine thread, any
        // threadpool worker and any windowed frame loop exists, and the runner's own read happens
        // later on this same thread.
        unsafe { std::env::set_var("BOYKO_PARTICLE_READBACK_FRAME", DEFAULT_READBACK_FRAME) };
    }
    let capture_frame: u32 = std::env::var("BOYKO_PARTICLE_READBACK_FRAME")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(30);
    println!("particle_counters_readback: capture_frame={capture_frame}");

    let mut app = particle_scene::build_app("boyko_app particles P0 counters readback");
    app.run();

    let witness = *app.world().resource::<LabClockWitness>();
    println!(
        "particle_counters_readback: frames={} substeps={} unpaused_frames={} off_rate_frames={}",
        witness.frames, witness.substeps, witness.unpaused_frames, witness.off_rate_frames,
    );

    // The absence of the resource is its own diagnosis, and a specific one: the runner inserts it
    // on the capture frame IF the GPU bundle exists, so "no resource" means either the loop never
    // reached the capture frame or the subsystem was not armed at boot. Both are stated.
    let rb = *app.world().try_resource::<ParticleCountersReadback>().unwrap_or_else(|| {
        panic!(
            "no ParticleCountersReadback in the world after {} frames: either the frame loop \
             never reached presented frame {capture_frame}, or the particle GPU bundle was never \
             built (ParticleConfig::mode == Off at boot — the fixture arms it, so a trip here \
             means the runner's boot-time read did not see the armed config)",
            witness.frames
        )
    });
    println!("particle_counters_readback: {}", rb.artifact_line());

    // ── Gate #7: the four-boundary partition, at the boundary a host CAN observe.
    assert!(
        rb.partition_holds(),
        "the pool partition is broken: alive_count_next ({}) + dead_count ({}) != CAP ({}). A \
         SMALL sum is a LEAK (slots on neither list — they can never be spawned into again); a \
         LARGE one is a DOUBLE-COUNT (a slot on both — two particles will own it). Neither is \
         visible in any image.",
        rb.alive_count_next,
        rb.dead_count,
        rb.capacity
    );
    assert!(
        rb.class_split_sums_to_list(),
        "the class split does not sum to the list counter: additive ({}) + alpha ({}) != \
         alive_count_next ({}). At P0 the alpha term must be 0 and the additive term must be \
         every survivor.",
        rb.additive_instance_count,
        rb.alpha_instance_count,
        rb.alive_count_next
    );
    assert_eq!(
        rb.alpha_instance_count, 0,
        "P0 declares no alpha draw, so a non-zero alpha render counter means survivors were \
         written to render slots nothing draws"
    );
    assert!(
        rb.draw_args_are_well_formed(),
        "the indirect draw block is malformed: first_instance additive={} alpha={}, \
         index_count={}. `drawIndirectFirstInstance` is not enabled on this device — a nonzero \
         first_instance is a silent corruption class (F5b) — and the billboard quad is 6 indices.",
        rb.additive_first_instance,
        rb.alpha_first_instance,
        rb.additive_index_count
    );

    // ── R9: the draw fetches the LIVE count, never the capacity. The pitfall this refutes is a
    //    pool that renders `CAP` instances because a buffer was sized from a constant.
    assert!(
        rb.additive_instance_count < rb.capacity,
        "the additive draw's instanceCount ({}) reached the pool capacity ({}) — the R9 pitfall: \
         the draw is rendering the pool, not the live set",
        rb.additive_instance_count,
        rb.capacity
    );
    assert_eq!(
        rb.clamped_spawns, 0,
        "kickoff refused {} spawn(s) for want of free slots; the fixture spawns {} particle(s) per \
         frame into a {}-slot pool, so a non-zero count means the free list is not being \
         replenished. (A rate that SATURATES the pool inside the window — gate #17's extension \
         ladder runs `rate = CAP/4` — refuses spawns because the pool is genuinely full, which is \
         a different reading of the same number; this gate is armed for the non-saturating ladder.)",
        rb.clamped_spawns,
        particle_scene::spawn_per_frame(),
        rb.capacity
    );

    // ── The gate must not be VACUOUS: a run where nothing ever spawned satisfies every equality
    //    above trivially (0 + CAP == CAP, 0 + 0 == 0). The scene emits one particle per frame with
    //    a lifetime far longer than the capture window, so the live count must be non-zero and, at
    //    the settled frame, must equal the number of frames that emitted.
    assert!(
        rb.alive_count_next > 0,
        "VACUOUS: no particle was alive at the capture, so every partition equality above held \
         trivially. The fixture arms one burst per frame with an {}s lifetime — a zero live count \
         means the spawn path never reached the device."
        ,
        particle_scene::LAB_LIFETIME
    );

    // ── Gate #9: the frame-0 shape, asserted only where it is meaningful.
    if rb.frames_presented == 1 {
        assert!(
            rb.frame_zero_shape_holds(),
            "frame 0's kickoff shape is wrong: alive_count_cur ({}) != real_emit_count ({}) or \
             emit_append_base ({}) != 0. With nothing alive before the first frame, `A = \
             alive_count_next + E` must reduce to `A = E` at a zero append base — that is what \
             makes the boot fill (p_dead = identity, dead_count = CAP) the whole of the frame-0 \
             contract.",
            rb.alive_count_cur,
            rb.real_emit_count,
            rb.emit_append_base
        );
        assert_eq!(
            rb.alive_count_next, rb.real_emit_count,
            "frame 0 retires nothing (the fixture's lifetime is {}s), so every emitted particle \
             must have survived onto the write list",
            particle_scene::LAB_LIFETIME
        );
    } else {
        // Past frame 0 the fixture's own arithmetic is checkable — AT ANY RATE. The bound is
        // DERIVED from the three mechanics this fixture pins, not from the default rate:
        //
        // 1. `lab_arm_burst` writes `burst = spawn_per_frame()` on every emitter each ECS frame,
        //    ordered BEFORE the fold, and `particle_scene::setup` spawns exactly ONE emitter.
        // 2. `particle_tick_emitters` folds `continuous + burst` into one `EmitRequestGpu` whose
        //    `spawn_count` is that value — `continuous` is 0 here (the emitter's `rate` is 0 and
        //    the paused clock leaves the fold's `dt` at zero) — and `ParticleEmitScratch::
        //    begin_frame` CLEARS the table every frame, so an ECS frame that never reached the
        //    upload contributes nothing rather than accumulating into the next one.
        // 3. Nothing retires inside the window (`LAB_LIFETIME` = 8 s against ~1 ms of virtual time
        //    per frame), so the live count only grows, and kickoff's clamp
        //    (`real_emit = min(requested, dead_count)`) can only make it grow by less.
        //
        // ⇒ at most `rate` particles reach the device per frame that RENDERED. `frames_presented`
        // counts frames whose present returned `Ok(true)`; a frame may submit its emit and only
        // then have `present` report a recreate (`frame_driver`'s step 7, the out-of-date /
        // suboptimal path), leaving those spawns on the device and uncounted here. ONE frame of
        // slack covers one such recreate — the same allowance the previous rate-1 form carried as
        // a literal `+ 1`, which is one frame's spawns at one per frame, and which is why that
        // form reddened every rate > 1 (gate #17's whole density ladder) after printing correct
        // numbers.
        //
        // MEASURED at gate #17 (`BOYKO_PARTICLE_READBACK_FRAME=21`): `alive == rate ×
        // frames_presented` EXACTLY at rates 1 / 8 / 64 / 512, so the slack term was never
        // consumed on a quiet window and this bound is tight to one frame rather than loose by
        // construction.
        let rate = u64::from(particle_scene::spawn_per_frame());
        let bound = rate * (u64::from(rb.frames_presented) + 1);
        assert!(
            u64::from(rb.alive_count_next) <= bound,
            "more particles are alive ({}) than the fixture could have spawned: {} presented \
             frame(s) at {rate} per frame, plus one frame of slack for a submitted-but-unpresented \
             recreate, is {bound}",
            rb.alive_count_next,
            rb.frames_presented
        );
    }
}
