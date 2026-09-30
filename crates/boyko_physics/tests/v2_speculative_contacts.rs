//! **V2's behaviour gates through the real schedule** (`levers/V2-speculative/01-DESIGN.md`):
//! each builds a small world with an explicit `speculative_distance`, so it pins what V2 does
//! whatever the default is.
//!
//! * **B1 (C3)** — a sphere pair whose surfaces are 10 mm apart is a candidate of every broadphase
//!   at `d = 20 mm` (each body's bounding sphere is inflated by `d / 2` at gather), and the
//!   narrowphase makes its one speculative point; at `d = 0` it is no candidate. Red before the
//!   margin: no candidate, so no manifold.
//! * **S2-S6 (C4)** — the solver's speculative branch, on the colored solver's AVX2 and scalar
//!   kernels and on the reference `SoftStepSolver`: a fast cube lands without sinking on a box
//!   floor (S2/S3) and on an SDF floor, whose surface is a sentinel lane (S3-SDF, critique W3); a
//!   cube released within `d` touches down (S4, the relaxation pass); a speculative point that
//!   does not close gets no restitution (S5); and a speculative pile keeps every bit identity
//!   (S6: W 1/2/4/8/16, scalar against AVX2, sleeping W1 against W8). Each names its red.

#![cfg(not(miri))]

use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::{add_physics_sdf, add_physics_systems};
use boyko_physics::resources::{
    BroadphaseKind, BroadphaseSelectMode, ContactPairs, Manifolds, PhysicsConfig,
};
use boyko_physics::sdf_query::SdfField;
use boyko_physics::solver::{DefaultRigidSolver, SoftStepSolver};
use boyko_sdf_math::{SdfEdit, sdf_op};

/// Fixed timestep.
const DT: f32 = 1.0 / 60.0;
/// The owner's distance (V2b).
const D: f32 = 0.02;

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its `size_of::<T>()` bytes
    // read-only for the duration of the borrow, which is the exact layout the component pool
    // stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// One body: dynamic (`inv_mass` 1, `Simulated`) or static.
fn spawn(world: &mut EcsMaster, shape: ColliderShape, position: Vec3, velocity: Vec3, restitution: f32, dynamic: bool) {
    let body = RigidBody { position, linear_velocity: velocity, rotation: Quat::IDENTITY, angular_velocity: Vec3::ZERO };
    let mass = RigidBodyMass {
        inv_inertia: if dynamic { Mat3::from_diagonal(Vec3::new(1.0, 1.0, 1.0)) } else { Mat3::ZERO },
        inv_mass: if dynamic { 1.0 } else { 0.0 },
        restitution,
        friction: 0.5,
    };
    let collider = Collider { shape, layer: 1, mask: 1 };
    let base = [RigidBody::component_id(), RigidBodyMass::component_id(), Collider::component_id()];
    let archetype = world.create_archetype(&base);
    let e = world
        .create_entity(archetype, &[(base[0], as_bytes(&body)), (base[1], as_bytes(&mass)), (base[2], as_bytes(&collider))])
        .expect("construction: the archetype accepts the three columns");
    if dynamic {
        world.enable::<Simulated>(e);
    }
}

/// The colored pipeline at one worker, with `configure` applied to its configuration.
fn world(configure: impl FnOnce(&mut PhysicsConfig)) -> (EcsMaster, ScheduleBuilder) {
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
    add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.dt = DT;
        configure(cfg);
    }
    (world, builder)
}

/// Builds the schedule once the bodies are spawned.
fn build(mut world: EcsMaster, builder: ScheduleBuilder) -> (EcsMaster, Schedule) {
    let schedule = builder.build(&mut world);
    (world, schedule)
}

