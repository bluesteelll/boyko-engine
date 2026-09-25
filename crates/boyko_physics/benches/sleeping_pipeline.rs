//! Sleeping pipeline: what defect A4's wake-on-contact-change costs on the REAL colored
//! physics schedule (docs/MEASUREMENT-QUEUE.md §8).
//!
//! Every sample times exactly one `Schedule::run` over Jolt's height-15 pyramid (1240
//! dynamic boxes on a static floor) on a serial pool.
//!
//! # Arms
//!
//! * `pyramid_sleeping_off` — the pyramid laid out exactly touching (separation 0), gravity
//!   on, sleeping off. `begin_step` does not run, so a change here is a leak, not a cost.
//! * `pyramid_awake_sleeping_on` — Jolt's pyramid (separation 0.5), gravity on, sleeping on
//!   with `sleep_threshold = 0`, so no island ever latches: the key loop's store-only path.
//! * `pyramid_frozen_sleeping_on` — the exactly touching pyramid with gravity OFF and the
//!   default sleep threshold and debounce, stepped until every dynamic row is frozen. Every
//!   contact then has separation exactly 0 and every velocity is exactly 0, so no body moves
//!   and no island's manifold count changes: the pile latches on the step its debounce
//!   completes, with or without the wake, and freezes in its spawn pose. The frozen state is
//!   therefore identical on a tree without A4, which is what lets the two trees' frozen
//!   steps be compared (round-2 critique W3). With gravity on, a pile of this height never
//!   came to rest before the A7 lane (defect A7), and a tree with the wake froze a different
//!   contact set from one without it; that holds on both of §8's arms, which predate the
//!   lane. A7a (`08fe7b9f`, clipped points carry their own feature ids) did not change it.
//!   Since A7b (S5, the face-versus-edge rule, the commit after `08fe7b9f`) the gravity-on
//!   pile does come to rest: A7-R2 freezes it at step 248 (msvc release, 2026-09-18; step 250
//!   since L9 C4 turned contact reuse on by default, 2026-09-24). The arm stays gravity-free
//!   all the same, because moving it onto a tree with A7a or A7b also
//!   moves the contact set it prices (MEASUREMENT-QUEUE §6). The setup asserts the freeze
//!   step, that no pose moved, and, on a tree with A4, that no contact-change wake fired.
//!
//! # Anti-vacuity of the frozen arm (round-3 review W2)
//!
//! A frozen CONTACT pile and a contact-FREE pile are indistinguishable to the three checks
//! above: with no manifold at all every dynamic row is its own island, `begin_step` stores
//! key 0 on every step so `contact_wakes` stays 0, the debounce still completes so the
//! freeze step is unchanged, and with gravity off nothing moves so the poses are trivially
//! identical. The pile's contacts all sit at separation exactly 0, so one narrowphase
//! epsilon (`box_box.rs` keeps a candidate axis on `depth >= 0` and a point on `separation
//! <= 0`) is the whole distance to the empty state, and the arm would go on pricing it while
//! §8's cross-tree comparison read a state the argument above does not describe. The setup
//! therefore reads the step's STRUCTURE too and puts it in the receipt: the solver manifold
//! count (asserted non-zero) and the island count, which is 1 for the connected pile and one
//! per dynamic row for a contact-free one. Both numbers are printed so the second tree's run
//! is compared on them, not only on the freeze step.
//!
//! # Running it on a tree without A4 (arm A of §8)
//!
//! `IslandSleep::contact_wakes` does not exist there: replace the body of this file's
//! `contact_wakes` with `0`. It is the only A4-only code.
//!
//! The structural checks run in criterion's test mode too:
//! `cargo test --release -p boyko-physics --bench sleeping_pipeline`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};
use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, Criterion, black_box, criterion_group, criterion_main};

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{
    ConstraintGraph, IslandSleep, Manifolds, PhysicsConfig, SolverScratch,
};

/// The fixed step (60 Hz).
const DT: f32 = 1.0 / 60.0;
/// Jolt's `cBoxSize`.
const BOX_SIZE: f32 = 2.0;
/// Jolt's `cHalfBoxSize`.
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
/// Jolt's `cBoxSeparation`.
const JOLT_SEPARATION: f32 = 0.5;
/// Jolt's `cPyramidHeight`: 1240 boxes.
const PYRAMID_HEIGHT: i32 = 15;
/// The floor: a static box centred at `(0, -1, 0)`, top face at `y = 0`.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
/// Warm-up steps before the awake arms are timed.
const WARM_STEPS: usize = 30;
/// The frozen arm's setup fails if the pile has not frozen by this step.
const FREEZE_LIMIT: usize = 3000;

