//! L9 C0: the narrowphase CLASS bench (`docs/physics/perf-campaign/levers/L9-contact-reuse/
//! 02-DESIGN-REV1.md`, commit C0 and the per-pair cost table under "Algorithms for critical paths").
//!
//! L9 predicts its gain per pair CLASS: a separated box pair (the SAT's early exit and the cached
//! separating axis, L9a), a touching box pair (the reuse hit, L9b) and a fast pair (today's path,
//! D8). Its refutation rule — "if the measured costs predict Δt_np(1) < 1.0 ms, stop and
//! escalate" — needs those classes counted and each one's cost measured on the scenes the
//! campaign runs. This bench does both, on snapshots of the real colored schedule.
//!
//! # Scenes
//!
//! | scene | what | snapshot |
//! |---|---|---|
//! | `jolt` | Jolt's pyramid (1240 boxes, layer gap 0.5, friction 0.2), the parity runner's `--scene jolt` | after step 300 |
//! | `rest` | the same pyramid exactly touching (gap 0, friction 0.5), the runner's `--scene rest` | after step 300 |
//! | `rain` | 1000 boxes on a 10×10×10 lattice (pitch 2.5) with seeded random orientations, velocities (down 5 m/s ± 1) and spins (±3 rad/s) falling onto the floor | after step 40 |
//!
//! Each scene is stepped on the colored schedule with one worker, sleeping off and the AllPairs
//! broadphase, and the snapshot is what the narrowphase read on the last step, copied by a probe
//! system ordered after the narrowphase and before the constraint graph (after the step the solve
//! has advanced the gathered bodies in place): the gathered `SolverScratch::bodies`, the
//! `ContactPairs` and the box-axis hysteresis table (`BoxAxisCache::get`), which after the
//! narrowphase holds the axis each touching pair chose — the hint a resting pair reads next step.
//!
//! # Classes
//!
//! Per candidate pair: `non_box` (a shape other than two boxes; none in these scenes), then
//! `fast` by D8's predicate — `3·dt²·(|v_S−v_F|² + |ω_F|²·|c_S−c_F|² + |ω_S−ω_F|²·(r_S+τ_eff)²) >
//! (τ_eff/2)²`, F the body with the larger bounding radius (A on a bitwise tie), τ = 1 mm,
//! τ_eff = min(τ, 0.05·min(r_A, r_B)); ruling O1's half-extent clamp does not bind on the unit
//! boxes and the floor measured here — then `separated` or `touching` by the public
//! [`box_box_contact`] (the box arm of `collide_pair`). Fast pairs are split by the same SAT
//! answer.
//!
//! # Structural asserts (a void is red)
//!
//! * **Closure:** the classes sum to the pair count.
//! * **Replica agreement:** a bench-local 15-axis SAT in canonical order finds a negative axis
//!   exactly on the pairs [`box_box_contact`] reports separated. The replica's first negative
//!   axis also gives the separated pairs' histogram (A face / B face / edge) and the mean number
//!   of axes an early exit evaluates (the review's OQ3: L9a (i)'s share against (ii)'s).
//! * **The step's contact set:** the touching count equals the step's solver manifold count
//!   (whether a box pair has a manifold is a function of the two poses alone, so the hint cannot
//!   change it).
//! * **The design's J counts:** separated and touching within 5 % of 5,037 and 4,524 (P0b).
//! * **The carried separating axis (L9 C2):** the pairs the step's narrowphase rejected by the
//!   separating axis their previous step carried (`Manifolds::separated_axis_hits`, the SAT did
//!   not run for them) are some of the separated pairs, and on a pile at rest there are some.
//! * **Moving scene:** `rain` has fast pairs.
//! * **Timed classes:** every class a scene times holds at least one pair.
//!
//! # Timing — NOT a result on a shared machine
//!
//! Per timed class, the median over K repetitions of the class's pairs collided in stream order
//! through [`box_box_contact`], in ns per pair, plus the whole stream. The design's refutation
//! reading is printed beside them: Δt_np(1) = N_sep·(t_sep − t_sep') + h·N_touch·(t_touch −
//! t_hit) − carry, with the design's post-L9 costs t_sep' ∈ [35, 60] ns, t_hit ∈ [60, 100] ns,
//! h ∈ [0.80, 0.97] and carry ∈ [0.03, 0.06] ms. Both are read in a quiet window only (the P0
//! protocol); a number printed on a shared machine is not a result. The counts are untimed and
//! are results.
//!
//! # Modes
//!
//! `cargo bench` passes `--bench`: the full scenes, the design-count asserts, K = 51. Without it
//! (`cargo test --all-targets` runs this binary with libtest's arguments) it runs a self-check —
//! height-4 pyramids at step 120 and a 3×3×3 rain at step 30 — with every structural assert except
//! the design's J counts, and one untimed repetition. `--list` prints nothing.
//!
//! # Build and run
//!
//! ```text
//! cargo bench --no-run --profile parity -p boyko-physics --bench narrowphase_classes
//! <target>/parity/deps/narrowphase_classes-<hash>.exe --bench
//! ```
//!
//! msvc host, no `RUSTFLAGS` (it would replace the `x86-64-v3` baseline in `.cargo/config.toml`).
//! Exit code: 0 ok; 101 a structural assert failed.

