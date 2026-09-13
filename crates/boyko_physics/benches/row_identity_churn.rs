//! Row identity churn: the price of the defect-A interim row identity map on the REAL
//! colored physics schedule, one arm per kind of structural change
//! (docs/MEASUREMENT-QUEUE.md §6).
//!
//! # Scene
//!
//! Jolt's pyramid (1240 dynamic boxes on a static floor, transcribed as in
//! `jolt_parity_pyramid.rs`) plus 96 resting spheres in the floor's far corner, 2 m apart,
//! clear of the pyramid and of each other. A sphere that is despawned is respawned at the
//! same pose, so the contact set stays stationary across iterations.
//!
//! * MARKED `{RigidBody, RigidBodyMass, Collider, Marker}` is created FIRST, so the gather
//!   walks it first. It holds 64 spheres.
//! * PLAIN `{RigidBody, RigidBodyMass, Collider}` holds the floor, the pyramid and 32
//!   spheres at its tail.
//!
//! # Arms, each × sleeping {off, on}
//!
//! The structural change is applied between steps and is NOT timed: every sample times
//! exactly one `Schedule::run`, whose gather pays for the row identity map.
//!
//! * `stable` — no structural change.
//! * `swap_churn` — despawn one PLAIN sphere that is not PLAIN's last row and respawn it at
//!   its pose (recycled id, row count constant).
//! * `archetype_shift` — remove `Marker` from one MARKED sphere, and insert it back on the
//!   next step; either way every PLAIN row shifts.
//! * `first_archetype_spawn` — even steps despawn one PLAIN sphere and spawn one into MARKED
//!   at its pose; odd steps undo it, so the scene stays stationary. Every step shifts
//!   PLAIN's rows.
//! * `burst_despawn` — despawn the 32 spheres in MARKED's first 32 rows (each despawn
//!   swap-moves a survivor from MARKED's tail) and respawn 32 at the same poses, flagged
//!   added.
//! * `burst_migrate` — insert `Marker` on the 32 PLAIN spheres, and remove it on the next
//!   step: more than `REMAP_WINDOW` rows land ahead of the pyramid, so the walk loses
//!   alignment.
//!
//! Before timing, each arm runs a few churn steps, asserts that every churn step builds
//! exactly one previous-row map (and `stable` none), and prints the structural receipt:
//! maps built and rows resolved by stage 2. Those counts are structure, not timing.
//!
//! # ⚠ What this cannot say on its own
//!
//! It runs only on a tree WITH the fix: the tree without it has no row identity map, so
//! what the always-on half costs is answered by `jolt_parity_pyramid`'s `full_step` on
//! both trees (§6, R1). This bench answers R2 and R3: what a change step costs against
//! `stable` on the same tree.

use std::sync::Arc;
use std::time::{Duration, Instant};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::{Commands, Res};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_macros::{Component, Resource};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};

use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass, Simulated};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{PhysicsConfig, SolverScratch};

// ── Scene constants ──────────────────────────────────────────────────────────

/// The fixed step (60 Hz).
const DT: f32 = 1.0 / 60.0;
/// Jolt's `cBoxSize`: the pitch between neighbouring boxes in a layer.
const BOX_SIZE: f32 = 2.0;
/// Jolt's `cBoxSeparation`: the vertical gap added on top of `cBoxSize` per layer.
const BOX_SEPARATION: f32 = 0.5;
/// Jolt's `cHalfBoxSize`.
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
/// Jolt's `cPyramidHeight`.
const PYRAMID_HEIGHT: i32 = 15;
/// The floor: a static box centred at `(0, -1, 0)`, top face at `y = 0`.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);

/// Churn spheres in the MARKED archetype, pose slots `0..MARKED_SPHERES`.
const MARKED_SPHERES: usize = 64;
/// Churn spheres at the PLAIN archetype's tail, the pose slots after MARKED's.
const PLAIN_SPHERES: usize = 32;
/// Rows moved by `burst_despawn` / `burst_migrate`; above `REMAP_WINDOW` (16).
const BURST: usize = 32;
/// Side of the square pose grid the spheres rest on.
const SPHERE_GRID: usize = 10;
/// Distance between neighbouring sphere poses.
const SPHERE_PITCH: f32 = 2.0;
/// `x` and `z` of pose slot 0: the floor's far corner, clear of the pyramid (`|x|, |z| <= 16`).
const SPHERE_ORIGIN: f32 = 24.0;
/// Sphere radius; the spheres rest on the floor at `y = SPHERE_RADIUS`.
const SPHERE_RADIUS: f32 = 0.5;

