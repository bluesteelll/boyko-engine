//! Defect A4, at scale: box piles must still freeze under wake-on-contact-change, and a
//! frozen pile must not be woken by structural changes that do not touch it. Defect A7:
//! box piles must come to rest at all.
//!
//! `IslandSleep::begin_step` unlatches a latched row whose island's manifold count changed
//! since the previous step. On a box pile that comparison has two costs these tests gate:
//!
//! * **Onset flicker** (design Decision 3, round-2 ruling). An exactly touching pile has
//!   knife-edge lateral and diagonal box pairs whose manifolds appear and vanish at rest
//!   (defect A7). If one of them changes between the step a pile latches and the next, the
//!   pile wakes and its debounce restarts, so it sleeps later or not at all. Every scene
//!   records its contact-change wake events. G2 (one height-5 pile) and G7 (sixteen
//!   height-4 draws) cap them, with budgets sized against the re-draw distribution that the
//!   `flicker_redraw_distribution` generator prints, and a budget overrun is reported the
//!   moment it happens rather than as a missed freeze.
//!
//! # What a red G2 or G7 means until A4's Decision 3 is decided again
//!
//! A4's round-2 ruling (§3.4) decides the onset-flicker comparison again only after A7. A7
//! has landed and that decision has not been taken, so this triage still stands. A red is
//! TRIAGED, never waived and never silenced by raising the budget (orchestrator ruling,
//! round 3). The failure message carries these four steps and the numbers step 3 needs:
//!
//! 1. Re-run the failing test under the M1 protocol (design-A4 §5.2 step 1, the wake block
//!    deleted). M1 separates a flicker budget overrun from a pile that no longer comes to rest.
//! 2. If the M1 leg ALSO fails, the red BLOCKS. The pile stopped resting, which is not a budget
//!    question.
//! 3. If the M1 leg passes, re-run the failing test once on a fresh draw. A second red on an
//!    INDEPENDENT draw BLOCKS. A single red is recorded in the A7 entry of
//!    `docs/OPEN-QUESTIONS.md` with its measured numbers — observed events, the measured wake
//!    probability p for that height, and the negative-binomial tail P(>= budget+1 events | p, N
//!    draws) — and does not block.
//! 4. A budget is re-sized ONLY from a fresh run of the `flicker_redraw_distribution` generator.
//!    The last run was on the A7b tree (msvc release, 2026-09-18): the pooled wake probability
//!    fell from 160/216 to 20/76 at height 5 and from 48/78 to 2/32 at height 4, and the
//!    budgets came down with it (G2 18 -> 3, G7 56 -> 6). Never raise a budget to turn a red
//!    green.
//! * **Row moves** (design Decision 1 case 5). A row move can change the axis hint a box pair
//!   reads from the box-axis cache while the poses stay put. Before A7b that could change
//!   whether a knife-edge pair had a manifold at all (A4's Known behaviour 2). Since A7b it
//!   cannot: for boxes with non-zero extents a box-box manifold exists exactly when the two
//!   poses overlap, whatever the hint, and the only exception is a reference face with a zero
//!   in-plane extent (`narrowphase/box_box.rs`; A7-N11 pins it over every hint). Known
//!   behaviour 2 is retired for box-box pairs: a hint change can still change WHICH contact a
//!   pair carries, never whether it has one, so never an island's manifold count. G3 and G4
//!   move one pile row across the pile (a swap-remove); G5 and G6 shift every pile row by one
//!   (a spawn into an archetype walked earlier). All four require that nothing wakes, and a
//!   failure names the kind of wake that fired: "count changed" (with every pair whose
//!   manifold appeared or vanished, each classed knife-edge or load-bearing), "count
//!   unchanged" or "latch lost". Every kind is a defect that blocks the commit — a
//!   "count changed" wake on knife-edge pairs only included.
//!
//! **Defect A7.** A resting box pyramid crept sideways with a fixed (-x, -z) bias and its
//! contact set kept changing at rest, so piles of height 7 or more never reached a 60-step
//! quiet window. It was two narrowphase defects. **A7a:** clipped face-contact points
//! inherited `min(prev, cur)` corner ids, so points of one manifold shared a warm-start key;
//! clipped points now carry injective ids. **A7b:** on a resting face pair the SAT took an
//! edge axis that nearly duplicates the face normal, and replaced a 4-point support with one
//! point on jitter; S5 applies Box3D's face-versus-edge rule (`FACE_AXIS_PREFERENCE` in
//! `narrowphase/box_box.rs`). Its tests — A7-R0 (a face contact that repeats a feature id),
//! A7-R1 (Jolt's pyramid creeps with sleeping off), A7-R2 (Jolt's pyramid freezes) and G8
//! (a height-6 pile freezes) — are all green, and none is `deferred:` any more.
//!
//! **What remains is a drift RATE, not a bounded offset.** One run of A7-R1's scene extended
//! to 5400 steps (msvc release, 2026-09-18, the kernel as committed) read D_max = 0.7180 mm
//! over steps 600-3000 and 1.0965 mm over 3000-5400 — on the same box in both windows (1227,
//! layer 12), with layers 10-14 moving toward (-x, -z) on average in both — and 1.8138 mm over
//! 600-5400: the windows add. That is ~0.38 mm per 1000 steps, ~2.3e-5 m/s, ~8 cm per hour of
//! simulated time with sleeping off, which is the default. It is 14× under A7-R1's bound over
//! A7-R1's window. It is not A7b: over steps 600-3000, 0 of 9 743 983 support manifold-steps
//! were on the edge path (A7a alone: 33.3 %), so the mechanism A7b removed is not acting on
//! the supports. It is an open question in `docs/OPEN-QUESTIONS.md`. Every figure in this
//! paragraph was read with contact reuse off, the default until L9 C4; with reuse on (the
//! default since) A7-R1 reads 0.6670 mm over steps 600-3000, and the 5400-step extension has
//! not been re-run.
//!
//! ## S5's rung, pre-registered before any pile run of it (2026-09-18)
//!
//! S5 (A7b) makes the SAT prefer a face axis over an edge axis unless the edge is shallower
//! than the face's REALIZED clipped patch by more than `FACE_AXIS_PREFERENCE` = 0.005 m, or
//! the face realizes no patch — the form Box3D's live `convex_manifold.c` ships, except that
//! here only an EMPTY clip counts as no face patch (Box3D: fewer than 3 vertices) and only
//! points with `separation <= 0` are kept (Box3D keeps speculative points), two differences
//! that predate S5. The C2 probe measured a different form, the edge against the face axis's
//! SAT depth, at D_max =
//! 0.0005157 m (0.0007534..0.0008507 m under ±1e-6 / ±2e-6 gravity perturbations). The
//! shipped form is judged against its own bands, written here before its first pile run.
//!
//! Conditions: A7a + S5, height 15, substeps 4, relax 2, sleeping off, D_max = the largest
//! horizontal displacement of any pile box between steps 600 and 3000 (A7-R1's reading),
//! msvc release.
//!
//! | D_max | verdict |
//! |---|---|
//! | < 0.002 m | **Confirms** the realized form: the order of the SAT form's 0.0005157 m. |
//! | [0.002, 0.01) m | **Acceptable, and reported** beside the SAT form's reading: green against A7-R1's bound, but not the probe's result. |
//! | >= 0.01 m | **STOP.** The realized form fails A7-R1's bound. Reported; the choice between it and the SAT form, with a stated validity band, is the orchestrator's. |
//!
//! **Reading: D_max = 0.0008583 m** (box 1240, layer 14) on the form first implemented — the
//! "confirms" row: 1.66× the SAT form's unperturbed 0.0005157 and 1.01× its worst perturbed
//! run. The review of S5 then adopted one more piece of Box3D's behaviour: a held face hint
//! that realizes no patch yields to the best face's patch when the choice already built it,
//! instead of falling back to the edge. **On the form as committed the rung reads D_max =
//! 0.0007180 m** (box 1227, layer 12), the same row. The receipts are that run's, taken on a
//! temporary copy of the kernel carrying relaxed counters and a per-call pre-S5 oracle
//! (restored by copy, sha256-checked); the first form's are in brackets:
//!
//! * Support manifold-steps on the edge path: 0 of 4060 supports at step 600, and 0 of
//!   9 743 983 over the window [1 of 9 743 988] (A7a alone: 33.3 %).
//! * Support point-count changes over the window: 1836 [2010] (A7a alone: 407 373; SAT form:
//!   1854).
//! * **The 5 mm comparison never chose the edge.** The SAT answered an edge 3 531 050 times
//!   [3 425 959], and the realized rule took the edge 264 times [260] — every one of them a
//!   face that realized no patch. The comparison against the patch chose the edge 0 times
//!   [0]: on this pile the value of `FACE_AXIS_PREFERENCE` does nothing, and what moved the
//!   supports off the edge path is comparing against the realized patch at all.
//! * **Fallbacks: 12 572 over the window [13 615].** The SAT-form probe had no fallback, so
//!   this is where the shipped form differs from the one measured. On 10 446 of them [11 294]
//!   the pre-S5 rule, given the same poses and hint, built an edge contact too: it held the
//!   edge hint that S5's class clause refuses. The other 2126 [2321], under one a step, are
//!   manifolds S5 adds. 12 567 were knife-edge pairs whose own best face realized no patch and
//!   5 a held face with no built patch to yield to; the yield itself ran once, before step
//!   600. Of the 12 836 edge-path manifold-steps (the fallbacks plus the 264), 4048 are
//!   same-layer face neighbours, 8788 same-layer diagonal neighbours and none a support; 3588
//!   have a separation above 0, at most 1.96e-6 m.
//! * Standing guard: worst drop 0.007229 m (box 1240) [0.007202 m].
//! * Height 7: D_max = 0.0003023 m, unchanged by the yield (SAT form: 0.0001360).
//! * At step 600: 6671 manifolds, 22 975 contact points [6678 / 22 974] (A7a alone: 5223 /
//!   14 605; SAT form: 6647 / 22 899). The live manifold count changed on 1739 of 2400 steps
//!   [1821 of 2399] (SAT form: 0.7338).
//!
//! A7-R1 itself, on the restored tree, then passed and printed the same D_max.
//!
//! # Legs
//!
//! * G1, G3, G4, G5 (height-4 piles, 30 boxes) and G8 (height 6, 91 boxes) run in both
//!   profiles, in the ordinary `cargo test -p boyko-physics --test sleep_settles_box_piles`
//!   run. G8 takes 2.4 s in debug.
//! * G2, G6 (height 5, 55 boxes), G7 (sixteen height-4 draws), A7-R1 and A7-R2 (height 15,
//!   1240 boxes) run ONLY in release, as part of the physics release run that `CLAUDE.md`
//!   names as their leg: `cargo test --release -p boyko-physics --no-fail-fast` (this file
//!   alone: `cargo test --release -p boyko-physics --test sleep_settles_box_piles`). There
//!   this binary prints `running 13 tests` and `12 passed; 0 failed; 1 ignored` (the
//!   generator). In a debug build they are ignored, and `-- --ignored` in a debug build is NOT
//!   their leg. A7-R1 is the long one: ~73 s in release, ~80 s for the whole binary before
//!   the SIMD on/off differential below was added (it runs A7-R1's scene twice more; its
//!   time is not measured yet). Since L9 C4 A7-R1 runs its scene twice itself, contact reuse
//!   on (the default) and off: 85.8 s for both in release (2026-09-24, on a shared machine, so
//!   a liveness figure, not a timing).
//! * `simd_solve_on_off_bit_identical` (release only, like A7) runs G2's scene, G7's sixteen
//!   draws and A7-R1's scene once with `simd_solve` on (the default since 2026-09-18) and
//!   once off, and requires the per-frame state hash, the contact-change wake events and
//!   A7-R1's D_max to be bit-identical: the O7 AVX2 cohort kernel is a pure speed path over
//!   the scalar colored oracle, so no budget and no bound here may move with the flag.
//! * A7-R0 is device-free and schedule-free. It runs in the ordinary run in both profiles,
//!   and under Miri.
//! * `flicker_redraw_distribution` is a `generator:`: it asserts nothing and prints the
//!   numbers G2's and G7's budgets are sized from. Re-run it (release, `--ignored --exact`)
//!   whenever a change re-draws pile trajectories, before touching a budget.
//!
//! # Scene
//!
//! The real `add_physics_colored_solve` schedule on a serial pool, sleeping on with the
//! default `sleep_frames` (60) and threshold, dt = 1/60, gravity (0, -9.81, 0), and the
//! default `simd_solve` (ON since 2026-09-18: the O7 AVX2 cohort kernel, bit-identical to the
//! scalar colored oracle the budgets and bounds here were first measured on). The archetype
//! `marked = {RigidBody, RigidBodyMass, Collider, BodyId, Marker}` is created FIRST, then
//! `plain` without `Marker`, so anything spawned into `marked` is walked before every `plain`
//! row. The floor is a static box of half-extents (50, 1, 50) at (0, -1, 0). The pile is the
//! Jolt `PyramidScene` loop (`benches/jolt_parity_pyramid.rs`) with its height as a parameter
//! and `BOX_SEPARATION = 0`: unit-half-extent boxes, `inv_mass` 0.125, inverse inertia 0.1875
//! on the diagonal, friction 0.5, restitution 0. Layer `i` of a height-`h` pile holds
//! `(h - i)²` boxes.
//!
//! # Why G3, G4, G5 and G6 see no box-axis-cache clear
//!
//! The plugin builds `Manifolds::with_capacity(1024)`, which boots the axis table with 2048
//! slots. A growth clear needs more than 1024 candidate pairs on one step, and an occupancy
//! clear more than 1024 live keys; the table stores a key only for a box pair that produced a
//! contact, and drops none until a clear.
//!
//! * G3, G4 and G5 (at most 45 rows): 45 * 44 / 2 = 990 distinct row-pair keys and at most
//!   that many candidate pairs, so neither clear can fire. That argument needs
//!   `Manifolds::with_capacity(n)` with n >= 513 (at n = 512 the table has only 1024 slots);
//!   each test asserts the row count stays at most 45 at every structural change.
//! * G6 (57 rows, height 5): the row bound does not hold, but a pair-count bound does, and G6
//!   measures it rather than arguing it. The harness counts the DISTINCT candidate row pairs
//!   of every step under each row set; the table is keyed by row pair and stores an entry
//!   only for a pair that produced a contact, so the live keys after the shift are at most
//!   the two counts added. G6 asserts that sum is at most 1024 and that no step had more than
//!   1024 candidate pairs, which rules out both clears whatever the pile's creep does. (The
//!   lattice predicts 315 per row set: 55 floor, 80 lateral, 60 diagonal, 120 vertical.) G6
//!   therefore adds scale over G5, not the clear path (OQ5), which stays unmeasured until the
//!   height-15 shift hold returns after A7.
//!
//! All four assert both remap-reset counters stay flat (no missed gather).
//!
//! Every scene runs on its own thread under a watchdog: a panic inside a scheduler worker
//! does not propagate out of `Schedule::run` today, it blocks the caller. The budgets are
//! liveness guards, not measurements. Device-free; the schedule spins up a
//! `boyko_threadpool`, which is intractable under Miri.

