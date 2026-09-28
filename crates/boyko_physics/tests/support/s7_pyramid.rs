//! Shared by the S7 binaries (`s7_lane_cap`, `s7_lane_cap_armed`): Jolt's pyramid on the real
//! colored schedule over a pool of a chosen width, in the two row shapes window 8 measured —
//! J-T (the default configuration, the tree broadphase, sleeping off) and J-A (`AllPairs` under
//! `Manual`, the scalar colour kernel, sleeping off) — with the colored solver's lane cap set
//! before the first step, and the final rows' pose hash.
//!
//! The pyramid is the attribution binary's (`alloc_frame_attribution.rs`, "D"), index for index:
//! Jolt's `cPyramidHeight` of 15 in a release build, 10 in a debug build.

use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{BroadphaseKind, BroadphaseSelectMode, PhysicsConfig, SolverScratch};
use boyko_physics::solver::{ColoredSoftStepSolver, LaneCap};

const BOX_SIZE: f32 = 2.0;
const BOX_SEPARATION: f32 = 0.5;
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
/// The fixed step (60 Hz).
pub const DT: f32 = 1.0 / 60.0;

/// Jolt's `cPyramidHeight`, scaled down in a debug build.
const fn pyramid_height() -> i32 {
    if cfg!(debug_assertions) { 10 } else { 15 }
}

/// Window 8's two row shapes.
//
// `dead_code`: this module is compiled into both S7 binaries, and only `s7_lane_cap` runs J-A.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// J-T: the default configuration, the tree broadphase, sleeping off.
    JT,
    /// J-A: `AllPairs` under `Manual`, the scalar colour kernel (`simd_solve` off), sleeping off.
    JA,
}

fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `T` is a `#[repr(C)]` POD physics component; the slice is exactly
    //   `size_of::<T>()` bytes read from a live `&T` and cannot outlive that borrow.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn spawn_body(world: &mut EcsMaster, position: Vec3, mass: RigidBodyMass, half_extents: Vec3, simulated: bool) {
    let body =
        RigidBody { position, linear_velocity: Vec3::ZERO, rotation: Quat::IDENTITY, angular_velocity: Vec3::ZERO };
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
        .expect("invariant: the RigidBodyBundle archetype accepts the three columns");
    if simulated {
        world.enable::<Simulated>(e);
    }
}

/// Jolt's pyramid, index for index.
fn spawn_jolt_pyramid(world: &mut EcsMaster) {
    let floor = RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass: 0.0, restitution: 0.0, friction: 0.5 };
    spawn_body(world, Vec3::new(0.0, -1.0, 0.0), floor, FLOOR_HALF_EXTENTS, false);
    let height = pyramid_height();
    let mass = RigidBodyMass {
        inv_inertia: Mat3::from_diagonal(Vec3::new(0.1875, 0.1875, 0.1875)),
        inv_mass: 0.125,
        restitution: 0.0,
        friction: 0.5,
    };
    for i in 0..height {
        let (lo, hi) = (i / 2, height - (i + 1) / 2);
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                let position = Vec3::new(
                    -(height as f32) + BOX_SIZE * j as f32 + odd,
                    1.0 + (BOX_SIZE + BOX_SEPARATION) * i as f32,
                    -(height as f32) + BOX_SIZE * k as f32 + odd,
                );
                spawn_body(world, position, mass, Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX), true);
            }
        }
    }
}

/// A pyramid world on the colored schedule.
pub struct Pile {
    pub world: EcsMaster,
    pub schedule: Schedule,
}

impl Pile {
    /// The pyramid in `row`'s shape on a `workers`-worker pool, the parallel solve on (off when
    /// `parallel` is false: the serial reference), the solver's lane cap set to `cap`.
    pub fn new(workers: usize, row: Row, parallel: bool, cap: LaneCap) -> Self {
        let mut world = EcsMaster::new();
        spawn_jolt_pyramid(&mut world);
        let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(workers).build());
        let _ = add_physics_colored_solve(&mut builder, &mut world);
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        {
            let cfg = world.resource_mut::<PhysicsConfig>();
            cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
            cfg.dt = DT;
            cfg.sleeping = false;
            cfg.parallel_solve = parallel;
            cfg.broadphase_select = BroadphaseSelectMode::Manual;
            match row {
                Row::JT => cfg.broadphase = BroadphaseKind::Tree,
                Row::JA => {
                    cfg.broadphase = BroadphaseKind::AllPairs;
                    cfg.simd_solve = false;
                }
            }
        }
        let schedule = builder.build(&mut world);
        world.resource_mut::<ColoredSoftStepSolver>().set_lane_cap(cap);
        Self { world, schedule }
    }

    /// One physics step.
    pub fn step(&mut self) {
        self.schedule.run(&mut self.world);
    }

    /// The colored solver resource.
    pub fn solver(&self) -> &ColoredSoftStepSolver {
        self.world.resource::<ColoredSoftStepSolver>()
    }

    /// FNV-1a 64 over every gathered row's position, rotation and velocities, as bits.
    //
    // `dead_code`: as for `Row`; the armed binary reads the counters, not the pose.
    #[allow(dead_code)]
    pub fn pose_hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for b in self.world.resource::<SolverScratch>().bodies() {
            let q = b.rotation;
            for v in [
                b.position.x, b.position.y, b.position.z, q.x, q.y, q.z, q.w, b.linear_velocity.x,
                b.linear_velocity.y, b.linear_velocity.z, b.angular_velocity.x, b.angular_velocity.y,
                b.angular_velocity.z,
            ] {
                h = (h ^ u64::from(v.to_bits())).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    }
}
