//! The box-box edge fallback claims no depth the boxes do not have, and whether a pair has a
//! manifold never depends on the hint (the `thinbox` lane, `design_rev2.md`).
//!
//! **The defect (red-first, T1-T3).** When the chosen face axis realizes no clipped patch,
//! `box_box_contact` hands the pair to `edge_fallback`, which adopted the SHALLOWEST EDGE axis
//! whatever its depth. On a box resting at a knife-edge touch on a larger box, the shallowest edge
//! axis is a near-duplicate of the face normal (`A.z × B.z` at a few mrad from it), and its depth is
//! the larger box's overhang times that angle — tens of millimetres to metres — while the face axis
//! reads micrometres. `edge_contact` then reported that depth as the separation, with witness points
//! on edges that do not touch (the segment parameter clamped, anchors metres apart). Found by L9's
//! G-L9b-1 generator, seed `0x531ff99f772d936c` (a 3 mm box on a 33 m slab: separation −0.0770 m,
//! anchors 16.95 m apart). Diagnosis: the `thinbox` lane's `diagnosis.md`.
//!
//! **The fix.** The fallback takes an edge only while its axis depth is within the face bound
//! `max(face + min(5 mm, 0.1·h_min), 1.05·face)`, decided from the SAT's own best face and best edge
//! alone; when every edge axis claims more, the pair gets the best face's own contact — its patch,
//! else its lowest clipped vertex as one speculative point. T4 pins that existence stays a function
//! of the poses, T5 the bound on every manifold, T7 the answer itself.
//!
//! The invariant T1-T3 and T5 check is the kernel's OWN documented slack: a manifold may claim at
//! most `HYSTERESIS_RATIO · d + FACE_AXIS_PREFERENCE` of penetration, `d` being the minimum SAT
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
use boyko_physics::narrowphase::box_box::{BoxBoxContact, box_box_contact};
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

    /// The generator of G-L9b-1 (`narrowphase/reuse.rs`) for `seed`.
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// A uniformly random unit vector (rejection sampling in the unit ball).
    fn direction(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(
                self.range(-1.0, 1.0),
                self.range(-1.0, 1.0),
                self.range(-1.0, 1.0),
            );
            let l = v.length_squared();
            if l > 1.0e-4 && l <= 1.0 {
                return v * l.sqrt().recip();
            }
        }
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
///
/// With the fix the pair's contact is the best face's speculative point (T7), and this
/// GRAVITY-OFF step leaves the box exactly at rest (measured: spin 0, moved 0). That zero is the
/// gravity-off step's, not a property of a speculative point: under a load the relaxation pass
/// resists any approach, so such a point supports the box as any contact does.
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
    println!(
        "T3 (gravity off): spin {:e} rad/s, speed {:e} m/s, moved {:e} m",
        b.angular_velocity.length(),
        b.linear_velocity.length(),
        (b.position - bc).length()
    );
}

// ── Shared by T4, T5 and T7 ─────────────────────────────────────────────────────────────────

/// A box pair: each box's centre, orientation and half-extents, A first.
#[derive(Clone, Copy)]
struct Pose {
    ac: Vec3,
    aq: Quat,
    ah: Vec3,
    bc: Vec3,
    bq: Quat,
    bh: Vec3,
}

impl Pose {
    /// The same two boxes with A and B exchanged.
    fn swapped(self) -> Self {
        Self {
            ac: self.bc,
            aq: self.bq,
            ah: self.bh,
            bc: self.ac,
            bq: self.aq,
            bh: self.ah,
        }
    }

    /// The kernel's answer for the pair under `hint`.
    fn contact(&self, hint: Option<usize>) -> Option<BoxBoxContact> {
        box_box_contact(
            A, B, self.ac, self.aq, self.ah, self.bc, self.bq, self.bh, hint,
        )
    }

    /// The minimum SAT depth over the 15 axes, `f64` on the kernel's frames.
    fn min_depth(&self) -> f64 {
        min_sat_depth(self.ac, self.aq, self.ah, self.bc, self.bq, self.bh)
    }

    /// The pose as `f32` bits, for a message a witness can be pinned from.
    fn bits(&self) -> String {
        let v = |v: Vec3| {
            format!(
                "[{:#010x}, {:#010x}, {:#010x}]",
                v.x.to_bits(),
                v.y.to_bits(),
                v.z.to_bits()
            )
        };
        let q = |q: Quat| {
            format!(
                "[{:#010x}, {:#010x}, {:#010x}, {:#010x}]",
                q.x.to_bits(),
                q.y.to_bits(),
                q.z.to_bits(),
                q.w.to_bits()
            )
        };
        format!(
            "ac {} aq {} ah {} bc {} bq {} bh {}",
            v(self.ac),
            q(self.aq),
            v(self.ah),
            v(self.bc),
            q(self.bq),
            v(self.bh)
        )
    }
}