/// Steps run before the receipt, so the first gather's map and the initial contact
/// settle are out of the way.
const SETTLE_STEPS: usize = 30;
/// Churn steps the structural receipt covers.
const RECEIPT_STEPS: u64 = 4;

// ── Bench-only component, resource and system ────────────────────────────────

/// A data component whose presence puts a body in the MARKED archetype.
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct Marker {
    _tag: u32,
}

/// The `Marker` inserts and removes the next `marker_ops` run applies through `Commands`.
#[derive(Resource, Default)]
struct MarkerPlan {
    insert: Vec<Entity>,
    remove: Vec<Entity>,
}

/// Applies the planned `Marker` inserts and removes (a real archetype migration).
//
// `clippy::needless_pass_by_value`: `Commands` / `Res` are by-value `SystemParam`s, the
// same false positive the physics systems carry.
#[allow(clippy::needless_pass_by_value)]
fn apply_marker_plan(mut commands: Commands, plan: Res<MarkerPlan>) {
    for &entity in &plan.insert {
        commands.entity(entity).insert(Marker { _tag: 0 });
    }
    for &entity in &plan.remove {
        commands.entity(entity).remove::<Marker>();
    }
}

// ── Harness ──────────────────────────────────────────────────────────────────

/// Views a `#[repr(C)]` POD component as its bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, initialised `#[repr(C)]` POD component borrowed for the
    // returned slice's lifetime; the slice covers exactly its `size_of::<T>()` bytes,
    // read-only, which is the layout the component pool stores.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A single-worker pool: the price of the map, not of dispatch.
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// The pose of sphere slot `slot`.
fn slot_pose(slot: usize) -> Vec3 {
    Vec3::new(
        SPHERE_ORIGIN + SPHERE_PITCH * (slot % SPHERE_GRID) as f32,
        SPHERE_RADIUS,
        SPHERE_ORIGIN + SPHERE_PITCH * (slot / SPHERE_GRID) as f32,
    )
}

/// The slot whose pose is nearest `position` (a resting sphere drifts far less than half
/// a pitch).
fn slot_of(position: Vec3) -> usize {
    let col = ((position.x - SPHERE_ORIGIN) / SPHERE_PITCH).round() as usize;
    let row = ((position.z - SPHERE_ORIGIN) / SPHERE_PITCH).round() as usize;
    row * SPHERE_GRID + col
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arm {
    Stable,
    SwapChurn,
    ArchetypeShift,
    FirstArchetypeSpawn,
    BurstDespawn,
    BurstMigrate,
}

impl Arm {
    const ALL: [Self; 6] = [
        Self::Stable,
        Self::SwapChurn,
        Self::ArchetypeShift,
        Self::FirstArchetypeSpawn,
        Self::BurstDespawn,
        Self::BurstMigrate,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::SwapChurn => "swap_churn",
            Self::ArchetypeShift => "archetype_shift",
            Self::FirstArchetypeSpawn => "first_archetype_spawn",
            Self::BurstDespawn => "burst_despawn",
            Self::BurstMigrate => "burst_migrate",
        }
    }
}

/// Where a sphere is spawned.
#[derive(Clone, Copy)]
enum Target {
    Marked,
    Plain,
}

struct Churn {
    world: EcsMaster,
    physics: Schedule,
    marker_ops: Schedule,
    marked: ArchetypeId,
    plain: ArchetypeId,
    /// `slots[s]` is the live sphere at pose slot `s`.
    slots: Vec<Entity>,
    /// Next PLAIN slot `swap_churn` / `first_archetype_spawn` cycle through.
    cursor: usize,
    /// Churn steps applied so far; its parity picks the half of an alternating arm.
    step: u64,
}

