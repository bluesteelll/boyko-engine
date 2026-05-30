//! Simulation driver: fixed-timestep loop over a per-target dispatch shell
//! (plan §6.7 / D9 / D10 / D15).
//!
//! Each display frame the runner advances the sim in fixed `dt` increments via
//! an accumulator, so physics is stable and independent of refresh rate, and
//! caps the sub-steps per frame to avoid a spiral of death after a hitch.
//!
//! ## Two dispatch paths behind ONE [`SimRunner`] API (plan D10 — THE web driver)
//!
//! The system FUNCTIONS and the components are 100% shared across targets; only
//! the *dispatch* differs, cfg-split here:
//!
//! * **Native** (`cfg(not(target_arch = "wasm32"))`): a real multi-threaded
//!   [`Schedule`] bound to a [`ThreadPool`]. `Schedule::run` fans `par_iter`
//!   systems across workers and auto-applies the [`Mode`] transition pass.
//! * **wasm** (`cfg(target_arch = "wasm32")`): NO [`ThreadPool`], NO
//!   `Schedule::run`. `wasm32-unknown-unknown` without atomics/SharedArrayBuffer
//!   (which header-less GitHub Pages cannot provide) cannot spawn OS threads, and
//!   `ThreadPoolBuilder::build` ALWAYS spawns at least one (`thread_pool.rs`), so
//!   the schedule is unusable. The wasm runner instead drives the SAME per-mode
//!   system functions sequentially through the `EcsMaster` direct API
//!   (`run_system`), replicating the transition pass + `on_enter`/`on_exit`/
//!   `in_state` gating inline (see [`run_sim_step_sequential`]). `par_iter_mut`
//!   inside an unchanged system body falls back to a sequential walk when no pool
//!   is attached (PAR7), so the bodies need no `#[cfg]`.
//!
//! ## Why option (b), not a no-pool [`Schedule`] (resolved against the code)
//!
//! `ScheduleBuilder::new` requires an `Arc<ThreadPool>` and `Schedule::run`
//! enters `pool.install(...)` and dispatches every system body through
//! `Scope::spawn` — there is no sequential / no-pool execution path in the
//! schedule. So the wasm path cannot reuse `Schedule`; it hand-rolls the
//! dependency-ordered sequential dispatch (plan D10 option (b)).
//!
//! ## Mode state machine (Wave 5)
//!
//! Native: [`SimRunner::new`] registers the [`Mode`] state
//! ([`ScheduleBuilder::insert_state`]) and all the mode-gated systems. The
//! transition pass (auto-run by `Schedule::run`, plan G10) drives spawn-on-enter
//! / despawn-on-exit and the `in_state`-gated per-mode sim systems. The
//! intra-frame order on a transition frame (plan H3) is pinned with Phase-15
//! `.before`:
//!
//! ```text
//! despawn-old  .before  spawn-new  .before  sync_gpu_instance
//! ```
//!
//! so the `for_each_chunk` upload (in `App::update`, after the step) reads the
//! switched, freshly-spawned column on the SAME transition frame. The wasm
//! sequential runner pins the same order by construction (despawn → spawn →
//! per-mode systems → GPU sync, in straight-line code).

// Native imports: the pool + schedule machinery.
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
#[cfg(not(target_arch = "wasm32"))]
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder, in_state, on_enter, on_exit};
#[cfg(not(target_arch = "wasm32"))]
use boyko_threadpool::ThreadPool;