use std::panic;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_macros::Component;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::BodyIndex;
use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::narrowphase::box_box::box_box_contact;
#[cfg(feature = "narrowphase-counts")]
use boyko_physics::narrowphase::box_box::fallback_census;
use boyko_physics::narrowphase::{feature_face_clip, feature_face_face};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{
    ConstraintGraph, ContactPairs, IslandSleep, Manifolds, PhysicsConfig, SolverScratch,
};

// ── Scene constants ──────────────────────────────────────────────────────────

/// The fixed step (60 Hz).
const DT: f32 = 1.0 / 60.0;
/// Jolt's box edge length.
const BOX_SIZE: f32 = 2.0;
/// Half of [`BOX_SIZE`].
const HALF_BOX: f32 = 1.0;
/// The layer gap: 0, so the pile starts exactly touching (Jolt uses 0.5).
const BOX_SEPARATION: f32 = 0.0;
/// The static floor box: top face at `y = 0`.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
/// The small pile's height: 16 + 9 + 4 + 1 = 30 boxes.
const SMALL: usize = 4;
/// The largest height that freezes with the fix (round-2 probe): 55 boxes.
const MEDIUM: usize = 5;
/// G8's height: 91 boxes.
const HEIGHT_6: usize = 6;
/// Jolt's pyramid height: 1240 boxes.
const JOLT: usize = 15;
/// Settle budget (steps) for the height-4 single scenes.
const SMALL_SETTLE_LIMIT: usize = 1500;
/// Settle budget (steps) for G8 and A7-R2.
const LONG_SETTLE_LIMIT: usize = 6000;
/// Settle budget (steps) for G2 and G6: twice the latest freeze of the 56 height-5 draws
/// `flicker_redraw_distribution` measured on the A7b tree (step 243, msvc release,
/// 2026-09-18), so that G2's event budget, not the step limit, is what a flicker overrun
/// reaches first: the most-woken draw took its four events by step 183. G2's own draw froze
/// at step 67 (step 65 at L9 C4, contact reuse on by default, 2026-09-24). Before A7b the
/// latest freeze was step 9987 and this limit 20 000.
const MEDIUM_SETTLE_LIMIT: usize = 2 * 243;
/// Settle budget (steps) for each of G7's draws: eight times the latest freeze of the 30
/// height-4 draws measured on the A7b tree (step 127, the same run). Eight, not two, because
/// one draw may spend all of [`G7_MAX_EVENTS`]: seven latch attempts, the last near step
/// 63 + 6 × 60 = 423, and the budget must fire before the step limit. Before A7b this was
/// eight times step 750, [`LONG_SETTLE_LIMIT`].
const G7_SETTLE_LIMIT: usize = 8 * 127;
/// G2's cap on contact-change wake events (steps on which `contact_wakes` rose), re-derived
/// from the generator's run on the A7b tree (msvc release, 2026-09-18). This file's rule is
/// the smallest budget whose tail `P(>= budget + 1 events)` ([`tail_probability`]) is below
/// 1 % at the measured wake probability plus one standard error, and it alone gives 3: with
/// one draw the tail is `p^4`, 0.48 % at the measured [`MEASURED_P_HEIGHT_5`] = 20/76 and
/// 0.97 % at p = 0.314 (one standard error above it).
///
/// The budget is 4 because the SAMPLE refutes 3. One of the 56 measured draws took 4 events,
/// so a budget of 3 reds about 1 re-drawn trajectory in 56, where the pooled geometric
/// model says 0.48 %: wake events cluster within a draw (that draw woke at steps 62, 63, 123
/// and 183), and a model that treats them as independent understates the tail. A budget is
/// therefore never set below the largest count its own generator sample produced. At 4 the
/// model's tail is `p^5`: 0.13 % at p and 0.30 % at one standard error above it. G2's own draw
/// takes 0, at L9 C4 too. Before A7b this was 18, sized for p = 160/216; a budget sized for the
/// pre-fix flicker is one the pre-fix flicker passes.
const G2_MAX_EVENTS: usize = 4;
/// G7's cap on the contact-change wake events summed over its sixteen draws, re-derived by
/// the same rule from the same run: `P(>= 7 events)` over sixteen draws is 0.03 % at the
/// measured [`MEASURED_P_HEIGHT_4`] = 2/32 and 0.59 % at p = 0.105 (one standard error above
/// it); a budget of 5 would be 1.8 % there. G7's own draws take 2 (movers 2 and 9, one
/// each); at L9 C4 (contact reuse on by default, 2026-09-24) they take 1 (mover 12). Before A7b
/// this was 56, sized for p = 48/78.
const G7_MAX_EVENTS: usize = 6;
/// The per-latch-attempt wake probability at height 4, pooled over the 30 height-4 draws
/// of `flicker_redraw_distribution` on the A7b tree (2 events in 32 attempts, msvc release,
/// 2026-09-18; before A7b, 48 in 78).
const MEASURED_P_HEIGHT_4: f64 = 2.0 / 32.0;
/// The same at height 5, pooled over its 56 draws (20 events in 76 attempts; before A7b,
/// 160 in 216).
const MEASURED_P_HEIGHT_5: f64 = 20.0 / 76.0;
/// G4's mover: layer 0's `j = 3, k = 0` corner, spawned at (2, 1, -4). A pinned
/// trajectory, not a structural property — every change to contact ids, impulses or
/// ordering re-draws it, so the premise is re-measured over all 16 layer-0 movers each time
/// and a failing mover is replaced from that probe, never by deleting the premise.
///
/// * Round-2 probe (`e2_movers.txt`, msvc release, before A7a): freezes at step 65 with no
///   contact wake; two face-face sideways manifolds (partners 9 and 14) whose reference box
///   swaps to the mover when it moves into row 1. 8 of the 16 movers failed the premise.
/// * Re-measured after A7a gave clipped points their own feature ids (2026-09-18, msvc,
///   debug and release printing identical lines): freezes at step 65 with no contact wake;
///   ONE face-face sideways manifold, partner 9, reference box 9 -> 13 (the partner-14
///   manifold no longer forms). Again 8 of 16 pass: 2, 3, 7, 10, 12, 13, 14, 15. Seven have
///   no lateral manifold before the move (1, 4, 5, 6, 8, 9, 16) and one keeps its
///   reference box (11). Only mover 7 swaps two (partners 6 and 11).
/// * Re-measured after A7b (S5, the face-versus-edge rule; 2026-09-18, msvc, a temporary
///   probe running this file's `swap_remove_scene` for each of the 16 movers, debug and
///   release printing identical lines): mover 13 freezes at step 64 with no contact wake;
///   TWO face-face sideways manifolds again, partners 9 and 14, whose reference boxes both
///   swap to 13 (9 -> 13, 14 -> 13). 13 of 16 pass: 3 and 5..=16. Three have no lateral
///   manifold before the move (1, 2, 4). Mover 13 is kept.
/// * Re-measured at L9 C4 (contact reuse on by default; 2026-09-24, msvc, the same temporary
///   probe over the 16 movers, debug and release printing identical lines). With reuse ON no
///   mover passes: 13 have face-face sideways manifolds and none swaps its reference box, and
///   three (1, 4, 5) have none. None of the 13 woke. A flipped pair of the frozen pile is served
///   from its record, and the record keeps its reference box across a row-order flip (L9
///   ruling W2), so the premise is a property of the full collision. With reuse OFF on the same
///   binary the A7b reading comes back exactly: 13 of 16 pass (3 and 5..=16), and mover 13
///   swaps partners 9 and 14 to 13. So G4 runs with contact reuse off (the L9 design's rule for
///   a test whose subject is the exact narrowphase), and mover 13 is kept.
const G4_MOVER_ID: u32 = 13;
/// A7-R1's window: the vertical settle is over by `CREEP_FROM`.
const CREEP_FROM: usize = 600;
const CREEP_TO: usize = 3000;
/// A7-R1's bound on horizontal displacement over the window: 2 × Box2D's `B2_LINEAR_SLOP`.
const CREEP_BOUND_M: f32 = 0.01;
/// A7-R1's exact reading on the default config, pinned by its bits: D_max = 0.0006670445 m
/// (`0x3a2e_dc99`; box 1240, layer 14), with contact reuse on. Re-pinned at L9 C4 (contact reuse
/// on by default; msvc release, 2026-09-24) from the old value 0.0007180063 m (`0x3a3c_3896`;
/// box 1227, layer 12), read at L9 C3 (`aef7dda4`) when the default had reuse off, which is now
/// [`A7_R1_D_MAX_BITS_REUSE_OFF`]. The reading is in the L9 design's pre-registered "confirms"
/// band (under 2 mm). The comparison is on bits. The test prints D_max with `{}`, which is
/// `f32`'s shortest round-trip form, so the printed decimal parses back to exactly these bits.
///
/// This is a regression pin, separate from [`CREEP_BOUND_M`], which is an acceptance
/// criterion and is never tightened toward a reading. The pin makes "nothing moved"
/// mechanical: until 2026-09-23 A7-R1 checked only the bound, and a scratch mutation that
/// turned contact reuse on by default moved D_max to 0.0006670445 m with the test green.
///
/// **Re-pin rule.** The value moves only with a commit that changes values by design (a
/// value-changing lever). That commit re-reads it from A7-R1's own `D_max` line (msvc
/// release), re-pins it here in the same commit, and names the lever and the old value in
/// this doc. In a commit that claims bit identity, a change is a defect, never a re-pin.
const A7_R1_D_MAX_BITS: u32 = 0x3a2e_dc99;
/// A7-R1's exact reading with contact reuse OFF, the exact narrowphase: D_max = 0.0007180063 m
/// (`0x3a3c_3896`; box 1227, layer 12). Read in msvc release at L9 C3 (`aef7dda4`), when it was
/// the default's, and again on the L9 C4 tree by the reuse-off arm: the L9 design requires that
/// arm to read it exactly. The S5 rung recorded it as 0.0007180 m (module header).
///
/// **Re-pin rule.** [`A7_R1_D_MAX_BITS`]'s, except that no contact-reuse change may move it:
/// with reuse off no reuse code runs.
const A7_R1_D_MAX_BITS_REUSE_OFF: u32 = 0x3a3c_3896;
/// A7-R1's standing guard: the largest vertical drop any pile box may have taken by
/// [`CREEP_FROM`]. Half a box edge — losing one layer costs a full [`BOX_SIZE`], while the
/// vertical settle's penetration slop is millimetres per layer. Without it a pile that had
/// already collapsed and come to rest before [`CREEP_FROM`] meets [`CREEP_BOUND_M`] for every
/// box and greens the very test meant to certify A7 (round-3 review O1); every sibling in
/// this file carries such a guard.
const STANDING_DROP_M: f32 = HALF_BOX;
/// The largest row count for which the row-count no-clear argument holds.
const NO_CLEAR_MAX_ROWS: usize = 45;
/// Half the box-axis table's 2048 boot slots: a growth clear needs more candidate pairs than
/// this on one step, and an occupancy clear more live keys (module header).
const AXIS_CLEAR_LIMIT: usize = 1024;
/// A pair of settled centres is "touching" when every axis-aligned gap is at most this.
const TOUCH_GAP: f32 = 1.0e-3;
/// The generator's settle budget.
const GENERATOR_SETTLE_LIMIT: usize = 20_000;

/// Liveness budgets. NOT performance measurements: sized far above any honest run so that
/// only a hang reaches them.
const SMALL_TIMEOUT: Duration = Duration::from_secs(if cfg!(debug_assertions) { 300 } else { 120 });
const MEDIUM_TIMEOUT: Duration = Duration::from_secs(600);
const G7_TIMEOUT: Duration = Duration::from_secs(900);
const JOLT_TIMEOUT: Duration = Duration::from_secs(1200);
const GENERATOR_TIMEOUT: Duration = Duration::from_secs(3600);

/// Body identities carried in [`BodyId`]. Pile boxes are `1..=n` in Jolt loop order.
const FLOOR_ID: u32 = 0;
const LONE_ID: u32 = 90_000;
const FAR_ID: u32 = 90_001;

// ── Test-only components ─────────────────────────────────────────────────────

/// Stable identity of a body, so assertions follow entities rather than rows.
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct BodyId {
    id: u32,
}

/// Puts a body into the `marked` archetype, which is walked before `plain`.
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct Marker {
    _tag: u32,
}

// ── Watchdog ─────────────────────────────────────────────────────────────────────

fn under_watchdog<T, F>(what: &str, budget: Duration, scene: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let handle = thread::Builder::new()
        .name("a4-box-pile".to_string())
        .spawn(move || {
            let outcome = scene();
            done_tx.send(()).ok();
            outcome
        })
        .expect("harness: the scene thread must spawn");
    if let Err(mpsc::RecvTimeoutError::Timeout) = done_rx.recv_timeout(budget) {
        panic!(
            "watchdog: the scene '{what}' did not finish within {budget:?}. A panic inside a \
             scheduler worker does not propagate out of `Schedule::run` today, it blocks the \
             caller; the worker's own message, if any, is printed above this one."
        );
    }
    match handle.join() {
        Ok(outcome) => outcome,
        Err(payload) => panic::resume_unwind(payload),
    }
}