impl Churn {
    fn new(sleeping: bool) -> Self {
        let mut world = EcsMaster::new();
        let marked = world.create_archetype(&[
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
            Marker::component_id(),
        ]);
        let plain = world.create_archetype(&[
            RigidBody::component_id(),
            RigidBodyMass::component_id(),
            Collider::component_id(),
        ]);

        let mut builder = ScheduleBuilder::new(serial_pool());
        let _keys = add_physics_colored_solve(&mut builder, &mut world);
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        let physics = builder.build(&mut world);
        {
            let cfg = world.resource_mut::<PhysicsConfig>();
            cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
            cfg.dt = DT;
            cfg.sleeping = sleeping;
        }

        world.insert_resource(MarkerPlan::default());
        let mut ops = ScheduleBuilder::new(serial_pool());
        ops.add_system(apply_marker_plan);
        let marker_ops = ops.build(&mut world);

        let mut churn = Self {
            world,
            physics,
            marker_ops,
            marked,
            plain,
            slots: Vec::with_capacity(MARKED_SPHERES + PLAIN_SPHERES),
            cursor: 0,
            step: 0,
        };
        for slot in 0..MARKED_SPHERES {
            let sphere = churn.spawn_sphere(slot, Target::Marked);
            churn.slots.push(sphere);
        }
        churn.spawn_floor_and_pyramid();
        for slot in MARKED_SPHERES..MARKED_SPHERES + PLAIN_SPHERES {
            let sphere = churn.spawn_sphere(slot, Target::Plain);
            churn.slots.push(sphere);
        }
        churn
    }

