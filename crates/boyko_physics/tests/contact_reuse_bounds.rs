//! G-L9b-7 (L9 C4, `docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md`, "Gates"
//! and "Correctness bounds at C4"): the two bounds contact reuse claims on the shipped path, each
//! on a scripted pose sequence through the default pipeline, which has contact reuse on since L9
//! C4 (asserted, not forced), at the default reuse distance τ = 1 mm.
//!
//! Before every step the test writes the moving box's pose and zeroes both of its velocities, so
//! the fast predicate (D8) classes the pair slow and the narrowphase answers it from its record
//! whenever the criterion (D4) keeps it. After the step it reads the pair's manifold and the pair's
//! class (`Manifolds::pair_classes`), and checks the manifold against `f64` geometry of the
//! scripted pose. Gravity is off; the solve still pushes the box during the step, and the next
//! step's scripted pose overwrites that.
//!
//! * **Tipping.** A cube of half-extent 0.25 m on a static slab (τ_eff = τ, since 5 % of the
//!   cube's half-extent is 12.5 mm) turns about its own centre, [`TIP_STEP`] per step about the z
//!   axis, from [`TIP_FROM`] through flat for [`TIP_STEPS`] poses. Its centre never moves, so the
//!   criterion's `|Δd|²` is exactly 0 and only its rotation term can force a miss. The bound: on
//!   every step, the deepest exact corner penetration exceeds the manifold's deepest penetration
//!   by at most `2·τ_eff + 1e-5` m (the design's "a slowly tipping box's new corner penetrates ≤
//!   2τ_eff", in the form of G-L9b-1's check (b)). Its witnesses: the pair both reuses and
//!   rebuilds its record; some reused step has a penetrating corner that no emitted point stands
//!   for — a corner the record does not hold; and on some reused step such a corner is the
//!   deepest, deeper than every held point by more than the `f32` slack, so the bound's left side
//!   was positive and not only noise.
//! * **Sliding.** The cube rests face-down 0.3 mm deep, straddling the +x edge of a static
//!   support, and moves [`SLIDE_STEP`] per step in +x across that edge. Its rotation never
//!   changes, so only the criterion's `|Δd|²` term can force a miss. The bound: every emitted
//!   point lies on both bodies' touching faces grown by `τ_eff + 1e-5` m (the design's "a slowly
//!   sliding box overhangs ≤ τ_eff + 1e-5"): the support's anchor inside the support's top face,
//!   the cube's anchor inside the cube's bottom face. A full collision clips its points to the
//!   overlap of the two faces, so any overhang past either face is a reused point carried by the
//!   motion since its record. Its witness: some reused step overhangs by at least a quarter of
//!   τ_eff.
//!
//! **Mutations** (each a scratch edit of `reuse::criterion`, reverted by copy; msvc debug,
//! 2026-09-24). M-b1, the criterion without its rotation term, keeps the tipping cube's first
//! record for the whole turn, so the corners it did not hold come down unseen: the tipping arm is
//! red at pose 42 (an unheld corner 2.034 mm deep, no point emitted, against 2.01 mm), and the
//! sliding arm stays green. Dropping the `|Δd|²` term keeps the sliding cube's record while the
//! cube slides on: the sliding arm is red at step 6 (a point 1.2 mm past the support's edge,
//! against 1.01 mm), and the tipping arm stays green. On the shipped criterion the tipping arm's
//! largest excess is 0.06 mm (pose 24, 0.12 mrad past flat, on a reused step) and the sliding
//! arm's largest overhang 0.6 mm (three 0.2 mm steps after a record).
//!
//! Both arms step two bodies for well under a second, so the test runs in every profile.

#![cfg(not(miri))]

use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::{Manifolds, PairClasses, PhysicsConfig};
use boyko_physics::solver::DefaultRigidSolver;
use boyko_physics::{ContactPoint, Manifold};

/// Fixed timestep.
const DT: f32 = 1.0 / 60.0;
/// The moving cube's half-extent. 5 % of it is 12.5 mm, so τ_eff is τ = 1 mm (D4, ruling O1).
const CUBE_HALF: f32 = 0.25;
/// The criterion's own `f32` slack, the same allowance G-L9b-1's check (b) makes.
const SLACK: f64 = 1.0e-5;

