//! RED-FIRST: the box-box edge fallback emits edge contacts far deeper than the two boxes overlap.
//!
//! When the chosen face axis realizes no clipped patch, `box_box_contact` hands the pair to
//! `edge_fallback`, which adopts the SHALLOWEST EDGE axis whatever its depth. On a box resting at
//! a knife-edge touch on a larger box, the shallowest edge axis is a near-duplicate of the face
//! normal (`A.z × B.z` at a few mrad from it), and its depth is the larger box's overhang times
//! that angle — tens of millimetres to metres — while the face axis reads micrometres.
//! `edge_contact` then reports that depth as the separation, with witness points on edges that do
//! not touch (the segment parameter clamped, anchors metres apart).
//!
//! Found by L9's G-L9b-1 generator, seed `0x531ff99f772d936c` (a 3 mm box on a 33 m slab:
//! separation −0.0770 m, anchors 16.95 m apart). Diagnosis: the `thinbox` lane's `diagnosis.md`.
//!
//! The invariant every test here checks is the kernel's OWN documented slack: a manifold may claim
//! at most `HYSTERESIS_RATIO · d + FACE_AXIS_PREFERENCE` of penetration, `d` being the minimum SAT
//! depth over the 15 axes (an upper bound on the true penetration depth — the minimum
//! translation distance is the minimum over ALL directions). The bound is computed in `f64` from
//! the kernel's own `f32` frames (`Mat3::from_quat` columns), so FP in the oracle cannot fire it.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::manifold::{BodyIndex, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::narrowphase::box_box::box_box_contact;
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::solver::SoftStepSolver;

/// Mirror of the kernel's private `HYSTERESIS_RATIO`: a held axis may be this much deeper.
const HYSTERESIS_RATIO: f64 = 1.05;
/// Mirror of the kernel's private `FACE_AXIS_PREFERENCE` (metres): a face patch may be this much
/// deeper than the edge axis it beat.
const FACE_AXIS_PREFERENCE: f64 = 0.005;
/// Oracle rounding allowance on top of the two design slacks.
const ORACLE_SLOP: f64 = 1.0e-5;

const A: BodyIndex = BodyIndex(0);
const B: BodyIndex = BodyIndex(1);

fn v3(bits: [u32; 3]) -> Vec3 {
    Vec3::new(
        f32::from_bits(bits[0]),
        f32::from_bits(bits[1]),
        f32::from_bits(bits[2]),
    )
}

fn q4(bits: [u32; 4]) -> Quat {
    Quat::new(
        f32::from_bits(bits[0]),
        f32::from_bits(bits[1]),
        f32::from_bits(bits[2]),
        f32::from_bits(bits[3]),
    )
}

/// The seed's two boxes: A the 33 m slab, B the 3 mm box.
fn seed_halves() -> (Vec3, Vec3) {
    (
        v3([0x4205_81cd, 0x3e55_47d4, 0x41c0_387b]),
        v3([0x3b87_b37a, 0x3b44_e355, 0x3c67_4a94]),
    )
}

/// The seed's poses: `(a_center, a_rotation, b_center, b_rotation)`; `0` is the record's (a
/// genuine 1-point face contact), `1` the perturbed one (the defect).
fn seed_pose(k: usize) -> (Vec3, Quat, Vec3, Quat) {
    if k == 0 {
        (
            v3([0x4098_8db8, 0xc029_0f6f, 0xbfba_9ec0]),
            q4([0x3f0c_3907, 0x3f4d_9557, 0xbe1c_421f, 0x3e36_8f8d]),
            v3([0xc017_c8a8, 0x418d_81d3, 0x4094_a028]),
            q4([0x3e68_281c, 0x3ec7_ec10, 0xbf07_edcf, 0x3f37_925c]),
        )
    } else {
        (
            v3([0x407c_4051, 0xc061_bdde, 0xbff1_0767]),
            q4([0x3f1f_c023, 0x3f3f_b98c, 0xbe3e_e285, 0x3dfa_7326]),
            v3([0x3fe1_aff9, 0x418d_694a, 0x409f_8713]),
            q4([0x3e81_4d3a, 0x3ecc_4676, 0xbf1b_de2e, 0x3f23_2f68]),
        )
    }
}