use std::hint::black_box;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_macros::Resource;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::manifold::BodyIndex;
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::narrowphase::box_box::box_box_contact;
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{
    BodyState, BroadphaseKind, BroadphaseSelectMode, ContactPairs, Manifolds, PhysicsConfig,
    SolverScratch,
};
use boyko_physics::systems::body_bounding_radius;

// ── Scene constants (the parity runner's, transcribed) ───────────────────────

/// `cBoxSize`: the pitch between neighbouring boxes in a pyramid layer.
const BOX_SIZE: f32 = 2.0;
/// Every box's half-extent on every axis.
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
/// Jolt's pyramid height (1240 boxes).
const PYRAMID_HEIGHT: i32 = 15;
/// The `jolt` scene's layer gap.
const JOLT_GAP: f32 = 0.5;
/// Jolt's default friction, the `jolt` scene's.
const JOLT_FRICTION: f32 = 0.2;
/// The `rest` scene's friction.
const REST_FRICTION: f32 = 0.5;
/// The floor's half-extents.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
/// A box's inverse mass.
const BOX_INV_MASS: f32 = 0.125;
/// A box's inverse inertia, per axis.
const BOX_INV_INERTIA: f32 = 0.1875;
/// The fixed step.
const DT: f32 = 1.0 / 60.0;

/// The pyramid snapshots' step (the design's "J and R at step 300").
const PYRAMID_SNAPSHOT_STEPS: usize = 300;
/// The rain lattice's side, in boxes.
const RAIN_SIDE: usize = 10;
/// The rain lattice's pitch: face neighbours pass the bounding-sphere broadphase (2·√3 > 2.5),
/// edge diagonals do not.
const RAIN_PITCH: f32 = 2.5;
/// The rain lattice's lowest layer's centre height.
const RAIN_Y0: f32 = 3.0;
/// The rain snapshot's step: the lower layers have landed, the upper ones are still falling.
const RAIN_SNAPSHOT_STEPS: usize = 40;
/// The rain scene's seed.
const RAIN_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// D11's default reuse distance τ, in metres.
const TAU: f32 = 0.001;

/// The design's J counts per step (P0b): SAT-separated box pairs and manifolds.
const DESIGN_J_SEPARATED: usize = 5_037;
/// See [`DESIGN_J_SEPARATED`].
const DESIGN_J_TOUCHING: usize = 4_524;
/// The band the J counts must fall in, as a fraction of the design's.
const DESIGN_COUNT_BAND: f64 = 0.05;

/// Timed repetitions per class in `--bench` mode.
const BENCH_REPS: usize = 51;

/// The design's post-L9 per-pair costs and hit rate (the refutation reading's inputs).
const SEP_AFTER_NS: (f64, f64) = (35.0, 60.0);
/// See [`SEP_AFTER_NS`].
const HIT_NS: (f64, f64) = (60.0, 100.0);
/// See [`SEP_AFTER_NS`].
const HIT_RATE: (f64, f64) = (0.80, 0.97);
/// See [`SEP_AFTER_NS`]; milliseconds.
const CARRY_MS: (f64, f64) = (0.03, 0.06);

// ── Scene construction ───────────────────────────────────────────────────────

