//! G-L5-3 — the parallel narrowphase reproduces the serial loop, step for step, bit for bit.
//!
//! The default world (`add_physics_systems::<DefaultRigidSolver>`, default `PhysicsConfig`) runs a
//! mixed box/sphere pile with static sensor boxes through the real schedule on pools of 1, 2, 4, 8
//! and 16 workers, each with `parallel_narrowphase` off and on — on is the default since L5 C4,
//! and [`Rig::new`] asserts the built world carries it before the arm sets its own value, so the
//! flag-off arms are the explicit override. Every [`CHURN_EVERY`] steps [`CHURN_BODIES`] bodies of
//! the pile are despawned and as many new ones spawned into a later archetype, so rows move. After
//! every step four hashes must equal the oracle's, the 1-worker flag-off run:
//!
//! * the solver manifold stream (`Manifolds::manifolds`), every field of every manifold in order;
//! * the sensor-overlap stream (`Manifolds::sensor_overlaps`), likewise;
//! * the hysteresis table state (`BoxAxisCache::fingerprint`: `(key, axis)` per slot and
//!   `occupied`, hashed by field);
//! * the pair tags of L9's pair carry (`Manifolds::pair_tags_fingerprint`, L9 G-C-2): every pair's
//!   tag, in pair order, including the separated pairs its carried separating axis rejected;
//! * the pose: every live body's `RigidBody` bits, in the order the bodies were spawned.
//!
//! # Non-vacuity
//!
//! Checked AFTER every arm has been compared, so a witness that falls short cannot hide the
//! comparison: a red witness still reports how many arms matched. The churn constants are
//! chosen for the load-clear witness; see [`STEPS`].
//!
//! - Every step has at least [`MIN_PAIRS`] candidate pairs, so every arm of two or more workers
//!   dispatches: at least two chunks at every worker count.
//! - `Manifolds::narrowphase_dispatches` rises by exactly one per step on every flag-on arm of two
//!   or more workers, and stays flat on the flag-off arms and on the 1-worker flag-on arm.
//! - The churn reaches every branch of the axis table's frame preparation: at least one frame whose
//!   rows changed (the carried axes were pre-read), at least one load-based clear and at least one
//!   grow, counted by the table's own diagnostics.
//! - The streams hold sensor overlaps and box-sphere manifolds whose box is the lower row, the pair
//!   `collide_pair` flips.
//! - L9's pair carry is exercised and never loses its place: carried separating axes reject pairs
//!   (`Manifolds::separated_axis_hits`), the churn's row moves build the jumper bitset
//!   (`pair_carry_jumper_builds`), and no step's carry is Reset (`pair_carry_resets` stays 0).
//!
//! # G-L5-3 W1 — one lane never dispatches
//!
//! [`one_worker_parallel_narrowphase_runs_the_serial_loop`]: with the flag on and a ONE-worker
//! pool, the dispatch counter does not move, while the same scene on two workers dispatches every
//! step. The scene has enough pairs that the work term alone gives six chunks at one lane, so the
//! zero at one worker can only come from the `lanes < 2` term (lever ruling W1): dropping it must
//! turn this red.
//!
//! # The Jolt pyramid (slow)
//!
//! [`jolt_pyramid_parallel_narrowphase_is_bit_identical`] runs Jolt's `PyramidScene.h` pile for 600
//! steps at W ∈ {1, 8, 16} with the flag on against the 1-worker flag-off oracle. Ignored as
//! `slow:`; run it in release with `-- --ignored`.
//!
//! Spins real thread pools (intractable under Miri), so `cfg(not(miri))`; the Miri leg of the same
//! property is `narrowphase::dispatch`'s `chunks_on_threads_equal_the_serial_loop`.

#![cfg(not(miri))]