#[cfg(not(target_arch = "wasm32"))]
use crate::render::WORLD_HALF_EXTENT;
#[cfg(not(target_arch = "wasm32"))]
use crate::sim::grid::SpatialGrid;
#[cfg(not(target_arch = "wasm32"))]
use crate::sim::modes::{
    BALL_COUNT, BOID_COUNT, Mode, despawn_balls, despawn_boids, despawn_particles, spawn_balls,
    spawn_boids, spawn_particles,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::sim::resources::{BoidParams, BoidSnapshot, PhysicsParams};
#[cfg(not(target_arch = "wasm32"))]
use crate::sim::resources::{BallSnapshot, SimParams};
#[cfg(not(target_arch = "wasm32"))]
use crate::sim::systems::boids::{boid_forces, build_grid, integrate_boids, snapshot_boids};
#[cfg(not(target_arch = "wasm32"))]
use crate::sim::systems::common::sync_gpu_instance;
#[cfg(not(target_arch = "wasm32"))]
use crate::sim::systems::particles::integrate_particles;
#[cfg(not(target_arch = "wasm32"))]
use crate::sim::systems::physics::{
    apply_ball_motion, build_ball_grid, collide_balls, integrate_balls, sync_ball_gpu,
    tint_collided, wall_bounce,
};

// Shared timestep constants are used by both targets.
use crate::sim::resources::DeltaTime;

/// Fixed simulation timestep in seconds (plan §6.7 default 1/60).
pub const FIXED_DT: f32 = 1.0 / 60.0;

/// Maximum display delta fed into the accumulator per frame, in seconds. A
/// hitch larger than this is clamped so the sim never tries to "catch up"
/// across a long stall (plan D9 spiral-of-death guard).
const MAX_FRAME_DT: f32 = 0.25;

/// Maximum fixed sub-steps run in one display frame (plan D9).
const MAX_SUBSTEPS: u32 = 5;

// ═══════════════════════════════════════════════════════════════════════════
// Native dispatch: real multi-threaded Schedule + thread pool.
// ═══════════════════════════════════════════════════════════════════════════

/// Owns the schedule and the fixed-timestep accumulator (plan §6.7).
#[cfg(not(target_arch = "wasm32"))]
pub struct SimRunner {
    schedule: Schedule,
    /// Carried-over fractional time not yet consumed by a fixed step.
    accumulator: f32,
}

#[cfg(not(target_arch = "wasm32"))]
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
        // One shared spatial grid serves both the boid and physics broad-phases
        // (the two modes never run in the same frame). It is sized for the larger
        // population (boids); each mode resets the cell size each frame via
        // `set_cell_size`, so sharing one grid resource is sound (plan D11).
        world.insert_resource(SpatialGrid::new(
            WORLD_HALF_EXTENT,
            boid_params.radius,
            BOID_COUNT.max(BALL_COUNT),
        ));
        world.insert_resource(BoidSnapshot::with_capacity(BOID_COUNT));
        world.insert_resource(boid_params);

        // Physics-mode resources (Wave 6): tunables + the reused ball snapshot /
        // collision scratch buffers (plan D13 / §11.2).
        world.insert_resource(PhysicsParams::default());
        world.insert_resource(BallSnapshot::with_capacity(BALL_COUNT));

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
        let despawn_balls_key = builder
            .add_system(despawn_balls)
            .run_if(on_exit(Mode::Physics))
            .key();
        let spawn_particles_key = builder
            .add_system(spawn_particles)
            .run_if(on_enter(Mode::Particles))
            // Despawn the outgoing mode before spawning this one (H3).
            .after(despawn_particles_key)
            .after(despawn_boids_key)
            .after(despawn_balls_key)
            .key();
        let spawn_boids_key = builder
            .add_system(spawn_boids)
            .run_if(on_enter(Mode::Boids))
            .after(despawn_particles_key)
            .after(despawn_boids_key)
            .after(despawn_balls_key)
            .key();
        let spawn_balls_key = builder
            .add_system(spawn_balls)
            .run_if(on_enter(Mode::Physics))
            .after(despawn_particles_key)
            .after(despawn_boids_key)
            .after(despawn_balls_key)
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

        // Physics: integrate -> build_grid -> collide (sequential) -> wall ->
        // apply (write-back), in strict order (each reads the previous stage's
        // output). All gated in_state(Physics). The collision/wall passes mutate
        // the snapshot; `apply_ball_motion` writes it back through the
        // change-tracking velocity guard so `Changed<Velocity>` (the tint) is
        // precise (plan D13 / G12).
        let integrate_balls_key = builder
            .add_system(integrate_balls)
            .run_if(in_state(Mode::Physics))
            .after(spawn_balls_key)
            .key();
        let build_ball_grid_key = builder
            .add_system(build_ball_grid)
            .run_if(in_state(Mode::Physics))
            .after(integrate_balls_key)
            .key();
        let collide_balls_key = builder
            .add_system(collide_balls)
            .run_if(in_state(Mode::Physics))
            .after(build_ball_grid_key)
            .key();
        let wall_bounce_key = builder
            .add_system(wall_bounce)
            .run_if(in_state(Mode::Physics))
            .after(collide_balls_key)
            .key();
        let apply_ball_motion_key = builder
            .add_system(apply_ball_motion)
            .run_if(in_state(Mode::Physics))
            .after(wall_bounce_key)
            .key();

        // ── GPU mirror (mode-agnostic) ───────────────────────────────────────
        // Runs after every integrator/write-back AND after the spawn systems, so
        // the GpuInstance column reflects the post-step, post-switch state before
        // the upload reads it (plan H3 / D3). In Physics mode it packs every
        // ball's position + base (speed-ramp) color; the tint then overlays the
        // collision flash on top.
        let sync_gpu_key = builder
            .add_system(sync_gpu_instance)
            .after(integrate_particles_key)
            .after(integrate_boids_key)
            .after(apply_ball_motion_key)
            .after(spawn_particles_key)
            .after(spawn_boids_key)
            .after(spawn_balls_key)
            .key();

        // Physics GPU sync: size balls by their actual radius and write a base
        // color, overriding the shared `sync_gpu_instance` (which sized them by
        // the particle slider). Runs after the shared sync; gated in_state.
        let sync_ball_gpu_key = builder
            .add_system(sync_ball_gpu)
            .run_if(in_state(Mode::Physics))
            .after(apply_ball_motion_key)
            .after(sync_gpu_key)
            .key();

        // The `Changed<Velocity>` showcase (plan D13): flash balls that collided
        // or bounced this frame. Runs AFTER `sync_ball_gpu` so the base color it
        // wrote is overlaid (not overwritten), and after `apply_ball_motion`
        // (which set the velocity change ticks). Gated in_state(Physics).
        builder
            .add_system(tint_collided)
            .run_if(in_state(Mode::Physics))
            .after(apply_ball_motion_key)
            .after(sync_ball_gpu_key);

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

// ═══════════════════════════════════════════════════════════════════════════
// wasm dispatch: sequential runner, NO thread pool, NO Schedule (plan D10).
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "wasm32")]
pub use wasm_runner::SimRunner;

