//! Headless physics-mode smoke test (plan §11.3 / Wave 6).
//!
//! Drives the [`Mode::Physics`] pipeline through a real [`SimRunner`] (thread
//! pool + schedule + transition pass) with NO window, and asserts the Wave-6
//! contract:
//!
//! * switching to `Physics` spawns the ball set (and switching away despawns it);
//! * after many steps every ball stays finite and inside the world box;
//! * the system stays bounded — total kinetic energy never blows up (restitution
//!   <= 1 dissipates; positional correction must not inject unbounded energy);
//! * overlapping balls separate — after settling, no pair deeply interpenetrates
//!   (the de-penetration in `collide_balls` does its job).
//!
//! This is the Wave-6 honesty check: if it passes, the sequential collision
//! solver (broad-phase grid + circle-circle narrow-phase + impulse resolution)
//! plus the change-tracking velocity write-back run correctly end-to-end through
//! the public API, with no window and no GPU.
//!
//! # Miri
//!
//! `#![cfg(not(miri))]`: drives `Schedule::run` (worker dispatch via
//! `Scope::spawn`, the Phase-9.1 Tree-Borrows deferral), like
//! `tests/mode_switch.rs` and `tests/sim_smoke.rs`.
#![cfg(not(miri))]

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_demo::sim::components::{BallTag, Position, Radius, Velocity};
use boyko_demo::sim::modes::{BALL_COUNT, Mode, PARTICLE_COUNT};
use boyko_demo::sim::resources::{InputState, SimParams};
use boyko_demo::sim::runner::SimRunner;

/// One engine fixed step (64 Hz, Phase 20) as an f32 display delta. A power-
/// of-two fraction, so from_secs_f32 converts it EXACTLY to 15,625,000 ns -
/// each step() call below expends exactly one substep with zero remainder.
const FIXED_DT: f32 = 1.0 / 64.0;

/// World half-extent the runner clamps positions to (mirrors `WORLD_HALF_EXTENT`).
const BOUND: f32 = 100.0;

/// Number of live entities carrying `BallTag`.
fn ball_count(world: &EcsMaster) -> usize {
    world.query_entities(&[BallTag::component_id()]).len()
}

/// Builds a headless world + runner with the sim resources, then switches into
/// Physics mode and steps once so the ball set is spawned.
fn setup_physics() -> (EcsMaster, SimRunner) {
    // Capacity for the larger (particle) population; balls are fewer.
    let mut world = EcsMaster::with_capacity(PARTICLE_COUNT, 3);
    world.insert_resource(InputState::default());
    world.insert_resource(SimParams::default());

    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let mut runner = SimRunner::new(pool, &mut world);

    // Frame 1 enters Particles (synthesized initial transition); queue the switch
    // to Physics and step so on_exit(Particles) + on_enter(Physics) fire.
    runner.step(&mut world, FIXED_DT);
    world.set_next_state(Mode::Physics);
    runner.step(&mut world, FIXED_DT);
    (world, runner)
}

/// Collects every ball's `(Position, Velocity, Radius)` in archetype row order.
fn collect_balls(world: &mut EcsMaster) -> (Vec<Position>, Vec<Velocity>, Vec<f32>) {
    let mut pos = Vec::new();
    let mut vel = Vec::new();
    let mut radius = Vec::new();
    world
        .query::<(&Position, &Velocity, &Radius), ()>()
        .for_each_chunk(
            |(positions, velocities, radii): (&[Position], &[Velocity], &[Radius])| {
                pos.extend_from_slice(positions);
                vel.extend_from_slice(velocities);
                radius.extend(radii.iter().map(|r| r.0));
            },
        );
    (pos, vel, radius)
}

/// Total kinetic energy (mass 1 per ball): `sum(|v|^2) / 2`.
fn kinetic_energy(vel: &[Velocity]) -> f64 {
    vel.iter()
        .map(|v| 0.5 * (v.x as f64 * v.x as f64 + v.y as f64 * v.y as f64))
        .sum()
}

