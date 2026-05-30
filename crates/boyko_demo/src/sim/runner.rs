//! Native simulation driver: thread pool + schedule + fixed-timestep loop
//! (plan §6.7 / D9 / D10 / D15).
//!
//! The runner owns the real multi-threaded [`Schedule`]. Each display frame it
//! advances the sim in fixed `dt` increments via an accumulator, so physics is
//! stable and independent of refresh rate, and caps the sub-steps per frame to
//! avoid a spiral of death after a hitch.
//!
//! ## Mode state machine (Wave 5)
//!
//! [`SimRunner::new`] registers the [`Mode`] state ([`ScheduleBuilder::insert_state`])
//! and all the mode-gated systems. The transition pass (auto-run by
//! `Schedule::run`, plan G10) drives spawn-on-enter / despawn-on-exit and the
//! `in_state`-gated per-mode sim systems. The intra-frame order on a transition
//! frame (plan H3) is pinned with Phase-15 `.before`:
//!
//! ```text
//! despawn-old  .before  spawn-new  .before  sync_gpu_instance
//! ```
//!
//! so the `for_each_chunk` upload (in `App::update`, after the step) reads the
//! switched, freshly-spawned column on the SAME transition frame.

use std::sync::Arc;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder, in_state, on_enter, on_exit};
use boyko_threadpool::ThreadPool;

use crate::render::WORLD_HALF_EXTENT;
use crate::sim::grid::SpatialGrid;
use crate::sim::modes::{
    BOID_COUNT, Mode, despawn_boids, despawn_particles, spawn_boids, spawn_particles,
};
use crate::sim::resources::{BoidParams, BoidSnapshot, DeltaTime, SimParams};
use crate::sim::systems::boids::{boid_forces, build_grid, integrate_boids, snapshot_boids};
use crate::sim::systems::common::sync_gpu_instance;
use crate::sim::systems::particles::integrate_particles;

/// Fixed simulation timestep in seconds (plan §6.7 default 1/60).
pub const FIXED_DT: f32 = 1.0 / 60.0;

/// Maximum display delta fed into the accumulator per frame, in seconds. A
/// hitch larger than this is clamped so the sim never tries to "catch up"
/// across a long stall (plan D9 spiral-of-death guard).
const MAX_FRAME_DT: f32 = 0.25;

/// Maximum fixed sub-steps run in one display frame (plan D9).
const MAX_SUBSTEPS: u32 = 5;

/// Owns the schedule and the fixed-timestep accumulator (plan §6.7).
pub struct SimRunner {
    schedule: Schedule,
    /// Carried-over fractional time not yet consumed by a fixed step.
    accumulator: f32,
}