#[cfg(target_arch = "wasm32")]
mod wasm_runner {
    use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;

    use crate::render::WORLD_HALF_EXTENT;
    use crate::sim::grid::SpatialGrid;
    use crate::sim::modes::{
        BALL_COUNT, BOID_COUNT, Mode, despawn_balls, despawn_boids, despawn_particles, spawn_balls,
        spawn_boids, spawn_particles,
    };
    use crate::sim::resources::{
        BallSnapshot, BoidParams, BoidSnapshot, PhysicsParams, SimParams,
    };
    use crate::sim::systems::boids::{boid_forces, build_grid, integrate_boids, snapshot_boids};
    use crate::sim::systems::common::sync_gpu_instance;
    use crate::sim::systems::particles::integrate_particles;
    use crate::sim::systems::physics::{
        apply_ball_motion, build_ball_grid, collide_balls, integrate_balls, sync_ball_gpu,
        wall_bounce,
    };

    use super::{DeltaTime, FIXED_DT, MAX_FRAME_DT, MAX_SUBSTEPS};

    /// Sequential simulation driver for `wasm32-unknown-unknown` (plan D10
    /// option (b)).
    ///
    /// Mirrors the native [`SimRunner`](super::SimRunner)'s public API
    /// (`new(world)` + `step(world, frame_dt) -> u32`) but holds NO
    /// [`Schedule`](boyko_ecs::ecs::core::schedule::Schedule) and NO
    /// `ThreadPool`. It seeds the same mode-sim resources the native builder
    /// inserts, then runs the SAME per-mode system functions sequentially each
    /// fixed step (see [`run_sim_step_sequential`]).
    ///
    /// The constructor takes no pool argument (one cfg seam in `app.rs`, plan
    /// §8.4): a pool cannot be built on header-less-Pages wasm.
    pub struct SimRunner {
        /// Carried-over fractional time not yet consumed by a fixed step.
        accumulator: f32,
        /// `true` until the synthesized initial `none -> Mode::default()`
        /// transition has fired (Phase-17 D7 equivalent). Drives the frame-1
        /// `on_enter(Particles)` spawn. Owned here (not in the world) because the
        /// wasm path drives the transition itself rather than through
        /// `Schedule::run`'s state pass.
        pending_initial: bool,
    }