/// Switching into Physics spawns the ball set; switching away despawns it.
#[test]
fn enter_physics_spawns_balls_exit_despawns() {
    let (mut world, mut runner) = setup_physics();

    assert_eq!(*world.state::<Mode>(), Mode::Physics, "state is Physics");
    assert_eq!(
        ball_count(&world),
        BALL_COUNT,
        "on_enter(Physics) spawns the full ball set"
    );

    // Switch back to Particles: balls despawn, particles respawn.
    world.set_next_state(Mode::Particles);
    runner.step(&mut world, FIXED_DT);
    assert_eq!(ball_count(&world), 0, "on_exit(Physics) despawned every ball");
}

/// After many steps every ball stays finite and inside the world box, and the
/// total kinetic energy stays bounded (no explosion from the collision solver).
#[test]
fn balls_stay_finite_in_box_and_bounded() {
    const STEPS: usize = 120;

    let (mut world, mut runner) = setup_physics();

    let (_, vel0, _) = collect_balls(&mut world);
    let energy0 = kinetic_energy(&vel0);

    for _ in 0..STEPS {
        runner.step(&mut world, FIXED_DT);
    }

    let (pos, vel, radius) = collect_balls(&mut world);
    assert_eq!(pos.len(), BALL_COUNT, "ball count is stable across steps");

    for (p, r) in pos.iter().zip(&radius) {
        assert!(
            p.x.is_finite() && p.y.is_finite(),
            "ball position went non-finite: {p:?}"
        );
        // Balls are clamped so their CENTER stays within the box minus the
        // radius; allow a tiny epsilon for the de-penetration nudge.
        assert!(
            p.x >= -BOUND - 1.0 && p.x <= BOUND + 1.0,
            "ball x escaped the box: {} (r={r})",
            p.x
        );
        assert!(
            p.y >= -BOUND - 1.0 && p.y <= BOUND + 1.0,
            "ball y escaped the box: {} (r={r})",
            p.y
        );
    }

    for v in &vel {
        assert!(
            v.x.is_finite() && v.y.is_finite(),
            "ball velocity went non-finite: {v:?}"
        );
    }

    // Energy must stay bounded. With restitution 0.9 (lossy) and gravity 0, the
    // only energy source is positional correction; it must not pump the system.
    // A generous 4x ceiling over the initial energy catches a runaway explosion
    // while tolerating the correction's small perturbations.
    let energy = kinetic_energy(&vel);
    assert!(
        energy <= energy0 * 4.0 + 1.0,
        "kinetic energy exploded: {energy} (initial {energy0})"
    );
}

/// After settling, no pair of balls deeply interpenetrates — the de-penetration
/// in `collide_balls` separates overlapping balls (plan D13).
///
/// The check is O(n^2); `BALL_COUNT` (a few thousand) keeps it well under a
/// second for a one-shot test. A small tolerance absorbs the single-pass solver's
/// residual overlap (it does not run a global solver to convergence).
#[test]
fn overlapping_balls_separate() {
    const STEPS: usize = 200;
    // Single-pass resolution leaves a little residual overlap; allow up to this
    // fraction of the summed radii before calling it a failure.
    const MAX_OVERLAP_FRACTION: f32 = 0.5;

    let (mut world, mut runner) = setup_physics();
    for _ in 0..STEPS {
        runner.step(&mut world, FIXED_DT);
    }

    let (pos, _, radius) = collect_balls(&mut world);
    let n = pos.len();
    assert_eq!(n, BALL_COUNT, "ball count is stable");

    let mut deep_overlaps = 0usize;
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = pos[j].x - pos[i].x;
            let dy = pos[j].y - pos[i].y;
            let dist = (dx * dx + dy * dy).sqrt();
            let radii = radius[i] + radius[j];
            // Penetration depth as a fraction of the contact distance.
            if dist < radii * (1.0 - MAX_OVERLAP_FRACTION) {
                deep_overlaps += 1;
            }
        }
    }

    assert_eq!(
        deep_overlaps, 0,
        "{deep_overlaps} ball pairs remain deeply overlapped after settling"
    );
}