// ── Flicker budgets ──────────────────────────────────────────────────────────

/// A cap on contact-change wake events, checked the moment an event is counted.
#[derive(Clone, Copy, Debug)]
struct Budget {
    /// The largest allowed number of events, summed over every draw of the test.
    max: usize,
    /// Events already counted by earlier draws of the same test.
    spent: usize,
    /// Earlier draws of the same test that froze (each is one successful latch attempt).
    frozen_draws: usize,
    /// Draws the budget covers.
    draws: u32,
    /// The measured per-attempt wake probability the budget was sized against.
    measured_p: f64,
    /// The failure message's opening words, before the event count.
    label: &'static str,
}

/// `P(X >= at_least)` for `X` the total wake events of `draws` independent draws whose
/// latch attempts are each woken with probability `p` (a sum of geometric variables: a
/// negative binomial).
fn tail_probability(draws: u32, p: f64, at_least: usize) -> f64 {
    let r = f64::from(draws);
    let mut term = (1.0 - p).powf(r);
    let mut below = 0.0;
    for x in 0..at_least {
        below += term;
        term *= p * (x as f64 + r) / (x as f64 + 1.0);
    }
    (1.0 - below).max(0.0)
}

/// The triage paragraph every flicker failure carries (round-2 critique W2).
fn flicker_triage(events: usize, attempts: usize, budget: &Budget) -> String {
    let p_hat = events as f64 / attempts.max(1) as f64;
    let tail = tail_probability(budget.draws, budget.measured_p, budget.max + 1);
    format!(
        "Triage: {events} events over at least {attempts} latch attempts, so the observed wake \
         fraction is at least {p_hat:.2}, against the measured p = {:.2} at this height; with that \
         p, P(>= {} events over {} draw(s)) = {tail:.4}. A fraction inside the measured range means \
         a re-drawn chaotic trajectory (re-size the budget from `flicker_redraw_distribution`); \
         one well above it means a flicker regression.\n\
         This red is TRIAGED, never waived and never silenced by raising the budget \
         (orchestrator ruling, round 3). Until A4's Decision 3 is decided again (due now that \
         A7 has landed):\n\
         1. Re-run this test under the M1 protocol (design-A4 §5.2 step 1, the wake block \
         deleted). M1 separates a flicker budget overrun from a pile that no longer comes to \
         rest.\n\
         2. If the M1 leg ALSO fails, this red BLOCKS: the pile stopped resting, which is not a \
         budget question.\n\
         3. If the M1 leg passes, re-run this test once on a fresh draw. A second red on an \
         INDEPENDENT draw BLOCKS. A single red is recorded in the A7 entry of \
         docs/OPEN-QUESTIONS.md together with the numbers above (observed events, the measured p \
         for this height, and the negative-binomial tail) and does not block.\n\
         4. A budget is re-sized ONLY from a fresh run of the `flicker_redraw_distribution` \
         generator (the last: the A7b tree, 2026-09-18). Never raise a budget to turn a red \
         green.\n\
         Escalate to the orchestrator with these numbers: Decision 3 is to be decided again now \
         that A7 has landed (A4 round-2 ruling §3.4), and has not been yet",
        budget.measured_p,
        budget.max + 1,
        budget.draws
    )
}

// ── Harness ──────────────────────────────────────────────────────────────────

/// Views a `#[repr(C)]` POD value as its bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, initialised `#[repr(C)]` `T` borrowed for the returned
    // slice's lifetime; the slice covers exactly its `size_of::<T>()` bytes read-only, the
    // layout the component pool stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// The Jolt pyramid loop with a height parameter: box centres in spawn order.
fn pyramid_positions(height: usize) -> Vec<Vec3> {
    let h = height as i32;
    let mut out = Vec::new();
    for i in 0..h {
        let lo = i / 2;
        let hi = h - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                out.push(Vec3::new(
                    -(h as f32) + BOX_SIZE * j as f32 + odd,
                    1.0 + (BOX_SIZE + BOX_SEPARATION) * i as f32,
                    -(h as f32) + BOX_SIZE * k as f32 + odd,
                ));
            }
        }
    }
    out
}

/// The layer of pile id `id` (Jolt loop order) in a pile of `height`: layer `i` holds
/// `(height - i)²` boxes.
fn layer_of(height: usize, id: u32) -> usize {
    let mut first = 1u32;
    for i in 0..height {
        let n = ((height - i) * (height - i)) as u32;
        if id < first + n {
            return i;
        }
        first += n;
    }
    panic!("harness: pile id {id} is past a height-{height} pile")
}

/// Every field of a `RigidBody`, as bits.
fn body_bits(b: &RigidBody) -> [u32; 13] {
    [
        b.position.x.to_bits(),
        b.position.y.to_bits(),
        b.position.z.to_bits(),
        b.linear_velocity.x.to_bits(),
        b.linear_velocity.y.to_bits(),
        b.linear_velocity.z.to_bits(),
        b.rotation.x.to_bits(),
        b.rotation.y.to_bits(),
        b.rotation.z.to_bits(),
        b.rotation.w.to_bits(),
        b.angular_velocity.x.to_bits(),
        b.angular_velocity.y.to_bits(),
        b.angular_velocity.z.to_bits(),
    ]
}

struct Harness {
    world: EcsMaster,
    physics: Schedule,
    marked: ArchetypeId,
    plain: ArchetypeId,
    /// Every entity spawned, by body id.
    entities: Vec<(u32, Entity)>,
    /// Candidate pairs seen since the last [`Harness::track_pairs`], as a row-pair bitmap
    /// (empty while not tracking).
    pairs: PairTracker,
}

/// The candidate pairs of every step under one row set.
#[derive(Debug, Default)]
struct PairTracker {
    /// The row count the bitmap is sized for; 0 while not tracking.
    rows: usize,
    /// `seen[min * rows + max]` for every candidate pair `(min, max)` seen.
    seen: Vec<bool>,
    /// Distinct candidate pairs seen.
    distinct: usize,
    /// The largest candidate-pair count of one step.
    max_per_step: usize,
}

/// What phase 1 observed.
#[derive(Debug)]
struct Settled {
    freeze_step: usize,
    contact_wake_rises: Vec<usize>,
    contact_wakes: u64,
}

/// What a hold observed.
#[derive(Debug, Default)]
struct Hold {
    /// `(step, awake pile ids)` for the first steps on which a pile row was awake.
    awake: Vec<(usize, Vec<u32>)>,
    /// The first step on which `contact_wakes` rose.
    first_contact_wake: Option<usize>,
    /// The pile's manifold count on every step.
    counts: Vec<usize>,
}

/// One manifold of the pile, by body id: `a < b`, and its deepest point's separation.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PairContact {
    a: u32,
    b: u32,
    min_separation: f32,
}

impl Harness {
    fn new() -> Self {
        let mut world = EcsMaster::new();
        let marked = world.create_archetype(&[
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
            BodyId::component_id(),
            Marker::component_id(),
        ]);
        let plain = world.create_archetype(&[
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
            BodyId::component_id(),
        ]);
        let mut builder = ScheduleBuilder::new(serial_pool());
        let _keys = add_physics_colored_solve(&mut builder, &mut world);
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        let physics = builder.build(&mut world);
        {
            let cfg = world.resource_mut::<PhysicsConfig>();
            cfg.sleeping = true;
            cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
            cfg.dt = DT;
        }
        Self {
            world,
            physics,
            marked,
            plain,
            entities: Vec::new(),
            pairs: PairTracker::default(),
        }
    }