/// The world axes the kernel builds for `rotation`: the columns of `Mat3::from_quat`, as `f64`.
fn axes64(rotation: Quat) -> [[f64; 3]; 3] {
    let r = Mat3::from_quat(rotation);
    let col = |i: usize| {
        let c = [r.rows[0], r.rows[1], r.rows[2]].map(|row| [row.x, row.y, row.z][i]);
        c.map(f64::from)
    };
    [col(0), col(1), col(2)]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The minimum SAT depth over the 15 box-box axes, in `f64` on the kernel's own `f32` frames.
fn min_sat_depth(ac: Vec3, aq: Quat, ah: Vec3, bc: Vec3, bq: Quat, bh: Vec3) -> f64 {
    let (ax, bx) = (axes64(aq), axes64(bq));
    let (ah, bh) = (
        [ah.x, ah.y, ah.z].map(f64::from),
        [bh.x, bh.y, bh.z].map(f64::from),
    );
    let d = [
        f64::from(bc.x) - f64::from(ac.x),
        f64::from(bc.y) - f64::from(ac.y),
        f64::from(bc.z) - f64::from(ac.z),
    ];
    let mut best = f64::INFINITY;
    for k in 0..15 {
        let raw = if k < 3 {
            ax[k]
        } else if k < 6 {
            bx[k - 3]
        } else {
            cross(ax[(k - 6) / 3], bx[(k - 6) % 3])
        };
        let len = dot(raw, raw).sqrt();
        if len < 1.0e-9 {
            continue;
        }
        let n = raw.map(|x| x / len);
        let ra: f64 = (0..3).map(|i| ah[i] * dot(n, ax[i]).abs()).sum();
        let rb: f64 = (0..3).map(|i| bh[i] * dot(n, bx[i]).abs()).sum();
        best = best.min(ra + rb - dot(d, n).abs());
    }
    best
}

/// The deepest penetration a manifold claims, `max(0, −min separation)`.
fn claimed_depth(m: &Manifold) -> f64 {
    m.points[..usize::from(m.count)]
        .iter()
        .fold(0.0f64, |deepest, p| deepest.max(-f64::from(p.separation)))
}

/// The kernel's own slack over the oracle bound (module docs).
fn allowed_depth(bound: f64) -> f64 {
    HYSTERESIS_RATIO * bound.max(0.0) + FACE_AXIS_PREFERENCE + ORACLE_SLOP
}

/// The seed at pose 1 (hint = pose 0's axis, and cold) claims no more penetration than the boxes
/// can have; pose 0, a genuine face touch, is the control.
///
/// RED on `integ/unified @ 983480a9`: pose 1 claims 7.6959e-2 m against a bound of ~2.2e-5 m.
#[test]
fn the_seed_claims_no_depth_the_boxes_do_not_have() {
    let (ah, bh) = seed_halves();

    let (ac, aq, bc, bq) = seed_pose(0);
    let c0 = box_box_contact(A, B, ac, aq, ah, bc, bq, bh, None)
        .expect("pose 0 is a genuine 1-point face touch (the control arm)");
    let bound0 = min_sat_depth(ac, aq, ah, bc, bq, bh);
    assert!(
        claimed_depth(&c0.manifold) <= allowed_depth(bound0),
        "control: pose 0 claims {:e} against a bound {bound0:e}",
        claimed_depth(&c0.manifold)
    );

    let (ac, aq, bc, bq) = seed_pose(1);
    let bound1 = min_sat_depth(ac, aq, ah, bc, bq, bh);
    // Anti-vacuity: pose 1 is an FP-thin touch (the SAT does not separate it), so the kernel is
    // asked to decide — a pose the SAT separates would pass by returning nothing.
    assert!(
        (0.0..1.0e-4).contains(&bound1),
        "pose 1 must be an FP-thin overlapping touch, bound {bound1:e}"
    );
    for hint in [Some(c0.reference_axis), None] {
        if let Some(c1) = box_box_contact(A, B, ac, aq, ah, bc, bq, bh, hint) {
            let claim = claimed_depth(&c1.manifold);
            assert!(
                claim <= allowed_depth(bound1),
                "hint {hint:?}: axis {} claims {claim:e} m of penetration, the boxes overlap by at \
                 most {bound1:e} m (allowed {:e})",
                c1.reference_axis,
                allowed_depth(bound1)
            );
        }
    }
}

/// xorshift64*: a seeded, dependency-free generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * ((self.next() >> 40) as f32 / (1u64 << 24) as f32)
    }
}