impl SimRunner {
    /// Builds the native runner: registers the [`Mode`] state and every
    /// mode-gated system into one [`Schedule`] bound to `pool` for `par_iter`
    /// fan-out (plan §6.6 / §6.7 / D15).
    ///
    /// `world` is borrowed only to resolve the schedule's archetype/query state
    /// and to seed the mode-sim resources at build time; it is not retained.
    /// The boid pipeline resources ([`SpatialGrid`], [`BoidSnapshot`],
    /// [`BoidParams`]) are inserted here because the runner is the single place
    /// that knows the world box and the max boid count.
    ///
    /// # System wiring (plan §6.6 / H3)
    ///
    /// * `insert_state(Mode::Particles)` — the synthesized initial transition
    ///   (D7) fires `on_enter(Particles)` on frame 1, spawning the startup cloud.
    /// * Spawn-on-enter / despawn-on-exit: EXCLUSIVE `fn(&mut EcsMaster)` gated
    ///   `.run_if(on_enter/on_exit(Mode::X))` (the STEP-0-gated C2 path).
    /// * Per-mode sim systems: `.run_if(in_state(Mode::X))`.
    /// * Intra-frame order: `despawn_* .before spawn_* .before sync_gpu_instance`
    ///   (so the upload sees the post-switch column on the transition frame).
    pub fn new(pool: Arc<ThreadPool>, world: &mut EcsMaster) -> Self {
        // Boid-pipeline resources. The grid cell size starts at the default boid
        // radius (refined each frame by `build_grid` if the UI changes it); both
        // Vecs are pre-sized to the max boid count so steady-state rebuilds never
        // allocate (plan §6.4 / §11.2).
        let boid_params = BoidParams::default();
        world.insert_resource(SpatialGrid::new(
            WORLD_HALF_EXTENT,
            boid_params.radius,
            BOID_COUNT,
        ));
        world.insert_resource(BoidSnapshot::with_capacity(BOID_COUNT));
        world.insert_resource(boid_params);

        let mut builder = ScheduleBuilder::new(pool);

        // Register the mode state. Initial = Particles (Mode::default), so frame
        // 1 synthesizes on_enter(Particles) and the startup cloud spawns there
        // (plan §10 W5 app note).
        builder.insert_state(Mode::Particles);

        // ── Transition systems (exclusive, gated on_enter/on_exit) ──────────
        // Capture their keys to pin the intra-frame order (H3). Each despawn
        // runs before each spawn so a switch tears down the old set before the
        // new one is created (avoids transiently double-populated archetypes).
        let despawn_particles_key = builder
            .add_system(despawn_particles)
            .run_if(on_exit(Mode::Particles))
            .key();
        let despawn_boids_key = builder
            .add_system(despawn_boids)
            .run_if(on_exit(Mode::Boids))
            .key();
        let spawn_particles_key = builder
            .add_system(spawn_particles)
            .run_if(on_enter(Mode::Particles))
            // Despawn the outgoing mode before spawning this one (H3).
            .after(despawn_particles_key)
            .after(despawn_boids_key)
            .key();
        let spawn_boids_key = builder
            .add_system(spawn_boids)
            .run_if(on_enter(Mode::Boids))
            .after(despawn_particles_key)
            .after(despawn_boids_key)
            .key();

        // ── Per-mode sim systems (gated in_state) ────────────────────────────
        // Particles: a single integration pass.
        let integrate_particles_key = builder
            .add_system(integrate_particles)
            .run_if(in_state(Mode::Particles))
            // Integrate only after this frame's (possible) spawn so a freshly
            // entered population is advanced the same frame.
            .after(spawn_particles_key)
            .key();

        // Boids: snapshot -> build_grid -> forces -> integrate, in strict order
        // (each reads the previous stage's output). All gated in_state(Boids).
        let snapshot_key = builder
            .add_system(snapshot_boids)
            .run_if(in_state(Mode::Boids))
            .after(spawn_boids_key)
            .key();
        let build_grid_key = builder
            .add_system(build_grid)
            .run_if(in_state(Mode::Boids))
            .after(snapshot_key)
            .key();
        let boid_forces_key = builder
            .add_system(boid_forces)
            .run_if(in_state(Mode::Boids))
            .after(build_grid_key)
            .key();
        let integrate_boids_key = builder
            .add_system(integrate_boids)
            .run_if(in_state(Mode::Boids))
            .after(boid_forces_key)
            .key();

        // ── GPU mirror (mode-agnostic) ───────────────────────────────────────
        // Runs after every integrator AND after the spawn systems, so the
        // GpuInstance column reflects the post-step, post-switch state before the
        // upload reads it (plan H3 / D3).
        builder
            .add_system(sync_gpu_instance)
            .after(integrate_particles_key)
            .after(integrate_boids_key)
            .after(spawn_particles_key)
            .after(spawn_boids_key);

        let schedule = builder.build(world);

        Self {
            schedule,
            accumulator: 0.0,
        }
    }

    /// Advances the simulation by `frame_dt` seconds of display time.
    ///
    /// Accumulates display time and runs the schedule once per whole `FIXED_DT`
    /// elapsed, up to [`MAX_SUBSTEPS`] times. Returns the number of fixed steps
    /// actually run this frame (0 when paused or when less than one step has
    /// accumulated). Each step writes [`DeltaTime`] before running so systems
    /// integrate against the fixed `dt`.
    ///
    /// A mode switch (queued via `NextState<Mode>` from the UI) applies inside
    /// `Schedule::run`'s transition pass, so the gated spawn/despawn/sim systems
    /// react within the steps run here (plan G10 / D15). When paused, the
    /// schedule does not run, so a switch queued while paused applies on the
    /// first step after unpausing.
    pub fn step(&mut self, world: &mut EcsMaster, frame_dt: f32) -> u32 {
        if world.resource::<SimParams>().paused {
            return 0;
        }

        self.accumulator += frame_dt.min(MAX_FRAME_DT);

        let mut sub = 0;
        while self.accumulator >= FIXED_DT && sub < MAX_SUBSTEPS {
            world.resource_mut::<DeltaTime>().0 = FIXED_DT;
            // The real multi-threaded schedule: par_iter systems fan out across
            // the pool's workers here; the transition pass applies mode switches.
            self.schedule.run(world);
            self.accumulator -= FIXED_DT;
            sub += 1;
        }
        // If we hit the substep cap, drop the backlog so we do not spiral.
        if sub == MAX_SUBSTEPS {
            self.accumulator = 0.0;
        }
        debug_assert!(sub <= MAX_SUBSTEPS);
        sub
    }
}