/// B1 (C3): two spheres of radius 0.5 whose surfaces are 10 mm apart, no gravity, on AllPairs,
/// Grid and Tree: at `d = 20 mm` every broadphase emits the pair and the narrowphase keeps one
/// speculative point with `s = +10 mm`; at `d = 0` there is no candidate.
#[test]
fn b1_every_broadphase_pairs_two_spheres_within_d() {
    for kind in [BroadphaseKind::AllPairs, BroadphaseKind::Grid, BroadphaseKind::Tree] {
        for d in [D, 0.0] {
            let (mut w, builder) = world(|cfg| {
                cfg.gravity = Vec3::ZERO;
                cfg.broadphase_select = BroadphaseSelectMode::Manual;
                cfg.broadphase = kind;
                cfg.speculative_distance = d;
            });
            let sphere = ColliderShape::Sphere { radius: 0.5 };
            spawn(&mut w, sphere, Vec3::new(0.0, 5.0, 0.0), Vec3::ZERO, 0.0, true);
            spawn(&mut w, sphere, Vec3::new(1.01, 5.0, 0.0), Vec3::ZERO, 0.0, true);
            let (mut w, mut schedule) = build(w, builder);
            schedule.run(&mut w);
            let pairs = w.resource::<ContactPairs>().pairs().len();
            let manifolds = w.resource::<Manifolds>().solver_manifolds().to_vec();
            if d > 0.0 {
                assert_eq!(pairs, 1, "B1 {kind:?}: the pair 10 mm apart is a candidate at d = {d}");
                assert_eq!(manifolds.len(), 1, "B1 {kind:?}: one manifold at d = {d}");
                let m = &manifolds[0];
                assert!(
                    m.count == 1 && (m.points[0].separation - 0.01).abs() < 1e-4 && m.points[0].separation > 0.0,
                    "B1 {kind:?}: one speculative point, s = +10 mm: {m:?}"
                );
            } else {
                assert_eq!(pairs, 0, "B1 {kind:?}: no candidate at d = 0");
                assert!(manifolds.is_empty(), "B1 {kind:?}: no manifold at d = 0");
            }
        }
    }
}

// ── C4: the solver's speculative branch ─────────────────────────────────────────────────────

/// The three solve paths V2's rule is in: the colored solver's AVX2 cohort kernel, its scalar
/// kernel, and the reference `SoftStepSolver`.
#[derive(Clone, Copy, Debug)]
enum Kernel {
    Avx2,
    Scalar,
    Reference,
}

const KERNELS: [Kernel; 3] = [Kernel::Avx2, Kernel::Scalar, Kernel::Reference];

/// [`world`] on `kernel`'s solver (SDF-wired iff `sdf`), with `configure` applied.
fn world_on(kernel: Kernel, sdf: bool, configure: impl FnOnce(&mut PhysicsConfig)) -> (EcsMaster, ScheduleBuilder) {
    let mut world = EcsMaster::new();
    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
    match (kernel, sdf) {
        (Kernel::Reference, false) => {
            let _ = add_physics_systems::<SoftStepSolver>(&mut builder, &mut world);
        }
        (Kernel::Reference, true) => {
            let _ = add_physics_sdf::<SoftStepSolver>(&mut builder, &mut world);
        }
        (_, false) => {
            let _ = add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
        }
        (_, true) => {
            let _ = add_physics_sdf::<DefaultRigidSolver>(&mut builder, &mut world);
        }
    }
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        cfg.dt = DT;
        cfg.simd_solve = matches!(kernel, Kernel::Avx2);
        configure(cfg);
    }
    (world, builder)
}

/// A static floor box whose top face is `y = 0`.
fn floor(world: &mut EcsMaster, restitution: f32) {
    let shape = ColliderShape::Box { half_extents: Vec3::new(10.0, 1.0, 10.0) };
    spawn(world, shape, Vec3::new(0.0, -1.0, 0.0), Vec3::ZERO, restitution, false);
}