/// The tipping cube's first angle about z, in radians.
const TIP_FROM: f32 = 0.009;
/// The tipping cube's turn per step. The criterion keeps a record while
/// `8 s² (r_S + τ_eff)² ≤ τ_eff²`, which for this cube is a turn of about 1.63 mrad since the
/// record: four steps of 0.38 mrad hit, the fifth misses. So records are built at 9, 7.1, 5.2,
/// 3.3 and 1.4 mrad, and the last of them, which holds only the lower edge's two corners, is
/// still kept 0.12 mrad past flat, where the upper edge has come down past the lower one.
const TIP_STEP: f32 = 0.000_38;
/// The tipping cube's poses: from [`TIP_FROM`] to -8.86 mrad.
const TIP_STEPS: usize = 48;
/// How deep the tipping cube's bottom face sits below the slab's top when it is flat.
const TIP_FLAT_DEPTH: f32 = 0.000_3;

/// The sliding cube's slide per step. The criterion keeps a record while `2 |Δd|² ≤ τ_eff²`,
/// a slide of 0.707 mm since the record: three steps of 0.2 mm hit, the fourth misses.
const SLIDE_STEP: f32 = 0.000_2;
/// Steps the sliding cube slides for.
const SLIDE_STEPS: usize = 200;
/// The sliding cube's depth below the support's top face.
const SLIDE_DEPTH: f32 = 0.000_3;
/// The support's half-extents; its top face is at y = 0 and its +x edge at x = 1.
const SUPPORT_HALF: Vec3 = Vec3::new(1.0, 0.5, 1.0);
/// The sliding cube's first centre x: it overhangs the support's +x edge by 0.15 m.
const SLIDE_FROM_X: f32 = 0.9;

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its `size_of::<T>()` bytes
    // read-only for the duration of the borrow, which is the exact layout the component pool
    // stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A rotation of `angle` radians about the unit `axis`.
fn about(axis: Vec3, angle: f32) -> Quat {
    let (s, c) = (0.5 * angle).sin_cos();
    Quat::new(axis.x * s, axis.y * s, axis.z * s, c).normalize()
}

/// The scene of one arm: a static box, a dynamic cube, and the default pipeline stepping them.
struct Scene {
    world: EcsMaster,
    physics: Schedule,
    cube: Entity,
    /// τ_eff of the pair, in `f64` from the extents and the configured τ (D4).
    tau_eff: f64,
}

impl Scene {
    /// Builds the default pipeline over a static box of `support_half` extents centred at
    /// `support_centre` and a dynamic cube, and turns gravity off. Asserts that the pipeline
    /// has contact reuse on by default.
    fn new(support_centre: Vec3, support_half: Vec3) -> Self {
        let mut world = EcsMaster::new();
        let archetype = world.create_archetype(&[
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
        ]);
        spawn(&mut world, archetype, support_centre, support_half, false);
        let cube = spawn(
            &mut world,
            archetype,
            Vec3::new(0.0, 5.0, 0.0),
            Vec3::new(CUBE_HALF, CUBE_HALF, CUBE_HALF),
            true,
        );
        let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
        add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        let physics = builder.build(&mut world);
        let cfg = world.resource_mut::<PhysicsConfig>();
        assert!(
            cfg.contact_reuse,
            "L9 C4: the default world must have contact reuse on; its `PhysicsConfig` resource \
             has it off"
        );
        cfg.gravity = Vec3::ZERO;
        let tau = f64::from(cfg.contact_reuse_distance);
        let halves = [support_half.x, support_half.y, support_half.z, CUBE_HALF];
        let h_min = halves.into_iter().fold(f32::INFINITY, f32::min);
        let r_min = support_half.length().min(Vec3::new(CUBE_HALF, CUBE_HALF, CUBE_HALF).length());
        let tau_eff = tau.min(0.05 * f64::from(r_min)).min(0.05 * f64::from(h_min));
        Self { world, physics, cube, tau_eff }
    }

    /// Writes the cube's pose with both velocities zero, runs one step, and returns the step's
    /// pair classes and the pair's manifold, if it has one.
    fn step(&mut self, position: Vec3, rotation: Quat) -> (PairClasses, Option<Manifold>) {
        {
            let mut body = self
                .world
                .get_component_mut::<RigidBody>(self.cube)
                .expect("invariant: the cube is live");
            body.position = position;
            body.rotation = rotation;
            body.linear_velocity = Vec3::ZERO;
            body.angular_velocity = Vec3::ZERO;
        }
        self.physics.run(&mut self.world);
        let manifolds = self.world.resource::<Manifolds>();
        let classes = manifolds.pair_classes();
        assert_eq!(classes.pairs, 1, "construction: the two bodies make one candidate pair");
        assert_eq!(classes.reused + classes.full, 1, "construction: the pair is a box pair");
        let m = manifolds.manifolds();
        assert!(m.len() <= 1, "construction: one pair makes at most one manifold");
        (classes, m.get(0))
    }
}