    fn spawn(
        &mut self,
        target: Target,
        body: RigidBody,
        mass: RigidBodyMass,
        collider: Collider,
        simulated: bool,
    ) -> Entity {
        let marker = Marker { _tag: 0 };
        let created = match target {
            Target::Marked => self.world.create_entity(
                self.marked,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                    (Marker::component_id(), as_bytes(&marker)),
                ],
            ),
            Target::Plain => self.world.create_entity(
                self.plain,
                &[
                    (RigidBody::component_id(), as_bytes(&body)),
                    (RigidBodyMass::component_id(), as_bytes(&mass)),
                    (Collider::component_id(), as_bytes(&collider)),
                ],
            ),
        };
        let entity = created.expect("construction: the body archetype accepts every column");
        if simulated {
            self.world.enable::<Simulated>(entity);
        }
        entity
    }

    fn spawn_sphere(&mut self, slot: usize, target: Target) -> Entity {
        self.spawn(
            target,
            RigidBody {
                position: slot_pose(slot),
                linear_velocity: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                angular_velocity: Vec3::ZERO,
            },
            RigidBodyMass {
                inv_inertia: Mat3::IDENTITY,
                inv_mass: 1.0,
                restitution: 0.0,
                friction: 0.5,
            },
            Collider {
                shape: ColliderShape::Sphere {
                    radius: SPHERE_RADIUS,
                },
                layer: 1,
                mask: 1,
            },
            true,
        )
    }

    /// The floor and Jolt's pyramid, index for index, into PLAIN.
    fn spawn_floor_and_pyramid(&mut self) {
        self.spawn(
            Target::Plain,
            RigidBody {
                position: Vec3::new(0.0, -1.0, 0.0),
                linear_velocity: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                angular_velocity: Vec3::ZERO,
            },
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
        for i in 0..PYRAMID_HEIGHT {
            let lo = i / 2;
            let hi = PYRAMID_HEIGHT - (i + 1) / 2;
            for j in lo..hi {
                for k in lo..hi {
                    let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                    let position = Vec3::new(
                        -(PYRAMID_HEIGHT as f32) + BOX_SIZE * j as f32 + odd,
                        1.0 + (BOX_SIZE + BOX_SEPARATION) * i as f32,
                        -(PYRAMID_HEIGHT as f32) + BOX_SIZE * k as f32 + odd,
                    );
                    self.spawn(
                        Target::Plain,
                        RigidBody {
                            position,
                            linear_velocity: Vec3::ZERO,
                            rotation: Quat::IDENTITY,
                            angular_velocity: Vec3::ZERO,
                        },
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
                        true,
                    );
                }
            }
        }
    }

    /// Despawns the sphere at `slot` and spawns a fresh one at its pose into `target`.
    fn respawn(&mut self, slot: usize, target: Target) {
        let old = self.slots[slot];
        assert!(
            self.world.delete_entity(old),
            "construction: a churn sphere is live"
        );
        self.slots[slot] = self.spawn_sphere(slot, target);
    }

    /// Runs `marker_ops` once with the given inserts and removes.
    fn migrate(&mut self, insert: &[Entity], remove: &[Entity]) {
        {
            let plan = self.world.resource_mut::<MarkerPlan>();
            plan.insert.clear();
            plan.insert.extend_from_slice(insert);
            plan.remove.clear();
            plan.remove.extend_from_slice(remove);
        }
        self.marker_ops.run(&mut self.world);
    }

    /// Applies one step's structural change for `arm`.
    fn churn_step(&mut self, arm: Arm) {
        let even = self.step.is_multiple_of(2);
        self.step += 1;
        match arm {
            Arm::Stable => {}
            Arm::SwapChurn => {
                // The newest PLAIN sphere is PLAIN's last row, and the cursor never names
                // the slot respawned on the previous step, so the despawn swap-moves it.
                let slot = MARKED_SPHERES + self.cursor;
                self.respawn(slot, Target::Plain);
                self.cursor = (self.cursor + 1) % PLAIN_SPHERES;
            }
            Arm::ArchetypeShift => {
                let sphere = self.slots[0];
                if even {
                    self.migrate(&[], &[sphere]);
                } else {
                    self.migrate(&[sphere], &[]);
                }
            }
            Arm::FirstArchetypeSpawn => {
                let slot = MARKED_SPHERES + self.cursor;
                if even {
                    self.respawn(slot, Target::Marked);
                } else {
                    self.respawn(slot, Target::Plain);
                    self.cursor = (self.cursor + 1) % PLAIN_SPHERES;
                }
            }
            Arm::BurstDespawn => {
                let front: Vec<usize> = {
                    let walk = self.world.query::<(&RigidBody, &Marker), ()>();
                    walk.iter()
                        .take(BURST)
                        .map(|(body, _)| slot_of(body.position))
                        .collect()
                };
                for &slot in &front {
                    assert!(
                        self.world.delete_entity(self.slots[slot]),
                        "construction: a churn sphere is live"
                    );
                }
                for &slot in &front {
                    self.slots[slot] = self.spawn_sphere(slot, Target::Marked);
                }
            }
            Arm::BurstMigrate => {
                let movers: Vec<Entity> = self.slots[MARKED_SPHERES..].to_vec();
                debug_assert_eq!(movers.len(), BURST);
                if even {
                    self.migrate(&movers, &[]);
                } else {
                    self.migrate(&[], &movers);
                }
            }
        }
    }

    /// `(maps built, rows resolved by stage 2)` so far.
    fn counters(&self) -> (u64, u64) {
        let scratch = self.world.resource::<SolverScratch>();
        (scratch.row_remap_builds(), scratch.row_remap_searched())
    }

    /// Runs `RECEIPT_STEPS` churn steps, asserting each builds exactly the maps the arm
    /// implies, and returns what they built and searched.
    fn receipt(&mut self, arm: Arm) -> (u64, u64) {
        let (built0, searched0) = self.counters();
        for _ in 0..RECEIPT_STEPS {
            let before = self.counters().0;
            self.churn_step(arm);
            self.physics.run(&mut self.world);
            let built = self.counters().0 - before;
            let expected = u64::from(arm != Arm::Stable);
            assert_eq!(
                built,
                expected,
                "anti-vacuity: arm {} must build {expected} previous-row map per step, built {built}",
                arm.name()
            );
        }
        let (built1, searched1) = self.counters();
        (built1 - built0, searched1 - searched0)
    }
}

fn bench_row_identity_churn(c: &mut Criterion) {
    let mut group = c.benchmark_group("row_identity_churn");
    group.sample_size(20);
    for sleeping in [false, true] {
        let label = if sleeping {
            "sleeping_on"
        } else {
            "sleeping_off"
        };
        for arm in Arm::ALL {
            let mut churn = Churn::new(sleeping);
            for _ in 0..SETTLE_STEPS {
                churn.physics.run(&mut churn.world);
            }
            let (built, searched) = churn.receipt(arm);
            eprintln!(
                "row_identity_churn/{}/{label}: {RECEIPT_STEPS} churn steps built {built} maps and \
                 resolved {searched} rows by stage 2",
                arm.name()
            );
            group.bench_function(BenchmarkId::new(arm.name(), label), |b| {
                b.iter_custom(|iters| {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        churn.churn_step(arm);
                        let start = Instant::now();
                        churn.physics.run(black_box(&mut churn.world));
                        total += start.elapsed();
                    }
                    total
                });
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench_row_identity_churn);
criterion_main!(benches);