/// An SDF floor whose surface is `y = 0` (a 20 m box below it).
fn sdf_floor(world: &mut EcsMaster) {
    world
        .resource_mut::<SdfField>()
        .push(SdfEdit::box_shape([0.0, -20.0, 0.0], [20.0, 20.0, 20.0], sdf_op::UNION, 0.0));
}

/// The lowest `y` of the unit cube (half-extent 0.5) at `body`'s pose: its bottom's separation
/// from the floor's top face.
fn cube_bottom(body: &RigidBody) -> f32 {
    let mut low = f32::INFINITY;
    for i in 0..8u32 {
        let s = |k: u32| if (i >> k) & 1 == 1 { 0.5 } else { -0.5 };
        low = low.min((body.position + body.rotation.rotate(Vec3::new(s(0), s(1), s(2)))).y);
    }
    low
}

/// The one dynamic body (the only one above the floor's centre).
fn only_dynamic(world: &mut EcsMaster) -> RigidBody {
    let q = world.query::<&RigidBody, ()>();
    let dynamic: Vec<RigidBody> = q.iter().copied().filter(|b| b.position.y > -0.5).collect();
    assert_eq!(dynamic.len(), 1, "construction: one dynamic body above the floor");
    dynamic[0]
}

/// A unit cube 15 mm above the floor falling at 10 m/s (it travels 167 mm in one step), one step,
/// on the given floor kind and kernel: its bottom's separation after the step.
fn fast_landing(kernel: Kernel, d: f32, sdf: bool) -> f32 {
    let (mut w, builder) = world_on(kernel, sdf, |cfg| cfg.speculative_distance = d);
    if sdf {
        sdf_floor(&mut w);
    } else {
        floor(&mut w, 0.0);
    }
    let cube = ColliderShape::Box { half_extents: Vec3::new(0.5, 0.5, 0.5) };
    spawn(&mut w, cube, Vec3::new(0.0, 0.515, 0.0), Vec3::new(0.0, -10.0, 0.0), 0.0, true);
    let (mut w, mut schedule) = build(w, builder);
    schedule.run(&mut w);
    cube_bottom(&only_dynamic(&mut w))
}

/// S2 + S3 (C4): a cube 15 mm above the floor at 10 m/s lands in one step on the face: at
/// `d = 20 mm` the speculative points close the gap within the first substep and hold the cube at
/// the face (within 1 mm either way), on every kernel. At `d = 0` it sinks by about
/// `v·dt − 15 mm` (the pre-V2 narrowphase makes no point). Red on the pre-V2 solver at
/// `d = 20 mm`: its soft rule on a positive separation is a ghost contact that stops the cube
/// 11.5 mm above the face (measured at C4 on the C3 solver).
///
/// S3 is the same gate read against the current separation: a kernel that solves the speculative
/// points on their gather-time separation (`s_cur = s0`) lets the cube close 15 mm in every
/// substep, three more than there is room for, and it ends up to 45 mm deep.
#[test]
fn s2_s3_a_fast_cube_lands_without_sinking_within_d() {
    for kernel in KERNELS {
        let at_d = fast_landing(kernel, D, false);
        let at_0 = fast_landing(kernel, 0.0, false);
        eprintln!("S2 ({kernel:?}): bottom {at_d} m at d = {D}, {at_0} m at d = 0");
        assert!(at_d.abs() <= 1.0e-3, "S2/S3 ({kernel:?}): the cube is {at_d} m off the face at d = {D}, over 1 mm");
        assert!(at_0 <= -0.1, "S2 ({kernel:?}): the pre-V2 rule sinks the cube ({at_0} m)");
    }
}

