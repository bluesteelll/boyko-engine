//! Shared by the two profiling validation binaries (`profiling_zone_counts`,
//! `profiling_bit_identity`): a libtest-shaped `main` for a one-test binary, the scene both run,
//! and the profiler plumbing they read it through.
//!
//! # Why each validation test is its own process
//!
//! Everything a profiled step touches is process-wide: the arm mask, the lane rings, each zone
//! handle's accumulators, and the world the profiler is bound to. The bind is once per process —
//! `bind_world` refuses a second world, and a test outside `boyko_ecs` cannot reach the kernel's
//! test-only unbind or its `exclusive()` lock. Under libtest the physics suite's other tests would
//! run beside these on other threads, inflating the exact counts or arming the "disarmed" arm. So
//! each test is the only thing in its binary (`harness = false`), and each binary binds exactly
//! one world: the measured one.
//!
//! # The scene
//!
//! [`FLOOR_BOXES`] bodies on a static floor whose top face is at `y = 0`, laid out on a grid whose
//! pitch keeps every neighbour out of contact, each sunk [`SINK`] into the floor so it has a
//! contact on the first step. Most are unit boxes (half-extent [`HALF_BOX`]); every
//! [`STACK_EVERY`]-th is a slab twice as long in `x` carrying two unit boxes side by side, each
//! sunk [`SINK`] into its top face and clear of the other. On the real `add_physics_colored_solve`
//! schedule over a [`WORKERS`]-worker pool with `parallel_solve` and `parallel_narrowphase` on,
//! and the tree broadphase forced onto its tree path (`BroadphaseKind::Tree`,
//! `brute_max_rows = 0`), so the four `phys_bp_*` spans and the three `phys_bp_*` structural
//! counters open on every step — `profiling_bit_identity` requires every zone to open, and
//! `profiling_zone_counts` pins their structural values (the floor is the one static: pending
//! at step 1, admitted at step 2, a member from then on).
//!
//! The grid is [`FLOOR_ROWS`] rows deep so the step has about 300 candidate pairs: at least two
//! narrowphase chunks of `NP_MIN_PAIRS_PER_CHUNK` (128), so the parallel narrowphase dispatches on
//! every step and its three zones open. At the nine rows the scene had before L5 it made 135 pairs,
//! one chunk, and ran the serial loop.
//!
//! The coloring is first-fit in manifold order, and the rows are spawned so the order is fixed:
//!
//! * the floor manifolds share no dynamic body, so they all take color 0: 4 slots each, far above
//!   the solver's inline floor of 256 — a WIDE color, which a pile small enough for a debug build
//!   does not otherwise have;
//! * each slab's first top box shares the slab with its floor manifold, so it takes color 1;
//! * its second top box shares the slab with both, so it takes color 2.
//!
//! One wide color and two narrow ones. The counts are deliberately UNEQUAL: with one of each, a
//! defect that swapped the two classes would leave both span counts where they were. The tests do
//! not rely on this layout beyond that: they recompute the classes from the constraint graph each
//! step and assert both are present with different counts.

use std::panic;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use boyko_diag::profiling_abi::{ZoneHandle, zone_id};
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::profiling::{
    ArmOutcome, LifetimeAcc, Profiler, ProfilerConfig, bind_world, fold_frame,
};
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::broadphase_tree::BroadphaseTree;
use boyko_physics::plugin::add_physics_colored_solve;
use boyko_physics::resources::{BroadphaseKind, PhysicsConfig};

// ── Scene constants ──────────────────────────────────────────────────────────