use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_macros::Component;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyMass, Sensor, Simulated,
};
use boyko_physics::manifold::Manifold;
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::narrowphase::NP_MIN_PAIRS_PER_CHUNK;
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{ContactPairs, Manifolds, PhysicsConfig, SolverScratch};
use boyko_physics::solver::DefaultRigidSolver;

/// Fixed timestep.
const DT: f32 = 1.0 / 60.0;

/// Steps each arm runs.
///
/// `STEPS`, [`CHURN_EVERY`] and [`CHURN_BODIES`] are chosen together, for the load-clear
/// witness. The table clears on load once live occupancy passes half of its 4096 slots, and
/// occupancy grows only by re-keyed pairs: a despawn shifts every row behind it, so each churn
/// frame re-keys most box-box contacts, and the NUMBER OF CHURN FRAMES drives the clears, not
/// the bodies a churn moves (20 bodies every 7 steps over 77 steps, 10 churns, never cleared;
/// 10 every 3, 25 churns, cleared once). With a churn every second step the clears come about
/// 78 steps apart (some 39 churns, about 52 new keys a churn): 160 steps hold two, at steps
/// 45 and 123, with the third about 40 steps past the end. The count drops to one only if
/// occupancy grew 30 % slower and to zero only at 3.5x slower; more contacts only add clears.
/// Three bodies a churn keep the 79 churns' 237 despawns under the pile's 294 bodies
/// (`churn`'s `% pile.len()`), with 57 left, and the fewest pairs any step has at 2.5x
/// [`MIN_PAIRS`]. Measured on the un-modified gate with its witness printed, per setting.
///
/// Measured 2026-09-20 at aac562a7 + the C3 working tree, debug and release alike:
/// load_clears = 2, prefetched_frames = 80, grows = 1, min_pairs = 1288.
const STEPS: usize = 160;

/// Rows move every `CHURN_EVERY` steps (see [`STEPS`]).
const CHURN_EVERY: usize = 2;

/// Bodies despawned from the pile, and spawned into the late archetype, per churn (see
/// [`STEPS`]).
const CHURN_BODIES: usize = 3;

/// The pile's grid: `PILE_X × PILE_Z` columns, `PILE_Y` layers.
const PILE_X: usize = 7;
/// See [`PILE_X`].
const PILE_Z: usize = 7;
/// See [`PILE_X`].
const PILE_Y: usize = 6;

/// Centre-to-centre spacing: each unit box overlaps its face neighbours by 1 cm, so every
/// neighbour is in contact from the first step.
const PITCH: f32 = 0.99;

/// Every fourth pile body is a sphere of radius 0.5; the rest are unit boxes.
const SPHERE_EVERY: usize = 4;

/// The fewest candidate pairs any step of the churned run may have: four chunks' worth, so every
/// arm of two or more workers dispatches with room to spare.
const MIN_PAIRS: usize = 4 * NP_MIN_PAIRS_PER_CHUNK;

/// The worker counts of the matrix.
const WORKERS: [usize; 5] = [1, 2, 4, 8, 16];

/// The late archetype's marker: bodies spawned by the churn live in their own archetype, walked
/// after the pile's, so every despawn from the pile shifts all of their rows.
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct Late {
    _tag: u32,
}

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its `size_of::<T>()` bytes
    // read-only for the duration of the borrow, which is the exact layout the component pool
    // stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// The three archetypes, created in gather order: the pile, the sensors, the late bodies.
struct Archetypes {
    pile: ArchetypeId,
    sensor: ArchetypeId,
    late: ArchetypeId,
}

/// Which archetype a body goes into.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Pile,
    Sensor,
    Late,
}

/// One world on the default pipeline, with its live bodies in spawn order.
struct Rig {
    world: EcsMaster,
    physics: Schedule,
    archetypes: Archetypes,
    /// Every live body, in spawn order; the churn removes and appends.
    live: Vec<Entity>,
    /// The pile's dynamic bodies still alive, in spawn order: the churn despawns only these.
    pile: Vec<Entity>,
    /// Bodies spawned by the churn so far, for their placement.
    spawned: usize,
}