/// Spawns a box of `half` extents at `position` into `archetype`, dynamic iff `dynamic`.
fn spawn(
    world: &mut EcsMaster,
    archetype: ArchetypeId,
    position: Vec3,
    half: Vec3,
    dynamic: bool,
) -> Entity {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: if dynamic {
            Mat3::from_diagonal(Vec3::new(24.0, 24.0, 24.0))
        } else {
            Mat3::ZERO
        },
        inv_mass: if dynamic { 1.0 } else { 0.0 },
        restitution: 0.0,
        friction: 0.5,
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
}

/// A point's two anchors as (on the lower body, on the upper body). The normal runs from body A
/// to body B, and in both arms the lower body is the static one and the normal is within a few
/// milliradians of ±y, so its sign names which anchor is whose.
fn lower_upper(m: &Manifold, p: &ContactPoint) -> (Vec3, Vec3) {
    assert!(
        m.normal.y.abs() > 0.99,
        "construction: the contact normal is vertical in these scenes, got {:?}",
        m.normal
    );
    if m.normal.y > 0.0 { (p.anchor_a, p.anchor_b) } else { (p.anchor_b, p.anchor_a) }
}

/// The deepest penetration of a manifold's points, `max(0, −min separation)`.
fn deepest(m: Option<&Manifold>) -> f64 {
    m.map_or(0.0, |m| {
        m.points[..usize::from(m.count)]
            .iter()
            .fold(0.0f64, |d, p| d.max(-f64::from(p.separation)))
    })
}

/// The eight corners of the cube centred at `centre` turned by `angle` about z, in `f64`.
fn cube_corners(centre: [f64; 3], angle: f64) -> [[f64; 3]; 8] {
    let h = f64::from(CUBE_HALF);
    let (s, c) = angle.sin_cos();
    std::array::from_fn(|k| {
        let x = if k & 1 == 1 { h } else { -h };
        let y = if k & 2 == 2 { h } else { -h };
        let z = if k & 4 == 4 { h } else { -h };
        [centre[0] + x * c - y * s, centre[1] + x * s + y * c, centre[2] + z]
    })
}

#[test]
fn a_slowly_tipping_cube_s_unseen_corner_penetrates_at_most_two_tau_eff() {
    // The slab's top face is y = 0; it is larger than the cube in every direction, so the cube's
    // corners all lie over it.
    let mut scene = Scene::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(2.0, 0.5, 2.0));
    let tau_eff = scene.tau_eff;
    assert!(
        (tau_eff - 1.0e-3).abs() < 1.0e-9,
        "construction: the tipping cube's τ_eff is the default τ, got {tau_eff}"
    );
    let centre = Vec3::new(0.0, CUBE_HALF - TIP_FLAT_DEPTH, 0.0);
    let centre64 = [0.0, f64::from(centre.y), 0.0];
    let z = Vec3::new(0.0, 0.0, 1.0);

    let (mut hits, mut rebuilds, mut unseen_witness) = (0usize, 0usize, 0usize);
    let mut worst = (f64::NEG_INFINITY, 0usize);
    let mut worst_hit = f64::NEG_INFINITY;
    let mut rows = Vec::with_capacity(TIP_STEPS);
    for k in 0..TIP_STEPS {
        let angle = TIP_FROM - TIP_STEP * k as f32;
        let (classes, m) = scene.step(centre, about(z, angle));
        let hit = classes.reused == 1;
        hits += usize::from(hit);
        rebuilds += usize::from(classes.records_built == 1);

        let corners = cube_corners(centre64, f64::from(angle));
        let depths = corners.map(|p| -p[1]);
        let exact = depths.into_iter().fold(0.0f64, f64::max);
        let emitted = deepest(m.as_ref());
        let excess = exact - emitted;
        if excess > worst.0 {
            worst = (excess, k);
        }
        if hit {
            worst_hit = worst_hit.max(excess);
        }
        rows.push((k, angle, hit, m.map_or(0, |m| m.count), exact, emitted));
        assert!(
            excess <= 2.0 * tau_eff + SLACK,
            "step {k} (angle {angle} rad, {}): the deepest exact corner penetrates {exact:e} m, \
             {excess:e} m past the manifold's deepest point {emitted:e} m, beyond 2·τ_eff \
             {:e} m + {SLACK:e}: a corner the contact does not hold went deeper than the reuse \
             bound allows ({classes:?}, {m:?})",
            if hit { "reused" } else { "full collision" },
            2.0 * tau_eff
        );

        // A penetrating corner with no emitted point within a centimetre of it horizontally is
        // one the record does not hold: both anchors of a point stand over the corner they came
        // from, give or take the penetration along the normal.
        if hit {
            let held = |p: &[f64; 3]| {
                m.is_some_and(|m| {
                    m.points[..usize::from(m.count)].iter().any(|q| {
                        let (lower, _) = lower_upper(&m, q);
                        let (dx, dz) = (f64::from(lower.x) - p[0], f64::from(lower.z) - p[2]);
                        (dx * dx + dz * dz).sqrt() < 0.01
                    })
                })
            };
            if corners.iter().zip(depths).any(|(p, d)| d > 0.0 && !held(p)) {
                unseen_witness += 1;
            }
        }
    }
    println!(
        "G-L9b-7 tipping: {TIP_STEPS} steps, {hits} reused, {rebuilds} records built, \
         {unseen_witness} reused steps with a penetrating corner the record does not hold; worst \
         excess {:e} m at step {} (on a reused step {worst_hit:e} m; bound {:e} m)",
        worst.0,
        worst.1,
        2.0 * tau_eff + SLACK
    );
    println!(
        "G-L9b-7 tipping (step, angle, reused, points, exact deepest, emitted deepest): {rows:?}"
    );
    assert!(
        hits > 0 && rebuilds > 1,
        "vacuous: the tipping pair must both reuse its record and rebuild it ({hits} reused, \
         {rebuilds} records built)"
    );
    assert!(
        unseen_witness > 0,
        "vacuous: no reused step had a penetrating corner that its record does not hold, so the \
         bound never measured a corner the reuse could miss"
    );
    assert!(
        worst_hit > SLACK,
        "vacuous: on no reused step was a corner the record does not hold the deepest one (largest \
         excess on a reused step {worst_hit:e} m), so the bound's left side was only ever noise"
    );
}