/// Which scene a spec builds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SceneKind {
    /// Jolt's pyramid (gap 0.5).
    Jolt,
    /// The exactly touching pyramid (gap 0).
    Rest,
    /// The falling lattice.
    Rain,
}

/// One scene at one size.
struct SceneSpec {
    name: &'static str,
    kind: SceneKind,
    /// Pyramid height, or the rain lattice's side.
    size: usize,
    /// Steps before the snapshot.
    steps: usize,
}

/// Views a `#[repr(C)]` POD component as its bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, initialised `#[repr(C)]` POD component borrowed for the returned
    // slice's lifetime; the slice covers exactly its `size_of::<T>()` bytes, read-only, which is
    // the layout the component pool stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Spawns one box into the `RigidBodyBundle` archetype. A dynamic one is enabled as
/// `Simulated`; the static floor is not.
fn spawn_box(world: &mut EcsMaster, body: RigidBody, friction: f32, dynamic: bool) {
    let (mass, half_extents) = if dynamic {
        (
            RigidBodyMass {
                inv_inertia: Mat3::from_diagonal(Vec3::new(
                    BOX_INV_INERTIA,
                    BOX_INV_INERTIA,
                    BOX_INV_INERTIA,
                )),
                inv_mass: BOX_INV_MASS,
                restitution: 0.0,
                friction,
            },
            Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
        )
    } else {
        (
            RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass: 0.0, restitution: 0.0, friction },
            FLOOR_HALF_EXTENTS,
        )
    };
    let collider = Collider { shape: ColliderShape::Box { half_extents }, layer: 1, mask: 1 };
    let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
    let e = world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("construction: the RigidBodyBundle archetype accepts the three columns");
    if dynamic {
        world.enable::<Simulated>(e);
    }
}

/// A body at rest at `position`.
fn at_rest(position: Vec3) -> RigidBody {
    RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    }
}

/// xorshift64*: a seeded, dependency-free generator for the rain draws.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[-1, 1)`.
    fn signed_unit(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }

    /// A unit quaternion, by rejection from the unit 4-ball.
    fn rotation(&mut self) -> Quat {
        loop {
            let q = Quat::new(
                self.signed_unit(),
                self.signed_unit(),
                self.signed_unit(),
                self.signed_unit(),
            );
            let n2 = q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w;
            if (0.01..=1.0).contains(&n2) {
                return q.normalize();
            }
        }
    }
}

/// Spawns the floor and the scene's dynamic boxes; returns the dynamic count.
fn spawn_scene(world: &mut EcsMaster, spec: &SceneSpec) -> usize {
    let friction = if spec.kind == SceneKind::Jolt { JOLT_FRICTION } else { REST_FRICTION };
    spawn_box(world, at_rest(Vec3::new(0.0, -1.0, 0.0)), friction, false);
    let mut boxes = 0usize;
    match spec.kind {
        SceneKind::Jolt | SceneKind::Rest => {
            let gap = if spec.kind == SceneKind::Jolt { JOLT_GAP } else { 0.0 };
            let h = i32::try_from(spec.size).expect("construction: a pyramid height fits i32");
            // Jolt's placement loop, index for index (the runner's, with the height a parameter).
            for i in 0..h {
                let lo = i / 2;
                let hi = h - (i + 1) / 2;
                for j in lo..hi {
                    for k in lo..hi {
                        let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                        let position = Vec3::new(
                            -(h as f32) + BOX_SIZE * j as f32 + odd,
                            1.0 + (BOX_SIZE + gap) * i as f32,
                            -(h as f32) + BOX_SIZE * k as f32 + odd,
                        );
                        spawn_box(world, at_rest(position), friction, true);
                        boxes += 1;
                    }
                }
            }
        }
        SceneKind::Rain => {
            let mut rng = Rng(RAIN_SEED);
            let side = spec.size as f32;
            for layer in 0..spec.size {
                for i in 0..spec.size {
                    for k in 0..spec.size {
                        let position = Vec3::new(
                            RAIN_PITCH * (i as f32 - 0.5 * side),
                            RAIN_Y0 + RAIN_PITCH * layer as f32,
                            RAIN_PITCH * (k as f32 - 0.5 * side),
                        );
                        let body = RigidBody {
                            position,
                            linear_velocity: Vec3::new(
                                rng.signed_unit(),
                                -5.0 + rng.signed_unit(),
                                rng.signed_unit(),
                            ),
                            rotation: rng.rotation(),
                            angular_velocity: Vec3::new(
                                3.0 * rng.signed_unit(),
                                3.0 * rng.signed_unit(),
                                3.0 * rng.signed_unit(),
                            ),
                        };
                        spawn_box(world, body, friction, true);
                        boxes += 1;
                    }
                }
            }
        }
    }
    boxes
}