/// One recorded pose, as `f32` bits.
struct Pinned {
    name: &'static str,
    ac: [u32; 3],
    aq: [u32; 4],
    ah: [u32; 3],
    bc: [u32; 3],
    bq: [u32; 4],
    bh: [u32; 3],
}

impl Pinned {
    fn pose(&self) -> Pose {
        Pose {
            ac: v3(self.ac),
            aq: q4(self.aq),
            ah: v3(self.ah),
            bc: v3(self.bc),
            bq: q4(self.bq),
            bh: v3(self.bh),
        }
    }
}

/// The seed's pose `k` (T1) as a [`Pose`].
fn seed(k: usize) -> Pose {
    let (ah, bh) = seed_halves();
    let (ac, aq, bc, bq) = seed_pose(k);
    Pose {
        ac,
        aq,
        ah,
        bc,
        bq,
        bh,
    }
}

/// The `f32` spacing at `|x|`: one ulp.
fn ulp(x: f32) -> f64 {
    let x = x.abs();
    f64::from(f32::from_bits(x.to_bits() + 1)) - f64::from(x)
}

/// The largest absolute coordinate of `v`.
fn max_abs(v: Vec3) -> f32 {
    v.x.abs().max(v.y.abs()).max(v.z.abs())
}

/// `ε_FP`, the oracle's rounding allowance: `max(1e-5, 8 ulp of the pair's largest |coordinate|)`,
/// L9's check (b) form (design rev 1 §6).
fn eps_fp(p: &Pose) -> f64 {
    (8.0 * ulp(max_abs(p.ac).max(max_abs(p.bc)))).max(1.0e-5)
}

/// `ε_anc`, the anchor tolerance: `2e-6·max(1, |c_a|∞ + |c_b|∞ + r_max)`, L9's check (a) form.
fn eps_anc(p: &Pose) -> f64 {
    let r_max = f64::from(p.ah.length().max(p.bh.length()));
    2.0e-6 * (f64::from(max_abs(p.ac)) + f64::from(max_abs(p.bc)) + r_max).max(1.0)
}

/// How far `x` lies outside the box of centre `c`, orientation `q` and half-extents `h` (the
/// largest of its three slab distances; `≤ 0` inside), `f64` on the kernel's frame.
fn outside(c: Vec3, q: Quat, h: Vec3, x: Vec3) -> f64 {
    let ax = axes64(q);
    let d = [
        f64::from(x.x) - f64::from(c.x),
        f64::from(x.y) - f64::from(c.y),
        f64::from(x.z) - f64::from(c.z),
    ];
    let h = [h.x, h.y, h.z].map(f64::from);
    (0..3)
        .map(|i| dot(d, ax[i]).abs() - h[i])
        .fold(f64::NEG_INFINITY, f64::max)
}

/// The rotation frame's world axes as `f32`: the columns of `Mat3::from_quat`, the kernel's.
fn axes32(q: Quat) -> [Vec3; 3] {
    let r = Mat3::from_quat(q);
    [
        Vec3::new(r.rows[0].x, r.rows[1].x, r.rows[2].x),
        Vec3::new(r.rows[0].y, r.rows[1].y, r.rows[2].y),
        Vec3::new(r.rows[0].z, r.rows[1].z, r.rows[2].z),
    ]
}

// ── T4: existence never depends on the hint ─────────────────────────────────────────────────

