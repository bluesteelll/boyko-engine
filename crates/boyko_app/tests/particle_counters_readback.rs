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
//!   With no alpha-class effect in the scene the alpha term is zero and this reads "every survivor
//!   got a render slot"; under rung P2's `BOYKO_PARTICLE_ALPHA` arm BOTH terms are asserted
//!   non-zero, which is the form the plan's P2 gate names and the only form in which the sum has
//!   discriminating power (a shipped-but-never-written alpha counter satisfies `additive + 0`
//!   exactly as well as a correct one).
//! * **R9 by construction** — `additive.instanceCount` is the live count, NOT the capacity. A pool
//!   of 65 536 that draws 65 536 instances is the pitfall this number refutes.
//! * **F5b** — `firstInstance == 0` in both slots, read back rather than trusted.
//! * **Gate #9, frame 0** — with `BOYKO_PARTICLE_READBACK_FRAME=1`: `alive_count_cur ==
//!   real_emit_count` and `emit_append_base == 0`, i.e. kickoff's `A = alive_count_next + E`
//!   reduced to `A = E` because nothing was alive before the first frame.
//! * **Rung P1b's skip census** (`BOYKO_PARTICLE_STATS=1`) — the three per-wave counters, asserted
//!   for internal consistency against the census's own construction and PRINTED as the two rates.
//!   The rate itself is never pinned: it is a property of the scene's wave coherence, which is what
//!   the rung exists to measure. See [`assert_skip_census`].
//! * **Rung P2 item 3's SORT MONOTONICITY** (`BOYKO_PARTICLE_SORT=1`, which the fixture refuses
//!   without `BOYKO_PARTICLE_ALPHA`) — the sorted alpha range's key sequence never decreases with
//!   rank, the range is conclusive (≥ 2 records over ≥ 2 bins), and the SAME frame's unsorted
//!   source is NOT monotone. The third is the control, and it is taken in the same submit so the
//!   measurement and its control describe the same particles. See [`assert_sort_monotonicity`].
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