/// What the narrowphase read on the snapshot step.
struct Snapshot {
    bodies: Vec<BodyState>,
    pairs: Vec<(BodyIndex, BodyIndex)>,
    /// The hysteresis table's axis per pair after the step's narrowphase.
    hints: Vec<Option<usize>>,
    /// The step's solver manifold count.
    manifolds: usize,
    /// The step's pairs its carried separating axis rejected (L9 C2).
    sep_axis_hits: usize,
}

/// The probe's capture: filled by [`snapshot_probe`] on an armed step.
#[derive(Resource, Default)]
struct Probe {
    armed: bool,
    bodies: Vec<BodyState>,
    pairs: Vec<(BodyIndex, BodyIndex)>,
    hints: Vec<Option<usize>>,
    manifolds: usize,
    sep_axis_hits: usize,
}

/// Runs after the narrowphase and before the constraint graph, so on an armed step it copies
/// exactly the bodies and pairs the narrowphase read: after the step the solve has advanced the
/// gathered bodies in place, so `SolverScratch::bodies` no longer holds them.
//
// `clippy::needless_pass_by_value`: `Res<_>` / `ResMut<_>` are by-value `SystemParam`s by protocol.
#[allow(clippy::needless_pass_by_value)]
fn snapshot_probe(
    scratch: Res<SolverScratch>,
    pairs: Res<ContactPairs>,
    manifolds: Res<Manifolds>,
    mut probe: ResMut<Probe>,
) {
    if !probe.armed {
        return;
    }
    probe.bodies = scratch.bodies().to_vec();
    probe.pairs = pairs.pairs().to_vec();
    probe.hints = pairs.pairs().iter().map(|&(a, b)| manifolds.box_axis_cache.get(a, b)).collect();
    probe.manifolds = manifolds.manifolds().len();
    probe.sep_axis_hits = manifolds.separated_axis_hits();
}

/// Steps the scene on the colored schedule (one worker, sleeping off, AllPairs) and snapshots
/// what the narrowphase read on the last step.
fn snapshot(spec: &SceneSpec) -> Snapshot {
    let mut world = EcsMaster::new();
    let boxes = spawn_scene(&mut world, spec);
    let expected = match spec.kind {
        SceneKind::Jolt | SceneKind::Rest => (1..=spec.size).map(|n| n * n).sum::<usize>(),
        SceneKind::Rain => spec.size * spec.size * spec.size,
    };
    assert_eq!(boxes, expected, "construction: {} holds {expected} dynamic boxes", spec.name);
    if spec.kind != SceneKind::Rain && spec.size == PYRAMID_HEIGHT as usize {
        assert_eq!(boxes, 1240, "construction: Jolt's pyramid holds exactly 1240 boxes");
    }
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
    let keys = add_physics_colored_solve(&mut builder, &mut world);
    world.insert_resource(Probe::default());
    let probe = builder.add_system(snapshot_probe);
    // `PhysicsStageKeys` carries each stage's `SystemKey` inner index; the type is not nameable
    // here but its field is public, so the stage keys are copies of the probe's own key with that
    // index written in (the parity runner's canary does the same).
    let mut after = probe.key();
    after.0 = keys.narrowphase;
    let mut before = probe.key();
    before.0 = keys
        .build_graph
        .expect("invariant: the colored pipeline registers build_graph");
    probe.after(after).before(before);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
        cfg.dt = DT;
        cfg.sleeping = false;
        cfg.broadphase_select = BroadphaseSelectMode::Manual;
        cfg.broadphase = BroadphaseKind::AllPairs;
    }
    let mut physics = builder.build(&mut world);
    assert!(spec.steps > 0, "construction: a snapshot needs at least one step");
    for step in 0..spec.steps {
        world.resource_mut::<Probe>().armed = step + 1 == spec.steps;
        physics.run(&mut world);
    }
    let probe = std::mem::take(world.resource_mut::<Probe>());
    assert_eq!(probe.bodies.len(), boxes + 1, "construction: the probe captured every row");
    assert!(!probe.pairs.is_empty(), "{}: the probe captured no pair (void)", spec.name);
    Snapshot {
        bodies: probe.bodies,
        pairs: probe.pairs,
        hints: probe.hints,
        manifolds: probe.manifolds,
        sep_axis_hits: probe.sep_axis_hits,
    }
}