/// Poses on which rev 1's guard (`chosen.depth > bound` → no contact, `design.md`) made existence
/// depend on the hint, found by the thinbox rev-2 probe (`r2_probe.log`); each is asked in both
/// A/B orders.
const PINNED: [Pinned; 5] = [
    // (a) a J/R 0.5 m box yawed ~45° on the 50 m floor: cold → contact; hint 8 (an edge held
    // within 1.05 of the best edge, deeper than the bound) → none under rev 1.
    Pinned {
        name: "a1 unit box on the floor, hint 8",
        ac: [0x0000_0000, 0xbf80_0000, 0x0000_0000],
        aq: [0x0000_0000, 0x0000_0000, 0x0000_0000, 0x3f80_0000],
        ah: [0x4248_0000, 0x3f80_0000, 0x4248_0000],
        bc: [0x416d_170c, 0x3f00_04bd, 0x40fb_d5f8],
        bq: [0xb766_f5de, 0x3ebf_3b65, 0x3850_03b2, 0x3f6d_792c],
        bh: [0x3f00_0000, 0x3f00_0000, 0x3f00_0000],
    },
    // (a) a random box over a thin slab's face: cold → contact; hint 12 → none under rev 1.
    Pinned {
        name: "a2 box over a slab, hint 12",
        ac: [0x4141_3aa4, 0xbfb1_ffc0, 0xc17a_1a4c],
        aq: [0xbe0e_6e0e, 0x3e33_27af, 0x3e19_dea1, 0x3f76_8a70],
        ah: [0x4216_83c8, 0x3e80_f4f5, 0x421a_d0a6],
        bc: [0xc1bf_e92e, 0xc184_7d70, 0xc1e6_32e2],
        bq: [0x3de9_5ece, 0xbf65_2ded, 0x3e2e_877f, 0x3eca_9083],
        bh: [0x3f1d_282b, 0x3f5f_4fa8, 0x3f80_d440],
    },
    // (a') a 50 µm plate on the floor, where τ = 5e-6 < SAT_EPS: cold → none under rev 1 (the
    // tie-rule best edge exceeds the bound); hint 12, shallower by less than SAT_EPS → contact.
    Pinned {
        name: "a3 50 um plate on the floor, hint 12",
        ac: [0x41d5_0bf6, 0x4094_ff22, 0x40ad_1020],
        aq: [0x3ed2_cd1e, 0x3f0c_7340, 0x3ebc_597e, 0x3f20_b8d7],
        ah: [0x4248_0000, 0x3f80_0000, 0x4248_0000],
        bc: [0xc185_dd8e, 0xc1d8_3368, 0x419b_9662],
        bq: [0x3ed2_cd16, 0x3f0c_733c, 0x3ebc_5989, 0x3f20_b8d9],
        bh: [0x3a03_126f, 0x3851_b718, 0x3a2a_64c3],
    },
    // (b)/(c) a face hint: cold → contact, hint 4 → none under rev 1 (and the swapped order the
    // other way round).
    Pinned {
        name: "c1 50 um plate on the floor, hint 4",
        ac: [0xc1b9_5ad4, 0x3dcf_3600, 0xc0ab_65f0],
        aq: [0x3dfa_5950, 0x3ee0_512f, 0x3e8e_5187, 0x3f58_9867],
        ah: [0x4248_0000, 0x3f80_0000, 0x4248_0000],
        bc: [0xc29c_bae8, 0xc19d_c191, 0xc13c_27b8],
        bq: [0x3dfa_a22c, 0x3ee0_19ba, 0x3e8e_4983, 0x3f58_a6c0],
        bh: [0x3a03_126f, 0x3851_b718, 0x3a2a_64c3],
    },
    Pinned {
        name: "c2 50 um plate under the floor, hint 4",
        ac: [0xbfb5_6360, 0xc22a_8e4e, 0xc046_1170],
        aq: [0x3ec7_771c, 0xbf3f_0376, 0x3eeb_f264, 0x3e90_056c],
        ah: [0x3a03_126f, 0x3851_b718, 0x3a2a_64c3],
        bc: [0x41d8_c634, 0xbe5f_bd80, 0xc1dc_2520],
        bq: [0x3ec6_4874, 0xbf3e_a6ac, 0x3eec_f0f2, 0x3e91_edf4],
        bh: [0x4248_0000, 0x3f80_0000, 0x4248_0000],
    },
];

/// The first hint under which the pair's existence differs from its cold answer, if any.
fn existence_flip(p: &Pose) -> Option<String> {
    let cold = p.contact(None).is_some();
    (0..15usize).find_map(|h| {
        let hinted = p.contact(Some(h)).is_some();
        (hinted != cold).then(|| format!("cold {cold}, hint {h} {hinted}"))
    })
}