/// Views a `#[repr(C)]` POD component as its bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, initialised `#[repr(C)]` POD component borrowed for the
    // returned slice's lifetime; the slice covers exactly its `size_of::<T>()` bytes,
    // read-only, which is the layout the component pool stores.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A single-worker pool: the price of the sleep bookkeeping, not of dispatch.
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// One scene: the world, its physics schedule and the pyramid's entities.
struct Pile {
    world: EcsMaster,
    physics: Schedule,
    boxes: Vec<Entity>,
}

impl Pile {
    /// The floor and Jolt's pyramid with layer gap `separation`, stepped by the real colored
    /// schedule under `gravity`, with sleeping `sleeping` and threshold `threshold`.
    fn new(separation: f32, gravity: Vec3, sleeping: bool, threshold: Option<f32>) -> Self {
        let mut world = EcsMaster::new();
        let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
        let mut builder = ScheduleBuilder::new(serial_pool());
        let _keys = add_physics_colored_solve(&mut builder, &mut world);
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        let physics = builder.build(&mut world);
        {
            let cfg = world.resource_mut::<PhysicsConfig>();
            cfg.gravity = gravity;
            cfg.dt = DT;
            cfg.sleeping = sleeping;
            if let Some(threshold) = threshold {
                cfg.sleep_threshold = threshold;
            }
        }

        let spawn = |world: &mut EcsMaster, position: Vec3, dynamic: bool| -> Entity {
            let body = RigidBody {
                position,
                linear_velocity: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                angular_velocity: Vec3::ZERO,
            };
            let (mass, half_extents) = if dynamic {
                (
                    RigidBodyMass {
                        inv_inertia: Mat3::from_diagonal(Vec3::new(0.1875, 0.1875, 0.1875)),
                        inv_mass: 0.125,
                        restitution: 0.0,
                        friction: 0.5,
                    },
                    Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
                )
            } else {
                (
                    RigidBodyMass {
                        inv_inertia: Mat3::ZERO,
                        inv_mass: 0.0,
                        restitution: 0.0,
                        friction: 0.5,
                    },
                    FLOOR_HALF_EXTENTS,
                )
            };
            let collider = Collider {
                shape: ColliderShape::Box { half_extents },
                layer: 1,
                mask: 1,
            };
            let entity = world
                .create_entity(
                    archetype,
                    &[
                        (RigidBody::component_id(), as_bytes(&body)),
                        (RigidBodyMass::component_id(), as_bytes(&mass)),
                        (Collider::component_id(), as_bytes(&collider)),
                    ],
                )
                .expect("construction: the body archetype accepts every column");
            if dynamic {
                world.enable::<Simulated>(entity);
            }
            entity
        };

        spawn(&mut world, Vec3::new(0.0, -1.0, 0.0), false);
        let mut boxes = Vec::with_capacity(1240);
        for i in 0..PYRAMID_HEIGHT {
            let lo = i / 2;
            let hi = PYRAMID_HEIGHT - (i + 1) / 2;
            for j in lo..hi {
                for k in lo..hi {
                    let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                    let position = Vec3::new(
                        -(PYRAMID_HEIGHT as f32) + BOX_SIZE * j as f32 + odd,
                        1.0 + (BOX_SIZE + separation) * i as f32,
                        -(PYRAMID_HEIGHT as f32) + BOX_SIZE * k as f32 + odd,
                    );
                    boxes.push(spawn(&mut world, position, true));
                }
            }
        }
        assert_eq!(
            boxes.len(),
            1240,
            "construction: Jolt's pyramid holds 1240 boxes"
        );
        Self {
            world,
            physics,
            boxes,
        }
    }

    fn step(&mut self) {
        self.physics.run(black_box(&mut self.world));
    }

    /// Whether any dynamic row was awake on the last step.
    fn any_dynamic_awake(&self) -> bool {
        let sleep = self.world.resource::<IslandSleep>();
        self.world
            .resource::<SolverScratch>()
            .bodies()
            .iter()
            .enumerate()
            .any(|(row, b)| b.inv_mass != 0.0 && sleep.is_row_awake(row))
    }

    /// The last step's solver manifold count — the structural half of the frozen arm's
    /// receipt. Sleeping skips only the solve and the integrate, so a frozen pile still
    /// runs a full narrowphase and a contact pile still reports its contacts.
    fn manifold_count(&self) -> usize {
        self.world.resource::<Manifolds>().manifolds().len()
    }

    /// The last step's island count. Every dynamic row gets an island id, so a pile with no
    /// manifold at all reports one island per box and a contact-connected pile reports one.
    fn island_count(&self) -> u32 {
        self.world.resource::<ConstraintGraph>().n_islands()
    }