// ── Classification ───────────────────────────────────────────────────────────

/// A pair's class (module docs, "Classes").
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    NonBox,
    FastSeparated,
    FastTouching,
    SlowSeparated,
    SlowTouching,
}

/// D8's fast predicate (module docs, "Classes").
fn is_fast(ba: &BodyState, bb: &BodyState) -> bool {
    let (ra, rb) = (body_bounding_radius(ba), body_bounding_radius(bb));
    // F is the body with the larger radius, A on a bitwise tie.
    let (f, s, rs) = if rb > ra { (bb, ba, ra) } else { (ba, bb, rb) };
    let tau_eff = TAU.min(0.05 * ra.min(rb));
    let dv = s.linear_velocity - f.linear_velocity;
    let dc = s.position - f.position;
    let dw = s.angular_velocity - f.angular_velocity;
    let reach = rs + tau_eff;
    let motion = dv.length_squared()
        + f.angular_velocity.length_squared() * dc.length_squared()
        + dw.length_squared() * reach * reach;
    3.0 * DT * DT * motion > 0.25 * tau_eff * tau_eff
}

/// The world axes of a box body: the columns of `Mat3::from_quat(rotation)`.
fn world_axes(rotation: Quat) -> [Vec3; 3] {
    let r = Mat3::from_quat(rotation);
    [
        Vec3::new(r.rows[0].x, r.rows[1].x, r.rows[2].x),
        Vec3::new(r.rows[0].y, r.rows[1].y, r.rows[2].y),
        Vec3::new(r.rows[0].z, r.rows[1].z, r.rows[2].z),
    ]
}

/// The bench's own SAT replica: the first axis, in the narrowphase's canonical order (A faces
/// 0..3, B faces 3..6, edges 6..15 a-major), on which the two boxes' projections do not overlap,
/// or `None` when every non-degenerate axis overlaps. The arithmetic is the narrowphase's
/// (`eval_axis`), so a disagreement with [`box_box_contact`] is a bench defect.
fn first_separating_axis(
    ca: Vec3,
    qa: Quat,
    ha: Vec3,
    cb: Vec3,
    qb: Quat,
    hb: Vec3,
) -> Option<usize> {
    let (aa, ab) = (world_axes(qa), world_axes(qb));
    let (ha, hb) = ([ha.x, ha.y, ha.z], [hb.x, hb.y, hb.z]);
    let radius = |axes: &[Vec3; 3], half: &[f32; 3], axis: Vec3| {
        half[0] * axis.dot(axes[0]).abs()
            + half[1] * axis.dot(axes[1]).abs()
            + half[2] * axis.dot(axes[2]).abs()
    };
    let negative = |raw: Vec3| {
        let len_sq = raw.length_squared();
        if len_sq < 1.0e-8 {
            return false;
        }
        let axis = raw * len_sq.sqrt().recip();
        let separation = (cb - ca).dot(axis);
        radius(&aa, &ha, axis) + radius(&ab, &hb, axis) - separation.abs() < 0.0
    };
    let mut index = 0usize;
    for axis in aa.iter().chain(ab.iter()) {
        if negative(*axis) {
            return Some(index);
        }
        index += 1;
    }
    for ea in &aa {
        for eb in &ab {
            if negative(ea.cross(*eb)) {
                return Some(index);
            }
            index += 1;
        }
    }
    None
}

/// A box body's shape, or `None`.
fn half_extents(body: &BodyState) -> Option<Vec3> {
    match body.shape {
        ColliderShape::Box { half_extents } => Some(half_extents),
        ColliderShape::Sphere { .. } => None,
    }
}