/// The fixed step (60 Hz).
pub const DT: f32 = 1.0 / 60.0;
/// Every dynamic box's half-extent.
pub const HALF_BOX: f32 = 0.5;
/// Bodies standing on the floor, as a `FLOOR_COLS × FLOOR_ROWS` grid.
pub const FLOOR_BOXES: usize = FLOOR_COLS * FLOOR_ROWS;
/// Grid columns.
const FLOOR_COLS: usize = 10;
/// Grid rows: twenty, so every step has at least two narrowphase chunks' worth of pairs (see the
/// module docs).
const FLOOR_ROWS: usize = 20;
/// Centre-to-centre spacing: a slab's bounding radius (≈ 1.39) plus a box's (≈ 0.87) is below it,
/// so no two neighbours are even broadphase candidates.
const PITCH: f32 = 3.0;
/// A slab's half-extent in `x`; its other two are [`HALF_BOX`].
const SLAB_HALF_X: f32 = 1.2;
/// Where a slab's two top boxes sit, as `±` this offset in `x` from its centre: 0.2 of clearance
/// between them, and each wholly on the slab.
const TOP_OFFSET_X: f32 = 0.6;
/// How far each box is sunk into what it rests on, so it has a contact on the first step.
pub const SINK: f32 = 0.01;
/// Every `STACK_EVERY`-th floor body is a slab carrying two boxes.
pub const STACK_EVERY: usize = 10;
/// Worker threads in the scene's pool.
pub const WORKERS: usize = 4;

/// Watchdog budget for the one test a binary runs. A panic inside a scheduler worker does not
/// propagate out of `Schedule::run` today, it blocks the caller; the watchdog turns that into a
/// failure instead of a hung run.
const TIMEOUT: Duration = Duration::from_secs(if cfg!(debug_assertions) { 300 } else { 120 });

// ── The one-test runner ──────────────────────────────────────────────────────

/// The subset of libtest's command line a runner sends to a one-test binary. Anything else that
/// starts with `-` is accepted and ignored, as libtest's own no-op flags are.
#[derive(Default)]
struct Cli {
    list: bool,
    ignored_only: bool,
    exact: bool,
    filters: Vec<String>,
    skips: Vec<String>,
}

impl Cli {
    fn from_env() -> Self {
        let mut cli = Cli::default();
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--list" => cli.list = true,
                "--ignored" => cli.ignored_only = true,
                "--exact" => cli.exact = true,
                "--skip" => cli.skips.extend(args.next()),
                // libtest's flags that take their value as the NEXT argument: consume it, or it
                // would be read as a name filter.
                "--test-threads" | "--color" | "--format" | "--logfile" | "--shuffle-seed"
                | "-Z" => {
                    let _ = args.next();
                }
                flag if flag.starts_with('-') => {}
                filter => cli.filters.push(filter.to_owned()),
            }
        }
        cli
    }

    /// libtest's selection rule for one non-ignored test.
    fn selects(&self, name: &str) -> bool {
        let hit = |pat: &String| {
            if self.exact {
                name == pat
            } else {
                name.contains(pat.as_str())
            }
        };
        !self.ignored_only
            && !self.skips.iter().any(hit)
            && (self.filters.is_empty() || self.filters.iter().any(hit))
    }
}

/// Runs `body` as the binary's only test, named `name`, with libtest's output contract:
/// `running 1 test`, `test <name> ... ok|FAILED`, the summary line, and exit code 101 on a
/// failure. `--list` lists it; a filter that excludes it prints `running 0 tests` and the
/// filtered-out count, so a vacuous run cannot read as a pass.
///
/// The body runs on its own thread under [`TIMEOUT`].
pub fn run_single_test(name: &'static str, body: fn()) {
    let cli = Cli::from_env();
    if cli.list {
        if cli.selects(name) {
            println!("{name}: test");
        }
        return;
    }
    if cfg!(miri) || !cli.selects(name) {
        if cfg!(miri) {
            println!("{name}: not run under Miri — the schedule spins a boyko_threadpool");
        }
        println!(
            "\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; \
             1 filtered out\n"
        );
        return;
    }

    println!("\nrunning 1 test");
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            // The fold's own `__fold` span is pushed from this thread. A thread the pool did not
            // create has no lane until it claims one, and a push from no lane is counted as an
            // `Unclaimed` drop — which the tests assert is zero.
            let claimed = boyko_diag::lane::claim_lane().is_some();
            if !claimed {
                println!("{name}: no spare diagnostics lane to claim for the test thread");
            }
            let outcome = panic::catch_unwind(body);
            let _ = tx.send(claimed && outcome.is_ok());
        })
        .expect("harness: the test thread spawns");
    let passed = match rx.recv_timeout(TIMEOUT) {
        Ok(passed) => passed,
        Err(_) => {
            println!("{name}: no result within {TIMEOUT:?} — a hung schedule reads as a failure");
            false
        }
    };
    if passed {
        println!(
            "test {name} ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; \
             0 filtered out\n"
        );
    } else {
        println!(
            "test {name} ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; \
             0 measured; 0 filtered out\n"
        );
        std::process::exit(101);
    }
}