/// S3-SDF (C4, critique W3): the same landing on an SDF floor, whose contacts put the immovable
/// surface on a sentinel lane (`body_b == body_a` in the tables). The sentinel's movement is
/// `(0, IDENTITY)`, never body A's: a kernel that read `Δ[ib]` there would see the cube's own
/// movement on both sides, never close the gap, and let it sink up to 45 mm. Red on the pre-V2
/// solver as S2 is (12.8 mm above the field).
#[test]
fn s3_sdf_a_fast_cube_lands_on_the_field_without_sinking() {
    for kernel in KERNELS {
        let at_d = fast_landing(kernel, D, true);
        let at_0 = fast_landing(kernel, 0.0, true);
        eprintln!("S3-SDF ({kernel:?}): bottom {at_d} m at d = {D}, {at_0} m at d = 0");
        assert!(at_d.abs() <= 1.0e-3, "S3-SDF ({kernel:?}): the cube is {at_d} m off the field at d = {D}, over 1 mm");
        assert!(at_0 <= -0.1, "S3-SDF ({kernel:?}): the pre-V2 rule sinks the cube ({at_0} m)");
    }
}

/// S4 (C4): a cube released at rest 15 mm above the floor touches down within 10 steps at
/// `d = 20 mm` (free fall takes about 3.3). Red under "the relaxation pass solves a speculative
/// lane as a rigid contact" (`-m·vn` there, F0d's NS20 ghost rule): each relax pass cancels the
/// approach the biased pass allowed, so the cube sinks by only `g·h²` a substep (0.7 mm a step)
/// and hovers.
#[test]
fn s4_a_cube_released_within_d_touches_down() {
    for kernel in KERNELS {
        let (mut w, builder) = world_on(kernel, false, |cfg| cfg.speculative_distance = D);
        floor(&mut w, 0.0);
        let cube = ColliderShape::Box { half_extents: Vec3::new(0.5, 0.5, 0.5) };
        spawn(&mut w, cube, Vec3::new(0.0, 0.515, 0.0), Vec3::ZERO, 0.0, true);
        let (mut w, mut schedule) = build(w, builder);
        let mut touched = None;
        for step in 0..10 {
            schedule.run(&mut w);
            if cube_bottom(&only_dynamic(&mut w)) <= 1.0e-3 {
                touched = Some(step);
                break;
            }
        }
        let bottom = cube_bottom(&only_dynamic(&mut w));
        assert!(touched.is_some(), "S4 ({kernel:?}): the cube still hovers {bottom} m above the floor after 10 steps");
    }
}

/// S5 (C4): restitution waits for the touch. A sphere with `e = 0.8`, 30 mm above the floor and
/// approaching at 1.2 m/s with no gravity (20 mm a step), at `d = 50 mm`: its point is speculative
/// and the gap does not close this step, so no impulse may act — it must still be falling, at its
/// own speed, bit for bit. Without the guard the restitution pass reads the approach (above the
/// 1 m/s threshold) and bounces it before any contact.
#[test]
fn s5_restitution_waits_for_the_touch() {
    for kernel in KERNELS {
        let (mut w, builder) = world_on(kernel, false, |cfg| {
            cfg.gravity = Vec3::ZERO;
            cfg.speculative_distance = 0.05;
        });
        floor(&mut w, 0.8);
        spawn(&mut w, ColliderShape::Sphere { radius: 0.5 }, Vec3::new(0.0, 0.53, 0.0), Vec3::new(0.0, -1.2, 0.0), 0.8, true);
        let (mut w, mut schedule) = build(w, builder);
        schedule.run(&mut w);
        let manifolds = w.resource::<Manifolds>().solver_manifolds().len();
        let b = only_dynamic(&mut w);
        assert_eq!(manifolds, 1, "S5 ({kernel:?}): anti-vacuity: the speculative contact exists");
        assert!(
            b.linear_velocity.y.to_bits() == (-1.2f32).to_bits(),
            "S5 ({kernel:?}): no impulse before the touch; velocity {:?}, height {}",
            b.linear_velocity,
            b.position.y
        );
    }
}