    impl SimRunner {
        /// Builds the wasm runner and seeds the mode-sim resources the system
        /// functions read.
        ///
        /// Inserts the SAME resources the native [`SimRunner::new`] does
        /// (`SpatialGrid`, `BoidSnapshot`, `BoidParams`, `PhysicsParams`,
        /// `BallSnapshot`) so the shared system bodies find every resource they
        /// expect. Does NOT register a `State<Mode>` transition pass: the
        /// sequential `step` drives the transition inline. The current `Mode`
        /// still lives in the world as `State<Mode>` (inserted by the app via
        /// `insert_state`) so the UI's `current_mode()` / `NextState<Mode>` path
        /// is identical to native.
        pub fn new(world: &mut EcsMaster) -> Self {
            // Insert the `Mode` state resources (`State<Mode>` / `NextState<Mode>`
            // / `StateTransitionRecord<Mode>`). Native does this via
            // `ScheduleBuilder::insert_state` at build; on wasm there is no
            // builder, so the runner inserts them directly. This makes the UI's
            // `current_mode()` (reads `State<Mode>`) and the mode buttons (write
            // `NextState<Mode>`) path byte-identical to native; the sequential
            // `step` drives the transition (it does not rely on a schedule pass).
            world.insert_state(Mode::default());

            let boid_params = BoidParams::default();
            world.insert_resource(SpatialGrid::new(
                WORLD_HALF_EXTENT,
                boid_params.radius,
                BOID_COUNT.max(BALL_COUNT),
            ));
            world.insert_resource(BoidSnapshot::with_capacity(BOID_COUNT));
            world.insert_resource(boid_params);
            world.insert_resource(PhysicsParams::default());
            world.insert_resource(BallSnapshot::with_capacity(BALL_COUNT));

            Self {
                accumulator: 0.0,
                pending_initial: true,
            }
        }

        /// Advances the simulation by `frame_dt` seconds of display time
        /// (sequential variant).
        ///
        /// Identical accumulator/sub-step rhythm to the native
        /// [`SimRunner::step`](super::SimRunner::step), but each fixed step calls
        /// [`run_sim_step_sequential`] instead of `Schedule::run`. Returns the
        /// number of fixed steps run this frame.
        pub fn step(&mut self, world: &mut EcsMaster, frame_dt: f32) -> u32 {
            if world.resource::<SimParams>().paused {
                return 0;
            }

            self.accumulator += frame_dt.min(MAX_FRAME_DT);

            let mut sub = 0;
            while self.accumulator >= FIXED_DT && sub < MAX_SUBSTEPS {
                world.resource_mut::<DeltaTime>().0 = FIXED_DT;
                run_sim_step_sequential(world, &mut self.pending_initial);
                self.accumulator -= FIXED_DT;
                sub += 1;
            }
            if sub == MAX_SUBSTEPS {
                self.accumulator = 0.0;
            }
            debug_assert!(sub <= MAX_SUBSTEPS);
            sub
        }
    }