// ── The scene ────────────────────────────────────────────────────────────────

/// Views a `#[repr(C)]` POD value as its bytes for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, initialised `#[repr(C)]` `T` borrowed for the returned slice's
    // lifetime; the slice covers exactly its `size_of::<T>()` bytes read-only, the layout the
    // component pool stores for `T`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Spawns one box body at rest into the `RigidBodyBundle` archetype; a dynamic one
/// (`inv_mass != 0`) is enabled as `Simulated`.
fn spawn_box(
    world: &mut EcsMaster,
    position: Vec3,
    half_extents: Vec3,
    inv_mass: f32,
    inv_inertia: Mat3,
) -> Entity {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass { inv_inertia, inv_mass, restitution: 0.0, friction: 0.5 };
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
    if inv_mass != 0.0 {
        world.enable::<Simulated>(e);
    }
    e
}

/// A dynamic box of half-extent [`HALF_BOX`] and unit mass: a solid box's inverse inertia is
/// `12 / (m · (a² + b²))` on each axis, with every full extent 1.
fn spawn_dynamic_box(world: &mut EcsMaster, centre: Vec3) -> Entity {
    spawn_box(
        world,
        centre,
        Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
        1.0,
        Mat3::from_diagonal(Vec3::new(6.0, 6.0, 6.0)),
    )
}

/// A dynamic slab: [`SLAB_HALF_X`] × [`HALF_BOX`] × [`HALF_BOX`], mass 2, and the solid box's
/// inverse inertia for those extents (rounded; nothing here depends on its value).
fn spawn_slab(world: &mut EcsMaster, centre: Vec3) -> Entity {
    spawn_box(
        world,
        centre,
        Vec3::new(SLAB_HALF_X, HALF_BOX, HALF_BOX),
        0.5,
        Mat3::from_diagonal(Vec3::new(3.0, 0.9, 0.9)),
    )
}

/// One world on the real colored schedule, with the scene spawned.
pub struct Scene {
    /// The world.
    pub world: EcsMaster,
    /// The physics schedule `add_physics_colored_solve` wired.
    pub physics: Schedule,
    /// Every dynamic body (slabs included), in spawn order.
    pub boxes: Vec<Entity>,
}

impl Scene {
    /// Builds the scene with sleeping off, `parallel_solve` and `parallel_narrowphase` on, the
    /// tree broadphase forced onto its tree path, dt = [`DT`] and gravity (0, -9.81, 0). Nothing
    /// else in the configuration is touched.
    pub fn spawn() -> Self {
        let mut world = EcsMaster::new();
        let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(WORKERS).build());
        let _keys = add_physics_colored_solve(&mut builder, &mut world);
        world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
        let physics = builder.build(&mut world);
        {
            let cfg = world.resource_mut::<PhysicsConfig>();
            cfg.sleeping = false;
            cfg.parallel_solve = true;
            cfg.parallel_narrowphase = true;
            cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
            cfg.dt = DT;
            cfg.broadphase = BroadphaseKind::Tree;
        }
        world.resource_mut::<BroadphaseTree>().set_brute_max_rows(0);