impl Rig {
    fn new(workers: usize, parallel_np: bool) -> Self {
        let mut world = EcsMaster::new();
        let base = [RigidBody::component_id(), RigidBodyMass::component_id(), Collider::component_id()];
        let archetypes = Archetypes {
            pile: world.create_archetype(&base),
            sensor: world.create_archetype(&[base[0], base[1], base[2], Sensor::component_id()]),
            late: world.create_archetype(&[base[0], base[1], base[2], Late::component_id()]),
        };
        let mut builder =
            ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(workers).build());
        add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        let physics = builder.build(&mut world);
        let cfg = world.resource_mut::<PhysicsConfig>();
        assert!(
            cfg.parallel_narrowphase,
            "L5 C4: the default world must request the parallel narrowphase; the flag-off arms \
             of this gate are the override, not the default"
        );
        cfg.parallel_narrowphase = parallel_np;
        let mut rig =
            Self { world, physics, archetypes, live: Vec::new(), pile: Vec::new(), spawned: 0 };
        rig.spawn_scene();
        rig
    }

    fn spawn(&mut self, kind: Kind, position: Vec3, shape: ColliderShape, inv_mass: f32) -> Entity {
        let body = RigidBody {
            position,
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        };
        let mass = RigidBodyMass {
            inv_inertia: if inv_mass == 0.0 {
                Mat3::ZERO
            } else {
                Mat3::from_diagonal(Vec3::new(6.0, 6.0, 6.0))
            },
            inv_mass,
            restitution: 0.0,
            friction: 0.5,
        };
        let collider = Collider { shape, layer: 1, mask: 1 };
        let base: [(_, &[u8]); 3] = [
            (RigidBody::component_id(), as_bytes(&body)),
            (RigidBodyMass::component_id(), as_bytes(&mass)),
            (Collider::component_id(), as_bytes(&collider)),
        ];
        let sensor = Sensor;
        let late = Late { _tag: 0 };
        let created = match kind {
            Kind::Pile => self.world.create_entity(self.archetypes.pile, &base),
            Kind::Sensor => self.world.create_entity(
                self.archetypes.sensor,
                &[base[0], base[1], base[2], (Sensor::component_id(), as_bytes(&sensor))],
            ),
            Kind::Late => self.world.create_entity(
                self.archetypes.late,
                &[base[0], base[1], base[2], (Late::component_id(), as_bytes(&late))],
            ),
        };
        let e = created.expect("construction: every archetype accepts its columns");
        if inv_mass != 0.0 {
            self.world.enable::<Simulated>(e);
        }
        e
    }

    /// A static floor, the pile and four static sensor boxes cutting through it.
    fn spawn_scene(&mut self) {
        let floor = ColliderShape::Box { half_extents: Vec3::new(40.0, 0.5, 40.0) };
        let e = self.spawn(Kind::Pile, Vec3::new(0.0, -0.5, 0.0), floor, 0.0);
        self.live.push(e);
        let unit_box = ColliderShape::Box { half_extents: Vec3::new(0.5, 0.5, 0.5) };
        let sphere = ColliderShape::Sphere { radius: 0.5 };
        let ox = -0.5 * PITCH * (PILE_X as f32 - 1.0);
        let oz = -0.5 * PITCH * (PILE_Z as f32 - 1.0);
        let mut k = 0;
        for y in 0..PILE_Y {
            for z in 0..PILE_Z {
                for x in 0..PILE_X {
                    let p = Vec3::new(ox + PITCH * x as f32, 0.49 + PITCH * y as f32, oz + PITCH * z as f32);
                    let shape = if k % SPHERE_EVERY == SPHERE_EVERY - 1 { sphere } else { unit_box };
                    let e = self.spawn(Kind::Pile, p, shape, 1.0);
                    self.live.push(e);
                    self.pile.push(e);
                    k += 1;
                }
            }
        }
        let slab = ColliderShape::Box { half_extents: Vec3::new(0.6, 3.0, 0.6) };
        for (x, z) in [(-1.5, -1.5), (1.5, -1.5), (-1.5, 1.5), (1.5, 1.5)] {
            let e = self.spawn(Kind::Sensor, Vec3::new(x, 2.5, z), slab, 0.0);
            self.live.push(e);
        }
    }

    /// Despawns [`CHURN_BODIES`] pile bodies, spread through the pile's spawn order, and spawns
    /// as many into the late archetype, resting on the pile's top.
    fn churn(&mut self) {
        let pile_top = 0.49 + PITCH * PILE_Y as f32;
        for i in 0..CHURN_BODIES {
            // Spread over the pile's spawn order, the same in every arm.
            let e = self.pile.remove((i * 13 + self.spawned) % self.pile.len());
            let at = self
                .live
                .iter()
                .position(|&l| l == e)
                .expect("invariant: every pile body is live");
            self.live.remove(at);
            assert!(self.world.delete_entity(e), "construction: a live body is despawnable");
        }
        let unit_box = ColliderShape::Box { half_extents: Vec3::new(0.5, 0.5, 0.5) };
        let sphere = ColliderShape::Sphere { radius: 0.5 };
        for _ in 0..CHURN_BODIES {
            let s = self.spawned;
            let p = Vec3::new(
                -2.5 + (s % 6) as f32,
                pile_top + (s / 36) as f32,
                -2.5 + ((s / 6) % 6) as f32,
            );
            let shape = if s % SPHERE_EVERY == 1 { sphere } else { unit_box };
            let e = self.spawn(Kind::Late, p, shape, 1.0);
            self.live.push(e);
            self.spawned += 1;
        }
    }

    fn step(&mut self) {
        self.physics.run(&mut self.world);
    }

    fn manifolds(&self) -> &Manifolds {
        self.world.resource::<Manifolds>()
    }
}