/// S6 (C4): the speculative rule keeps the engine's bit identities — worker counts 1, 2, 4, 8 and
/// 16 with the parallel solve and narrowphase on, the scalar kernel against the AVX2 one, and
/// sleeping on at 1 and 8 workers, on a small Jolt pyramid at `d = 20 mm`, every frame's poses.
#[test]
fn s6_a_speculative_pile_keeps_every_bit_identity() {
    const HEIGHT: i32 = if cfg!(debug_assertions) { 6 } else { 9 };
    const FRAMES: usize = 90;
    let run = |workers: usize, simd_solve: bool, sleeping: bool| -> (Vec<u64>, usize) {
        let mut w = EcsMaster::new();
        let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(workers).build());
        add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut w);
        w.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        {
            let cfg = w.resource_mut::<PhysicsConfig>();
            cfg.dt = DT;
            cfg.speculative_distance = D;
            cfg.simd_solve = simd_solve;
            cfg.sleeping = sleeping;
            cfg.parallel_solve = true;
            cfg.parallel_narrowphase = true;
        }
        let floor_shape = ColliderShape::Box { half_extents: Vec3::new(50.0, 1.0, 50.0) };
        spawn(&mut w, floor_shape, Vec3::new(0.0, -1.0, 0.0), Vec3::ZERO, 0.0, false);
        let cube = ColliderShape::Box { half_extents: Vec3::new(1.0, 1.0, 1.0) };
        for i in 0..HEIGHT {
            let (lo, hi) = (i / 2, HEIGHT - (i + 1) / 2);
            for j in lo..hi {
                for k in lo..hi {
                    let odd = if i & 1 != 0 { 1.0 } else { 0.0 };
                    let p = Vec3::new(
                        -(HEIGHT as f32) + 2.0 * j as f32 + odd,
                        1.0 + 2.5 * i as f32,
                        -(HEIGHT as f32) + 2.0 * k as f32 + odd,
                    );
                    spawn(&mut w, cube, p, Vec3::ZERO, 0.0, true);
                }
            }
        }
        let (mut w, mut schedule) = build(w, builder);
        let mut hashes = Vec::with_capacity(FRAMES);
        let mut speculative = 0usize;
        for _ in 0..FRAMES {
            schedule.run(&mut w);
            speculative += w
                .resource::<Manifolds>()
                .solver_manifolds()
                .iter()
                .flat_map(|m| m.points[..usize::from(m.count)].iter())
                .filter(|p| p.separation > 0.0)
                .count();
            let q = w.query::<&RigidBody, ()>();
            let mut h = 0xcbf2_9ce4_8422_2325_u64;
            for b in q.iter() {
                for c in [
                    b.position.x, b.position.y, b.position.z, b.linear_velocity.x, b.linear_velocity.y,
                    b.linear_velocity.z, b.rotation.x, b.rotation.y, b.rotation.z, b.rotation.w,
                    b.angular_velocity.x, b.angular_velocity.y, b.angular_velocity.z,
                ] {
                    for byte in c.to_bits().to_le_bytes() {
                        h = (h ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
                    }
                }
            }
            hashes.push(h);
        }
        (hashes, speculative)
    };
    let (reference, speculative) = run(1, true, false);
    assert!(speculative > 0, "S6: anti-vacuity: the pile kept speculative points");
    let mut diverged = Vec::new();
    for (label, workers, simd_solve) in
        [("W2", 2, true), ("W4", 4, true), ("W8", 8, true), ("W16", 16, true), ("W1 scalar", 1, false), ("W8 scalar", 8, false)]
    {
        if run(workers, simd_solve, false).0 != reference {
            diverged.push(label);
        }
    }
    let (asleep_1, _) = run(1, true, true);
    let (asleep_8, _) = run(8, true, true);
    if asleep_8 != asleep_1 {
        diverged.push("sleeping W8 vs W1");
    }
    assert!(diverged.is_empty(), "S6: the speculative pile diverged from W1 on: {diverged:?}");
    println!("S6: {speculative} speculative points over {FRAMES} frames, every arm bit-identical");
}