    /// Runs ONE fixed sim step sequentially, replicating what the native
    /// `Schedule::run` does for this demo's system graph (plan D10 option (b)).
    ///
    /// The sequence is the dependency order the native builder pins via
    /// `.after(...)`, executed as straight-line code:
    ///
    /// 1. **Transition pass** — replicates `Schedule::run`'s Phase-17 state pass
    ///    + the `on_exit`/`on_enter` gating. Reads `NextState<Mode>` and the
    ///    `pending_initial` flag, performs the despawn-old / spawn-new directly
    ///    (the SAME exclusive `spawn_*` / `despawn_*` functions), and swaps
    ///    `State<Mode>`. Despawn-old runs before spawn-new (H3).
    /// 2. **Per-mode systems** — only the active mode's systems run (the inline
    ///    `in_state` gate), in their `.after(...)` order, via
    ///    `EcsMaster::run_system` (sequential init + run + apply).
    /// 3. **GPU mirror** — `sync_gpu_instance` always; the physics override
    ///    (`sync_ball_gpu` + tint) only in Physics mode.
    ///
    /// `par_iter_mut` inside a system body falls back to a sequential walk
    /// because no pool is attached (PAR7 — `try_with_active_pool` returns
    /// `None`), so every system function is reused UNCHANGED.
    ///
    /// # `Changed<Velocity>` on wasm (documented divergence)
    ///
    /// `tint_collided` (the native `Changed<Velocity>` showcase) is intentionally
    /// NOT run here. `EcsMaster::run_system` re-`initialize`s the system each
    /// call, resetting its tick window to the `MAX_CHANGE_AGE` sentinel, so a
    /// `Changed<T>` filter would read as ALWAYS-TRUE (flash every ball) — the
    /// documented unguarded-tick footgun (`docs/DEMO-DOGFOODING.md` W6-1). With
    /// the change-detection flash dropped, the wasm physics view renders every
    /// ball at its `sync_ball_gpu` base color; the collision response itself is
    /// identical (it does not depend on ticks). This is the single behavioral
    /// difference from native, and it is purely cosmetic.
    fn run_sim_step_sequential(world: &mut EcsMaster, pending_initial: &mut bool) {
        // ── 1. Transition pass (replicates the Phase-17 state pass + gating) ──
        apply_mode_transition(world, pending_initial);

        // The post-transition mode decides which per-mode systems run (the
        // inline `in_state` gate). Read it once: `State<Mode>` reflects the swap
        // `apply_mode_transition` just performed.
        let mode = *world.state::<Mode>();

        // ── 2. Per-mode systems, in the native `.after(...)` dependency order ─
        match mode {
            Mode::Particles => {
                world.run_system(integrate_particles);
            }
            Mode::Boids => {
                world.run_system(snapshot_boids);
                world.run_system(build_grid);
                world.run_system(boid_forces);
                world.run_system(integrate_boids);
            }
            Mode::Physics => {
                world.run_system(integrate_balls);
                world.run_system(build_ball_grid);
                world.run_system(collide_balls);
                world.run_system(wall_bounce);
                world.run_system(apply_ball_motion);
            }
        }

        // ── 3. GPU mirror (mode-agnostic), then the physics override ─────────
        // `sync_gpu_instance` runs in every mode (native: after all integrators
        // + spawns). The physics-specific `sync_ball_gpu` overrides ball scale +
        // base color afterward. `tint_collided` is dropped on wasm (see docs).
        world.run_system(sync_gpu_instance);
        if mode == Mode::Physics {
            world.run_system(sync_ball_gpu);
        }
    }