/// The per-scene counts (untimed; results).
#[derive(Default)]
struct Counts {
    pairs: usize,
    non_box: usize,
    fast_separated: usize,
    fast_touching: usize,
    slow_separated: usize,
    slow_touching: usize,
    /// First separating axis of every separated pair: A face, B face, edge.
    first_axis: [usize; 3],
    /// Σ (first separating axis index + 1) over separated pairs: the axes an early exit evaluates.
    early_exit_axes: usize,
}

impl Counts {
    fn separated(&self) -> usize {
        self.fast_separated + self.slow_separated
    }

    fn touching(&self) -> usize {
        self.fast_touching + self.slow_touching
    }

    fn fast(&self) -> usize {
        self.fast_separated + self.fast_touching
    }
}

/// Classifies every pair of the snapshot and checks the replica against the narrowphase.
fn classify(name: &str, snap: &Snapshot) -> (Vec<Class>, Counts) {
    let mut classes = Vec::with_capacity(snap.pairs.len());
    let mut counts = Counts { pairs: snap.pairs.len(), ..Counts::default() };
    for (k, &(a, b)) in snap.pairs.iter().enumerate() {
        let ba = &snap.bodies[a.0 as usize];
        let bb = &snap.bodies[b.0 as usize];
        let (Some(ha), Some(hb)) = (half_extents(ba), half_extents(bb)) else {
            counts.non_box += 1;
            classes.push(Class::NonBox);
            continue;
        };
        let contact = box_box_contact(
            a,
            b,
            ba.position,
            ba.rotation,
            ha,
            bb.position,
            bb.rotation,
            hb,
            snap.hints[k],
        );
        let replica = first_separating_axis(ba.position, ba.rotation, ha, bb.position, bb.rotation, hb);
        assert_eq!(
            contact.is_none(),
            replica.is_some(),
            "{name}: pair {k} ({}, {}): the replica SAT disagrees with box_box_contact (contact {}, \
             first separating axis {replica:?})",
            a.0,
            b.0,
            contact.is_some()
        );
        let fast = is_fast(ba, bb);
        let class = match (fast, replica) {
            (true, Some(_)) => Class::FastSeparated,
            (true, None) => Class::FastTouching,
            (false, Some(_)) => Class::SlowSeparated,
            (false, None) => Class::SlowTouching,
        };
        match class {
            Class::FastSeparated => counts.fast_separated += 1,
            Class::FastTouching => counts.fast_touching += 1,
            Class::SlowSeparated => counts.slow_separated += 1,
            Class::SlowTouching => counts.slow_touching += 1,
            Class::NonBox => unreachable!("invariant: box pairs only reach here"),
        }
        if let Some(axis) = replica {
            counts.first_axis[match axis {
                0..3 => 0,
                3..6 => 1,
                _ => 2,
            }] += 1;
            counts.early_exit_axes += axis + 1;
        }
        classes.push(class);
    }
    assert_eq!(
        counts.non_box + counts.separated() + counts.touching(),
        counts.pairs,
        "{name}: closure: the classes must sum to the pair count"
    );
    assert_eq!(
        counts.touching(),
        snap.manifolds,
        "{name}: the touching pairs must be the step's solver manifolds (a box pair's manifold \
         exists by its two poses alone)"
    );
    (classes, counts)
}

// ── Timing ───────────────────────────────────────────────────────────────────

/// Collides `pairs[idx]` in order through [`box_box_contact`] and returns the elapsed time.
fn collide_all(snap: &Snapshot, idx: &[u32]) -> Duration {
    let start = Instant::now();
    for &k in idx {
        let k = k as usize;
        let (a, b) = snap.pairs[k];
        let ba = &snap.bodies[a.0 as usize];
        let bb = &snap.bodies[b.0 as usize];
        if let (Some(ha), Some(hb)) = (half_extents(ba), half_extents(bb)) {
            black_box(box_box_contact(
                a,
                b,
                ba.position,
                ba.rotation,
                ha,
                bb.position,
                bb.rotation,
                hb,
                black_box(snap.hints[k]),
            ));
        }
    }
    start.elapsed()
}

