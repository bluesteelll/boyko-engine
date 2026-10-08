//! S5 C3b: `phys_bp_query`'s in-zone canary — the routing and the receipt
//! (`boyko_physics::profiling`'s module docs, "The in-zone canary"; the parity runner's
//! `--canary-zone phys_bp_query`).
//!
//! * [`bp_query_canary_routing`]: `CANARY_ZONES` lists `phys_bp_query`, and every zone of the list
//!   is accepted by exactly one instrument, the one `canary_holder` names — the tree broadphase
//!   takes `phys_bp_query` alone and the colored solver the solve build's five, so the runner's
//!   route (by `canary_holder`) never sets a canary where it cannot run. A zone outside the list
//!   has no holder and is refused by both.
//! * [`bp_query_canary_never_spins_disarmed`]: a world whose tree holds the canary, stepped while
//!   nothing in the process has ever armed, never spins — on the tree path, the S5 dispatch
//!   included. Nothing in this binary's own process arms: every armed run below is a child.
//! * [`bp_query_canary_receipt_w1`], [`bp_query_canary_receipt_w8_serial`],
//!   [`bp_query_canary_receipt_w8_dispatched`]: the receipt, armed, each in its own child process
//!   (the profiler binds one world per process). G3's stacks on a W-worker pool with the canary
//!   at [`GATE_NS`]: every step opens `phys_bp_query` once and its span reads at least the spin;
//!   the tree counts exactly one spin per opening; at W8 with `parallel_tree_query` on, the query
//!   dispatched on every step after the first (the spin sits before the dispatch, inside the
//!   zone), and on none otherwise; the store dropped no sample.
//!
//! # The span bound
//!
//! The span is read in ticks over the clock's calibrated scale and the spin is timed by
//! `Instant`, so the bound is `(1 − 1/100) × GATE_NS` per opening: the 1 % absorbs the scale's
//! calibration error. A correct build cannot fail it under load (a preemption only lengthens the
//! span around the spin). A spin outside the zone leaves the span at the query's own cost, which
//! is what the mutation "the canary called before the zone opens" reads (impl record).
//!
//! # How to run
//!
//! ```text
//! cargo test -p boyko-physics --test profiling_bp_query_canary
//! ```
//!
//! Counts and a structural span bound, not timings: it needs no quiet machine. Spins real thread
//! pools (intractable under Miri), so `cfg(not(miri))`.

#![cfg(not(miri))]

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use boyko_diag::clock::ticks_per_ns;
use boyko_diag::profiling_abi::{ZoneHandle, zone_id};
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::profiling::{
    ArmOutcome, LifetimeAcc, Profiler, ProfilerConfig, bind_world, fold_frame,
};
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_physics::broadphase_tree::{BroadphaseTree, S5_MIN_LEAVES};
use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems;
use boyko_physics::profiling::{
    CANARY_ZONES, CanaryHolder, PHYS_BP_ASSEMBLE, PHYS_BP_BUILD, PHYS_BP_QUERY, PHYS_BP_VERIFY,
    PHYS_GRAVITY, PHYS_STORE, ZONES_COMPILED, canary_holder,
};
use boyko_physics::resources::{BroadphaseKind, PhysicsConfig};
use boyko_physics::solver::{ColoredSoftStepSolver, DefaultRigidSolver};

/// Fixed timestep.
const DT: f32 = 1.0 / 60.0;

/// G3's grid: 13 × 13 two-box stacks, 338 dynamic boxes over 43 active leaf nodes with sleeping
/// off — above `S5_MIN_LEAVES`, so the switch-on W8 run dispatches, and above the tree's
/// `brute_max_rows`, so every step takes the tree path.
const GRID: usize = 13;

/// Steps per run.
const STEPS: u64 = 8;

/// The canary's busy-wait per opening: far above the query's own cost on this scene in a debug
/// build, so a spin that lands outside the zone cannot pass the span bound.
const GATE_NS: u64 = 20_000_000;

/// The child's scenario, by name, in this variable; absent in the parent.
const CHILD_ENV: &str = "BOYKO_C3B_CANARY_CHILD";