    fn spawn(
        &mut self,
        id: u32,
        body: RigidBody,
        mass: RigidBodyMass,
        collider: Collider,
        marked: bool,
    ) -> Entity {
        let tag = BodyId { id };
        let marker = Marker { _tag: 0 };
        let created = if marked {
            self.world.create_entity(
                self.marked,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                    (BodyId::component_id(), as_bytes(&tag)),
                    (Marker::component_id(), as_bytes(&marker)),
                ],
            )
        } else {
            self.world.create_entity(
                self.plain,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                    (BodyId::component_id(), as_bytes(&tag)),
                ],
            )
        };
        let entity = created.expect("construction: the archetype accepts every column");
        if mass.inv_mass != 0.0 {
            self.world.enable::<Simulated>(entity);
        }
        self.entities.push((id, entity));
        entity
    }

    fn at_rest(position: Vec3) -> RigidBody {
        RigidBody {
            position,
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        }
    }

    fn spawn_floor(&mut self) {
        self.spawn(
            FLOOR_ID,
            Self::at_rest(Vec3::new(0.0, -1.0, 0.0)),
            RigidBodyMass {
                inv_inertia: Mat3::ZERO,
                inv_mass: 0.0,
                restitution: 0.0,
                friction: 0.5,
            },
            Collider {
                shape: ColliderShape::Box {
                    half_extents: FLOOR_HALF_EXTENTS,
                },
                layer: 1,
                mask: 1,
            },
            false,
        );
    }

    fn spawn_box(&mut self, id: u32, position: Vec3) -> Entity {
        self.spawn(
            id,
            Self::at_rest(position),
            RigidBodyMass {
                inv_inertia: Mat3::from_diagonal(Vec3::new(0.1875, 0.1875, 0.1875)),
                inv_mass: 0.125,
                restitution: 0.0,
                friction: 0.5,
            },
            Collider {
                shape: ColliderShape::Box {
                    half_extents: Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
                },
                layer: 1,
                mask: 1,
            },
            false,
        )
    }

    fn spawn_far_sphere(&mut self) -> Entity {
        self.spawn(
            FAR_ID,
            Self::at_rest(Vec3::new(200.0, 50.0, 0.0)),
            RigidBodyMass {
                inv_inertia: Mat3::IDENTITY,
                inv_mass: 1.0,
                restitution: 0.0,
                friction: 0.5,
            },
            Collider {
                shape: ColliderShape::Sphere { radius: 0.5 },
                layer: 1,
                mask: 1,
            },
            true,
        )
    }

    fn entity(&self, id: u32) -> Entity {
        self.entities
            .iter()
            .find(|&&(spawned, _)| spawned == id)
            .map(|&(_, e)| e)
            .unwrap_or_else(|| panic!("harness: body {id} was never spawned"))
    }

    fn step(&mut self) {
        self.physics.run(&mut self.world);
        let tracker = &mut self.pairs;
        if tracker.rows > 0 {
            let pairs = self.world.resource::<ContactPairs>().pairs();
            tracker.max_per_step = tracker.max_per_step.max(pairs.len());
            for &(a, b) in pairs {
                let (lo, hi) = (a.0.min(b.0) as usize, a.0.max(b.0) as usize);
                assert!(
                    hi < tracker.rows,
                    "harness: row {hi} is past the {} rows the pair tracker was started for; \
                     call track_pairs after every structural change",
                    tracker.rows
                );
                let seen = &mut tracker.seen[lo * tracker.rows + hi];
                if !*seen {
                    *seen = true;
                    tracker.distinct += 1;
                }
            }
        }
    }

    /// Starts counting candidate pairs under the current row set; returns what the previous
    /// row set saw as `(distinct pairs, largest per-step count)`.
    fn track_pairs(&mut self) -> (usize, usize) {
        let rows = self.walk_ids().len();
        let previous = (self.pairs.distinct, self.pairs.max_per_step);
        self.pairs = PairTracker {
            rows,
            seen: vec![false; rows * rows],
            distinct: 0,
            max_per_step: 0,
        };
        previous
    }

    /// Body ids in the gather's walk order: row `i` of the solver is element `i`. Every body
    /// here carries `BodyId`, so this query selects exactly the body set, in its order.
    fn walk_ids(&mut self) -> Vec<u32> {
        let q = self
            .world
            .query::<(&RigidBody, &RigidBodyMass, &Collider, &BodyId), ()>();
        q.iter().map(|(_, _, _, id)| id.id).collect()
    }

    fn row_of(&mut self, id: u32) -> usize {
        self.walk_ids()
            .iter()
            .position(|&x| x == id)
            .unwrap_or_else(|| panic!("harness: body {id} is not in the gather walk"))
    }

    /// Checks the walk against the solver's own snapshot after a step: gather row `i` holds
    /// exactly the pose of the body the walk puts at `i`.
    fn assert_walk_matches_gather(&mut self) {
        let walked: Vec<(u32, [u32; 13])> = {
            let q = self
                .world
                .query::<(&RigidBody, &RigidBodyMass, &Collider, &BodyId), ()>();
            q.iter()
                .map(|(b, _, _, id)| (id.id, body_bits(b)))
                .collect()
        };
        let gathered: Vec<[u32; 3]> = self
            .world
            .resource::<SolverScratch>()
            .bodies()
            .iter()
            .map(|b| {
                [
                    b.position.x.to_bits(),
                    b.position.y.to_bits(),
                    b.position.z.to_bits(),
                ]
            })
            .collect();
        assert_eq!(
            walked.len(),
            gathered.len(),
            "harness: the test's walk and the gather disagree on the row count"
        );
        for (row, ((id, bits), snap)) in walked.iter().zip(&gathered).enumerate() {
            assert_eq!(
                &bits[..3],
                &snap[..],
                "harness: walk row {row} is body {id}, but the gather's row {row} holds another pose"
            );
        }
    }

    fn contact_wakes(&self) -> u64 {
        self.world.resource::<IslandSleep>().contact_wakes()
    }

    /// `(box-axis cache remap resets, sleep latch remap resets)`.
    fn remap_resets(&self) -> (u64, u64) {
        (
            self.world
                .resource::<Manifolds>()
                .box_axis_cache
                .remap_resets(),
            self.world.resource::<IslandSleep>().remap_resets(),
        )
    }

    fn n_islands(&self) -> u32 {
        self.world.resource::<ConstraintGraph>().n_islands()
    }

    fn island_of_row(&self, row: usize) -> u32 {
        self.world
            .resource::<ConstraintGraph>()
            .island_of(row as u32)
    }

    /// The number of manifolds with at least one point on the last step.
    fn manifold_count(&self) -> usize {
        self.world
            .resource::<Manifolds>()
            .manifolds()
            .iter()
            .filter(|m| m.count > 0)
            .count()
    }

    /// Rows of dynamic bodies that were awake on the last step.
    fn dynamic_rows_awake(&self) -> Vec<usize> {
        let sleep = self.world.resource::<IslandSleep>();
        self.world
            .resource::<SolverScratch>()
            .bodies()
            .iter()
            .enumerate()
            .filter(|(row, b)| b.inv_mass != 0.0 && sleep.is_row_awake(*row))
            .map(|(row, _)| row)
            .collect()
    }

    /// Ids in `ids` whose row was awake on the last step.
    fn awake_among(&mut self, ids: &[u32]) -> Vec<u32> {
        let walk = self.walk_ids();
        let sleep = self.world.resource::<IslandSleep>();
        walk.iter()
            .enumerate()
            .filter(|(row, id)| ids.contains(id) && sleep.is_row_awake(*row))
            .map(|(_, &id)| id)
            .collect()
    }

    /// The largest `|v|² + |ω|²` over the dynamic rows of the last step's snapshot.
    fn peak_speed_sq(&self) -> f32 {
        self.world
            .resource::<SolverScratch>()
            .bodies()
            .iter()
            .filter(|b| b.inv_mass != 0.0)
            .map(|b| {
                b.linear_velocity.dot(b.linear_velocity)
                    + b.angular_velocity.dot(b.angular_velocity)
            })
            .fold(0.0, f32::max)
    }

    /// Every body's full state, keyed by id, for `ids`.
    fn poses(&self, ids: &[u32]) -> Vec<(u32, [u32; 13])> {
        ids.iter()
            .map(|&id| {
                let body = self
                    .world
                    .get_component::<RigidBody>(self.entity(id))
                    .expect("harness: a tracked body is live");
                (id, body_bits(body))
            })
            .collect()
    }

    /// Every body's centre, keyed by id, for `ids`.
    fn centres(&self, ids: &[u32]) -> Vec<(u32, Vec3)> {
        ids.iter()
            .map(|&id| {
                let body = self
                    .world
                    .get_component::<RigidBody>(self.entity(id))
                    .expect("harness: a tracked body is live");
                (id, body.position)
            })
            .collect()
    }

    /// Every manifold of the last step with a point and at least one pile body, by id,
    /// sorted. Its length is the pile island's manifold count, the key A4 compares.
    fn pile_pairs(&mut self, pile: &[u32]) -> Vec<PairContact> {
        let walk = self.walk_ids();
        let mut out: Vec<PairContact> = self
            .world
            .resource::<Manifolds>()
            .manifolds()
            .iter()
            .filter(|m| m.count > 0)
            .filter_map(|m| {
                let ia = *walk.get(m.body_a.0 as usize)?;
                let ib = *walk.get(m.body_b.0 as usize)?;
                if !pile.contains(&ia) && !pile.contains(&ib) {
                    return None;
                }
                let min_separation = m.points[..usize::from(m.count)]
                    .iter()
                    .map(|p| p.separation)
                    .fold(f32::INFINITY, f32::min);
                Some(PairContact {
                    a: ia.min(ib),
                    b: ia.max(ib),
                    min_separation,
                })
            })
            .collect();
        out.sort_unstable_by_key(|p| (p.a, p.b));
        out
    }

    /// Phase 1: steps until no dynamic row is awake, recording every step on which
    /// `contact_wakes` rose. With a `budget`, an event past it fails on the step it is
    /// counted, before the step limit can.
    fn settle_until_frozen(&mut self, limit: usize, what: &str, budget: Option<Budget>) -> Settled {
        let mut rises = Vec::new();
        let mut last = self.contact_wakes();
        // Diagnostics for a timeout: the largest dynamic-row speed² over the final
        // `sleep_frames` steps, against the sleep threshold.
        let (threshold, frames) = {
            let cfg = self.world.resource::<PhysicsConfig>();
            (cfg.sleep_threshold, usize::from(cfg.sleep_frames))
        };
        let mut tail_peak = 0.0f32;
        for step in 1..=limit {
            self.step();
            let now = self.contact_wakes();
            if now != last {
                rises.push(step);
                last = now;
                if let Some(b) = budget {
                    let events = b.spent + rises.len();
                    if events > b.max {
                        // Every attempt of this draw so far was woken; each earlier frozen
                        // draw adds its one successful attempt.
                        let attempts = events + b.frozen_draws;
                        panic!(
                            "{what}, {} {events} contact-change wake events (budget {}), the last \
                             at step {step} of this draw (its rises at {rises:?}, contact_wakes \
                             {now}). {}",
                            b.label,
                            b.max,
                            flicker_triage(events, attempts, &b)
                        );
                    }
                }
            }
            if self.dynamic_rows_awake().is_empty() {
                return Settled {
                    freeze_step: step,
                    contact_wake_rises: rises,
                    contact_wakes: now,
                };
            }
            if step + frames > limit {
                tail_peak = tail_peak.max(self.peak_speed_sq());
            }
        }
        let awake = self.dynamic_rows_awake().len();
        panic!(
            "{what}: did not freeze within {limit} steps; contact_wakes rose at steps {rises:?} \
             ({last} total). K > 0: onset flicker (A4 Decision 3, round-2 ruling; attribute under \
             M1). K == 0: the pile does not come to rest (defect A7); run it under M1. On the \
             last step {awake} dynamic rows were awake in {} islands; the largest dynamic-row \
             |v|² + |ω|² over the last {frames} steps was {tail_peak} (sleep threshold \
             {threshold})",
            self.n_islands()
        );
    }

    /// Steps `steps` times, recording the steps on which a row of `pile` was awake, the
    /// first step on which `contact_wakes` left `wakes_before`, and the pile's manifold
    /// count on every step.
    fn hold(&mut self, steps: usize, pile: &[u32], wakes_before: u64) -> Hold {
        let mut out = Hold::default();
        for step in 1..=steps {
            self.step();
            let awake = self.awake_among(pile);
            if !awake.is_empty() && out.awake.len() < 8 {
                out.awake.push((step, awake));
            }
            if out.first_contact_wake.is_none() && self.contact_wakes() != wakes_before {
                out.first_contact_wake = Some(step);
            }
            out.counts.push(self.pile_pairs(pile).len());
        }
        out
    }

    /// Every manifold of `subject` on the last step: `(partner id, normal, point count)`.
    fn manifolds_of(&mut self, subject: u32) -> Vec<(u32, Vec3, u8)> {
        let walk = self.walk_ids();
        let row = walk
            .iter()
            .position(|&id| id == subject)
            .expect("harness: the subject is walked") as u32;
        self.world
            .resource::<Manifolds>()
            .manifolds()
            .iter()
            .filter(|m| m.count > 0 && (m.body_a.0 == row || m.body_b.0 == row))
            .map(|m| {
                let other = if m.body_a.0 == row {
                    m.body_b.0
                } else {
                    m.body_a.0
                };
                (
                    walk.get(other as usize).copied().unwrap_or(u32::MAX),
                    m.normal,
                    m.count,
                )
            })
            .collect()
    }

    /// For every face-face lateral manifold between `subject` and a body of `pile`, the
    /// partner id and the physical reference box's id, from the last step's manifolds.
    ///
    /// The reference face `rf = ref_axis * 2 + ref_positive` (`box_box.rs`) is read with
    /// [`face_face_reference_face`] from EVERY live point, and the points must agree: `rf`
    /// odd means the reference face is the reference box's `+axis` face, so the reference
    /// box is the one with the LOWER centre coordinate along the normal's dominant axis.
    fn lateral_reference_boxes(&mut self, subject: u32, pile: &[u32]) -> Lateral {
        let walk = self.walk_ids();
        let subject_row = walk
            .iter()
            .position(|&id| id == subject)
            .expect("harness: the subject is walked") as u32;
        let centres: Vec<Vec3> = self
            .world
            .resource::<SolverScratch>()
            .bodies()
            .iter()
            .map(|b| b.position)
            .collect();
        let mut out = Lateral::default();
        for m in self.world.resource::<Manifolds>().manifolds() {
            if m.count == 0 {
                continue;
            }
            let (a, b) = (m.body_a.0, m.body_b.0);
            let other = if a == subject_row {
                b
            } else if b == subject_row {
                a
            } else {
                continue;
            };
            let Some(&other_id) = walk.get(other as usize) else {
                continue;
            };
            if !pile.contains(&other_id) || m.normal.y.abs() >= 0.5 {
                continue;
            }
            out.lateral += 1;
            let ids: Vec<u32> = m.points[..usize::from(m.count)]
                .iter()
                .map(|p| p.feature_id)
                .collect();
            let Some(rf) = face_face_reference_face(ids[0]) else {
                continue;
            };
            assert!(
                ids.iter()
                    .all(|&id| face_face_reference_face(id) == Some(rf)),
                "harness: one reference face clips a whole face-face manifold, but the points of \
                 ({subject}, {other_id}) name different ones; feature ids {ids:x?}"
            );
            let n = m.normal;
            let axis = if n.x.abs() >= n.y.abs() && n.x.abs() >= n.z.abs() {
                0
            } else if n.y.abs() >= n.z.abs() {
                1
            } else {
                2
            };
            let coord = |p: Vec3| [p.x, p.y, p.z][axis];
            let (ca, cb) = (coord(centres[a as usize]), coord(centres[b as usize]));
            let (lower, higher) = if ca < cb { (a, b) } else { (b, a) };
            let reference = if rf & 1 == 1 { lower } else { higher };
            out.face_face.push((other_id, walk[reference as usize]));
        }
        out.face_face.sort_unstable();
        out
    }
}

/// Lateral manifolds of one body, from one step.
#[derive(Debug, Default)]
struct Lateral {
    /// Lateral manifolds of any class.
    lateral: usize,
    /// `(partner id, reference box id)` for the face-face ones.
    face_face: Vec<(u32, u32)>,
}

/// The reference-face index `ref_axis * 2 + ref_positive` a face-face feature id carries, or
/// `None` for the edge-edge and vertex-face classes (bit 15 set).
///
/// The two face-face encoders store the reference face at different bits:
/// [`feature_face_face`] (an ORIGINAL incident corner, bit 13 clear) as `ref_face << 3 |
/// corner`, [`feature_face_clip`] (a vertex the clip CREATED, bit 13 set) as `bit 13 |
/// ref_face << 6 | cut_edge << 2 | plane`. Reading only the first layout named the wrong
/// reference box for every clipped point once A7a gave clipped points their own ids. Every
/// decode is re-encoded through the shipped encoder and must reproduce the id, so a future
/// layout change reds on the first face-face contact instead of mis-naming a box.
fn face_face_reference_face(feature_id: u32) -> Option<u32> {
    if feature_id & 0x8000 != 0 {
        return None;
    }
    let (rf, re_encoded) = if feature_id & 0x2000 != 0 {
        let rf = (feature_id >> 6) & 0x7;
        (
            rf,
            feature_face_clip(rf, (feature_id >> 2) & 0xF, feature_id & 0x3),
        )
    } else {
        let rf = feature_id >> 3;
        (rf, feature_face_face(rf, feature_id & 0x7))
    };
    assert_eq!(
        re_encoded, feature_id,
        "harness: face-face feature id {feature_id:#x} does not re-encode to itself as reference \
         face {rf}; this decoder no longer matches `narrowphase::feature_face_face` / \
         `feature_face_clip`"
    );
    Some(rf)
}

/// Floor first, then (optionally) a lone box at (40, 1, 0), then the pile of `height`.
/// `last` names a pile index (Jolt loop order) that is spawned after the rest of the pile.
/// Returns the pile ids, in spawn order.
fn spawn_scene(h: &mut Harness, height: usize, lone: bool, last: Option<usize>) -> Vec<u32> {
    h.spawn_floor();
    if lone {
        h.spawn_box(LONE_ID, Vec3::new(40.0, 1.0, 0.0));
    }
    let positions = pyramid_positions(height);
    let mut order: Vec<usize> = (0..positions.len()).collect();
    if let Some(index) = last {
        order.retain(|&i| i != index);
        order.push(index);
    }
    let mut ids = Vec::with_capacity(order.len());
    for i in order {
        let id = i as u32 + 1;
        h.spawn_box(id, positions[i]);
        ids.push(id);
    }
    ids
}

/// The id of the pile box the Jolt loop spawns last (the top box).
fn top_id(height: usize) -> u32 {
    pyramid_positions(height).len() as u32
}

/// Phases 1 and 2 of G1, G2, G8 and A7-R2: freeze (recording contact-change wakes, and
/// failing on the first event past `budget`), then hold bit-identically.
fn freeze_and_hold(
    h: &mut Harness,
    pile: &[u32],
    settle_limit: usize,
    hold_steps: usize,
    what: &str,
    budget: Option<Budget>,
) -> Settled {
    let settled = h.settle_until_frozen(settle_limit, what, budget);
    println!(
        "{what}: froze at step {} (contact_wakes {}, {} rise events at {:?})",
        settled.freeze_step,
        settled.contact_wakes,
        settled.contact_wake_rises.len(),
        settled.contact_wake_rises
    );
    if let Some(b) = budget {
        // `settle_until_frozen` already failed on the first event past the budget; this is
        // the same bound, restated where the phase ends.
        assert!(
            b.spent + settled.contact_wake_rises.len() <= b.max,
            "{what}, phase 1: {} past the budget after the freeze",
            b.label
        );
    }
    h.assert_walk_matches_gather();

    let poses = h.poses(pile);
    let resets = h.remap_resets();
    let held = h.hold(hold_steps, pile, settled.contact_wakes);
    assert!(
        held.awake.is_empty() && held.first_contact_wake.is_none(),
        "{what}, phase 2: a frozen pile must stay frozen for {hold_steps} steps; awake pile ids by \
         step: {:?}; first contact-change wake at step {:?}; the pile's manifold count by step \
         (frozen at step 0 with K = {}): {:?}",
        held.awake,
        held.first_contact_wake,
        settled.contact_wakes,
        held.counts
    );
    assert_eq!(
        (h.contact_wakes(), h.remap_resets()),
        (settled.contact_wakes, resets),
        "{what}, phase 2: (contact_wakes, (axis-cache remap resets, sleep remap resets)) must stay \
         at their values at the freeze"
    );
    assert!(
        h.poses(pile) == poses,
        "{what}, phase 2: every frozen box must keep its exact pose, velocity and rotation"
    );
    settled
}