/// FNV-1a 64 over `words`, continuing from `h`.
fn fnv(h: u64, words: impl IntoIterator<Item = u32>) -> u64 {
    words.into_iter().fold(h, |h, w| {
        w.to_le_bytes().iter().fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
    })
}

/// FNV-1a 64's offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// Every field of every manifold, in stream order.
fn stream_hash(stream: &[Manifold]) -> u64 {
    stream.iter().fold(FNV_OFFSET, |h, m| {
        let h = fnv(h, [m.body_a.0, m.body_b.0, u32::from(m.count)]);
        let h = fnv(h, [m.normal.x, m.normal.y, m.normal.z].map(f32::to_bits));
        m.points.iter().fold(h, |h, p| {
            let h = fnv(
                h,
                [p.anchor_a.x, p.anchor_a.y, p.anchor_a.z, p.anchor_b.x, p.anchor_b.y, p.anchor_b.z]
                    .map(f32::to_bits),
            );
            fnv(h, [p.separation.to_bits(), p.feature_id])
        })
    })
}

/// The full state of every body of `bodies`, as bits, in the given order.
fn pose_hash(world: &EcsMaster, bodies: &[Entity]) -> u64 {
    bodies.iter().fold(FNV_OFFSET, |h, &e| {
        let b = world.get_component::<RigidBody>(e).expect("invariant: a live body has a RigidBody");
        fnv(
            h,
            [
                b.position.x,
                b.position.y,
                b.position.z,
                b.linear_velocity.x,
                b.linear_velocity.y,
                b.linear_velocity.z,
                b.rotation.x,
                b.rotation.y,
                b.rotation.z,
                b.rotation.w,
                b.angular_velocity.x,
                b.angular_velocity.y,
                b.angular_velocity.z,
            ]
            .map(f32::to_bits),
        )
    })
}

/// One step's five hashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StepHashes {
    stream: u64,
    sensor: u64,
    table: u64,
    tags: u64,
    pose: u64,
}