#[test]
fn a_slowly_sliding_cube_s_contact_overhangs_its_faces_by_at_most_tau_eff() {
    let support_centre = Vec3::new(0.0, -SUPPORT_HALF.y, 0.0);
    let mut scene = Scene::new(support_centre, SUPPORT_HALF);
    let tau_eff = scene.tau_eff;
    assert!(
        (tau_eff - 1.0e-3).abs() < 1.0e-9,
        "construction: the sliding cube's τ_eff is the default τ, got {tau_eff}"
    );
    let bound = tau_eff + SLACK;
    let h = f64::from(CUBE_HALF);
    let (sx, sz) = (f64::from(SUPPORT_HALF.x), f64::from(SUPPORT_HALF.z));

    let (mut hits, mut rebuilds) = (0usize, 0usize);
    let mut worst_hit = (0.0f64, 0usize);
    for k in 0..SLIDE_STEPS {
        let x = SLIDE_FROM_X + SLIDE_STEP * k as f32;
        let centre = Vec3::new(x, CUBE_HALF - SLIDE_DEPTH, 0.0);
        let (classes, m) = scene.step(centre, Quat::IDENTITY);
        let hit = classes.reused == 1;
        hits += usize::from(hit);
        rebuilds += usize::from(classes.records_built == 1);
        let m = m.unwrap_or_else(|| {
            panic!("construction: the resting cube touches the support at step {k} ({classes:?})")
        });
        let (cx, cz) = (f64::from(centre.x), f64::from(centre.z));
        for p in &m.points[..usize::from(m.count)] {
            let (lower, upper) = lower_upper(&m, p);
            // Past the support's top face, and past the cube's bottom face (the cube does not
            // turn, so its face is axis-aligned about its centre).
            let on_support = (f64::from(lower.x).abs() - sx).max(f64::from(lower.z).abs() - sz);
            let on_cube =
                ((f64::from(upper.x) - cx).abs() - h).max((f64::from(upper.z) - cz).abs() - h);
            let overhang = on_support.max(on_cube).max(0.0);
            if hit && overhang > worst_hit.0 {
                worst_hit = (overhang, k);
            }
            assert!(
                overhang <= bound,
                "step {k} (cube x {x}, {}): a contact point overhangs the touching faces by \
                 {overhang:e} m (support side {on_support:e}, cube side {on_cube:e}), beyond \
                 τ_eff + {SLACK:e} = {bound:e} m ({classes:?}, {m:?})",
                if hit { "reused" } else { "full collision" }
            );
        }
    }
    println!(
        "G-L9b-7 sliding: {SLIDE_STEPS} steps, {hits} reused, {rebuilds} records built; largest \
         overhang on a reused step {:e} m at step {} (bound {bound:e} m)",
        worst_hit.0, worst_hit.1
    );
    assert!(
        hits > 0 && rebuilds > 1,
        "vacuous: the sliding pair must both reuse its record and rebuild it ({hits} reused, \
         {rebuilds} records built)"
    );
    assert!(
        worst_hit.0 >= 0.25 * tau_eff,
        "vacuous: no reused step overhung the touching faces by a quarter of τ_eff ({:e} m at \
         most), so the bound never measured a point the reuse carried past an edge",
        worst_hit.0
    );
}