/// A child's wall-clock budget; past it the child is a hang and is killed.
const DEADLINE: Duration = Duration::from_secs(if cfg!(debug_assertions) { 600 } else { 180 });

/// The marker a child prints after its receipt holds, so a child that ran no test (a filter
/// that matched nothing) cannot pass as one whose receipt held.
const MARKER: &str = "C3B-CANARY-RECEIPT";

// ── The scene ────────────────────────────────────────────────────────────────

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its `size_of::<T>()` bytes
    // read-only for the duration of the borrow, which is the exact layout the component pool
    // stores.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Spawns one box: dynamic with unit mass when `inv_mass` is 1, static when 0.
fn spawn_box(world: &mut EcsMaster, position: Vec3, half_extents: Vec3, inv_mass: f32) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        // A unit cube of mass 1 has I = 1/6 on the diagonal.
        inv_inertia: if inv_mass == 0.0 {
            Mat3::ZERO
        } else {
            Mat3::from_diagonal(Vec3::new(6.0, 6.0, 6.0))
        },
        inv_mass,
        restitution: 0.0,
        friction: 0.5,
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
    if inv_mass != 0.0 {
        world.enable::<Simulated>(e);
    }
}

/// A static floor and [`GRID`] × [`GRID`] two-box stacks (`broadphase_tree_s5.rs`'s G3 scene),
/// each box overlapping what it rests on by 1 cm so every contact exists from the first step.
fn spawn_stacks(world: &mut EcsMaster) {
    spawn_box(world, Vec3::new(0.0, -0.5, 0.0), Vec3::new(40.0, 0.5, 40.0), 0.0);
    let half = Vec3::new(0.5, 0.5, 0.5);
    let origin = -0.75 * (GRID as f32 - 1.0);
    for i in 0..GRID {
        for k in 0..GRID {
            let x = origin + 1.5 * i as f32;
            let z = origin + 1.5 * k as f32;
            spawn_box(world, Vec3::new(x, 0.49, z), half, 1.0);
            spawn_box(world, Vec3::new(x, 1.48, z), half, 1.0);
        }
    }
}

/// The default world over the stacks on a `workers`-wide pool, sleeping off (the parity rows'
/// setting) and S5's switch `dispatched`, with the tree's canary at [`GATE_NS`] on
/// `phys_bp_query`.
fn canary_world(workers: usize, dispatched: bool) -> (EcsMaster, Schedule) {
    let mut world = EcsMaster::new();
    spawn_stacks(&mut world);
    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
    let schedule = builder.build(&mut world);
    {
        let cfg = world.resource_mut::<PhysicsConfig>();
        assert_eq!(cfg.broadphase, BroadphaseKind::Tree, "construction: the default world runs the tree");
        cfg.sleeping = false;
        cfg.parallel_tree_query = dispatched;
    }
    assert!(
        world.resource_mut::<BroadphaseTree>().set_zone_canary(&PHYS_BP_QUERY, GATE_NS),
        "C3b: the tree refused the canary on its own zone, phys_bp_query"
    );
    (world, schedule)
}

// ── The routing ──────────────────────────────────────────────────────────────