/// What the churned run exercised, for the non-vacuity asserts.
#[derive(Debug, Default)]
struct Witness {
    min_pairs: usize,
    sensor_manifolds: usize,
    box_sphere_flips: usize,
    prefetched_frames: u64,
    load_clears: u64,
    grows: u64,
    /// Pairs rejected by their carried separating axis, over every step (L9a (ii)).
    separated_axis_hits: usize,
    /// Steps whose pair carry built its jumper bitset (the rows moved).
    jumper_builds: u64,
    /// Steps whose pair carry was Reset although stamped.
    carry_resets: u64,
}

/// Runs one arm of the churned scene and returns its per-step hashes and its witness. Asserts the
/// dispatch counter's per-step movement for the arm's worker count and flag.
fn run_churned(workers: usize, parallel_np: bool) -> (Vec<StepHashes>, Witness) {
    let mut rig = Rig::new(workers, parallel_np);
    let dispatches_per_step = u64::from(parallel_np && workers >= 2);
    let mut hashes = Vec::with_capacity(STEPS);
    let mut witness = Witness { min_pairs: usize::MAX, ..Witness::default() };
    for step in 0..STEPS {
        if step > 0 && step % CHURN_EVERY == 0 {
            rig.churn();
        }
        let before = rig.manifolds().narrowphase_dispatches();
        rig.step();
        let m = rig.manifolds();
        assert_eq!(
            m.narrowphase_dispatches() - before,
            dispatches_per_step,
            "W={workers}, parallel_narrowphase {parallel_np}, step {step}: narrowphase dispatches \
             this step"
        );
        let pairs = rig.world.resource::<ContactPairs>().pairs().len();
        witness.min_pairs = witness.min_pairs.min(pairs);
        witness.sensor_manifolds += m.sensor_overlaps().len();
        let bodies = rig.world.resource::<SolverScratch>().bodies();
        let is_box = |row: u32| matches!(bodies[row as usize].shape, ColliderShape::Box { .. });
        witness.box_sphere_flips +=
            m.manifolds().iter().filter(|mf| is_box(mf.body_a.0) && !is_box(mf.body_b.0)).count();
        witness.separated_axis_hits += m.separated_axis_hits();
        hashes.push(StepHashes {
            stream: stream_hash(m.manifolds()),
            sensor: stream_hash(m.sensor_overlaps()),
            table: m.box_axis_cache.fingerprint(),
            tags: m.pair_tags_fingerprint(),
            pose: pose_hash(&rig.world, &rig.live),
        });
    }
    let cache = &rig.manifolds().box_axis_cache;
    witness.prefetched_frames = cache.prefetched_frames();
    witness.load_clears = cache.load_clears();
    witness.grows = cache.grows();
    witness.jumper_builds = rig.manifolds().pair_carry_jumper_builds();
    witness.carry_resets = rig.manifolds().pair_carry_resets();
    (hashes, witness)
}

/// Compares an arm's hashes with the oracle's, step by step, naming the first difference.
fn assert_matches(oracle: &[StepHashes], arm: &[StepHashes], workers: usize, parallel_np: bool) {
    assert_eq!(arm.len(), oracle.len());
    for (step, (got, want)) in arm.iter().zip(oracle).enumerate() {
        assert_eq!(
            got, want,
            "W={workers}, parallel_narrowphase {parallel_np}: step {step} differs from the 1-worker \
             serial oracle"
        );
    }
}