/// T4: whether a box pair has a manifold never depends on the axis hint (`box_box.rs` module doc;
/// A7-N11), and a pair that truly overlaps has one. Each pair is asked cold and under every hint
/// 0..15, in both A/B orders: the pinned poses on which rev 1's guard broke it (a1, a2, a3, c1,
/// c2), the seed's two poses, and a thin/tilted family on the 50×1×50 floor (small boxes and
/// plates at ratios 1e3..1e5, near-parallel yaw, tilt, knife-edge or shallow, some at the rim,
/// axis-aligned or rotated world).
///
/// Two asserts: (1) `is_some` is equal across the 16 calls — the hint picks which contact, never
/// whether; (2) every pair whose `f64` minimum depth `D` exceeds `ε_FP` has a manifold, the "iff"
/// half of the invariant: (1) alone cannot see an answer of "none" decided from hint-free inputs.
/// The seed's pose 1 (`D` 2.2e-5 against `ε_FP` 1.5e-5) carries (2).
///
/// Rev 1's guard, on a scratch copy, turned (1) red on 8 of the 1 742 asked pairs, every one a
/// pinned witness; the family flips nothing on its own (`r2_rev1_t4.log`).
#[test]
fn manifold_existence_never_depends_on_the_hint() {
    let mut flips = Vec::new();
    let mut missing = Vec::new();
    let (mut asked, mut overlapping) = (0usize, 0usize);
    let mut ask = |name: &str, p: Pose| {
        for (tag, q) in [("A/B", p), ("B/A", p.swapped())] {
            asked += 1;
            if let Some(f) = existence_flip(&q) {
                flips.push(format!("{name} ({tag}): {f}"));
            }
            let (d, eps) = (q.min_depth(), eps_fp(&q));
            if d > eps {
                overlapping += 1;
                if q.contact(None).is_none() {
                    missing.push(format!(
                        "{name} ({tag}): D {d:e} > ε_FP {eps:e}, no manifold"
                    ));
                }
            }
        }
    };
    for p in &PINNED {
        ask(p.name, p.pose());
    }
    let s1 = seed(1);
    assert!(
        s1.min_depth() > eps_fp(&s1),
        "the seed's pose 1 must carry assert (2): D {:e}, ε_FP {:e}",
        s1.min_depth(),
        eps_fp(&s1)
    );
    ask("seed pose 0", seed(0));
    ask("seed pose 1", s1);
    let mut rng = Rng(0x7a1b_0c5e_ed00_0004);
    let floor = Vec3::new(50.0, 1.0, 50.0);
    for ratio in [1.0e3f32, 1.0e4, 1.0e5] {
        for psi in [0.0f32, 1.0e-5, 1.0e-3, 0.3] {
            for theta in [1.0e-5f32, 1.0e-3, 1.0e-1] {
                for rot in [false, true] {
                    for _ in 0..12 {
                        let h0 = 50.0 / ratio;
                        let hs = if rng.below(2) == 0 {
                            Vec3::new(h0, 0.8 * h0, 1.3 * h0)
                        } else {
                            Vec3::new(h0, 0.1 * h0, 1.3 * h0)
                        };
                        let sign = if rng.below(2) == 0 { 1.0 } else { -1.0 };
                        let yaw = about(Vec3::new(0.0, 1.0, 0.0), sign * psi);
                        let tdir = rng.range(0.0, core::f32::consts::TAU);
                        let qrel = about(Vec3::new(tdir.cos(), 0.0, tdir.sin()), theta).mul(yaw);
                        let (g, o) = if rot {
                            (
                                about(rng.direction(), rng.range(0.0, core::f32::consts::PI)).mul(
                                    about(rng.direction(), rng.range(0.0, core::f32::consts::PI)),
                                ),
                                Vec3::new(
                                    rng.range(-30.0, 30.0),
                                    rng.range(-5.0, 5.0),
                                    rng.range(-30.0, 30.0),
                                ),
                            )
                        } else {
                            (
                                Quat::IDENTITY,
                                Vec3::new(
                                    rng.range(-30.0, 30.0),
                                    rng.range(0.0, 5.0),
                                    rng.range(-30.0, 30.0),
                                ),
                            )
                        };
                        let ax = axes64(qrel);
                        let ext_y = (f64::from(hs.x) * ax[0][1].abs()
                            + f64::from(hs.y) * ax[1][1].abs()
                            + f64::from(hs.z) * ax[2][1].abs())
                            as f32;
                        let r_s = hs.length();
                        let reach = (50.0 - r_s).max(0.0);
                        let (mut x, mut z) =
                            (rng.range(-1.0, 1.0) * reach, rng.range(-1.0, 1.0) * reach);
                        if rng.below(5) == 0 {
                            let edge = if rng.below(2) == 0 { 50.0 } else { -50.0 }
                                + rng.range(-0.5, 0.5) * r_s;
                            if rng.below(2) == 0 {
                                x = edge;
                            } else {
                                z = edge;
                            }
                        }
                        let hmin = hs.x.min(hs.y).min(hs.z);
                        let gap = if rng.below(10) < 7 {
                            rng.range(-4.0e-6, 4.0e-6)
                        } else {
                            -rng.range(0.0, 0.05) * hmin
                        };
                        let local = Vec3::new(x, floor.y + ext_y + gap, z);
                        let p = Pose {
                            ac: o,
                            aq: g,
                            ah: floor,
                            bc: o + g.rotate(local),
                            bq: g.mul(qrel),
                            bh: hs,
                        };
                        ask("family", p);
                    }
                }
            }
        }
    }
    assert!(
        flips.is_empty(),
        "whether a box pair has a manifold changed with the axis hint on {} of {asked} asked pairs; \
         it must depend on the poses alone: {}",
        flips.len(),
        flips.join("; ")
    );
    assert!(
        missing.is_empty(),
        "{} of {overlapping} pairs that overlap by more than ε_FP have no manifold: {}",
        missing.len(),
        missing.join("; ")
    );
    println!(
        "T4: {asked} pairs asked cold and under 15 hints, no existence flip; {overlapping} overlap \
         by more than ε_FP, each with a manifold"
    );
}

// ── T7: the phantom answer ──────────────────────────────────────────────────────────────────