// ── The kind of wake a structural change caused (round-2 ruling §5.1 item 11) ──

/// One step of a structural-change trace.
#[derive(Debug)]
struct TraceStep {
    /// The pile's manifolds, by id.
    pairs: Vec<PairContact>,
    /// Pile ids awake on this step.
    awake: Vec<u32>,
    /// `contact_wakes` after this step.
    wakes: u64,
}

/// A frozen pile around one structural change: `steps[0]` is the last step before the
/// change (s = -1), `steps[1]` the step that applies it (s = 0), then the hold.
#[derive(Debug)]
struct Trace {
    steps: Vec<TraceStep>,
    /// Pile centres before the change, for classing a pair.
    centres_before: Vec<(u32, Vec3)>,
}

impl Trace {
    fn start(h: &mut Harness, pile: &[u32]) -> Self {
        let mut trace = Self {
            steps: Vec::with_capacity(61),
            centres_before: h.centres(pile),
        };
        trace.record(h, pile);
        trace
    }

    fn record(&mut self, h: &mut Harness, pile: &[u32]) {
        let step = TraceStep {
            pairs: h.pile_pairs(pile),
            awake: h.awake_among(pile),
            wakes: h.contact_wakes(),
        };
        self.steps.push(step);
    }

    /// A pair is knife-edge when both bodies are pile boxes in the same layer (the pair
    /// carries no weight) whose centres touch: every axis-aligned gap is at most
    /// [`TOUCH_GAP`]; a floor pair or a pair between layers is load-bearing. Before A7b only
    /// knife-edge pairs were exposed to Known behaviour 2 (a hint change changing whether a
    /// box pair has a manifold). Since A7b no box pair is: a box-box manifold exists exactly
    /// when the poses overlap, except on a zero-extent reference face (`narrowphase/box_box.rs`,
    /// pinned by A7-N11). The class is kept because it says which pairs a failure names, not
    /// whether it may be deferred.
    fn is_knife_edge(&self, a: u32, b: u32) -> bool {
        let find = |id: u32| {
            self.centres_before
                .iter()
                .find(|&&(x, _)| x == id)
                .map(|&(_, c)| c)
        };
        let (Some(ca), Some(cb)) = (find(a), find(b)) else {
            return false;
        };
        let d = cb - ca;
        d.y.abs() < HALF_BOX
            && [
                d.x.abs() - BOX_SIZE,
                d.y.abs() - BOX_SIZE,
                d.z.abs() - BOX_SIZE,
            ]
            .iter()
            .all(|&g| g <= TOUCH_GAP)
    }

    /// Passes if nothing woke; otherwise fails naming the kind of wake at the first
    /// disturbed step.
    fn assert_undisturbed(&self, what: &str, change: &str, context: &str) {
        let Some(i) = (1..self.steps.len()).find(|&i| {
            !self.steps[i].awake.is_empty() || self.steps[i].wakes != self.steps[i - 1].wakes
        }) else {
            return;
        };
        let s = i - 1;
        let (prev, cur) = (&self.steps[i - 1], &self.steps[i]);
        let later: Vec<(usize, &Vec<u32>)> = self.steps[i..]
            .iter()
            .enumerate()
            .filter(|(_, st)| !st.awake.is_empty())
            .take(4)
            .map(|(k, st)| (s + k, &st.awake))
            .collect();
        let counts: Vec<usize> = self.steps.iter().map(|st| st.pairs.len()).collect();
        let rose = cur.wakes != prev.wakes;
        let (kind, verdict, detail) = if !rose {
            (
                "latch lost",
                "an A4 or row-identity defect (a Reset or a permute lost the latch); it blocks \
                 the commit",
                String::new(),
            )
        } else if cur.pairs.len() == prev.pairs.len() {
            (
                "count unchanged",
                "an A4 defect: the stored key is not the island's manifold count, or did not \
                 follow its row; it blocks the commit",
                String::new(),
            )
        } else {
            let appeared: Vec<&PairContact> = cur
                .pairs
                .iter()
                .filter(|p| !prev.pairs.iter().any(|q| (q.a, q.b) == (p.a, p.b)))
                .collect();
            let vanished: Vec<&PairContact> = prev
                .pairs
                .iter()
                .filter(|p| !cur.pairs.iter().any(|q| (q.a, q.b) == (p.a, p.b)))
                .collect();
            let class = |p: &PairContact| {
                format!(
                    "({}, {}) depth {:e} {}",
                    p.a,
                    p.b,
                    p.min_separation,
                    if self.is_knife_edge(p.a, p.b) {
                        "knife-edge"
                    } else {
                        "LOAD-BEARING"
                    }
                )
            };
            let knife_edge_only = appeared
                .iter()
                .chain(&vanished)
                .all(|p| self.is_knife_edge(p.a, p.b));
            let detail = format!(
                "; manifolds that appeared: [{}]; vanished: [{}]",
                appeared
                    .iter()
                    .map(|p| class(p))
                    .collect::<Vec<_>>()
                    .join(", "),
                vanished
                    .iter()
                    .map(|p| class(p))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if knife_edge_only {
                (
                    "count changed (knife-edge pairs only)",
                    "a DEFECT: it blocks the commit. With the poses fixed a knife-edge box \
                     pair's manifold appeared or vanished, and since A7b a box-box manifold \
                     exists exactly when the poses overlap, whatever the axis hint \
                     (narrowphase/box_box.rs, A7-N11), so this is not Known behaviour 2 (A4 \
                     Decision 1 case 5), which is retired for box-box pairs and no longer \
                     deferrable: look for a narrowphase input other than the poses, or a \
                     row-identity defect that fed the wrong pose",
                    detail,
                )
            } else {
                (
                    "count changed (a load-bearing pair)",
                    "a DEFECT: it blocks the commit. A load-bearing manifold appeared or \
                     vanished with the poses fixed, a narrowphase or row-identity defect (Known \
                     behaviour 2 never covered load-bearing pairs, and since A7b covers no box \
                     pair at all)",
                    detail,
                )
            }
        };
        panic!(
            "{what}: the {change} woke the frozen pile; kind of wake: {kind} at step s = {s} (s = 0 \
             is the {change} step): {verdict}. Pile manifold count c(s-1) = {}, c(s) = {}{detail}; \
             contact_wakes {} -> {}; awake pile ids from s: {later:?}; the pile's manifold count \
             from s = -1: {counts:?}; {context}",
            prev.pairs.len(),
            cur.pairs.len(),
            prev.wakes,
            cur.wakes,
        );
    }
}

// ── G1, G2, G7, G8: a box pile freezes ─────────────────────────────────────────

/// G1: a height-4 pile freezes (its contact-change wake events are recorded) and holds
/// bit-identically; then deleting its bottom corner box (row 1) wakes the whole pile on
/// that step.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool until a 30-box pile \
              freezes; intractable under Miri"
)]
fn a_small_box_pile_freezes_without_a_contact_wake_and_a_support_loss_wakes_all_of_it() {
    under_watchdog("G1", SMALL_TIMEOUT, || {
        let mut h = Harness::new();
        let pile = spawn_scene(&mut h, SMALL, false, None);
        assert_eq!(
            pile.len(),
            30,
            "construction: a height-4 pile holds 30 boxes"
        );
        assert_eq!(
            h.row_of(pile[0]),
            1,
            "construction: the first pile box is row 1"
        );

        freeze_and_hold(&mut h, &pile, SMALL_SETTLE_LIMIT, 120, "G1", None);

        let bottom = pile[0];
        let before = h.contact_wakes();
        assert!(
            h.world.delete_entity(h.entity(bottom)),
            "construction: the bottom box must be despawnable"
        );
        h.step();
        let rest: Vec<u32> = pile.iter().copied().filter(|&id| id != bottom).collect();
        let awake = h.awake_among(&rest);
        assert!(
            h.contact_wakes() > before && awake.len() == rest.len(),
            "phase 3 positive control: deleting the bottom box of a frozen pile must wake every \
             remaining box on that step; contact_wakes {before} before the delete, {} after, \
             awake {} of {} (awake ids {:?})",
            h.contact_wakes(),
            awake.len(),
            rest.len(),
            awake
        );
    });
}

/// G2: a height-5 pile (55 boxes, the largest that freezes with the fix) freezes within its
/// onset-flicker budget and holds.
#[test]
#[cfg_attr(
    any(miri, debug_assertions),
    ignore = "slow: a 55-box pile through the real schedule until frozen; release only, \
              intractable under Miri"
)]
fn a_height_5_box_pyramid_freezes_within_the_onset_flicker_budget() {
    under_watchdog("G2", MEDIUM_TIMEOUT, || {
        let mut h = Harness::new();
        let pile = spawn_scene(&mut h, MEDIUM, false, None);
        assert_eq!(
            pile.len(),
            55,
            "construction: a height-5 pile holds 55 boxes"
        );
        let budget = Budget {
            max: G2_MAX_EVENTS,
            spent: 0,
            frozen_draws: 0,
            draws: 1,
            measured_p: MEASURED_P_HEIGHT_5,
            label: "phase 1: flicker budget:",
        };
        freeze_and_hold(&mut h, &pile, MEDIUM_SETTLE_LIMIT, 120, "G2", Some(budget));
    });
}

/// G7: sixteen height-4 draws (each layer-0 box spawned last in turn, with the lone box)
/// freeze within one summed onset-flicker budget.
#[test]
#[cfg_attr(
    any(miri, debug_assertions),
    ignore = "slow: sixteen 30-box piles through the real schedule until frozen; release only, \
              intractable under Miri"
)]
fn sixteen_height_4_draws_freeze_within_the_onset_flicker_budget() {
    under_watchdog("G7", G7_TIMEOUT, || {
        let mut rows = Vec::with_capacity(16);
        let mut spent = 0usize;
        for mover in 1..=16u32 {
            let mut h = Harness::new();
            let pile = spawn_scene(&mut h, SMALL, true, Some(mover as usize - 1));
            assert_eq!(
                (
                    pile.len(),
                    *pile.last().expect("construction: the pile is not empty")
                ),
                (30, mover),
                "construction: a height-4 pile with mover {mover} spawned last"
            );
            let budget = Budget {
                max: G7_MAX_EVENTS,
                spent,
                frozen_draws: rows.len(),
                draws: 16,
                measured_p: MEASURED_P_HEIGHT_4,
                label: "flicker budget: the sixteen height-4 draws took",
            };
            let settled = h.settle_until_frozen(G7_SETTLE_LIMIT, "G7", Some(budget));
            spent += settled.contact_wake_rises.len();
            rows.push((
                mover,
                settled.freeze_step,
                settled.contact_wake_rises.len(),
                settled.contact_wakes,
            ));
        }
        let sum_freeze: usize = rows.iter().map(|r| r.1).sum();
        let sum_k: u64 = rows.iter().map(|r| r.3).sum();
        println!(
            "G7: (mover, freeze step, events, K) {rows:?}; Σ events {spent}, Σ freeze steps \
             {sum_freeze}, Σ K {sum_k}"
        );
        let budget = Budget {
            max: G7_MAX_EVENTS,
            spent: 0,
            frozen_draws: 0,
            draws: 16,
            measured_p: MEASURED_P_HEIGHT_4,
            label: "flicker budget: the sixteen height-4 draws took",
        };
        // `spent` is the sum of the per-draw counts each `settle_until_frozen` already
        // checked against this same cap, so this assertion cannot fire: it restates the bound
        // where the loop ends, and carries the triage paragraph for a reader who arrives here.
        assert!(
            spent <= G7_MAX_EVENTS,
            "G7, {} {spent} contact-change wake events (budget {G7_MAX_EVENTS}); (mover, freeze \
             step, events, K) {rows:?}. {}",
            budget.label,
            flicker_triage(spent, spent + rows.len(), &budget)
        );
    });
}

/// G8 (A7 / A4 Decision 3): a height-6 pile (91 boxes) freezes and holds.
///
/// A7a greened it, which is why the `deferred:` ignore is gone. The readings are msvc,
/// 2026-09-18, in release unless stated:
///
/// * Base `9f712204` (before A7a) is RED. It did not freeze within [`LONG_SETTLE_LIMIT`]
///   steps. `contact_wakes` rose at steps [2315, 2614, 2744, 4550] (364 in total). On the
///   last step all 91 rows were awake in one island, and the largest `|v|² + |ω|²` over
///   the last 60 steps was 0.012338251, against a sleep threshold of 1e-4.
/// * With A7a (clipped face-contact points carry injective feature ids) it is GREEN. It
///   froze at step 128 with `contact_wakes` 91 and one rise event, at step 68. The same
///   line prints in debug (1.52 s there, 0.16 s in release).
/// * With A7a + A7b (S5, the face-versus-edge rule) it froze at step 185 with
///   `contact_wakes` 182 and two rise events, at steps [65, 125]; 0.26 s in release, and
///   2.43 s and 2.44 s in two debug runs, printing the same line. The kernel as committed
///   (a held face that realizes no patch yields to the built best-face patch) prints the
///   same line in release.
/// * L9 C4 (contact reuse on by default, 2026-09-24): it froze at step 66 with `contact_wakes`
///   0 and no rise event, the same line in release and debug.
///
/// Every contact change re-draws that freeze step, so it is a reading, not a pin. The
/// test's bound is [`LONG_SETTLE_LIMIT`]. At 2.4 s in debug a `slow:` ignore would be a false
/// statement, so it runs in both profiles, like G1; only Miri skips it.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool until a 91-box \
              pile freezes; intractable under Miri"
)]
fn a_height_6_box_pyramid_freezes() {
    under_watchdog("G8", MEDIUM_TIMEOUT, || {
        let mut h = Harness::new();
        let pile = spawn_scene(&mut h, HEIGHT_6, false, None);
        assert_eq!(
            pile.len(),
            91,
            "construction: a height-6 pile holds 91 boxes"
        );
        freeze_and_hold(&mut h, &pile, LONG_SETTLE_LIMIT, 60, "G8", None);
    });
}

