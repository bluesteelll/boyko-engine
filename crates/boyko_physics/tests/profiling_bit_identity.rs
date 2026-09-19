//! `instrumented_step_is_bit_identical_armed_and_disarmed`: arming the profiler changes no bit of
//! the simulation, and the check is not vacuous — the armed run recorded samples on every physics
//! zone and every system, and the disarmed run recorded none.
//!
//! # Why the sample assertions are the test
//!
//! A zone writes no simulation data, so two runs of the same scene agree whether or not the zones
//! are armed, compiled, or wired at all: the pose comparison alone cannot fail. What makes it a
//! test of the instrument is its two companions. The armed run must have recorded at least one
//! sample on each of the twenty physics zones and on each system span — so the comparison was
//! made with the instrument live — and the disarmed run must have pushed nothing at all into any
//! lane — so the "disarmed" arm was not quietly armed.
//!
//! # The runs
//!
//! The shared scene (`support/profiling_harness.rs`: one wide color and two narrow ones, a
//! 4-worker pool, `parallel_solve` on), [`STEPS`] steps each: sleeping off for the first [`STEPS_OFF`]
//! (the parity configuration), then on, so the three sleep zones run too. The two worlds get the
//! same toggles at the same steps, and every dynamic box's full `RigidBody` is compared as bits
//! after every step.
//!
//! 1. **Disarmed**, first, while nothing in the process has ever armed (asserted): no world holds a
//!    profiler. Around the run, every lane region's pending and refused sample counts and every
//!    physics span zone's own interval count must not move. Before any arm no lane has a buffer,
//!    so a push that got past a gate would be refused and counted rather than vanish. The world
//!    and its pool are dropped before the armed run starts.
//! 2. **Armed**: the only world this process binds, folded after every step. Afterwards each
//!    physics zone's and each system's lifetime sample count is above zero, each physics span
//!    zone's own interval count rose, and the store dropped nothing.
//!
//! Under a profile whose tier folds the zones, the armed run's correct reading is zero on every
//! physics zone, and that is what is asserted instead.
//!
//! # Shown red (2026-09-19, msvc, debug), one per clause
//!
//! * gravity scaled by `1.000001` only while the gravity zone is admitted: box 0 differs on
//!   step 0 — the pose comparison can fail;
//! * the store's zone opened without its gate: the disarmed run pushed 100 samples (one refused
//!   push per step) — the disarmed clause can fail;
//! * the relax-pass zone deleted: "never opened `phys_pass_relax`"; the broadphase counter
//!   deleted: "no sample on `phys_bp_pairs`" — the armed clause can fail, for spans and counters.
//!
//! # How to run
//!
//! ```text
//! cargo test -p boyko-physics --test profiling_bit_identity
//! ```
//!
//! `harness = false`: the binary's only test runs alone in its process (see the harness module
//! for why). Counts and bits, not timings: it needs no quiet machine.

#[path = "support/profiling_harness.rs"]
mod harness;

use boyko_diag::lane::LANE_COUNT;
use boyko_diag::profiling_abi::{ZoneHandle, any_armed};
use boyko_diag::sample::{Region, overflow, pending};
use boyko_ecs::ecs::core::profiling::SYSTEM_ZONES_COMPILED;
use boyko_physics::components::RigidBody;
use boyko_physics::profiling::{COUNTER_ZONES, SPAN_ZONES, ZONES_COMPILED};

use harness::{Scene, run_single_test};

/// The name libtest would list this test under.
const TEST_NAME: &str = "instrumented_step_is_bit_identical_armed_and_disarmed";
/// Steps per run: the plan's 100.
const STEPS: usize = 100;
/// Steps with sleeping off before it is turned on.
const STEPS_OFF: usize = 50;

fn main() {
    run_single_test(TEST_NAME, instrumented_step_is_bit_identical_armed_and_disarmed);
}

/// Every field of a `RigidBody`, as bits.
fn body_bits(b: &RigidBody) -> [u32; 13] {
    [
        b.position.x.to_bits(),
        b.position.y.to_bits(),
        b.position.z.to_bits(),
        b.linear_velocity.x.to_bits(),
        b.linear_velocity.y.to_bits(),
        b.linear_velocity.z.to_bits(),
        b.rotation.x.to_bits(),
        b.rotation.y.to_bits(),
        b.rotation.z.to_bits(),
        b.rotation.w.to_bits(),
        b.angular_velocity.x.to_bits(),
        b.angular_velocity.y.to_bits(),
        b.angular_velocity.z.to_bits(),
    ]
}