/// The rotation by `angle` about the unit `axis`.
fn about(axis: Vec3, angle: f32) -> Quat {
    let (s, c) = (0.5 * angle).sin_cos();
    Quat::new(axis.x * s, axis.y * s, axis.z * s, c).normalize()
}

/// The authored-placement case: a J/R 0.5 m box placed on the 50×1×50 floor with its lowest corner
/// at ±2 µm (an editor snapping a tilted crate onto the ground), random yaw, tilted by `theta`.
/// Returns (poses with a manifold, poses over the bound, the worst claim over its bound).
fn authored(theta: f32, poses: u32) -> (u32, u32, f64) {
    let fh = Vec3::new(50.0, 1.0, 50.0);
    let bh = Vec3::new(0.5, 0.5, 0.5);
    let fc = Vec3::new(0.0, -1.0, 0.0);
    let fq = Quat::new(0.0, 0.0, 0.0, 1.0);
    let mut rng = Rng(0x0a07_e0de_5eed_0001 ^ u64::from(theta.to_bits()));
    let (mut touching, mut over, mut worst) = (0u32, 0u32, 0.0f64);
    for _ in 0..poses {
        let yaw = about(Vec3::new(0.0, 1.0, 0.0), rng.range(-3.2, 3.2));
        let dir = rng.range(0.0, core::f32::consts::TAU);
        let q = about(Vec3::new(dir.cos(), 0.0, dir.sin()), theta).mul(yaw);
        let ax = axes64(q);
        let ext = f64::from(bh.x) * ax[0][1].abs()
            + f64::from(bh.y) * ax[1][1].abs()
            + f64::from(bh.z) * ax[2][1].abs();
        let gap = rng.range(-2.0e-6, 2.0e-6);
        let c = Vec3::new(
            rng.range(-49.0, 49.0),
            ext as f32 + gap,
            rng.range(-49.0, 49.0),
        );
        if let Some(k) = box_box_contact(A, B, fc, fq, fh, c, q, bh, None) {
            touching += 1;
            let bound = min_sat_depth(fc, fq, fh, c, q, bh);
            let claim = claimed_depth(&k.manifold);
            if claim > allowed_depth(bound) {
                over += 1;
                worst = worst.max(claim - bound);
            }
        }
    }
    (touching, over, worst)
}

/// A tilted 0.5 m box snapped onto the 50 m floor claims no depth it does not have. The untilted
/// placement is the control (its fallback edge axes duplicate the face normal to FP).
///
/// RED on `integ/unified @ 983480a9` at θ = 1e-2 (the thinbox lane measured 380 over-claims in
/// 20 000 such poses, the worst 0.56 m).
#[test]
fn a_tilted_box_snapped_onto_a_large_floor_claims_no_phantom_depth() {
    let (touching0, over0, worst0) = authored(0.0, 2000);
    assert!(touching0 > 0, "control: no untilted placement touched");
    assert_eq!(
        over0, 0,
        "control: an untilted placement over-claimed by {worst0:e} m"
    );
    let (touching, over, worst) = authored(1.0e-2, 2000);
    assert!(
        touching > 0,
        "no tilted placement touched (the arm would pass by emptiness)"
    );
    assert_eq!(
        over, 0,
        "{over} of {touching} touching placements claim more than the boxes overlap; worst by {worst:e} m"
    );
}