use boyko_app::particle_readback::{ParticleCountersReadback, ParticleSortReadback};
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
         alive_count_next ({}). Every survivor of EITHER class takes its LIST index from \
         alive_count_next and its RENDER index from its own class counter, so a shortfall here is \
         the alpha leak M2 exists to catch: survivors that got a list position and no render \
         position, or the reverse.",
        rb.additive_instance_count,
        rb.alpha_instance_count,
        rb.alive_count_next
    );
    // ── The M2 assertion, in the form each arm can actually make.
    if particle_scene::alpha_class_armed() {
        // RUNG P2, and the gate the plan asks for by name: BOTH terms non-zero. Until this arm
        // existed the split above was satisfied by `additive + 0 == alive`, which a shipped alpha
        // counter that was never written satisfies exactly as well as a correct one.
        assert!(
            rb.alpha_instance_count > 0 && rb.additive_instance_count > 0,
            "the alpha arm is armed, so BOTH classes must be live: additive={} alpha={}. A zero \
             alpha term means the second emitter's survivors never reached the alpha render \
             counter — they would then be drawn by neither command while the sum above still \
             held, because a class that reserved nothing contributes nothing to either side.",
            rb.additive_instance_count,
            rb.alpha_instance_count
        );
    } else {
        assert_eq!(
            rb.alpha_instance_count, 0,
            "no alpha-class effect is in this scene, so nothing may have reserved a position on \
             the alpha render counter. A non-zero value here means the class predicate selected \
             the wrong arm and those survivors were written to the FAR END of p_render, where the \
             additive draw does not look."
        );
    }
    assert!(
        rb.draw_args_are_well_formed(),
        "the indirect draw block is malformed: first_instance additive={} alpha={}, \
         index_count={}. `drawIndirectFirstInstance` is not enabled on this device — a nonzero \
         first_instance is a silent corruption class (F5b) — and the billboard quad is 6 indices.",
        rb.additive_first_instance,
        rb.alpha_first_instance,
        rb.additive_index_count
    );

    // ── Whether the pool SATURATED inside the window, read off the device rather than derived
    //    from the env: `dead_count == 0` is "no free slot is left", which is the state gate #17's
    //    extension ladder (`rate = CAP/4` ⇒ full pool from frame 3) runs in on purpose and rung
    //    P1b's skip-rate ladder inherits.
    //
    //    Two assertions below MEAN DIFFERENT THINGS on the two ladders, and each states both
    //    readings rather than being armed for one. Before this was written the saturated ladder
    //    could not be run through this gate at all — it reddens on numbers that are correct — which
    //    would have made every rung-P1b density a red-by-construction run whose output a harness
    //    could not tell from a real failure.
    let saturated = rb.dead_count == 0;

    // ── R9: the draw fetches the LIVE count, never the capacity. The pitfall this refutes is a
    //    pool that renders `CAP` instances because a buffer was sized from a constant.
    if saturated {
        // At `dead_count == 0` the live set IS the pool, so `additive == CAP` is FORCED by the two
        // assertions already made (partition: `alive_next + dead == CAP`; class split: `additive ==
        // alive_next`). It is stated as the equality it degenerates into rather than skipped — a
        // skipped assertion looks the same in a log as one that was never reached — but it carries
        // NO discriminating power here, and the next assertion is the one that does.
        assert_eq!(
            rb.additive_instance_count + rb.alpha_instance_count,
            rb.capacity,
            "the pool is saturated (dead_count == 0), so every slot is alive and every survivor \
             must have a render position in ONE of the two classes: additive ({}) + alpha ({}) \
             must equal CAP ({})",
            rb.additive_instance_count,
            rb.alpha_instance_count,
            rb.capacity
        );

        // THE NON-VACUOUS ONE. With a full free list and a live spawn request, kickoff's clamp
        // `real_emit = min(requested, dead_count)` MUST have refused every spawn, and D15's
        // accumulator must therefore be non-zero. This is the ONLY place either ladder asserts that
        // `clamped_spawns` is a live datum rather than a word nobody writes: the unsaturated ladder
        // asserts it is ZERO, which a deleted accumulator satisfies perfectly. Without this, the
        // `+=` at `particle_kickoff.comp.hlsl`'s clamp could be dropped and every gate stays green.
        if particle_scene::spawn_per_frame() * particle_scene::emitter_count() > 0 {
            assert!(
                rb.clamped_spawns > 0,
                "the pool is saturated (dead_count == 0) and the fixture asks for {} spawn(s) per \
                 frame, so kickoff MUST have refused some and accumulated the shortfall — yet \
                 clamped_spawns is 0. Either the free list is not actually empty, or D15's \
                 accumulator is not being written at all (the unsaturated ladder's `== 0` cannot \
                 tell those apart).",
                particle_scene::spawn_per_frame()
            );
        }
    } else {
        assert!(
            rb.additive_instance_count < rb.capacity,
            "the additive draw's instanceCount ({}) reached the pool capacity ({}) while the free \
             list was NOT empty ({} slots) — the R9 pitfall: the draw is rendering the pool, not \
             the live set",
            rb.additive_instance_count,
            rb.capacity,
            rb.dead_count
        );
        assert_eq!(
            rb.clamped_spawns, 0,
            "kickoff refused {} spawn(s) while the free list still held {} slot(s); the fixture \
             spawns {} particle(s) per frame into a {}-slot pool, so a non-zero count here means \
             the free list is not being replenished. (On the SATURATED ladder the same number is a \
             genuinely full pool — a different reading, which is why this arm is gated on \
             `dead_count != 0` rather than on the ladder's name.)",
            rb.clamped_spawns,
            rb.dead_count,
            particle_scene::spawn_per_frame(),
            rb.capacity
        );
    }

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
        //    ordered BEFORE the fold, and `particle_scene::setup` spawns `emitter_count()` of them
        //    — ONE by default, TWO under rung P2's `BOYKO_PARTICLE_ALPHA` arm. The per-frame spawn
        //    is the PRODUCT, and reading the count off the fixture rather than assuming one is what
        //    keeps this bound from reddening on a correct two-class run.
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
        let rate = u64::from(particle_scene::spawn_per_frame())
            * u64::from(particle_scene::emitter_count());
        let bound = rate * (u64::from(rb.frames_presented) + 1);
        assert!(
            u64::from(rb.alive_count_next) <= bound,
            "more particles are alive ({}) than the fixture could have spawned: {} presented \
             frame(s) at {rate} per frame ({} per emitter × {} emitters), plus one frame of slack \
             for a submitted-but-unpresented recreate, is {bound}",
            rb.alive_count_next,
            rb.frames_presented,
            particle_scene::spawn_per_frame(),
            particle_scene::emitter_count()
        );
    }

    assert_skip_census(&rb, &witness);
    assert_sort_monotonicity(&app);
}