/// Every zone of `CANARY_ZONES` is accepted by exactly its holder; `phys_bp_query` is listed and
/// held by the tree; a zone outside the list has no holder and is refused by both.
#[test]
fn bp_query_canary_routing() {
    assert!(
        CANARY_ZONES.iter().any(|&h| std::ptr::eq(h, &PHYS_BP_QUERY)),
        "C3b: phys_bp_query is not one of CANARY_ZONES, so the runner refuses --canary-zone phys_bp_query"
    );
    assert_eq!(
        canary_holder(&PHYS_BP_QUERY),
        Some(CanaryHolder::Tree),
        "C3b: phys_bp_query's canary must be the tree broadphase's, whose step opens the zone"
    );
    let mut tree = BroadphaseTree::default();
    let mut solver = ColoredSoftStepSolver::default();
    let mut held = [0usize; 2];
    for &zone in &CANARY_ZONES {
        let holder = canary_holder(zone).expect("invariant: every zone of CANARY_ZONES has a holder");
        held[usize::from(holder == CanaryHolder::Tree)] += 1;
        let accepted = (tree.set_zone_canary(zone, GATE_NS), solver.set_zone_canary(zone, GATE_NS));
        assert_eq!(
            accepted,
            (holder == CanaryHolder::Tree, holder == CanaryHolder::Solver),
            "`{}`: (tree, solver) accepted {accepted:?}; only its holder {holder:?} may",
            zone.desc.name
        );
    }
    assert_eq!(held, [5, 1], "the solver holds the solve build's five zones and the tree one");
    let outside: [&'static ZoneHandle; 5] =
        [&PHYS_BP_VERIFY, &PHYS_BP_BUILD, &PHYS_BP_ASSEMBLE, &PHYS_GRAVITY, &PHYS_STORE];
    for zone in outside {
        assert_eq!(canary_holder(zone), None, "`{}` is not a canary zone", zone.desc.name);
        assert!(
            !tree.set_zone_canary(zone, GATE_NS) && !solver.set_zone_canary(zone, GATE_NS),
            "`{}` is not a canary zone, and both instruments must refuse it",
            zone.desc.name
        );
    }
    // A refusal changes nothing, and `ns == 0` removes the canary: no step ran, so no spin.
    assert!(tree.set_zone_canary(&PHYS_BP_QUERY, 0));
    assert_eq!(tree.zone_canary_spins(), 0);
}

/// A disarmed `phys_bp_query` never spins: the canary sits in the zone's admitted arm.
#[test]
fn bp_query_canary_never_spins_disarmed() {
    let (mut world, mut schedule) = canary_world(2, true);
    let diag_before = world.resource::<BroadphaseTree>().diag();
    for _ in 0..3 {
        schedule.run(&mut world);
    }
    let tree = world.resource::<BroadphaseTree>();
    let diag = tree.diag();
    assert!(
        diag.leaf_list_leaves + diag.fallback_leaves > diag_before.leaf_list_leaves + diag_before.fallback_leaves,
        "construction: the tree path ran its query"
    );
    assert!(tree.query_dispatches() >= 1, "construction: the S5 dispatch ran inside the zone");
    assert_eq!(
        tree.zone_canary_spins(),
        0,
        "C3b: the canary spun on a disarmed phys_bp_query (it must sit in the zone's admitted arm)"
    );
}

// ── The receipt ──────────────────────────────────────────────────────────────

/// The armed receipt on W1: the serial query.
#[test]
fn bp_query_canary_receipt_w1() {
    receipt("bp_query_canary_receipt_w1", 1, false);
}

/// The armed receipt on W8 with S5's switch off: the serial query on a worker pool.
#[test]
fn bp_query_canary_receipt_w8_serial() {
    receipt("bp_query_canary_receipt_w8_serial", 8, false);
}

/// The armed receipt on W8 with S5's switch on: the dispatched query inside the zone.
#[test]
fn bp_query_canary_receipt_w8_dispatched() {
    receipt("bp_query_canary_receipt_w8_dispatched", 8, true);
}

/// In the child (`CHILD_ENV` names `test_name`), runs the armed receipt; in the parent, re-runs
/// this binary on `test_name` alone as that child and requires its success and its marker.
fn receipt(test_name: &'static str, workers: usize, dispatched: bool) {
    if std::env::var(CHILD_ENV).as_deref() == Ok(test_name) {
        armed_receipt(test_name, workers, dispatched);
        return;
    }
    let exe = std::env::current_exe().expect("test setup: this test binary's own path");
    let mut child = Command::new(exe)
        .args(["--exact", test_name, "--nocapture", "--test-threads", "1"])
        .env(CHILD_ENV, test_name)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("test setup: re-exec of this test binary");
    // The pipes are drained on their own threads, so a child that fills one cannot block.
    let mut out = child.stdout.take().expect("test setup: the child's stdout is piped");
    let mut err = child.stderr.take().expect("test setup: the child's stderr is piped");
    let out_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait().expect("test setup: poll the child") {
            Some(status) => break Some(status),
            None if start.elapsed() >= DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    let marker = format!("{MARKER} {test_name}");
    assert!(
        status.is_some_and(|s| s.success()) && stdout.contains(&marker),
        "{test_name}: the child {} (marker {:?} {}):\n--- stdout\n{stdout}\n--- stderr\n{stderr}",
        match status {
            Some(s) => format!("exited {s}"),
            None => format!("ran past {DEADLINE:?} and was killed"),
        },
        marker,
        if stdout.contains(&marker) { "printed" } else { "missing" },
    );
}

/// The child's run: binds and arms this process's profiler to the canary world, steps it
/// [`STEPS`] times folding after each, and checks the receipt step by step.
fn armed_receipt(test_name: &str, workers: usize, dispatched: bool) {
    // The fold's own span is pushed from this thread, and so may the broadphase's be when the
    // schedule runs a system on the calling thread: a push from a thread without a lane is an
    // `Unclaimed` drop.
    assert!(boyko_diag::lane::claim_lane().is_some(), "test setup: a spare diagnostics lane for the test thread");
    let (mut world, mut schedule) = canary_world(workers, dispatched);
    bind_world(world.world_id().get()).expect("test setup: this child binds exactly one world");
    world.insert_resource(Profiler::new());
    let outcome = world.resource_mut::<Profiler>().arm(ProfilerConfig::default());
    assert_eq!(outcome, ArmOutcome::Armed, "test setup: the child's first arm");
    let tpn = ticks_per_ns();
    let id = zone_id(&PHYS_BP_QUERY);
    let lifetime = |world: &EcsMaster| -> LifetimeAcc {
        world
            .resource::<Profiler>()
            .lifetime(id)
            .expect("test setup: phys_bp_query's id is inside the armed geometry")
    };
    let bound = GATE_NS as f64 * (1.0 - 1.0 / 100.0);
    let dispatches_before = world.resource::<BroadphaseTree>().query_dispatches();
    let mut before = lifetime(&world);
    let mut openings = 0u64;
    for step in 0..STEPS {
        schedule.run(&mut world);
        fold_frame(&mut world);
        let now = lifetime(&world);
        let (k, ticks) = (now.count - before.count, now.total.wrapping_sub(before.total));
        before = now;
        let want = u64::from(ZONES_COMPILED);
        assert_eq!(k, want, "{test_name} step {step}: phys_bp_query opened {k} times armed, the tree path opens it {want}");
        let span_ns = ticks as f64 / tpn;
        assert!(
            k == 0 || span_ns >= bound * k as f64,
            "{test_name} step {step}: phys_bp_query read {span_ns:.0} ns over {k} span(s), below the \
             canary's {GATE_NS} ns each (bound {bound:.0}): the spin is outside the zone or never ran"
        );
        openings += k;
    }
    let tree = world.resource::<BroadphaseTree>();
    let spins = tree.zone_canary_spins();
    assert_eq!(
        spins, openings,
        "{test_name}: the canary spun {spins} times over {openings} armed openings of phys_bp_query"
    );
    assert!(spins > 0 || !ZONES_COMPILED, "{test_name}: the canary never spun");
    let dispatches = tree.query_dispatches() - dispatches_before;
    let want_dispatches = if dispatched && workers >= 2 { STEPS - 1 } else { 0 };
    assert_eq!(
        dispatches, want_dispatches,
        "{test_name}: the query dispatched on {dispatches} of {STEPS} steps (switch {dispatched}, \
         W{workers}); the scene's {GRID}x{GRID} stacks are above S5_MIN_LEAVES = {S5_MIN_LEAVES}"
    );
    let drops = world.resource::<Profiler>().drops();
    assert_eq!(drops.total(), 0, "{test_name}: the store dropped samples: {drops:?}");
    println!("{MARKER} {test_name} openings={openings} spins={spins} dispatches={dispatches} compiled={ZONES_COMPILED}");
}