#[test]
fn parallel_narrowphase_matches_the_serial_oracle_under_churn() {
    let (oracle, witness) = run_churned(1, false);
    println!("G-L5-3 oracle witness: {witness:?}");

    // Every arm is compared before any witness is asserted, so a witness that falls short
    // cannot hide the comparison; its result is in every witness message below.
    let mut arms = 0usize;
    for workers in WORKERS {
        for parallel_np in [false, true] {
            if workers == 1 && !parallel_np {
                continue;
            }
            let (arm, arm_witness) = run_churned(workers, parallel_np);
            assert_matches(&oracle, &arm, workers, parallel_np);
            assert_eq!(
                (arm_witness.prefetched_frames, arm_witness.load_clears, arm_witness.grows),
                (witness.prefetched_frames, witness.load_clears, witness.grows),
                "W={workers}, parallel_narrowphase {parallel_np}: the axis table's frame \
                 preparation took a different path than the oracle's"
            );
            arms += 1;
        }
    }
    let compared = format!("{arms} arms matched the oracle on all {STEPS} steps; {witness:?}");
    println!("G-L5-3: {compared}");

    assert!(
        witness.min_pairs >= MIN_PAIRS,
        "every step needs at least {MIN_PAIRS} candidate pairs, or an arm may not dispatch \
         ({compared})"
    );
    assert!(witness.sensor_manifolds > 0, "the sensors overlapped nothing ({compared})");
    assert!(
        witness.box_sphere_flips > 0,
        "no box-sphere manifold with the box as body A ({compared})"
    );
    assert!(witness.prefetched_frames > 0, "no frame pre-read carried axes ({compared})");
    assert!(witness.load_clears > 0, "the axis table never cleared on load ({compared})");
    assert!(witness.grows > 0, "the axis table never grew ({compared})");
    assert!(
        witness.separated_axis_hits > 0,
        "no carried separating axis rejected a pair (L9a (ii)) ({compared})"
    );
    assert!(witness.jumper_builds > 0, "the pair carry never built its jumper bitset ({compared})");
    assert_eq!(witness.carry_resets, 0, "the pair carry lost its place ({compared})");
    let moved = oracle.first().map(|h| h.pose) != oracle.last().map(|h| h.pose);
    assert!(moved, "the scene must move ({compared})");
}

/// Steps the un-churned scene `steps` times and returns the dispatch counter's rise and the fewest
/// candidate pairs any step had.
fn dispatches_over(workers: usize, steps: usize) -> (u64, usize) {
    let mut rig = Rig::new(workers, true);
    let mut min_pairs = usize::MAX;
    for _ in 0..steps {
        rig.step();
        min_pairs = min_pairs.min(rig.world.resource::<ContactPairs>().pairs().len());
    }
    (rig.manifolds().narrowphase_dispatches(), min_pairs)
}

#[test]
fn one_worker_parallel_narrowphase_runs_the_serial_loop() {
    const STEPS_W1: usize = 3;
    let (one, min_pairs) = dispatches_over(1, STEPS_W1);
    assert!(
        min_pairs / NP_MIN_PAIRS_PER_CHUNK >= 6,
        "non-vacuity: the work term alone must give six chunks at one lane, so only the lanes \
         term can keep this world serial ({min_pairs} pairs)"
    );
    assert_eq!(one, 0, "a one-worker pool with parallel_narrowphase on dispatched the narrowphase");
    let (two, _) = dispatches_over(2, STEPS_W1);
    assert_eq!(two, STEPS_W1 as u64, "control: the same scene on two workers dispatches every step");
}

// ── The Jolt pyramid (slow) ──────────────────────────────────────────────────────

/// Jolt's `cPyramidHeight`.
const PYRAMID_HEIGHT: i32 = 15;
/// Jolt's `cBoxSize`.
const JOLT_BOX_SIZE: f32 = 2.0;
/// Jolt's `cBoxSeparation`.
const JOLT_SEPARATION: f32 = 0.5;
/// Steps the pyramid arms run.
const PYRAMID_STEPS: usize = 600;