/// Every dynamic box's full state, in spawn order, as bits.
fn pose_bits(scene: &Scene) -> Vec<[u32; 13]> {
    scene
        .boxes
        .iter()
        .map(|&e| {
            body_bits(
                scene
                    .world
                    .get_component::<RigidBody>(e)
                    .expect("harness: a spawned box is live"),
            )
        })
        .collect()
}

/// Σ over every lane and both regions of the samples pending and the samples refused: every
/// push that got past a gate moves one of the two.
fn ring_traffic() -> u64 {
    (0..LANE_COUNT)
        .flat_map(|lane| [Region::Engine, Region::User].map(|region| (lane, region)))
        .map(|(lane, region)| u64::from(pending(lane, region)) + overflow(lane, region))
        .sum()
}

/// Each physics span zone's own closed-interval count (the guard's accumulator, which only an
/// opened guard moves).
fn span_calls() -> Vec<u64> {
    SPAN_ZONES.iter().map(|h| h.calls()).collect()
}

/// Steps `scene` [`STEPS`] times with the sleeping toggle, recording the poses after each step;
/// `after_step` runs between the step and the recording.
fn run(scene: &mut Scene, mut after_step: impl FnMut(&mut Scene)) -> Vec<Vec<[u32; 13]>> {
    let mut poses = Vec::with_capacity(STEPS);
    for step in 0..STEPS {
        scene.set_sleeping(step >= STEPS_OFF);
        scene.step();
        after_step(scene);
        poses.push(pose_bits(scene));
    }
    poses
}

fn name_of(handle: &ZoneHandle) -> &'static str {
    handle.desc.name
}

fn instrumented_step_is_bit_identical_armed_and_disarmed() {
    // ── 1. Disarmed ──
    assert!(!any_armed(), "the disarmed run must start before anything in the process armed");
    let traffic_before = ring_traffic();
    let calls_before = span_calls();
    let disarmed = {
        let mut scene = Scene::spawn();
        run(&mut scene, |_| {})
    };
    assert!(!any_armed(), "nothing armed during the disarmed run");
    assert_eq!(
        ring_traffic(),
        traffic_before,
        "the disarmed run pushed samples into the lanes"
    );
    for (k, (&before, after)) in calls_before.iter().zip(span_calls()).enumerate() {
        assert_eq!(
            after,
            before,
            "the disarmed run opened zone `{}`",
            name_of(SPAN_ZONES[k])
        );
    }

    // ── 2. Armed ──
    let mut scene = Scene::spawn();
    scene.arm_profiler();
    let armed = run(&mut scene, Scene::fold);

    for (step, (a, d)) in armed.iter().zip(&disarmed).enumerate() {
        if let Some(i) = (0..a.len()).find(|&i| a[i] != d[i]) {
            panic!(
                "step {step}: box {i} differs armed vs disarmed (sleeping {}): armed {:08x?}, \
                 disarmed {:08x?}",
                step >= STEPS_OFF,
                a[i],
                d[i]
            );
        }
    }
    assert_eq!(armed.len(), STEPS);
    assert!(!armed[0].is_empty(), "the scene has boxes to compare");

    for (k, (&before, after)) in calls_before.iter().zip(span_calls()).enumerate() {
        let opened = after - before;
        if ZONES_COMPILED {
            assert!(opened > 0, "the armed run never opened zone `{}`", name_of(SPAN_ZONES[k]));
        } else {
            assert_eq!(opened, 0, "a folded zone `{}` opened", name_of(SPAN_ZONES[k]));
        }
    }
    for &handle in SPAN_ZONES.iter().chain(COUNTER_ZONES.iter()) {
        let samples = scene.lifetime(handle).count;
        if ZONES_COMPILED {
            assert!(samples > 0, "the armed run recorded no sample on `{}`", name_of(handle));
        } else {
            assert_eq!(samples, 0, "a folded zone `{}` recorded samples", name_of(handle));
        }
    }
    let systems: Vec<(&'static str, u16)> = scene.physics.system_zones().collect();
    assert_eq!(
        systems.len(),
        if SYSTEM_ZONES_COMPILED { scene.physics.len() } else { 0 },
        "every system is listed once, or none under a folded tier"
    );
    for &(name, id) in &systems {
        assert!(scene.lifetime_of(id).count > 0, "the armed run recorded no span on `{name}`");
    }

    let drops = scene.profiler().drops();
    assert_eq!(drops.total(), 0, "the store dropped samples: {drops:?}");
}