    /// Replicates the native transition pass for [`Mode`] (Phase-17 D7 + the
    /// `on_exit`/`on_enter`-gated despawn/spawn), driven inline for the wasm
    /// sequential runner.
    ///
    /// * On the FIRST call (`*pending_initial`), fires the synthesized
    ///   `none -> Mode::default()` transition: it spawns the initial mode's set
    ///   (`on_enter`) with no exit. This mirrors native frame 1.
    /// * On later calls, drains `NextState<Mode>`: if a real switch is queued
    ///   (`requested != current`), it despawns the current mode's entities
    ///   (`on_exit`) then spawns the requested mode's (`on_enter`) — despawn
    ///   before spawn (H3) — and swaps `State<Mode>` to the requested value.
    /// * An identity request (`requested == current`) is drained to `Unchanged`
    ///   with no spawn/despawn (Phase-17 D6).
    ///
    /// `NextState<Mode>` is reset to `Unchanged` via `set_next_state` after a
    /// switch — but here we drain it by replacing the resource value directly
    /// (the same effect the native pass's `std::mem::take` has).
    fn apply_mode_transition(world: &mut EcsMaster, pending_initial: &mut bool) {
        use boyko_ecs::ecs::core::state::NextState;

        // Synthesized initial transition (D7): spawn the default mode's set on
        // the first step, then clear the flag.
        if *pending_initial {
            *pending_initial = false;
            let initial = *world.state::<Mode>();
            // If a switch was queued before the first step, native semantics let
            // the real transition override the initial (last-write-wins). We
            // resolve that below by NOT early-returning: drain `NextState` after
            // the initial spawn so a pre-queued switch still applies this step.
            spawn_for_mode(world, initial);
        }

        // Drain `NextState<Mode>`: take the pending value (if any) and reset the
        // resource to `Unchanged` (mirrors the native `std::mem::take`).
        let requested = {
            let next = world.resource_mut::<NextState<Mode>>();
            let pending = next.pending().copied();
            *next = NextState::Unchanged;
            pending
        };
        let Some(requested) = requested else {
            return;
        };

        let current = *world.state::<Mode>();
        if requested == current {
            // Identity transition (D6): nothing to spawn/despawn.
            return;
        }

        // Real transition: despawn the old set, then spawn the new (H3 order),
        // then swap the current state to the requested value. `NextState` was
        // already drained to `Unchanged` above.
        despawn_for_mode(world, current);
        spawn_for_mode(world, requested);
        set_current_mode(world, requested);
    }

    /// Spawns the entity set for `mode` by calling the SAME exclusive spawn
    /// function the native `on_enter(mode)` gate calls.
    fn spawn_for_mode(world: &mut EcsMaster, mode: Mode) {
        match mode {
            Mode::Particles => spawn_particles(world),
            Mode::Boids => spawn_boids(world),
            Mode::Physics => spawn_balls(world),
        }
    }

    /// Despawns the entity set for `mode` by calling the SAME exclusive despawn
    /// function the native `on_exit(mode)` gate calls.
    fn despawn_for_mode(world: &mut EcsMaster, mode: Mode) {
        match mode {
            Mode::Particles => despawn_particles(world),
            Mode::Boids => despawn_boids(world),
            Mode::Physics => despawn_balls(world),
        }
    }

    /// Sets `State<Mode>` to `mode` by writing its [`Resource`] slot directly —
    /// exactly what the native transition pass does
    /// (`*world.resource_mut::<State<Mode>>() = State::new(requested)`). The
    /// public `set_next_state` only *queues* a transition, so it cannot perform
    /// the immediate swap the sequential runner needs.
    ///
    /// [`Resource`]: boyko_ecs::ecs::core::resources::resource::Resource
    fn set_current_mode(world: &mut EcsMaster, mode: Mode) {
        use boyko_ecs::ecs::core::state::State;
        *world.resource_mut::<State<Mode>>() = State::new(mode);
    }
}