/// T7: past an overlapping SAT whose every edge axis claims more than the face allows, the answer
/// is the best face's own contact — on the seed's pose 1, a one-point face contact on face axis 1
/// at the box's lowest clipped corner, 1.43 µm above the slab (speculative), anchors 1.43 µm apart,
/// under every hint asked (cold, the record's face 1, the phantom edge 14, another edge 8).
///
/// It separates the fix from rev 1 (no manifold) and from the clamped edge answers, which pass T3
/// but keep axis 14 with anchors 16.95 m apart (`r2_rc_t3.log`).
#[test]
fn the_phantom_answer_is_the_best_faces_own_point() {
    let s1 = seed(1);
    for hint in [None, Some(1), Some(14), Some(8)] {
        let c = s1.contact(hint).expect("an overlapping SAT has a manifold");
        let p = c.manifold.points[0];
        assert!(
            c.reference_axis == 1
                && c.manifold.count == 1
                && p.separation > 0.0
                && p.separation < 1.0e-5
                && (p.anchor_a - p.anchor_b).length() < 1.0e-5,
            "hint {hint:?}: axis {} count {} separation {:e} anchors {:e} apart",
            c.reference_axis,
            c.manifold.count,
            p.separation,
            (p.anchor_a - p.anchor_b).length()
        );
    }
}

// ── T5: every box-box manifold is bounded ───────────────────────────────────────────────────

/// A pair at poses 0 and 1, G-L9b-1's.
struct SoundCase {
    ha: Vec3,
    hb: Vec3,
    pose0: [(Vec3, Quat); 2],
    pose1: [(Vec3, Quat); 2],
}

impl SoundCase {
    /// Pose `k` of the case as a [`Pose`].
    fn pose(&self, k: usize) -> Pose {
        let p = if k == 0 { self.pose0 } else { self.pose1 };
        Pose {
            ac: p[0].0,
            aq: p[0].1,
            ah: self.ha,
            bc: p[1].0,
            bq: p[1].1,
            bh: self.hb,
        }
    }
}

/// G-L9b-1's `sound_case` (`narrowphase/reuse.rs` tests), verbatim: a box resting on a base box or
/// slab, tilted so its lowest corner sits within a few τ of the base's top face, then moved relative
/// to the base; the base is body A or body B at random.
fn sound_case(rng: &mut Rng) -> SoundCase {
    const TAU_EFF_FRACTION: f32 = 0.05;
    let tau = [2.5e-4, 5.0e-4, 1.0e-3, 2.0e-3][rng.below(4) as usize];
    let far = rng.below(8) == 0;
    let slab = far || rng.below(3) == 0;
    let base_half = if slab {
        Vec3::new(
            rng.range(20.0, 60.0),
            rng.range(0.05, 1.0),
            rng.range(20.0, 60.0),
        )
    } else {
        Vec3::new(
            rng.range(0.2, 1.5),
            rng.range(0.2, 1.5),
            rng.range(0.2, 1.5),
        )
    };
    let top_half = match rng.below(4) {
        0 => Vec3::new(
            rng.range(0.002, 0.02),
            rng.range(0.002, 0.02),
            rng.range(0.002, 0.02),
        ),
        1 => Vec3::new(
            rng.range(0.01, 0.6),
            rng.range(0.001, 0.02),
            rng.range(0.01, 0.6),
        ),
        _ => Vec3::new(
            rng.range(0.2, 1.2),
            rng.range(0.2, 1.2),
            rng.range(0.2, 1.2),
        ),
    };
    let g = about(rng.direction(), rng.range(0.0, core::f32::consts::PI));
    let base_c = Vec3::new(
        rng.range(-5.0, 5.0),
        rng.range(-5.0, 5.0),
        rng.range(-5.0, 5.0),
    );
    let lever = top_half.x.max(top_half.z);
    let phi = if rng.below(3) == 0 {
        0.0
    } else {
        rng.range(0.0, 3.0 * tau / lever)
    };
    let yaw = about(Vec3::new(0.0, 1.0, 0.0), rng.range(-3.2, 3.2));
    let local_q = about(Vec3::new(1.0, 0.0, 0.0), phi).mul(yaw);
    let reach = if far {
        (base_half.x - 1.0).max(0.0)
    } else {
        base_half.x.min(base_half.z)
    };
    let (ox, oz) = if far {
        (rng.range(0.7, 1.0) * reach, rng.range(-0.3, 0.3) * reach)
    } else {
        (rng.range(-1.0, 1.0) * reach, rng.range(-1.0, 1.0) * reach)
    };
    let axes = axes32(local_q);
    let extent_y =
        top_half.x * axes[0].y.abs() + top_half.y * axes[1].y.abs() + top_half.z * axes[2].y.abs();
    let depth = rng.range(-tau, 2.0 * tau);
    let local_c = Vec3::new(ox, base_half.y + extent_y - depth, oz);
    let top0 = (base_c + g.rotate(local_c), g.mul(local_q));
    let base0 = (base_c, g);
    let r_top = top_half.length();
    let h_min = top_half
        .x
        .min(top_half.y)
        .min(top_half.z)
        .min(base_half.x)
        .min(base_half.y)
        .min(base_half.z);
    let tau_eff = tau
        .min(TAU_EFF_FRACTION * r_top.min(base_half.length()))
        .min(TAU_EFF_FRACTION * h_min);
    let axis = if rng.below(2) == 0 {
        if rng.below(2) == 0 {
            Vec3::new(1.0, 0.0, 0.0)
        } else {
            Vec3::new(0.0, 0.0, 1.0)
        }
    } else {
        rng.direction()
    };
    let (psi_max, shift_max) = if rng.below(2) == 0 {
        (0.5, 0.5)
    } else {
        (20.0, 3.0)
    };
    let psi = rng.unit() * psi_max * tau_eff / r_top;
    let shift = rng.direction() * (rng.unit() * shift_max * tau_eff);
    let top1_local = (local_c + shift, about(axis, psi).mul(local_q));
    let h = about(rng.direction(), rng.range(0.0, 0.3));
    let t = rng.direction() * rng.range(0.0, 1.0);
    let base1 = (h.rotate(base_c) + t, h.mul(g));
    let top1 = (
        h.rotate(base_c + g.rotate(top1_local.0)) + t,
        h.mul(g.mul(top1_local.1)),
    );
    if rng.below(2) == 0 {
        SoundCase {
            ha: base_half,
            hb: top_half,
            pose0: [base0, top0],
            pose1: [base1, top1],
        }
    } else {
        SoundCase {
            ha: top_half,
            hb: base_half,
            pose0: [top0, base0],
            pose1: [top1, base1],
        }
    }
}