/// Jolt's pyramid on one world: a (50, 1, 50) floor and 1240 unit-density boxes of half-extent 1,
/// friction 0.2, placed index for index as `PyramidScene.h` places them.
fn pyramid(workers: usize, parallel_np: bool) -> (EcsMaster, Schedule, Vec<Entity>) {
    let mut world = EcsMaster::new();
    let archetype = world.create_archetype(&[
        RigidBody::component_id(),
        RigidBodyMass::component_id(),
        Collider::component_id(),
    ]);
    let spawn = |world: &mut EcsMaster, position: Vec3, dynamic: bool| {
        let body = RigidBody {
            position,
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        };
        let (mass, half) = if dynamic {
            (
                RigidBodyMass {
                    inv_inertia: Mat3::from_diagonal(Vec3::new(0.1875, 0.1875, 0.1875)),
                    inv_mass: 0.125,
                    restitution: 0.0,
                    friction: 0.2,
                },
                Vec3::new(1.0, 1.0, 1.0),
            )
        } else {
            (
                RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass: 0.0, restitution: 0.0, friction: 0.2 },
                Vec3::new(50.0, 1.0, 50.0),
            )
        };
        let collider = Collider { shape: ColliderShape::Box { half_extents: half }, layer: 1, mask: 1 };
        let e = world
            .create_entity(
                archetype,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                ],
            )
            .expect("construction: the body archetype accepts its columns");
        if dynamic {
            world.enable::<Simulated>(e);
        }
        e
    };
    spawn(&mut world, Vec3::new(0.0, -1.0, 0.0), false);
    let mut boxes = Vec::with_capacity(1240);
    for i in 0..PYRAMID_HEIGHT {
        let lo = i / 2;
        let hi = PYRAMID_HEIGHT - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { 0.5 * JOLT_BOX_SIZE } else { 0.0 };
                let position = Vec3::new(
                    -(PYRAMID_HEIGHT as f32) + JOLT_BOX_SIZE * j as f32 + odd,
                    1.0 + (JOLT_BOX_SIZE + JOLT_SEPARATION) * i as f32,
                    -(PYRAMID_HEIGHT as f32) + JOLT_BOX_SIZE * k as f32 + odd,
                );
                boxes.push(spawn(&mut world, position, true));
            }
        }
    }
    assert_eq!(boxes.len(), 1240, "construction: Jolt's pyramid holds 1240 boxes");
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(workers).build());
    add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    let physics = builder.build(&mut world);
    world.resource_mut::<PhysicsConfig>().parallel_narrowphase = parallel_np;
    (world, physics, boxes)
}

/// Per-step hashes of the pyramid, and the number of dispatched steps.
fn run_pyramid(workers: usize, parallel_np: bool) -> (Vec<StepHashes>, u64) {
    let (mut world, mut physics, boxes) = pyramid(workers, parallel_np);
    let mut hashes = Vec::with_capacity(PYRAMID_STEPS);
    for _ in 0..PYRAMID_STEPS {
        physics.run(&mut world);
        let m = world.resource::<Manifolds>();
        hashes.push(StepHashes {
            stream: stream_hash(m.manifolds()),
            sensor: stream_hash(m.sensor_overlaps()),
            table: m.box_axis_cache.fingerprint(),
            tags: m.pair_tags_fingerprint(),
            pose: pose_hash(&world, &boxes),
        });
    }
    let dispatches = world.resource::<Manifolds>().narrowphase_dispatches();
    (hashes, dispatches)
}

#[test]
#[ignore = "slow: Jolt's 1240-box pyramid for 600 steps on four worlds; run in release with -- --ignored"]
fn jolt_pyramid_parallel_narrowphase_is_bit_identical() {
    let (oracle, none) = run_pyramid(1, false);
    assert_eq!(none, 0, "the flag-off oracle must not dispatch");
    for workers in [1, 8, 16] {
        let (arm, dispatches) = run_pyramid(workers, true);
        let want = if workers >= 2 { PYRAMID_STEPS as u64 } else { 0 };
        assert_eq!(dispatches, want, "W={workers}: dispatched steps");
        assert_matches(&oracle, &arm, workers, true);
    }
}