/// **Rung P2 item 3's SORT MONOTONICITY gate** (plan P2, "sort monotonicity readback") — the sort's
/// correctness instrument, with the non-vacuity control it was taken beside.
///
/// # Why this and not an image
///
/// Gate #16's order-independence argument does not transfer to a non-commutative blend, so the plan
/// forbids an image pin over overlapping alpha billboards — and rung P2 item 2 measured the harder
/// half: forcing the alpha index transform to the identity produced a dump BYTE-IDENTICAL to the
/// `particle_additive` golden. A byte-identical golden can hide a wrong answer, so the sort's gate
/// has to be a statement about the ORDER.
///
/// # The three assertions, and what each refuses
///
/// 1. **The destination is monotone** — the recomputed key never decreases with rank. This is the
///    property; alone it proves nothing, which is why it is never asserted alone.
/// 2. **The destination is CONCLUSIVE** — at least two records and at least two distinct bins.
///    Without it, a scatter that wrote NOTHING (leaving `p_render_sorted` at its boot zeroes, every
///    record at the origin, every key identical) reports zero inversions and passes.
/// 3. **The SOURCE is not monotone** — `p_render`, the same frame's unsorted class, read in the
///    same submit. This is the control, and it is what makes the instrument prove in every run that
///    it can tell an ordered range from an unordered one. A control taken as a second RUN would
///    only be a distribution comparison: the two runs do not share a spawn seed.
///
/// Plus the oracle-free half — no adjacent pair rises in depth by more than one bin's width — which
/// holds without the host reproducing the device's quantization bit for bit.
fn assert_sort_monotonicity(app: &boyko_app::prelude::App) {
    let sorted_run = particle_scene::sort_arming() != boyko_render::ParticleSortMode::None;
    let rb = app.world().try_resource::<ParticleSortReadback>().copied();
    if !sorted_run {
        // Structural absence, asserted rather than assumed: an UNSORTED run must produce no sort
        // readback at all, because the runner only takes one when `p_render_sorted` exists. A
        // resource here would mean the sorted buffer was allocated on a `SortMode::None` run.
        assert!(
            rb.is_none(),
            "BOYKO_PARTICLE_SORT is unset, so no sorted render buffer should exist and the runner \
             should have taken no sort readback — yet one is in the world. Structural absence is \
             what makes `SortMode::None` byte-identical to rung P2 item 2."
        );
        println!("particle_counters_readback: SORT unarmed (BOYKO_PARTICLE_SORT unset)");
        return;
    }
    let rb = rb.expect(
        "BOYKO_PARTICLE_SORT is set but no ParticleSortReadback reached the world: either the \
         frame loop never reached the capture frame, or the sort bundle was not built at boot \
         (the fixture's `sort_arming` resolves to Radix, so a trip here means the runner's \
         boot-time read did not see it)",
    );
    println!("{}", rb.artifact_lines());

    // (2) first: a conclusive range is the precondition for (1) meaning anything, and saying so in
    // this order is what keeps a vacuous pass from being reported as a green one.
    assert!(
        rb.sorted.is_conclusive(),
        "VACUOUS: the sorted range holds {} record(s) across {} distinct bin(s) — a range with \
         fewer than two of either is monotone for reasons that have nothing to do with the sort. \
         A scatter that wrote NOTHING reports exactly this (boot zeroes ⇒ every record at the \
         origin ⇒ one bin). Arm a denser fixture or a wider depth spread.",
        rb.sorted.records_checked,
        rb.sorted.distinct_keys
    );
    // (3) the CONTROL, asserted BEFORE the measurement so a run whose fixture cannot produce
    // disorder fails as a fixture problem rather than as a sort success.
    assert!(
        !rb.source.is_monotone(),
        "the CONTROL is vacuous: `p_render`'s unsorted alpha range is ALREADY monotone ({} \
         inversion(s) over {} record(s)). The sim's wave-retirement order happened to be \
         back-to-front, so this frame cannot distinguish a working sort from a scatter that copied \
         the range verbatim. Nothing is proven — re-run at a density or a camera where the spawn \
         order is not already depth order.",
        rb.source.inversions,
        rb.source.records_checked
    );
    // (1) the property itself.
    assert!(
        rb.sorted.is_monotone(),
        "the sorted alpha range is NOT back-to-front: {} inversion(s), the first at rank {} \
         (keys run {} → {}, depths {:.4} → {:.4} over {} records). The class is drawn from rank 0 \
         outward, so an inversion is a near billboard composited before a far one — which \
         `alpha_over` cannot recover from and no golden in this tree would show.",
        rb.sorted.inversions,
        rb.sorted.first_inversion_rank,
        rb.sorted.key_first,
        rb.sorted.key_last,
        rb.sorted.depth_first,
        rb.sorted.depth_last,
        rb.sorted.records_checked
    );
    // The ORACLE-FREE half, stated in the key's own units so a bin-boundary rounding difference
    // between the host mirror and the device cannot make it red on a correct range.
    let tolerance = boyko_app::particle_readback::particle_sort_bin_depth_ratio();
    assert!(
        rb.sorted.depth_order_holds(tolerance),
        "the sorted range's depths rise by up to {:.6}x between adjacent ranks, more than one \
         bin's width ({tolerance:.6}). This claim needs NO host oracle — two records out of depth \
         order must share a bin, and one bin is exactly that wide — so a failure here is a real \
         mis-ordering rather than a quantization artefact.",
        rb.sorted.max_depth_ratio
    );
    // The class was fully walked, or the verdict is about a PREFIX and says so.
    println!(
        "particle_counters_readback: SORT PROVEN sorted_inversions={} source_inversions={} \
         complete={} distinct_bins={} max_depth_ratio={:.6} (tolerance {tolerance:.6})",
        rb.sorted.inversions,
        rb.source.inversions,
        rb.sorted.is_complete(),
        rb.sorted.distinct_keys,
        rb.sorted.max_depth_ratio
    );
    assert!(
        rb.sort_is_proven(),
        "the readback's own composite predicate disagrees with the three assertions above — \
         `sort_is_proven` and this gate must be one statement, not two"
    );
}