// ── A7: box piles come to rest ───────────────────────────────────────────────

/// A7-R0 (A7a): a face contact between two unit boxes overlapping by a quarter of a face
/// (the pile's layer-to-layer contact) carries a distinct feature id on every point, so no
/// two points of one manifold share a warm-start key.
///
/// Red until S1 of the A7 lane, which named a clipped vertex by the two features that
/// created it ([`feature_face_clip`](boyko_physics::narrowphase::feature_face_clip)) instead
/// of by `min(prev, cur)` of an endpoint's corner index. The `#[ignore]` comes off in the
/// same commit that turns it green: a reason string saying "red until the lane lands" on a
/// passing test is a false statement, and the census checks that a reason EXISTS, not that
/// it is true.
#[test]
fn a_quarter_overlap_face_contact_has_distinct_feature_ids() {
    let mut repeats = Vec::new();
    for (sx, sz) in [(1.0f32, 1.0f32), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        let contact = box_box_contact(
            BodyIndex(0),
            BodyIndex(1),
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::ONE,
            Vec3::new(sx, 2.0 - 1e-3, sz),
            Quat::IDENTITY,
            Vec3::ONE,
            None,
        );
        let contact = contact.unwrap_or_else(|| {
            panic!("construction: offset ({sx}, {sz}) overlaps by 1 mm, so it must be a contact")
        });
        let count = usize::from(contact.manifold.count);
        assert!(
            count >= 3,
            "construction: offset ({sx}, {sz}) is a quarter-face overlap, so its manifold must \
             hold at least 3 points; it holds {count}"
        );
        let ids: Vec<u32> = contact.manifold.points[..count]
            .iter()
            .map(|p| p.feature_id)
            .collect();
        let distinct = ids.iter().enumerate().all(|(i, id)| !ids[..i].contains(id));
        if !distinct {
            repeats.push(format!(
                "offset ({sx}, {sz}): {count} points carry feature ids {ids:?}: two points of one \
                 manifold share the warm-start key pack(a, b, feature_id), so \
                 WarmStartTable::insert overwrites one with the other. A clipped point's id must \
                 name both features that created it (narrowphase::feature_face_clip); before \
                 A7a they inherited min(prev, cur) corner ids and (1, 1) read [24, 24, 24, 24]"
            ));
        }
    }
    assert!(repeats.is_empty(), "{}", repeats.join("; "));
}

/// A7-R1: Jolt's height-15 pyramid, sleeping off, does not creep sideways once the vertical
/// settle is over. The window opens with a standing guard ([`STANDING_DROP_M`]), so a pile
/// that collapsed before it cannot meet the creep bound by having nothing left to move.
///
/// Its readings, msvc release, D_max over steps 600-3000:
///
/// * Base `9f712204`: 0.6361069 m — red.
/// * A7a (`08fe7b9f`): 0.0972966 m, box 1041 in layer 7 — red.
/// * A7a + A7b (S5): 0.0008583 m, box 1240 in layer 14, on the form first implemented;
///   0.0007180 m, box 1227 in layer 12, on the kernel as committed (a held face that
///   realizes no patch yields to the built best-face patch). Green, in the pre-registered
///   "confirms" band of the module header. The residue is a drift rate, not an offset: over
///   steps 3000-5400 it adds another 1.0965 mm (module header).
/// * L9 C4 (contact reuse on by default, 2026-09-24): the test runs the pile twice. The default
///   arm (reuse on) reads 0.0006670445 m, box 1240 in layer 14, in the design's pre-registered
///   "confirms" band (under 2 mm; 2-10 mm acceptable and reported, 10 mm or more a stop). The
///   reuse-off arm, the exact narrowphase, reads 0.0007180063 m, box 1227 in layer 12, as the
///   design requires. At step 600 the default arm holds 6932 manifolds and 22 295 contact
///   points, the reuse-off arm 6671 and 22 975 (the S5 rung's reading).
///
/// [`CREEP_BOUND_M`] is not tightened toward the reading: it is an acceptance criterion at
/// Box2D's slop scale, not a regression pin. The regression pins are separate: D_max must also
/// equal [`A7_R1_D_MAX_BITS`] (reuse on) and [`A7_R1_D_MAX_BITS_REUSE_OFF`] (reuse off)
/// exactly, each re-pinned only under its constant's rule.
///
/// A revert of A7b's selection alone (the edge taken
/// whenever its SAT depth is below the face's by more than `SAT_EPS`) is caught at the kernel
/// by A7-N9 (`narrowphase/box_box.rs`) and in the scene by THIS test: it read D_max =
/// 0.10017 m under that mutation (msvc release, 2026-09-18), while A7-R2 still froze and
/// stayed green. The mutation leaves no best-face patch built, so the held-face yield is
/// unreachable under it and the reading holds for the kernel as committed.
///
/// **The fallback census** (the `thinbox` lane, `design_rev2.md` §6.2 (ii)): under the
/// non-default `narrowphase-counts` feature the run also reads the box-box fallback's census over
/// the settle (steps 0-600) and over the window (600-3000) of each arm, and asserts the face bound
/// never fired in either — no phantom answer, no capped hint, no corner — with the fallback
/// exercised in the reuse-off arm's window. The fix changes a run's output only when one of those
/// fires, so this is what says a moved D_max was not the fix. Run it alone (`--exact`): the
/// counters are process-wide.
#[test]
#[cfg_attr(
    any(miri, debug_assertions),
    ignore = "slow: a 1240-box pile for 3000 steps through the real schedule, twice (contact \
              reuse on and off); release only, intractable under Miri"
)]
fn a_resting_jolt_pyramid_does_not_creep_with_sleeping_off() {
    // The shipped default first (contact reuse on since L9 C4), then the exact narrowphase on the
    // same binary: each arm must hold the creep bound, and each reads its own pin.
    let default_arm = under_watchdog("A7-R1, contact reuse on", JOLT_TIMEOUT, || {
        a7_r1_arm("contact reuse on (the default)", None)
    });
    let reuse_off_arm = under_watchdog("A7-R1, contact reuse off", JOLT_TIMEOUT, || {
        a7_r1_arm("contact reuse off", Some(false))
    });
    for (arm, reading, pinned, pin) in [
        ("contact reuse on (the default)", default_arm, A7_R1_D_MAX_BITS, "A7_R1_D_MAX_BITS"),
        (
            "contact reuse off",
            reuse_off_arm,
            A7_R1_D_MAX_BITS_REUSE_OFF,
            "A7_R1_D_MAX_BITS_REUSE_OFF",
        ),
    ] {
        assert_eq!(
            reading.d_max.to_bits(),
            pinned,
            "A7-R1, {arm}: D_max moved: {} m ({:#010x}, box {}), pinned {} m ({pinned:#010x}). It \
             is still within the {CREEP_BOUND_M} m bound, so this is a value change, not a creep. \
             In a commit that claims bit identity it is a defect; only a value-changing lever \
             re-pins, under `{pin}`'s rule",
            reading.d_max,
            reading.d_max.to_bits(),
            reading.worst_id,
            f32::from_bits(pinned)
        );
    }
}

/// What one arm of A7-R1 read: D_max and the box that moved it.
#[derive(Clone, Copy, Debug)]
struct CreepReading {
    /// The largest horizontal displacement of any pile box between [`CREEP_FROM`] and
    /// [`CREEP_TO`].
    d_max: f32,
    /// The pile id of the box that moved `d_max`.
    worst_id: u32,
}

/// One arm of A7-R1: Jolt's height-15 pile with sleeping off, and `contact_reuse` set to the
/// value given (the default's when `None`), stepped to [`CREEP_TO`]. Asserts the standing guard
/// and [`CREEP_BOUND_M`], prints D_max and the step-[`CREEP_FROM`] contact counts, and returns
/// the reading its caller compares with the arm's pin.
fn a7_r1_arm(arm: &'static str, contact_reuse: Option<bool>) -> CreepReading {
    let mut h = Harness::new();
    {
        let cfg = h.world.resource_mut::<PhysicsConfig>();
        cfg.sleeping = false;
        if let Some(reuse) = contact_reuse {
            cfg.contact_reuse = reuse;
        }
    }
    let reuse = h.world.resource::<PhysicsConfig>().contact_reuse;
    let pile = spawn_scene(&mut h, JOLT, false, None);
    assert_eq!(
        pile.len(),
        1240,
        "construction: Jolt's pyramid holds 1240 boxes"
    );
    #[cfg(feature = "narrowphase-counts")]
    let _ = fallback_census::take();
    for _ in 0..CREEP_FROM {
        h.step();
    }
    #[cfg(feature = "narrowphase-counts")]
    let settle_census = fallback_census::take();
    let from = h.centres(&pile);
    let points: usize = h
        .world
        .resource::<Manifolds>()
        .manifolds()
        .iter()
        .map(|m| usize::from(m.count))
        .sum();
    println!(
        "A7-R1, {arm} (contact_reuse {reuse}): step {CREEP_FROM}: {} manifolds, {points} contact \
         points",
        h.manifold_count()
    );

    // The window's premise: `CREEP_FROM`'s doc says the vertical settle is over by now,
    // and a collapsed-and-resting pile satisfies the creep bound below for every box.
    let spawn = pyramid_positions(JOLT);
    let mut worst_drop = (0.0f32, 0u32);
    for &(id, centre) in &from {
        let drop = spawn[id as usize - 1].y - centre.y;
        if drop > worst_drop.0 {
            worst_drop = (drop, id);
        }
    }
    assert!(
        worst_drop.0 <= STANDING_DROP_M,
        "A7-R1 premise, {arm}: at step {CREEP_FROM} the pyramid must still be a pyramid, but box \
         id {} in layer {} has dropped {} m below its spawn height (bound {STANDING_DROP_M} m). \
         A pile that collapsed and came to rest before the window meets the creep bound \
         trivially, so the creep reading below would be vacuous",
        worst_drop.1,
        layer_of(JOLT, worst_drop.1),
        worst_drop.0
    );

    for _ in CREEP_FROM..CREEP_TO {
        h.step();
    }
    #[cfg(feature = "narrowphase-counts")]
    a7_r1_fallback_census(arm, settle_census, fallback_census::take(), !reuse);
    let to = h.centres(&pile);
    let mut layer_max = vec![0.0f32; JOLT];
    let mut top_sum = (0.0f64, 0.0f64, 0usize);
    let mut worst = (0.0f32, 0u32);
    for (&(id, a), &(id_b, b)) in from.iter().zip(&to) {
        assert_eq!(id, id_b, "harness: both records follow the pile order");
        let (dx, dz) = (b.x - a.x, b.z - a.z);
        let d = (dx * dx + dz * dz).sqrt();
        let layer = layer_of(JOLT, id);
        layer_max[layer] = layer_max[layer].max(d);
        if layer >= 10 {
            top_sum.0 += f64::from(dx);
            top_sum.1 += f64::from(dz);
            top_sum.2 += 1;
        }
        if d > worst.0 {
            worst = (d, id);
        }
    }
    let n = top_sum.2.max(1) as f64;
    println!(
        "A7-R1, {arm}: D_max = {} m (box {}, layer {}); mean (dx, dz) of layers 10-14 = ({:.6}, \
         {:.6}) m",
        worst.0,
        worst.1,
        layer_of(JOLT, worst.1),
        top_sum.0 / n,
        top_sum.1 / n
    );
    assert!(
        worst.0 <= CREEP_BOUND_M,
        "A7-R1, {arm}: a resting height-15 box pyramid with sleeping off moved sideways by \
         D_max = {} m between steps {CREEP_FROM} and {CREEP_TO} (bound {CREEP_BOUND_M} m): \
         worst box id {} in layer {} (layer i holds (15 - i)² boxes); mean (dx, dz) of layers \
         10-14 = ({:.4}, {:.4}) m; per-layer maximum, layer 0 first: {layer_max:?}",
        worst.0,
        worst.1,
        layer_of(JOLT, worst.1),
        top_sum.0 / n,
        top_sum.1 / n
    );
    CreepReading { d_max: worst.0, worst_id: worst.1 }
}

/// A7-R1's fallback census over one arm's settle and window (`narrowphase-counts` only): printed,
/// with the margin of the face bound's 5 mm over the largest accepted edge excess (τ is
/// `min(5 mm, 0.1·h_min)` and the pile's boxes are 1 m half-extents), and asserted to hold no event
/// that changes the kernel's output on either arm. Pre-registered for the window
/// (`design_rev2.md` §6.2 (ii)), on the trajectory that is now the contact-reuse-off arm: `calls`
/// 12 572, `phantom` 0, `hint_capped` 0, `max_accepted_excess` ≤ 2e-4. That arm must exercise the
/// fallback in the window (`require_calls`); the reuse-on arm need not, since its slow pairs skip
/// the full collision that runs it. `refresh_stale` is printed, not asserted: on a reuse-on arm the
/// thin-box R1's refusal of a stale refresh changes the trajectory by design.
#[cfg(feature = "narrowphase-counts")]
fn a7_r1_fallback_census(
    arm: &str,
    settle: fallback_census::Snapshot,
    window: fallback_census::Snapshot,
    require_calls: bool,
) {
    const TAU: f32 = 5.0e-3;
    for (phase, s) in [("settle 0-600", settle), ("window 600-3000", window)] {
        println!(
            "A7-R1 fallback census, {arm}, {phase}: {s:?}; margin τ / max_accepted_excess = {}",
            TAU / s.max_accepted_excess
        );
        assert!(
            s.phantom == 0 && s.hint_capped == 0 && s.corner == 0,
            "A7-R1, {arm}: the box-box fallback's face bound fired during the {phase}: {s:?}. \
             That changes the kernel's output on this pile; the pinned D_max no longer measures \
             the unchanged kernel"
        );
    }
    assert!(
        !require_calls || window.calls > 0,
        "A7-R1, {arm}: the fallback never ran in the window: {window:?}"
    );
}