/// Views a `#[repr(C)]` physics component as its bytes for the raw spawn path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T` (a `#[repr(C)]` physics component whose byte image is what
    // its pool stores), viewed read-only for `size_of::<T>()` bytes; the slice borrows `value`
    // and cannot outlive it.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn spawn(world: &mut EcsMaster, body: RigidBody, mass: RigidBodyMass, half: Vec3) {
    let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
    let collider = Collider {
        shape: ColliderShape::Box { half_extents: half },
        layer: 1,
        mask: 1,
    };
    let e = world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("invariant: RigidBodyBundle archetype accepts the three columns");
    world.enable::<Simulated>(e);
}

/// One physics step of the seed's pose 1 — the 3 mm box (1000 kg/m³) at rest on the static slab,
/// no gravity. The boxes overlap by at most ~2e-5 m, so nothing may be pushed or spun.
///
/// RED on `integ/unified @ 983480a9`: the fallback's −0.077 m contact at the far end of the box's
/// 28 mm edge spins it to 8.26 rad/s and moves it 1.76 mm along the normal in one step.
#[test]
fn a_knife_edge_touch_does_not_spin_up_a_thin_box() {
    let (ah, bh) = seed_halves();
    let (ac, aq, bc, bq) = seed_pose(1);
    let m = 8.0 * f64::from(bh.x) * f64::from(bh.y) * f64::from(bh.z) * 1000.0;
    let (x2, y2, z2) = (
        f64::from(bh.x).powi(2),
        f64::from(bh.y).powi(2),
        f64::from(bh.z).powi(2),
    );
    let inv_i = [
        3.0 / (m * (y2 + z2)),
        3.0 / (m * (x2 + z2)),
        3.0 / (m * (x2 + y2)),
    ];
    let ax = axes64(bq);
    let entry =
        |r: usize, c: usize| (0..3).map(|k| ax[k][r] * inv_i[k] * ax[k][c]).sum::<f64>() as f32;
    let inv_inertia = Mat3::from_rows(
        Vec3::new(entry(0, 0), entry(0, 1), entry(0, 2)),
        Vec3::new(entry(1, 0), entry(1, 1), entry(1, 2)),
        Vec3::new(entry(2, 0), entry(2, 1), entry(2, 2)),
    );

    let mut world = EcsMaster::new();
    let slab = RigidBody {
        position: ac,
        linear_velocity: Vec3::ZERO,
        rotation: aq,
        angular_velocity: Vec3::ZERO,
    };
    let slab_mass = RigidBodyMass {
        inv_inertia: Mat3::ZERO,
        inv_mass: 0.0,
        restitution: 0.0,
        friction: 0.5,
    };
    spawn(&mut world, slab, slab_mass, ah);
    let small = RigidBody {
        position: bc,
        linear_velocity: Vec3::ZERO,
        rotation: bq,
        angular_velocity: Vec3::ZERO,
    };
    let small_mass = RigidBodyMass {
        inv_inertia,
        inv_mass: (1.0 / m) as f32,
        restitution: 0.0,
        friction: 0.5,
    };
    spawn(&mut world, small, small_mass, bh);

    let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
    let _keys = add_physics_systems::<SoftStepSolver>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(
        1.0 / 60.0,
    )));
    let mut schedule = builder.build(&mut world);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::ZERO;
    schedule.run(&mut world);

    let q = world.query::<&RigidBody, ()>();
    let bodies: Vec<RigidBody> = q.iter().copied().collect();
    assert_eq!(bodies.len(), 2);
    let b = bodies[1];
    assert!(
        b.angular_velocity.length() < 1.0e-3 && b.linear_velocity.length() < 1.0e-3,
        "a ~2e-5 m touch spun the 3 mm box to {:e} rad/s and pushed it to {:e} m/s in one step \
         (moved {:e} m)",
        b.angular_velocity.length(),
        b.linear_velocity.length(),
        (b.position - bc).length()
    );
}