/// The device wave width the census's bounds are derived against — 32 on this part.
///
/// It enters only the LANE bounds, never the wave ones, and both are stated as inequalities that a
/// wider wave would still satisfy in one direction: see the derivations at each assertion.
const WAVE_WIDTH: u64 = 32;

/// **Rung P1b: the skip census, asserted for INTERNAL CONSISTENCY against its own construction.**
///
/// This is not a re-statement of the skip rate — the rate is a MEASUREMENT and has no expected
/// value, since it is a property of the scene's wave coherence. What is checkable is that the three
/// counters could have come from the census the shader carries, and every bound below is derived
/// from that construction rather than from an observed run:
///
/// 1. **Armed iff the module says so.** `waves_evaluated + waves_skipped > 0` exactly when the sim
///    was built from `-D SDF_COLLIDE_STATS`. The two directions are different defects: zero on an
///    armed run means the census never executed (a selector that resolved to the shipping module —
///    the swapped-arm class gate #12 exists for); non-zero on an unarmed run means a shipping module
///    is writing the stats words, which would make every future run's ratio garbage.
/// 2. **`waves_evaluated ≤ lanes_evaluated`.** The evaluating arm is entered only when
///    `eval_lanes > 0`, and it adds `1` to one counter and `eval_lanes` to the other in the same
///    leader block. Equality means perfect incoherence (one lane per wave needed the field).
/// 3. **`lanes_evaluated ≤ waves_evaluated × W`.** A wave has at most `W` lanes to contribute.
///    Violating this means the ballot counted lanes outside its own wave.
/// 4. **`wave_substeps ≤ substeps_driven × ceil(alive / W)`.** Nothing retires inside the window,
///    so the alive count is monotone and the final one is the maximum; a SUBSTEP therefore
///    contributes at most `ceil(alive / W)` participating waves. `substeps_driven` is
///    [`LabClockWitness::substeps`] — the fixture's own count of substeps it drove — and it is an
///    upper bound on what the device ran, because an ECS frame whose present reported a recreate
///    drove a substep the device never dispatched. **Derived from the substep count rather than
///    from a frame count**: hard-coding one substep per frame would red spuriously on a legitimate
///    multi-substep run instead of catching the reconvergence hazard, which is enforced at the
///    host instead (`gpu_scene::particle::assert_one_substep_for_the_census`).
/// 5. **The counters cannot have WRAPPED.** All three are `u32` and none is ever reset, so the same
///    derived ceiling is checked to fit in `u32` before any of the bounds above are believed. A
///    wrapped counter satisfies every inequality here while reporting a rate that is nonsense.
///
/// A run that armed the census prints its rates and passes; a run that did not prints
/// `census=false` and asserts only direction 1's other half. Nothing here has an expected NUMBER —
/// the number is the deliverable, and it is read off the artifact line.
fn assert_skip_census(rb: &ParticleCountersReadback, witness: &LabClockWitness) {
    let armed = particle_scene::collision_stats_armed();
    assert_eq!(
        rb.skip_census_is_armed(),
        armed,
        "the skip census {} while BOYKO_PARTICLE_STATS was {}: waves_evaluated={} \
         waves_skipped={} lanes_evaluated={}. An armed run with a silent census means the pipeline \
         was built from a SHIPPING sim module (the swapped-arm class); an unarmed run with a live \
         one means a shipping module is writing rung P1b's words.",
        if rb.skip_census_is_armed() { "RAN" } else { "did not run" },
        if armed { "set" } else { "unset" },
        rb.waves_evaluated,
        rb.waves_skipped,
        rb.lanes_evaluated,
    );
    if !armed {
        return;
    }

    // The ceiling comes FIRST, because it is also the wrap detector and every bound below is only
    // meaningful on counters that did not wrap.
    //
    // `substeps` is the fixture's own count of substeps DRIVEN, which is an upper bound on what the
    // device dispatched (an ECS frame whose present reported a recreate drove one the device never
    // ran). `alive_count_cur` is the maximum alive over the window because nothing retires inside
    // it, so `ceil(alive / W)` is the most waves any one substep could have engaged.
    let waves_per_substep = u64::from(rb.alive_count_cur).div_ceil(WAVE_WIDTH);
    // Already `u64` on the witness — the substep total is exactly the quantity that must not
    // overflow a narrower type on a long run.
    let substeps_driven = witness.substeps;
    let ceiling = substeps_driven * waves_per_substep;
    assert!(
        substeps_driven > 0,
        "the fixture drove ZERO substeps, so no wave-substep bound can be derived and the census's \
         numbers are unanchored"
    );

    // O1's wrap detector, and it is the derived ceiling doing double duty: all THREE counters are
    // `u32` and none is ever reset, so a long or dense enough run silently wraps and then satisfies
    // every inequality below while reporting nonsense. `lanes_evaluated` is the first to go — it
    // grows `W` times faster than the wave pair — so its ceiling is the one checked.
    let lane_ceiling = ceiling * WAVE_WIDTH;
    assert!(
        lane_ceiling <= u64::from(u32::MAX),
        "this run could overflow the census counters: up to {ceiling} wave-substeps x {WAVE_WIDTH} \
         lanes = {lane_ceiling} lane-substeps against u32::MAX. The three stats words accumulate \
         from boot and are never reset, so a wrapped counter would pass every consistency bound \
         below and report a rate that means nothing. Shorten the window or lower the density."
    );

    let waves_eval = u64::from(rb.waves_evaluated);
    let lanes_eval = u64::from(rb.lanes_evaluated);
    assert!(
        waves_eval <= lanes_eval,
        "every evaluating wave contributes at least one evaluating lane, but waves_evaluated ({}) \
         exceeds lanes_evaluated ({}) — the two counters cannot have come from the same leader \
         block",
        rb.waves_evaluated,
        rb.lanes_evaluated
    );
    assert!(
        lanes_eval <= waves_eval * WAVE_WIDTH,
        "lanes_evaluated ({}) exceeds waves_evaluated ({}) x the {WAVE_WIDTH}-lane wave width — \
         the ballot counted lanes outside its own wave",
        rb.lanes_evaluated,
        rb.waves_evaluated
    );

    assert!(
        rb.wave_substeps() <= ceiling,
        "the census saw {} wave-substeps, more than the {ceiling} the fixture could have run: \
         {substeps_driven} substep(s) driven over at most {waves_per_substep} participating wave(s) \
         each ({} alive / {WAVE_WIDTH} lanes, rounded up). Either the counters are accumulating \
         something other than wave-substeps, or a wave-substep was counted more than once — which \
         is what a still-split wave at the top of the substep loop would do (see \
         `assert_one_substep_for_the_census`).",
        rb.wave_substeps(),
        rb.alive_count_cur,
    );

    // The rates themselves, PRINTED and never asserted: they are the rung's deliverable, and a
    // gate that pinned one would pin a property of this fixture's scene.
    println!(
        "particle_counters_readback: SKIP CENSUS wave_substeps={} wave_skip_rate={:.4} \
         lane_skip_rate={:.4} (lanes_evaluated={} of {} lane-substeps)",
        rb.wave_substeps(),
        rb.wave_skip_rate().expect("armed above"),
        rb.lane_skip_rate(WAVE_WIDTH as u32).expect("armed above"),
        rb.lanes_evaluated,
        rb.wave_substeps() * WAVE_WIDTH,
    );
}