/// A7-R2: Jolt's height-15 pyramid (1240 boxes) freezes and holds; its contact-change wake
/// events are recorded, not budgeted.
///
/// Its readings, msvc release:
///
/// * A7a (`08fe7b9f`): red. No freeze within [`LONG_SETTLE_LIMIT`] steps; the largest
///   `|v|² + |ω|²` was 0.0069859.
/// * A7a + A7b (S5): green. On the form first implemented it froze at step 188 with
///   `contact_wakes` 2480 and two rise events, at steps [68, 128]; 4.95 s. On the kernel as
///   committed (a held face that realizes no patch yields to the built best-face patch) it
///   froze at step 248 with `contact_wakes` 3720 and three rise events, at [68, 128, 188].
/// * L9 C4 (contact reuse on by default, 2026-09-24): it froze at step 250 with
///   `contact_wakes` 3720 and three rise events, at [70, 130, 190].
///
/// The freeze step is a reading, not a pin: every contact change re-draws it — the two kernels
/// differ only in that yield, so it fired in this scene and moved the freeze by one latch
/// attempt — which is why [`LONG_SETTLE_LIMIT`] stays at 6000 rather than being sized from
/// it. It does not catch
/// a revert of A7b's selection alone: under that mutation it still froze (A7-R1 is the scene
/// gate that reds).
#[test]
#[cfg_attr(
    any(miri, debug_assertions),
    ignore = "slow: a 1240-box pile through the real schedule until frozen; release only, \
              intractable under Miri"
)]
fn a_jolt_scale_box_pyramid_freezes() {
    under_watchdog("A7-R2", JOLT_TIMEOUT, || {
        let mut h = Harness::new();
        let pile = spawn_scene(&mut h, JOLT, false, None);
        assert_eq!(
            pile.len(),
            1240,
            "construction: Jolt's pyramid holds 1240 boxes"
        );
        freeze_and_hold(&mut h, &pile, LONG_SETTLE_LIMIT, 60, "A7-R2", None);
    });
}

/// What one scene run produced, for the SIMD on/off differential.
#[derive(Debug, PartialEq, Eq)]
struct SimdAbTrace {
    /// FNV-1a over every body's [`body_bits`] in walk order, after every step.
    hashes: Vec<u64>,
    /// Steps on which `contact_wakes` rose.
    rises: Vec<usize>,
    /// `contact_wakes` after the last step.
    contact_wakes: u64,
    /// The step on which no dynamic row was awake, if one was reached.
    freeze_step: Option<usize>,
}

/// FNV-1a over every body's [`body_bits`], in walk order.
fn state_hash(h: &mut Harness) -> u64 {
    let q = h.world.query::<&RigidBody, ()>();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for body in q.iter() {
        for word in body_bits(body) {
            for byte in word.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    hash
}

/// Runs a sleeping-on pile of `height` (with the lone box and `last` as in G7 when
/// `lone`) with `simd_solve`, until it freezes or `limit` steps pass, then `hold` more
/// steps, hashing the state after every step.
fn sleeping_trace(
    simd_solve: bool,
    height: usize,
    lone: bool,
    last: Option<usize>,
    limit: usize,
    hold: usize,
) -> SimdAbTrace {
    let mut h = Harness::new();
    h.world.resource_mut::<PhysicsConfig>().simd_solve = simd_solve;
    spawn_scene(&mut h, height, lone, last);
    let mut trace = SimdAbTrace {
        hashes: Vec::with_capacity(limit + hold),
        rises: Vec::new(),
        contact_wakes: 0,
        freeze_step: None,
    };
    let mut last_wakes = h.contact_wakes();
    for step in 1..=limit {
        h.step();
        trace.hashes.push(state_hash(&mut h));
        let now = h.contact_wakes();
        if now != last_wakes {
            trace.rises.push(step);
            last_wakes = now;
        }
        if h.dynamic_rows_awake().is_empty() {
            trace.freeze_step = Some(step);
            break;
        }
    }
    for _ in 0..hold {
        h.step();
        trace.hashes.push(state_hash(&mut h));
    }
    trace.contact_wakes = h.contact_wakes();
    trace
}

/// A7-R1's scene with `simd_solve`: returns the per-step state hashes and the bits of
/// D_max, the largest horizontal displacement of any pile box between steps
/// [`CREEP_FROM`] and [`CREEP_TO`].
fn creep_trace(simd_solve: bool) -> (Vec<u64>, u32) {
    let mut h = Harness::new();
    {
        let cfg = h.world.resource_mut::<PhysicsConfig>();
        cfg.sleeping = false;
        cfg.simd_solve = simd_solve;
    }
    let pile = spawn_scene(&mut h, JOLT, false, None);
    let mut hashes = Vec::with_capacity(CREEP_TO);
    for _ in 0..CREEP_FROM {
        h.step();
        hashes.push(state_hash(&mut h));
    }
    let from = h.centres(&pile);
    for _ in CREEP_FROM..CREEP_TO {
        h.step();
        hashes.push(state_hash(&mut h));
    }
    let to = h.centres(&pile);
    let d_max = from
        .iter()
        .zip(&to)
        .map(|(&(_, a), &(_, b))| {
            let (dx, dz) = (b.x - a.x, b.z - a.z);
            (dx * dx + dz * dz).sqrt()
        })
        .fold(0.0f32, f32::max);
    (hashes, d_max.to_bits())
}

/// The first step on which two per-step hash sequences differ, for a failure message.
fn first_divergence(on: &[u64], off: &[u64]) -> String {
    match on.iter().zip(off).position(|(a, b)| a != b) {
        Some(i) => format!("the state first differs after step {}", i + 1),
        None => format!("the runs have {} and {} steps", on.len(), off.len()),
    }
}

/// G2 of the default-world lane: the schedule-level SIMD on/off differential over the
/// scenes that carry this file's measured budgets and bounds (G2's height-5 pile, G7's
/// sixteen height-4 draws, A7-R1's height-15 pile with sleeping off).
///
/// `simd_solve` became the default on 2026-09-18 after G2's and G7's budgets and A7-R1's
/// bound were measured on the scalar colored solve. The O7 kernel is bit-identical to that
/// oracle, so every reading must be too: a difference here is a kernel defect, never a
/// budget or a bound to re-measure. Non-vacuous only on an AVX2 build with the flag on by
/// default, which the test asserts first (G1 in `default_world_colored_simd.rs` pins the
/// same two inputs of the dispatch fork).
#[test]
#[cfg_attr(
    any(miri, debug_assertions),
    ignore = "slow: G2's, G7's and A7-R1's scenes twice each (1240 boxes for 3000 steps, twice) \
              through the real schedule; release only, intractable under Miri"
)]
// `assertions_on_constants`: the `cfg!` is constant per build, and asserting it is the
// point — it is the build-configuration witness that makes the on/off comparison
// non-vacuous, and it reds exactly the build (no AVX2) where both arms are the oracle.
#[allow(clippy::assertions_on_constants)]
fn simd_solve_on_off_bit_identical() {
    assert!(
        cfg!(all(target_arch = "x86_64", target_feature = "avx2")),
        "non-vacuity: without x86_64 + avx2 both arms are the scalar oracle"
    );
    assert!(
        PhysicsConfig::default().simd_solve,
        "non-vacuity: the default run must be the SIMD one"
    );

    under_watchdog("SIMD on/off: G2's scene", MEDIUM_TIMEOUT, || {
        let on = sleeping_trace(true, MEDIUM, false, None, MEDIUM_SETTLE_LIMIT, 120);
        let off = sleeping_trace(false, MEDIUM, false, None, MEDIUM_SETTLE_LIMIT, 120);
        assert!(on.freeze_step.is_some(), "construction: G2's pile freezes: {:?}", on.rises);
        assert!(
            on == off,
            "G2's scene: simd_solve on/off diverged ({}); on: freeze {:?}, rises {:?}, K {}; \
             off: freeze {:?}, rises {:?}, K {}",
            first_divergence(&on.hashes, &off.hashes),
            on.freeze_step,
            on.rises,
            on.contact_wakes,
            off.freeze_step,
            off.rises,
            off.contact_wakes
        );
    });

    under_watchdog("SIMD on/off: G7's draws", G7_TIMEOUT, || {
        for mover in 1..=16usize {
            let on = sleeping_trace(true, SMALL, true, Some(mover - 1), G7_SETTLE_LIMIT, 0);
            let off = sleeping_trace(false, SMALL, true, Some(mover - 1), G7_SETTLE_LIMIT, 0);
            assert!(
                on == off,
                "G7 draw {mover}: simd_solve on/off diverged ({}); on: freeze {:?}, rises {:?}, \
                 K {}; off: freeze {:?}, rises {:?}, K {}",
                first_divergence(&on.hashes, &off.hashes),
                on.freeze_step,
                on.rises,
                on.contact_wakes,
                off.freeze_step,
                off.rises,
                off.contact_wakes
            );
        }
    });

    under_watchdog("SIMD on/off: A7-R1's scene", 2 * JOLT_TIMEOUT, || {
        let (on, d_on) = creep_trace(true);
        let (off, d_off) = creep_trace(false);
        assert!(
            on == off && d_on == d_off,
            "A7-R1's scene: simd_solve on/off diverged ({}); D_max on {} m ({d_on:#010x}), off \
             {} m ({d_off:#010x})",
            first_divergence(&on, &off),
            f32::from_bits(d_on),
            f32::from_bits(d_off)
        );
        println!("SIMD on/off: A7-R1's scene D_max = {} m in both runs", f32::from_bits(d_on));
    });
}

/// The re-draw distribution G2's and G7's budgets are sized from: every height-4 draw of
/// G7's scene (each pile box spawned last, with the lone box) and every height-5 draw of
/// G2's (natural order, then each pile box spawned last), each settled for up to
/// [`GENERATOR_SETTLE_LIMIT`] steps. Prints `(last, freeze step, events, K)` per draw and
/// the pooled per-attempt wake fraction per height. Asserts nothing.
#[test]
#[ignore = "generator: prints the onset-flicker re-draw distribution that sizes G2's and G7's \
            budgets; asserts nothing; release: cargo test --release -p boyko-physics --test \
            sleep_settles_box_piles -- --ignored --exact flicker_redraw_distribution"]
fn flicker_redraw_distribution() {
    under_watchdog("generator", GENERATOR_TIMEOUT, || {
        for (height, lone, lasts) in [
            (
                SMALL,
                true,
                (0..30).map(Some).collect::<Vec<Option<usize>>>(),
            ),
            (
                MEDIUM,
                false,
                std::iter::once(None)
                    .chain((0..55).map(Some))
                    .collect::<Vec<Option<usize>>>(),
            ),
        ] {
            let mut draws = Vec::with_capacity(lasts.len());
            for last in lasts {
                // A draw that does not freeze panics on its own thread; its message is
                // printed and the draw is recorded as unfrozen.
                let outcome = thread::spawn(move || {
                    let mut h = Harness::new();
                    spawn_scene(&mut h, height, lone, last);
                    let settled = h.settle_until_frozen(GENERATOR_SETTLE_LIMIT, "generator", None);
                    (
                        settled.freeze_step,
                        settled.contact_wake_rises,
                        settled.contact_wakes,
                    )
                })
                .join()
                .ok();
                match outcome {
                    Some((freeze, rises, k)) => {
                        println!(
                            "generator h={height} last={last:?}: froze at {freeze}, {} events at \
                             {rises:?}, K {k}",
                            rises.len()
                        );
                        draws.push((last, Some(freeze), rises.len(), k));
                    }
                    None => {
                        println!(
                            "generator h={height} last={last:?}: DID NOT FREEZE within \
                             {GENERATOR_SETTLE_LIMIT} steps (message above)"
                        );
                        draws.push((last, None, 0, 0));
                    }
                }
            }
            let frozen: Vec<_> = draws.iter().filter(|d| d.1.is_some()).collect();
            let events: usize = frozen.iter().map(|d| d.2).sum();
            let mut per_draw: Vec<usize> = frozen.iter().map(|d| d.2).collect();
            per_draw.sort_unstable();
            let mut freezes: Vec<usize> = frozen.iter().filter_map(|d| d.1).collect();
            freezes.sort_unstable();
            println!(
                "generator h={height}: {} draws, {} froze; Σ events over frozen draws {events}, \
                 attempts {}, pooled p = {:.4}; events per draw sorted {per_draw:?}; freeze steps \
                 sorted {freezes:?}",
                draws.len(),
                frozen.len(),
                events + frozen.len(),
                events as f64 / (events + frozen.len()).max(1) as f64
            );
        }
    });
}

// ── G3, G4: a swap-remove moves one pile row across the pile ──────────────────────

