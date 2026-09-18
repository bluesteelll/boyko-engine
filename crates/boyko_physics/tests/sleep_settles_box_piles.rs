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
//! # What a red G2 or G7 means before A7 lands
//!
//! It is TRIAGED, never waived and never silenced by raising the budget (orchestrator ruling,
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
//! 4. A budget is re-sized ONLY from a fresh run of the `flicker_redraw_distribution` generator,
//!    and only once A7 has changed the trajectories. Never raise a budget to turn a red green.
//! * **Hint changes** (design Decision 1 case 5, Known behaviour 2). Whether a knife-edge box
//!   pair produces a manifold can depend on the axis hint the pair reads from the box-axis
//!   cache, and a row move can change that hint while the poses stay put. G3 and G4 move one
//!   pile row across the pile (a swap-remove); G5 and G6 shift every pile row by one (a spawn
//!   into an archetype walked earlier). All four require that nothing wakes, and a failure
//!   names the kind of wake that fired: "count changed" (with every pair whose manifold
//!   appeared or vanished, each classed knife-edge or load-bearing), "count unchanged" or
//!   "latch lost". Only a "count changed" wake whose pairs are all knife-edge is Known
//!   behaviour 2; every other kind is a defect.
//!
//! **Defect A7.** A resting box pyramid creeps sideways with a fixed (-x, -z) bias and its
//! contact set keeps changing at rest, so piles of height 7 or more never reach a 60-step
//! quiet window. Its red-first tests are A7-R0 (a face contact that repeats a feature id),
//! A7-R1 (Jolt's pyramid creeps with sleeping off), A7-R2 (Jolt's pyramid freezes) and G8
//! (a height-6 pile freezes). A7a (clipped face-contact points carry injective feature ids)
//! greened A7-R0 and G8, and their ignores are gone. A7-R1 and A7-R2 stay `deferred:`
//! until the rest of the A7 lane lands.
//!
//! # Legs
//!
//! * G1, G3, G4 and G5 (height-4 piles, 30 boxes) run in both profiles, in the ordinary
//!   `cargo test -p boyko-physics --test sleep_settles_box_piles` run.
//! * G2, G6 (height 5, 55 boxes), G7 (sixteen height-4 draws) and G8 (height 6, 91 boxes)
//!   run ONLY in release, as part of the ordinary release run:
//!   `cargo test --release -p boyko-physics --test sleep_settles_box_piles -- --test-threads=6`.
//!   In a debug build they are ignored, and `-- --ignored` in a debug build is NOT their leg.
//! * A7-R1 and A7-R2 are `deferred:` and run by name with `--ignored`, in release:
//!   `cargo test --release -p boyko-physics --test sleep_settles_box_piles -- --ignored --exact <name>`.
//! * A7-R0 is device-free and schedule-free. It runs in the ordinary run in both profiles,
//!   and under Miri.
//! * `flicker_redraw_distribution` is a `generator:`: it asserts nothing and prints the
//!   numbers G2's and G7's budgets are sized from. Re-run it (release, `--ignored --exact`)
//!   whenever a change re-draws pile trajectories, before touching a budget.
//!
//! # Scene
//!
//! The real `add_physics_colored_solve` schedule on a serial pool, sleeping on with the
//! default `sleep_frames` (60) and threshold, dt = 1/60, gravity (0, -9.81, 0). The archetype
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
/// `flicker_redraw_distribution` measured (step 9987, msvc release, 2026-09-17), so that
/// G2's event budget, not the step limit, is what a flicker overrun reaches first. G2's own
/// draw froze at step 2574.
const MEDIUM_SETTLE_LIMIT: usize = 20_000;
/// Settle budget (steps) for each of G7's draws: eight times the latest freeze of the 30
/// height-4 draws measured (step 750).
const G7_SETTLE_LIMIT: usize = LONG_SETTLE_LIMIT;
/// G2's cap on contact-change wake events (steps on which `contact_wakes` rose). With one
/// draw of wake probability p per latch attempt, `P(>= 19 events) = p^19`: 0.3 % at the
/// measured [`MEASURED_P_HEIGHT_5`], 0.7 % at p = 0.77 (one standard error above it). The
/// most any of the 56 measured draws took was 13; G2's own draw takes 5.
const G2_MAX_EVENTS: usize = 18;
/// G7's cap on the contact-change wake events summed over its sixteen draws.
/// `P(>= 57 events)` over sixteen draws is 0.1 % at the measured [`MEASURED_P_HEIGHT_4`] and
/// 1.1 % at p = 0.66 (one standard error above it). G7's draws took 29.
const G7_MAX_EVENTS: usize = 56;
/// The per-latch-attempt wake probability at height 4, pooled over the 30 height-4 draws
/// of `flicker_redraw_distribution` (48 events in 78 attempts).
const MEASURED_P_HEIGHT_4: f64 = 48.0 / 78.0;
/// The same at height 5, pooled over its 56 draws (160 events in 216 attempts).
const MEASURED_P_HEIGHT_5: f64 = 160.0 / 216.0;
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
const G4_MOVER_ID: u32 = 13;
/// A7-R1's window: the vertical settle is over by `CREEP_FROM`.
const CREEP_FROM: usize = 600;
const CREEP_TO: usize = 3000;
/// A7-R1's bound on horizontal displacement over the window: 2 × Box2D's `B2_LINEAR_SLOP`.
const CREEP_BOUND_M: f32 = 0.01;
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
         (orchestrator ruling, round 3). Until A7 lands:\n\
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
         generator, and only once A7 has changed the trajectories. Never raise a budget to turn \
         a red green.\n\
         Escalate to the orchestrator with these numbers: Decision 3 is decided again only after \
         A7 (A4 round-2 ruling §3.4)",
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
    /// [`TOUCH_GAP`]. Only such pairs are exposed to Known behaviour 2
    /// (`IslandSleep::begin_step`, "Wakes from a box-axis hint change"); a floor pair or a
    /// pair between layers is load-bearing.
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
                    "Known behaviour 2 (A4 Decision 1 case 5): a hint change changed a knife-edge \
                     pair's existence with the poses fixed; deferrable under the round-2 ruling \
                     §5.2 step 3 only if the scene is green under M1",
                    detail,
                )
            } else {
                (
                    "count changed (a load-bearing pair)",
                    "NOT Known behaviour 2, which is scoped to knife-edge pairs: a load-bearing \
                     manifold appeared or vanished with the poses fixed, a narrowphase or \
                     row-identity defect; it blocks the commit",
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
/// A7a greened it, which is why the `deferred:` ignore is gone. Both readings are msvc
/// release, 2026-09-18:
///
/// * Base `9f712204` (before A7a) is RED. It did not freeze within [`LONG_SETTLE_LIMIT`]
///   steps. `contact_wakes` rose at steps [2315, 2614, 2744, 4550] (364 in total). On the
///   last step all 91 rows were awake in one island, and the largest `|v|² + |ω|²` over
///   the last 60 steps was 0.012338251, against a sleep threshold of 1e-4.
/// * With A7a (clipped face-contact points carry injective feature ids) it is GREEN. It
///   froze at step 128 with `contact_wakes` 91 and one rise event, at step 68. The same
///   line prints in debug (1.52 s there, 0.16 s in release).
///
/// Every contact change re-draws that freeze step, so it is a reading, not a pin. The
/// test's bound is [`LONG_SETTLE_LIMIT`].
#[test]
#[cfg_attr(
    any(miri, debug_assertions),
    ignore = "slow: a 91-box pile through the real schedule until frozen; release only, \
              intractable under Miri"
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
#[test]
#[ignore = "deferred: A7 — a resting box pyramid creeps; red until the A7 lane lands; release \
            only: cargo test --release -p boyko-physics --test sleep_settles_box_piles -- \
            --ignored a_resting_jolt_pyramid_does_not_creep_with_sleeping_off"]
fn a_resting_jolt_pyramid_does_not_creep_with_sleeping_off() {
    under_watchdog("A7-R1", JOLT_TIMEOUT, || {
        let mut h = Harness::new();
        h.world.resource_mut::<PhysicsConfig>().sleeping = false;
        let pile = spawn_scene(&mut h, JOLT, false, None);
        assert_eq!(
            pile.len(),
            1240,
            "construction: Jolt's pyramid holds 1240 boxes"
        );
        for _ in 0..CREEP_FROM {
            h.step();
        }
        let from = h.centres(&pile);

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
            "A7-R1 premise: at step {CREEP_FROM} the pyramid must still be a pyramid, but box id \
             {} in layer {} has dropped {} m below its spawn height (bound {STANDING_DROP_M} m). \
             A pile that collapsed and came to rest before the window meets the creep bound \
             trivially, so the creep reading below would be vacuous",
            worst_drop.1,
            layer_of(JOLT, worst_drop.1),
            worst_drop.0
        );

        for _ in CREEP_FROM..CREEP_TO {
            h.step();
        }
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
        assert!(
            worst.0 <= CREEP_BOUND_M,
            "A7-R1: a resting height-15 box pyramid with sleeping off moved sideways by \
             D_max = {} m between steps {CREEP_FROM} and {CREEP_TO} (bound {CREEP_BOUND_M} m): \
             worst box id {} in layer {} (layer i holds (15 - i)² boxes); mean (dx, dz) of layers \
             10-14 = ({:.4}, {:.4}) m; per-layer maximum, layer 0 first: {layer_max:?}",
            worst.0,
            worst.1,
            layer_of(JOLT, worst.1),
            top_sum.0 / n,
            top_sum.1 / n
        );
    });
}

/// A7-R2: Jolt's height-15 pyramid (1240 boxes) freezes and holds; its contact-change wake
/// events are recorded, not budgeted.
#[test]
#[ignore = "deferred: A7 — box piles of height 7 and more never come to rest; red until the A7 \
            lane lands; release only: cargo test --release -p boyko-physics --test \
            sleep_settles_box_piles -- --ignored a_jolt_scale_box_pyramid_freezes"]
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
/// delete.
fn swap_remove_scene(what: &'static str, mover_index: Option<usize>) -> (Lateral, Lateral) {
    let mut h = Harness::new();
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
        swap_remove_scene("G3", None);
    });
}

/// G4: pile id 13, layer 0's `j = 3, k = 0` corner box (x = 2, y = 1, z = -4), moves from
/// the last row into row 1, so its lateral knife-edge pairs reverse order and swap their
/// reference box: nothing may wake.
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
        let (before, after) = swap_remove_scene("G4", Some(index));
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
    // P2: at least one touching box pair with no manifold (necessary, not sufficient).
    let n_set = touching_no_contact_pairs(&mut h, &pile);
    assert!(
        !n_set.is_empty(),
        "scene-fitness: the settled pile has no touching no-contact box pair, so {what} cannot \
         exercise a hint change on one — escalate"
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