        spawn_box(
            &mut world,
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(40.0, 1.0, 40.0),
            0.0,
            Mat3::ZERO,
        );
        let mut boxes = Vec::with_capacity(FLOOR_BOXES + 2 * FLOOR_BOXES / STACK_EVERY + 2);
        let half_span_x = PITCH * (FLOOR_COLS - 1) as f32 * 0.5;
        let half_span_z = PITCH * (FLOOR_ROWS - 1) as f32 * 0.5;
        for k in 0..FLOOR_BOXES {
            let x = PITCH * (k % FLOOR_COLS) as f32 - half_span_x;
            let z = PITCH * (k / FLOOR_COLS) as f32 - half_span_z;
            let bottom = HALF_BOX - SINK;
            if k % STACK_EVERY == 0 {
                // Slab first, then its two boxes: the rows, and so the manifold order the
                // first-fit coloring walks, are the spawn order.
                boxes.push(spawn_slab(&mut world, Vec3::new(x, bottom, z)));
                let top = bottom + 2.0 * HALF_BOX - SINK;
                for dx in [-TOP_OFFSET_X, TOP_OFFSET_X] {
                    boxes.push(spawn_dynamic_box(&mut world, Vec3::new(x + dx, top, z)));
                }
            } else {
                boxes.push(spawn_dynamic_box(&mut world, Vec3::new(x, bottom, z)));
            }
        }
        Self { world, physics, boxes }
    }

    /// One physics step.
    pub fn step(&mut self) {
        self.physics.run(&mut self.world);
    }

    /// Turns sleeping on or off for the following steps.
    pub fn set_sleeping(&mut self, on: bool) {
        self.world.resource_mut::<PhysicsConfig>().sleeping = on;
    }

    /// Selects the broadphase for the following steps.
    //
    // `dead_code`: the harness is compiled into every test binary that includes it, and only
    // `profiling_zone_counts` calls this.
    #[allow(dead_code)]
    pub fn set_broadphase(&mut self, kind: BroadphaseKind) {
        self.world.resource_mut::<PhysicsConfig>().broadphase = kind;
    }

    /// The gathered row count of the last step.
    //
    // `dead_code`: as for `set_broadphase`.
    #[allow(dead_code)]
    pub fn rows(&self) -> u64 {
        self.world.resource::<boyko_physics::resources::SolverScratch>().bodies_len() as u64
    }

    /// Binds the process's profiler to this world, gives the world the store, and arms it.
    ///
    /// # Panics
    ///
    /// When another world of this process already holds the binding: each validation binary
    /// binds exactly one world, and a second would be refused by the kernel (`E9204`).
    pub fn arm_profiler(&mut self) {
        bind_world(self.world.world_id().get())
            .expect("harness: this binary binds exactly one world, the measured one");
        self.world.insert_resource(Profiler::new());
        let outcome = self.world.resource_mut::<Profiler>().arm(ProfilerConfig::default());
        assert_eq!(outcome, ArmOutcome::Armed, "harness: the first arm of the process");
    }

    /// Drains every lane into the store: after a step, that step's samples are all folded.
    pub fn fold(&mut self) {
        fold_frame(&mut self.world);
    }

    /// The store.
    pub fn profiler(&self) -> &Profiler {
        self.world.resource::<Profiler>()
    }

    /// Zone `id`'s whole-session accumulator.
    pub fn lifetime_of(&self, id: u16) -> LifetimeAcc {
        self.profiler()
            .lifetime(id)
            .expect("harness: every zone id of this process is inside the armed geometry")
    }

    /// `handle`'s whole-session accumulator.
    pub fn lifetime(&self, handle: &'static ZoneHandle) -> LifetimeAcc {
        self.lifetime_of(zone_id(handle))
    }
}