/// The shared body of G3 and G4: floor, lone box, pile with `mover` spawned last; settle;
/// delete the lone box so `mover` swap-moves from the last row into row 1 while the island
/// ids renumber; then hold. Returns the mover's lateral reference boxes before and after.
/// When `mover_index` names a box (G4), the mover must have a lateral manifold before the
/// delete. `contact_reuse` overrides the default's when it is `Some`.
fn swap_remove_scene(
    what: &'static str,
    mover_index: Option<usize>,
    contact_reuse: Option<bool>,
) -> (Lateral, Lateral) {
    let mut h = Harness::new();
    if let Some(reuse) = contact_reuse {
        h.world.resource_mut::<PhysicsConfig>().contact_reuse = reuse;
    }
    let pile = spawn_scene(&mut h, SMALL, true, mover_index);
    let mover = *pile.last().expect("construction: the pile is not empty");
    let walk = h.walk_ids();
    assert!(
        walk.len() == 32
            && walk[1] == LONE_ID
            && walk[31] == mover
            && walk.len() <= NO_CLEAR_MAX_ROWS,
        "{what} construction: floor, lone box, then 30 pile rows with the mover last; walk {walk:?}"
    );

    let settled = h.settle_until_frozen(SMALL_SETTLE_LIMIT, what, None);
    println!(
        "{what}: froze at step {} (contact_wakes {}, rises at {:?})",
        settled.freeze_step, settled.contact_wakes, settled.contact_wake_rises
    );
    h.assert_walk_matches_gather();
    let mover_row = h.row_of(mover);
    assert!(
        h.island_of_row(mover_row) == 1 && h.n_islands() == 2,
        "{what} premise: the lone box is island 0 and the pile island 1 before the delete; the \
         mover's island {}, islands {}",
        h.island_of_row(mover_row),
        h.n_islands()
    );
    let before = h.lateral_reference_boxes(mover, &pile);
    if mover_index.is_some() && before.lateral == 0 {
        let manifolds = h.manifolds_of(mover);
        let touching: Vec<(u32, u32, [f32; 3])> = touching_no_contact_pairs(&mut h, &pile)
            .into_iter()
            .filter(|&(a, b, _)| a == mover || b == mover)
            .collect();
        panic!(
            "{what} premise: the mover must have at least one lateral manifold with a pile box \
             before the delete; lateral {}, face-face {:?}. The mover's manifolds (partner id, \
             normal, point count): {manifolds:?}; its touching candidate partners with no \
             manifold (ids, axis-aligned gaps): {touching:?}",
            before.lateral, before.face_face
        );
    }
    let lone = h.manifolds_of(LONE_ID);
    assert!(
        lone.len() == 1 && lone[0].0 == FLOOR_ID,
        "{what} construction: the lone box touches only the floor; its manifolds {lone:?}"
    );
    let resets = h.remap_resets();
    let poses = h.poses(&pile);
    let mut trace = Trace::start(&mut h, &pile);
    assert_eq!(
        trace.steps[0].pairs.len(),
        h.manifold_count() - lone.len(),
        "{what} construction: the pile's manifold count is every manifold but the lone box's"
    );

    assert!(
        h.world.delete_entity(h.entity(LONE_ID)),
        "construction: the lone box must be despawnable"
    );
    let walk = h.walk_ids();
    assert!(
        walk.len() == 31 && walk[1] == mover,
        "{what} construction: the delete swap-moves the mover into row 1; walk {walk:?}"
    );
    h.step();
    // Read on the move step, asserted after the trace is classed: a wake that drops a
    // manifold also splits the island, and the kind of wake is the more useful report.
    let islands_after = (h.island_of_row(1), h.n_islands());
    let after = h.lateral_reference_boxes(mover, &pile);
    trace.record(&mut h, &pile);
    for _ in 0..59 {
        h.step();
        trace.record(&mut h, &pile);
    }

    trace.assert_undisturbed(
        what,
        "swap-remove",
        &format!(
            "remap resets {resets:?} -> {:?}; (row 1's island, islands) on the move step \
             {islands_after:?}; the mover's lateral contacts before {before:?}, after {after:?}",
            h.remap_resets()
        ),
    );
    assert!(
        islands_after == (0, 1),
        "{what} construction: after the delete the pile is island 0 and the only island; (row 1's \
         island, islands) {islands_after:?}"
    );
    assert_eq!(
        h.remap_resets(),
        resets,
        "{what}: both remap-reset counters must stay flat (no missed gather)"
    );
    assert!(
        h.poses(&pile) == poses,
        "{what}: every frozen box must keep its exact state across the row move"
    );
    (before, after)
}

/// G3: the top box moves from the last row into row 1 (its vertical pairs reverse order) and
/// the island ids renumber: nothing may wake.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool until a 30-box pile \
              freezes; intractable under Miri"
)]
fn a_frozen_box_pile_ignores_a_despawn_that_renumbers_its_island_and_moves_its_row() {
    under_watchdog("G3", SMALL_TIMEOUT, || {
        assert_eq!(
            top_id(SMALL),
            30,
            "construction: the Jolt loop spawns the top box last"
        );
        swap_remove_scene("G3", None, None);
    });
}

/// G4: pile id 13, layer 0's `j = 3, k = 0` corner box (x = 2, y = 1, z = -4), moves from
/// the last row into row 1, so its lateral knife-edge pairs reverse order and swap their
/// reference box: nothing may wake.
///
/// Contact reuse OFF, although it is on by default since L9 C4: the swap is the full
/// collision's (the SAT takes the reference face by pair order), and with reuse on every
/// flipped pair of the frozen pile is served from its record, which keeps its reference box
/// across a row-order flip (L9 ruling W2), so no mover can meet the premise
/// ([`G4_MOVER_ID`]'s L9 C4 probe). The same scene with reuse on is G3's.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool until a 30-box pile \
              freezes; intractable under Miri"
)]
fn a_frozen_box_pile_whose_bottom_corner_reverses_its_pair_order_takes_no_contact_wake() {
    under_watchdog("G4", SMALL_TIMEOUT, || {
        let index = G4_MOVER_ID as usize - 1;
        assert_eq!(
            pyramid_positions(SMALL)[index],
            Vec3::new(2.0, 1.0, -4.0),
            "construction: pile id 13 is layer 0's j = 3, k = 0 corner"
        );
        // Pile ids follow the Jolt index, so the mover keeps its id while spawned last.
        let (before, after) = swap_remove_scene("G4", Some(index), Some(false));
        let swapped: Vec<(u32, u32, u32)> = before
            .face_face
            .iter()
            .filter_map(|&(partner, reference)| {
                after
                    .face_face
                    .iter()
                    .find(|&&(p, _)| p == partner)
                    .filter(|&&(_, r)| r != reference)
                    .map(|&(_, r)| (partner, reference, r))
            })
            .collect();
        assert!(
            !swapped.is_empty(),
            "scene-fitness: the flip swapped no lateral contact's reference box, so G4 exercised \
             no swap — escalate. Face-face lateral contacts before {:?}, after {:?}",
            before.face_face,
            after.face_face
        );
        println!("G4: reference boxes swapped (partner, before, after): {swapped:?}");
    });
}

// ── G5, G6: a spawn shifts every pile row by one ─────────────────────────────

/// Candidate box pairs of the pile that produced no manifold on the last step although their
/// settled centres touch (every axis-aligned gap at most [`TOUCH_GAP`]): `(id, id, gaps)`.
fn touching_no_contact_pairs(h: &mut Harness, pile: &[u32]) -> Vec<(u32, u32, [f32; 3])> {
    let walk = h.walk_ids();
    let centres: Vec<Vec3> = h
        .world
        .resource::<SolverScratch>()
        .bodies()
        .iter()
        .map(|b| b.position)
        .collect();
    let touching: Vec<(u32, u32)> = h
        .world
        .resource::<Manifolds>()
        .manifolds()
        .iter()
        .filter(|m| m.count > 0)
        .map(|m| (m.body_a.0.min(m.body_b.0), m.body_a.0.max(m.body_b.0)))
        .collect();
    let mut out = Vec::new();
    for &(a, b) in h.world.resource::<ContactPairs>().pairs() {
        let (ra, rb) = (a.0.min(b.0), a.0.max(b.0));
        let (Some(&ia), Some(&ib)) = (walk.get(ra as usize), walk.get(rb as usize)) else {
            continue;
        };
        if !pile.contains(&ia) || !pile.contains(&ib) || touching.contains(&(ra, rb)) {
            continue;
        }
        let d = centres[rb as usize] - centres[ra as usize];
        let gaps = [
            d.x.abs() - BOX_SIZE,
            d.y.abs() - BOX_SIZE,
            d.z.abs() - BOX_SIZE,
        ];
        if gaps.iter().all(|&g| g <= TOUCH_GAP) {
            out.push((ia, ib, gaps));
        }
    }
    out
}

/// Which no-clear argument a shift scene rests on (module header).
#[derive(Clone, Copy, Debug)]
enum ClearBound {
    /// At most [`NO_CLEAR_MAX_ROWS`] rows.
    Rows,
    /// The distinct candidate pairs under the two row sets sum to at most
    /// [`AXIS_CLEAR_LIMIT`], and no step has more than that.
    Pairs,
}

/// The shared body of G5 and G6: floor and pile in `plain`; settle; hold 5; spawn a far
/// sphere into `marked`, which shifts every `plain` row by one with the order kept; hold 60.
fn shift_scene(what: &'static str, height: usize, settle_limit: usize, bound: ClearBound) {
    let mut h = Harness::new();
    let pile = spawn_scene(&mut h, height, false, None);
    // The rows stay fixed until the far sphere spawns.
    h.track_pairs();
    if let ClearBound::Rows = bound {
        let rows = h.walk_ids().len();
        assert!(
            rows <= NO_CLEAR_MAX_ROWS,
            "{what} construction: {rows} rows exceed the no-clear bound {NO_CLEAR_MAX_ROWS}"
        );
    }
    let settled = h.settle_until_frozen(settle_limit, what, None);
    println!(
        "{what}: froze at step {} (contact_wakes {}, {} rise events at {:?}; {} distinct candidate \
         pairs, at most {} per step)",
        settled.freeze_step,
        settled.contact_wakes,
        settled.contact_wake_rises.len(),
        settled.contact_wake_rises,
        h.pairs.distinct,
        h.pairs.max_per_step
    );
    let pre = h.hold(5, &pile, h.contact_wakes());
    assert!(
        pre.awake.is_empty(),
        "{what} construction: the pile must stay frozen for the 5-step pre-hold: {:?}",
        pre.awake
    );
    h.assert_walk_matches_gather();

    // P1: one island, id 0.
    let walk = h.walk_ids();
    let off_island: Vec<u32> = walk
        .iter()
        .enumerate()
        .filter(|(row, id)| pile.contains(id) && h.island_of_row(*row) != 0)
        .map(|(_, &id)| id)
        .collect();
    assert!(
        h.n_islands() == 1 && off_island.is_empty(),
        "{what} premise P1: the pile is the only island, id 0; islands {}, pile ids off island 0: \
         {off_island:?}",
        h.n_islands()
    );
    // P2: at least one touching box pair with no manifold. These knife-edge pairs are the
    // ones a narrowphase input other than the poses could flip. Before A7b the axis hint was
    // such an input (A4's Known behaviour 2); since A7b a box-box manifold exists exactly
    // when the poses overlap, except on a zero-extent reference face (A7-N11 pins it), so a
    // row shift must leave every one of them without a manifold — the property this scene
    // holds the pile to.
    let n_set = touching_no_contact_pairs(&mut h, &pile);
    assert!(
        !n_set.is_empty(),
        "scene-fitness: the settled pile has no touching no-contact box pair, so {what} holds no \
         pair whose existence a non-pose input could flip — escalate"
    );
    println!(
        "{what}: {} touching no-contact pairs (first ids and gaps: {:?})",
        n_set.len(),
        &n_set[..n_set.len().min(8)]
    );

    let resets = h.remap_resets();
    let poses = h.poses(&pile);
    let mut trace = Trace::start(&mut h, &pile);

    h.spawn_far_sphere();
    let before_shift = h.track_pairs();
    let shifted = h.walk_ids();
    let mut expected = vec![FAR_ID];
    expected.extend(&walk);
    assert!(
        shifted == expected,
        "{what} construction: the far sphere is walked first and every other row shifts by one \
         with its order kept; walk before {walk:?}, after {shifted:?}"
    );
    if let ClearBound::Rows = bound {
        assert!(
            shifted.len() <= NO_CLEAR_MAX_ROWS,
            "{what} construction: {} rows exceed the no-clear bound",
            shifted.len()
        );
    }

    h.step();
    // Read on the spawn step, asserted after the trace is classed (see `swap_remove_scene`).
    let renumbered: Vec<u32> = shifted
        .iter()
        .enumerate()
        .filter(|(row, id)| pile.contains(id) && h.island_of_row(*row) != 1)
        .map(|(_, &id)| id)
        .collect();
    let islands_after = (h.n_islands(), h.island_of_row(0));
    trace.record(&mut h, &pile);
    for _ in 0..59 {
        h.step();
        trace.record(&mut h, &pile);
    }

    if let ClearBound::Pairs = bound {
        let after_shift = h.track_pairs();
        println!(
            "{what}: (distinct candidate pairs, most per step) before the shift {before_shift:?}, \
             after it {after_shift:?}"
        );
        assert!(
            before_shift.0 + after_shift.0 <= AXIS_CLEAR_LIMIT
                && before_shift.1.max(after_shift.1) <= AXIS_CLEAR_LIMIT,
            "{what} construction: the pair-count no-clear bound needs at most {AXIS_CLEAR_LIMIT} \
             distinct candidate pairs over the two row sets and at most {AXIS_CLEAR_LIMIT} on any \
             step; (distinct, most per step) before the shift {before_shift:?}, after it \
             {after_shift:?}"
        );
    }
    trace.assert_undisturbed(
        what,
        "whole-pile row shift",
        &format!(
            "remap resets {resets:?} -> {:?}; (islands, row 0's island) on the spawn step \
             {islands_after:?}, pile ids off island 1 {renumbered:?}",
            h.remap_resets()
        ),
    );
    assert!(
        islands_after == (2, 0) && renumbered.is_empty(),
        "{what} construction: the far sphere is island 0 and the pile island 1 after the spawn; \
         (islands, row 0's island) {islands_after:?}, pile ids off island 1: {renumbered:?}"
    );
    assert_eq!(
        h.remap_resets(),
        resets,
        "{what}: both remap-reset counters must stay flat (no missed gather)"
    );
    assert!(
        h.poses(&pile) == poses,
        "{what}: every frozen box must keep its exact state across the row shift"
    );
}

/// G5: a height-4 pile, every row shifted by one.
#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: runs the real physics schedule on a boyko_threadpool until a 30-box pile \
              freezes; intractable under Miri"
)]
fn a_frozen_box_pile_ignores_a_spawn_that_shifts_every_row_it_holds() {
    under_watchdog("G5", SMALL_TIMEOUT, || {
        shift_scene("G5", SMALL, SMALL_SETTLE_LIMIT, ClearBound::Rows);
    });
}

/// G6: a height-5 pile (57 rows with the floor and the far sphere), every row shifted by one.
/// The row-count argument does not hold at 57 rows, but the pair-count bound in the module
/// header does and is asserted, so a clear is still ruled out: G6 adds scale over G5, not the
/// clear path (OQ5).
#[test]
#[cfg_attr(
    any(miri, debug_assertions),
    ignore = "slow: a 55-box pile through the real schedule until frozen; release only, \
              intractable under Miri"
)]
fn a_height_5_box_pyramid_ignores_a_spawn_that_shifts_every_row_it_holds() {
    under_watchdog("G6", MEDIUM_TIMEOUT, || {
        shift_scene("G6", MEDIUM, MEDIUM_SETTLE_LIMIT, ClearBound::Pairs);
    });
}