    /// Every box's position, as bits.
    fn position_bits(&self) -> Vec<[u32; 3]> {
        self.boxes
            .iter()
            .map(|&entity| {
                let p = self
                    .world
                    .get_component::<RigidBody>(entity)
                    .expect("construction: a pyramid box is live")
                    .position;
                [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]
            })
            .collect()
    }
}

/// The contact-change wakes so far. A4-only: on a tree without A4 the body is `0`.
fn contact_wakes(pile: &Pile) -> u64 {
    pile.world.resource::<IslandSleep>().contact_wakes()
}

/// The frozen arm's scene, stepped until every dynamic row is frozen, with its structural
/// receipt printed and checked.
fn frozen_pile() -> Pile {
    let mut pile = Pile::new(0.0, Vec3::ZERO, true, None);
    let spawned = pile.position_bits();
    let frames = usize::from(pile.world.resource::<PhysicsConfig>().sleep_frames);
    let mut freeze_step = None;
    for step in 1..=FREEZE_LIMIT {
        pile.step();
        if !pile.any_dynamic_awake() {
            freeze_step = Some(step);
            break;
        }
    }
    let freeze_step = freeze_step.unwrap_or_else(|| {
        panic!(
            "setup: the gravity-free exactly touching pyramid did not freeze within \
             {FREEZE_LIMIT} steps; this arm prices a frozen step and cannot run"
        )
    });
    let manifolds = pile.manifold_count();
    let islands = pile.island_count();
    eprintln!(
        "sleeping_pipeline/pyramid_frozen_sleeping_on: froze at step {freeze_step}, manifolds \
         {manifolds}, islands {islands}"
    );
    let wakes = contact_wakes(&pile);
    eprintln!("sleeping_pipeline/pyramid_frozen_sleeping_on: contact_wakes {wakes}");
    assert!(
        manifolds > 0,
        "setup: the frozen pile reported {manifolds} manifolds over {islands} islands, so it \
         holds no contact and this arm would price an empty scene: every check below passes on a \
         contact-free pile (no manifold means no island change, so `contact_wakes` stays 0; the \
         debounce still completes, so the freeze step is unchanged; gravity is off, so no pose \
         moves). The pile's contacts sit at separation exactly 0, so a narrowphase epsilon is the \
         whole distance to this state - re-read `box_box.rs`'s candidate-axis and point \
         predicates before touching this arm"
    );
    assert_eq!(
        wakes, 0,
        "setup: a contact-change wake fired, so this is not the frozen state a tree without A4 \
         reaches"
    );
    assert_eq!(
        freeze_step,
        frames + 1,
        "setup: the pile must latch on the step its {frames}-step debounce completes, as it does \
         without the wake"
    );
    assert!(
        pile.position_bits() == spawned,
        "setup: no box may move in a gravity-free exactly touching pyramid, or the two trees \
         freeze different states"
    );
    pile
}

/// Times one `Schedule::run` of `pile` per iteration.
fn bench_steps(group: &mut BenchmarkGroup<'_, WallTime>, name: &str, pile: &mut Pile) {
    group.bench_function(name, |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let start = Instant::now();
                pile.step();
                total += start.elapsed();
            }
            total
        });
    });
}

fn bench_sleeping_pipeline(c: &mut Criterion) {
    let mut off = Pile::new(0.0, Vec3::new(0.0, -9.81, 0.0), false, None);
    let mut awake = Pile::new(JOLT_SEPARATION, Vec3::new(0.0, -9.81, 0.0), true, Some(0.0));
    let mut frozen = frozen_pile();
    for _ in 0..WARM_STEPS {
        off.step();
        awake.step();
    }
    assert!(
        awake.any_dynamic_awake(),
        "setup: with a zero threshold no island may latch, so the awake arm's pile is awake"
    );

    let mut group = c.benchmark_group("sleeping_pipeline");
    group.sample_size(20);
    bench_steps(&mut group, "pyramid_sleeping_off", &mut off);
    bench_steps(&mut group, "pyramid_awake_sleeping_on", &mut awake);
    bench_steps(&mut group, "pyramid_frozen_sleeping_on", &mut frozen);
    group.finish();

    assert!(
        awake.any_dynamic_awake() && !frozen.any_dynamic_awake(),
        "the sleeping-on arms must keep their sleep state while measured: awake arm awake {}, \
         frozen arm awake {}",
        awake.any_dynamic_awake(),
        frozen.any_dynamic_awake()
    );
}

criterion_group!(benches, bench_sleeping_pipeline);
criterion_main!(benches);
