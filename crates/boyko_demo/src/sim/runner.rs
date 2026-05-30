//! Native simulation driver: thread pool + schedule + fixed-timestep loop
//! (plan §6.7 / D9 / D10).
//!
//! The runner owns the real multi-threaded [`Schedule`]. Each display frame it
//! advances the sim in fixed `dt` increments via an accumulator, so physics is
//! stable and independent of refresh rate, and caps the sub-steps per frame to
//! avoid a spiral of death after a hitch.

use std::sync::Arc;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_threadpool::ThreadPool;

use crate::sim::resources::{DeltaTime, SimParams};
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
    /// Builds the native runner: a [`Schedule`] over `integrate_particles` then
    /// `sync_gpu_instance` (ordered after it so the GPU mirror reflects the
    /// post-integration state), bound to `pool` for `par_iter` fan-out.
    ///
    /// `world` is borrowed only to resolve the schedule's archetype/query state
    /// at build time; it is not retained.
    pub fn new(pool: Arc<ThreadPool>, world: &mut EcsMaster) -> Self {
        let mut builder = ScheduleBuilder::new(pool);
        // `add_system` returns a `SystemConfig` handle; capture the integrator's
        // `SystemKey` so the GPU-sync system can be ordered strictly after it
        // (`.after` takes a `SystemKey`, Phase 15 ordering API). Sync must read
        // the post-integration `Position`/`Velocity`.
        let integrate_key = builder.add_system(integrate_particles).key();
        builder.add_system(sync_gpu_instance).after(integrate_key);
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
    pub fn step(&mut self, world: &mut EcsMaster, frame_dt: f32) -> u32 {
        if world.resource::<SimParams>().paused {
            return 0;
        }

        self.accumulator += frame_dt.min(MAX_FRAME_DT);

        let mut sub = 0;
        while self.accumulator >= FIXED_DT && sub < MAX_SUBSTEPS {
            world.resource_mut::<DeltaTime>().0 = FIXED_DT;
            // The real multi-threaded schedule: par_iter systems fan out across
            // the pool's workers here.
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