/// splitmix64 of `i`: the generator seeds of the thinbox probe's G-L9b-1 population.
fn gen_seed(i: u64) -> u64 {
    let mut z = i.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The rev-1 architect's parallel-edge family (`arch_probe.rs::family_pose`): a small box or plate
/// of half-extent `50/ratio` on the 50×1×50 floor, yawed `±psi`, tilted `theta` about a random
/// horizontal axis, at a knife-edge gap (70 %) or up to 5 % of its thinnest extent deep, one in
/// five at the floor's rim, in an axis-aligned or a rotated and offset world.
fn family_pose(rng: &mut Rng, ratio: f32, psi: f32, theta: f32, rot: bool) -> Pose {
    let hf = Vec3::new(50.0, 1.0, 50.0);
    let h0 = 50.0 / ratio;
    let hs = if rng.below(2) == 0 {
        Vec3::new(h0, 0.8 * h0, 1.3 * h0)
    } else {
        Vec3::new(h0, 0.1 * h0, 1.3 * h0)
    };
    let sign = if rng.below(2) == 0 { 1.0 } else { -1.0 };
    let yaw = about(Vec3::new(0.0, 1.0, 0.0), sign * psi);
    let tdir = rng.range(0.0, core::f32::consts::TAU);
    let qrel = about(Vec3::new(tdir.cos(), 0.0, tdir.sin()), theta).mul(yaw);
    let (g, o) = if rot {
        (
            about(rng.direction(), rng.range(0.0, core::f32::consts::PI)).mul(about(
                rng.direction(),
                rng.range(0.0, core::f32::consts::PI),
            )),
            Vec3::new(
                rng.range(-30.0, 30.0),
                rng.range(-5.0, 5.0),
                rng.range(-30.0, 30.0),
            ),
        )
    } else {
        (
            Quat::IDENTITY,
            Vec3::new(
                rng.range(-30.0, 30.0),
                rng.range(0.0, 5.0),
                rng.range(-30.0, 30.0),
            ),
        )
    };
    let ax = axes32(qrel);
    let ext_y = hs.x * ax[0].y.abs() + hs.y * ax[1].y.abs() + hs.z * ax[2].y.abs();
    let r_s = hs.length();
    let reach = (50.0 - r_s).max(0.0);
    let (mut x, mut z) = (rng.range(-1.0, 1.0) * reach, rng.range(-1.0, 1.0) * reach);
    if rng.below(5) == 0 {
        let edge = if rng.below(2) == 0 { 50.0 } else { -50.0 } + rng.range(-0.5, 0.5) * r_s;
        if rng.below(2) == 0 {
            x = edge;
        } else {
            z = edge;
        }
    }
    let hmin_s = hs.x.min(hs.y).min(hs.z);
    let gap = if rng.below(10) < 7 {
        rng.range(-4.0e-6, 4.0e-6)
    } else {
        -rng.range(0.0, 0.05) * hmin_s
    };
    let local_c = Vec3::new(x, hf.y + ext_y + gap, z);
    Pose {
        ac: o,
        aq: g,
        ah: hf,
        bc: o + g.rotate(local_c),
        bq: g.mul(qrel),
        bh: hs,
    }
}

/// What T5 saw over its population.
#[derive(Default, Debug)]
struct BoundSeen {
    calls: u64,
    contacts: u64,
    /// Single-point face contacts with a positive separation: the fix's speculative answers.
    speculative: u64,
    /// Contacts past the scaled bound (I1-τ) where it is only printed (the ratio-1e5 row).
    tau_residual: u64,
    /// Edge contacts whose anchors lie beyond `|s| + ε_anc` of the other box (printed).
    edge_other: u64,
    /// The worst such offset.
    edge_other_worst: f64,
    /// The largest `claim − D` of any manifold.
    worst_claim_minus_d: f64,
}

/// One T5 call: the pair under `hint`, checked against I1-abs, I1-τ (asserted iff `tau_asserted`),
/// I2-own and, for face contacts, I2-other.
fn bounded(
    p: &Pose,
    hint: Option<usize>,
    tau_asserted: bool,
    seen: &mut BoundSeen,
) -> Result<(), String> {
    seen.calls += 1;
    let Some(c) = p.contact(hint) else {
        return Ok(());
    };
    seen.contacts += 1;
    let m = &c.manifold;
    let d = p.min_depth().max(0.0);
    let claim = claimed_depth(m);
    let eps = eps_fp(p);
    let h_min = [p.ah.x, p.ah.y, p.ah.z, p.bh.x, p.bh.y, p.bh.z]
        .map(f64::from)
        .into_iter()
        .fold(f64::INFINITY, f64::min);
    let tau = FACE_AXIS_PREFERENCE.min(0.1 * h_min);
    seen.worst_claim_minus_d = seen.worst_claim_minus_d.max(claim - d);
    let at = || {
        format!(
            "hint {hint:?}, axis {}, count {}: {}",
            c.reference_axis,
            m.count,
            p.bits()
        )
    };
    if claim > HYSTERESIS_RATIO * d + FACE_AXIS_PREFERENCE + eps {
        return Err(format!(
            "I1-abs: the manifold claims {claim:e} m, past 1.05·D + 5 mm + ε_FP (D {d:e}, ε_FP {eps:e}); {}",
            at()
        ));
    }
    if claim > HYSTERESIS_RATIO * d + tau + eps {
        if tau_asserted {
            return Err(format!(
                "I1-τ: the manifold claims {claim:e} m, past 1.05·D + τ + ε_FP (D {d:e}, τ {tau:e}, \
                 ε_FP {eps:e}); {}",
                at()
            ));
        }
        seen.tau_residual += 1;
    }
    let anc = eps_anc(p);
    let face = c.reference_axis < 6;
    if face && m.count == 1 && m.points[0].separation > 0.0 {
        seen.speculative += 1;
    }
    for pt in &m.points[..usize::from(m.count)] {
        let own =
            outside(p.ac, p.aq, p.ah, pt.anchor_a).max(outside(p.bc, p.bq, p.bh, pt.anchor_b));
        if own > anc {
            return Err(format!(
                "I2-own: an anchor lies {own:e} m outside its own box (ε_anc {anc:e}); {}",
                at()
            ));
        }
        let s = f64::from(pt.separation).abs();
        let other = (outside(p.bc, p.bq, p.bh, pt.anchor_a) - s)
            .max(outside(p.ac, p.aq, p.ah, pt.anchor_b) - s);
        if other > anc {
            if face {
                return Err(format!(
                    "I2-other: a face contact's anchor lies {other:e} m beyond |s| of the other \
                     box (ε_anc {anc:e}); {}",
                    at()
                ));
            }
            seen.edge_other += 1;
            seen.edge_other_worst = seen.edge_other_worst.max(other);
        }
    }
    Ok(())
}

/// The fixed cases T5 runs first, before any drawn one: G-L9b-1's seed (the defect's witness,
/// T1) and the generator's worst over-claim on the unfixed kernel (3.64 m).
const FIXED_SEEDS: [u64; 2] = [0x531f_f99f_772d_936c, 0x65ff_5fe6_7f34_e65e];

/// T5's third fixed case: a box of half-extents 4–6.5 mm on the 50×1×50 floor (a rotated world,
/// the family's ratio 1e4) whose every edge
/// axis claims more than the face allows and whose best face's clip keeps one vertex just above
/// the floor. The fix answers with that clipped vertex; an unclipped corner answer (mutation
/// M-Spec, the speculative tier removed) puts the floor's anchor 4.8 mm outside the floor, which
/// I2-own reds here before any drawn case. Found by T5's own family population under M-Spec (50
/// calls red there; `impl/mut_M-Spec_probe.log`).
const OVERHANG: Pinned = Pinned {
    name: "the M-Spec overhang witness",
    ac: [0xc061_3440, 0xbf8e_3eba, 0xc1c4_8127],
    aq: [0x3ed3_0de2, 0x3f55_9c67, 0x3e83_2967, 0xbe85_b5a0],
    ah: [0x4248_0000, 0x3f80_0000, 0x4248_0000],
    bc: [0x41cd_4cfc, 0xc209_f80a, 0xc246_854a],
    bq: [0x3ed3_1a82, 0x3f55_9522, 0x3e83_6144, 0xbe85_9947],
    bh: [0x3ba3_d70a, 0x3b83_126f, 0x3bd4_fdf3],
};

/// T5: every box-box manifold is bounded, and its anchors lie on the boxes.
///
/// * **I1-abs** (the theorem): claim ≤ 1.05·D + 5 mm + ε_FP, on every call. Every emitted manifold
///   claims at most its axis depth, a held axis at most 1.05 × the best, and a fallback edge at most
///   the face bound — so this can fail only through a defect.
/// * **I1-τ** (a measurement on this population): claim ≤ 1.05·D + τ + ε_FP with τ = min(5 mm,
///   0.1·h_min), everywhere but the family's ratio-1e5 row, which carries face-path residuals of a
///   50 µm plate 50–80 m out, where τ = 5e-6 is about an ulp (`design_rev2.md` §4, identical in
///   every variant of the fallback); that row's count is printed.
/// * **I2-own**: every anchor within ε_anc of its own box.
/// * **I2-other**: a face contact's anchors within |s| + ε_anc of the other box. An accepted edge
///   fallback can keep witnesses on edges that do not touch (rev 1 §7's follow-up), so for edge
///   contacts the count and the worst offset are printed, not asserted.
///
/// Population: the fixed cases first (G-L9b-1's two seeds, both poses, and [`OVERHANG`], a pose on
/// which an unclipped corner answer would overhang; both A/B orders, every hint), then the parallel-edge
/// family (6 ratios × 8 yaws × 7 tilts × 2 worlds × 50) and `sound_case` over 4 096 generator
/// seeds (both poses), each in both A/B orders, cold and under one random hint.
#[test]
fn every_box_box_manifold_is_bounded() {
    let mut seen = BoundSeen::default();
    let mut fixed = Vec::new();
    for s in FIXED_SEEDS {
        let case = sound_case(&mut Rng::new(s));
        for k in 0..2 {
            fixed.push((format!("seed {s:#x} pose {k}"), case.pose(k)));
        }
    }
    fixed.push((OVERHANG.name.to_owned(), OVERHANG.pose()));
    for (name, p) in &fixed {
        for q in [*p, p.swapped()] {
            for hint in core::iter::once(None).chain((0..15).map(Some)) {
                if let Err(why) = bounded(&q, hint, true, &mut seen) {
                    panic!("fixed case {name}: {why}");
                }
            }
        }
    }
    println!("T5 fixed cases: {seen:?}");
    seen = BoundSeen::default();

    let mut failures = Vec::new();
    let mut hints = Rng::new(0x7457_0b0d_0000_0005);
    let mut draw =
        |p: Pose, tau_asserted: bool, seen: &mut BoundSeen, failures: &mut Vec<String>| {
            for q in [p, p.swapped()] {
                for hint in [None, Some(hints.below(15) as usize)] {
                    if let Err(why) = bounded(&q, hint, tau_asserted, seen) {
                        failures.push(why);
                    }
                }
            }
        };
    const RATIOS: [f32; 6] = [1.0, 10.0, 100.0, 1.0e3, 1.0e4, 1.0e5];
    const PSIS: [f32; 8] = [0.0, 1.0e-6, 1.0e-5, 1.0e-4, 3.0e-4, 1.0e-3, 1.0e-2, 0.3];
    const THETAS: [f32; 7] = [0.0, 1.0e-6, 1.0e-5, 1.0e-4, 1.0e-3, 1.0e-2, 0.1];
    let mut rng = Rng::new(0xa4c1_7ec7_0000_0001);
    for &ratio in &RATIOS {
        for &psi in &PSIS {
            for &theta in &THETAS {
                for rot in [false, true] {
                    for _ in 0..50 {
                        let p = family_pose(&mut rng, ratio, psi, theta, rot);
                        draw(p, ratio <= 1.0e4, &mut seen, &mut failures);
                    }
                }
            }
        }
    }
    let family = std::mem::take(&mut seen);
    for i in 0..4096u64 {
        let case = sound_case(&mut Rng::new(gen_seed(i)));
        for k in 0..2 {
            draw(case.pose(k), true, &mut seen, &mut failures);
        }
    }
    println!("T5 family: {family:?}");
    println!("T5 generator: {seen:?}");
    assert!(
        family.speculative > 0 && family.contacts > 0 && seen.contacts > 0,
        "the population must reach contacts and the fix's speculative answer: family {family:?}, \
         generator {seen:?}"
    );
    assert!(
        failures.is_empty(),
        "{} calls broke a bound; the first: {}",
        failures.len(),
        failures
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join(" | ")
    );
}