/// The median over `reps` repetitions of ns per pair over `idx`.
fn ns_per_pair(snap: &Snapshot, idx: &[u32], reps: usize) -> f64 {
    let mut samples: Vec<f64> =
        (0..reps).map(|_| collide_all(snap, idx).as_nanos() as f64 / idx.len() as f64).collect();
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

/// Pair indices of the classes in `want`, in stream order.
fn indices(classes: &[Class], want: &[Class]) -> Vec<u32> {
    classes
        .iter()
        .enumerate()
        .filter(|(_, c)| want.contains(c))
        .map(|(k, _)| u32::try_from(k).expect("construction: a pair index fits u32"))
        .collect()
}

// ── Driver ───────────────────────────────────────────────────────────────────

/// One scene's report line material.
struct SceneReport {
    name: &'static str,
    counts: Counts,
    /// The step's pairs its carried separating axis rejected (L9 C2).
    sep_axis_hits: usize,
    timings: Vec<(&'static str, usize, f64)>,
}

/// Snapshots, classifies, asserts and times one scene.
fn run_scene(spec: &SceneSpec, reps: usize, full: bool) -> SceneReport {
    let snap = snapshot(spec);
    let (classes, counts) = classify(spec.name, &snap);
    let timed: &[(&'static str, &[Class])] = match spec.kind {
        SceneKind::Jolt | SceneKind::Rest => &[
            ("separated", &[Class::SlowSeparated]),
            ("touching", &[Class::SlowTouching]),
        ],
        SceneKind::Rain => &[("fast", &[Class::FastSeparated, Class::FastTouching])],
    };
    if spec.kind == SceneKind::Rain {
        assert!(counts.fast() > 0, "{}: a moving scene must have fast pairs", spec.name);
    }
    assert!(
        snap.sep_axis_hits <= counts.separated(),
        "{}: {} pairs rejected by a carried separating axis, but only {} are separated",
        spec.name,
        snap.sep_axis_hits,
        counts.separated()
    );
    if spec.kind != SceneKind::Rain {
        assert!(
            snap.sep_axis_hits > 0,
            "{}: no carried separating axis held on a pile (the carry is not live)",
            spec.name
        );
    }
    if full && spec.kind == SceneKind::Jolt {
        for (what, got, want) in [
            ("separated", counts.separated(), DESIGN_J_SEPARATED),
            ("touching", counts.touching(), DESIGN_J_TOUCHING),
        ] {
            let off = (got as f64 - want as f64).abs() / want as f64;
            assert!(
                off <= DESIGN_COUNT_BAND,
                "{}: {what} pairs {got} are {:.1} % off the design's {want} (band {:.0} %): the \
                 snapshot is not the scene the design priced",
                spec.name,
                100.0 * off,
                100.0 * DESIGN_COUNT_BAND
            );
        }
    }
    let mut timings = Vec::new();
    for &(label, want) in timed {
        let idx = indices(&classes, want);
        assert!(!idx.is_empty(), "{}: timed class `{label}` is empty (void)", spec.name);
        timings.push((label, idx.len(), ns_per_pair(&snap, &idx, reps)));
    }
    let all: Vec<u32> = (0..snap.pairs.len())
        .map(|k| u32::try_from(k).expect("construction: a pair index fits u32"))
        .collect();
    assert!(!all.is_empty(), "{}: the scene has no candidate pairs (void)", spec.name);
    timings.push(("stream", all.len(), ns_per_pair(&snap, &all, reps)));
    SceneReport { name: spec.name, counts, sep_axis_hits: snap.sep_axis_hits, timings }
}

/// The design's refutation reading from J's counts and costs (module docs, "Timing"): the low
/// and high ends of Δt_np(1), in ms.
fn predicted_gain_ms(report: &SceneReport) -> Option<(f64, f64)> {
    let cost = |label: &str| report.timings.iter().find(|t| t.0 == label).map(|t| t.2);
    let (t_sep, t_touch) = (cost("separated")?, cost("touching")?);
    let (n_sep, n_touch) = (report.counts.slow_separated as f64, report.counts.slow_touching as f64);
    let low = n_sep * (t_sep - SEP_AFTER_NS.1) + HIT_RATE.0 * n_touch * (t_touch - HIT_NS.1);
    let high = n_sep * (t_sep - SEP_AFTER_NS.0) + HIT_RATE.1 * n_touch * (t_touch - HIT_NS.0);
    Some((low * 1e-6 - CARRY_MS.1, high * 1e-6 - CARRY_MS.0))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        return ExitCode::SUCCESS;
    }
    let full = args.iter().any(|a| a == "--bench");
    let (specs, reps) = if full {
        (
            vec![
                SceneSpec {
                    name: "jolt",
                    kind: SceneKind::Jolt,
                    size: PYRAMID_HEIGHT as usize,
                    steps: PYRAMID_SNAPSHOT_STEPS,
                },
                SceneSpec {
                    name: "rest",
                    kind: SceneKind::Rest,
                    size: PYRAMID_HEIGHT as usize,
                    steps: PYRAMID_SNAPSHOT_STEPS,
                },
                SceneSpec {
                    name: "rain",
                    kind: SceneKind::Rain,
                    size: RAIN_SIDE,
                    steps: RAIN_SNAPSHOT_STEPS,
                },
            ],
            BENCH_REPS,
        )
    } else {
        println!(
            "narrowphase_classes: no --bench, so this is the self-check (height-4 pyramids at step \
             120, a 3x3x3 rain at step 30, one untimed repetition)"
        );
        (
            vec![
                SceneSpec { name: "jolt-h4", kind: SceneKind::Jolt, size: 4, steps: 120 },
                SceneSpec { name: "rest-h4", kind: SceneKind::Rest, size: 4, steps: 120 },
                SceneSpec { name: "rain-3", kind: SceneKind::Rain, size: 3, steps: 30 },
            ],
            1,
        )
    };

    let mut json = String::from("{\"mode\":\"");
    json.push_str(if full { "bench" } else { "self-check" });
    json.push_str("\",\"scenes\":[");
    for (i, spec) in specs.iter().enumerate() {
        let r = run_scene(spec, reps, full);
        let c = &r.counts;
        let separated = c.separated().max(1) as f64;
        println!(
            "{}: pairs {} non_box {} separated {} (slow {} fast {}) touching {} (slow {} fast {}); \
             first separating axis A-face {} B-face {} edge {}; axes an early exit evaluates {:.2} \
             of 15 per separated pair",
            r.name,
            c.pairs,
            c.non_box,
            c.separated(),
            c.slow_separated,
            c.fast_separated,
            c.touching(),
            c.slow_touching,
            c.fast_touching,
            c.first_axis[0],
            c.first_axis[1],
            c.first_axis[2],
            c.early_exit_axes as f64 / separated
        );
        println!(
            "{}: the carried separating axis rejected {} of the {} separated pairs without the SAT \
             (L9 C2)",
            r.name,
            r.sep_axis_hits,
            c.separated()
        );
        for &(label, n, ns) in &r.timings {
            println!(
                "{}: {label} {n} pairs {ns:.1} ns/pair (NOT A RESULT: timed on a shared machine; \
                 read in a quiet window)",
                r.name
            );
        }
        if let Some((low, high)) = predicted_gain_ms(&r) {
            println!(
                "{}: design refutation reading dt_np(1) in [{low:.3}, {high:.3}] ms (NOT A RESULT \
                 off a quiet window; the design stops below 1.0 ms)",
                r.name
            );
        }
        if i > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            "{{\"scene\":\"{}\",\"pairs\":{},\"non_box\":{},\"slow_separated\":{},\
             \"fast_separated\":{},\"slow_touching\":{},\"fast_touching\":{},\
             \"first_axis\":[{},{},{}],\"early_exit_axes\":{},\"sep_axis_hits\":{},\
             \"timings_not_a_result\":{{",
            r.name,
            c.pairs,
            c.non_box,
            c.slow_separated,
            c.fast_separated,
            c.slow_touching,
            c.fast_touching,
            c.first_axis[0],
            c.first_axis[1],
            c.first_axis[2],
            c.early_exit_axes,
            r.sep_axis_hits
        ));
        for (j, &(label, n, ns)) in r.timings.iter().enumerate() {
            if j > 0 {
                json.push(',');
            }
            json.push_str(&format!("\"{label}\":{{\"pairs\":{n},\"ns_per_pair\":{ns:.2}}}"));
        }
        json.push_str("}}");
    }
    json.push_str("]}");
    println!("SUMMARY {json}");
    ExitCode::SUCCESS
}
