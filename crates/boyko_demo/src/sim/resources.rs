//! ECS resources for the simulation (plan §6.3).
//!
//! Resources are the per-frame inputs and tunables systems read. The app owns
//! `&mut EcsMaster`, so the UI and the input pump write these directly
//! (`world.resource_mut::<_>()`) and the next `Schedule::run` picks them up with
//! zero plumbing. `InputState` covers the one gap the engine leaves to the
//! application (plan §9 G9: no built-in input resource). Time is the engine's
//! since Phase 20: systems read `Res<FixedTime>` (`delta_secs()` = the fixed
//! step); the old demo-local `DeltaTime(f32)` stopgap is deleted.

use boyko_macros::Resource;

use crate::sim::components::{Position, Velocity};

/// Per-frame pointer state, mapped from egui into world space by the app
/// (plan §7). Systems read it to apply the mouse gravity well.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct InputState {
    /// Cursor position in world units, or `None` when the pointer is off the
    /// scene or captured by an egui widget.
    pub cursor_world: Option<[f32; 2]>,
    /// Whether the primary mouse button is held (drives the gravity well).
    pub primary_down: bool,
}

/// Tunable simulation parameters (plan §6.3 / §7). The Wave-4 control panel
/// mutates these in place each frame via `world.resource_mut::<SimParams>()`
/// before the next `Schedule::run`, so a slider edit takes effect on the very
/// next step with zero plumbing.
#[derive(Resource, Clone, Copy, Debug)]
pub struct SimParams {
    /// Gravity-well strength: acceleration scale toward the cursor (world
    /// units/s^2 at unit distance) while the primary button is held.
    pub gravity: f32,
    /// Per-second velocity retention (1.0 = no damping). Applied as
    /// `v *= damping^dt`-style decay each step.
    pub damping: f32,
    /// Speed clamp in world units per second; bounds the well's pull so
    /// particles stay on screen.
    pub max_speed: f32,
    /// Rendered half-extent of a particle quad in world units (plan §7 size
    /// slider). Read by `sync_gpu_instance` so the slider drives the dot size
    /// live.
    pub particle_size: f32,
    /// When `false`, the gravity well is disabled even while the primary button
    /// is held (plan §7 "enable/disable the gravity well" toggle). The
    /// integrator still reads `InputState`, but treats the well as inactive.
    pub gravity_enabled: bool,
    /// Number of particles a single scene click spawns (plan §7 "click spawns N"
    /// burst). The actual spawn is clamped to remaining capacity (plan D6/M5).
    pub spawn_burst: u32,
    /// Desired population shown next to the live entity count (plan §7
    /// target_count). Wave 4 surfaces it as a readout/target; population
    /// maintenance against it is a later wave.
    pub target_count: u32,
    /// When `true`, the runner skips stepping the schedule (plan §7 pause).
    pub paused: bool,
}

impl Default for SimParams {
    fn default() -> Self {
        // Tuned so a 100k cloud in the +/-100 world box reacts visibly to the
        // mouse well without flying off: a moderate pull, light damping, and a
        // speed cap well under the box half-extent per second.
        Self {
            gravity: 1_200.0,
            damping: 0.96,
            max_speed: 220.0,
            particle_size: 0.6,
            gravity_enabled: true,
            spawn_burst: 2_000,
            target_count: 100_000,
            paused: false,
        }
    }
}

/// Tunable boid (flocking) parameters (plan §6.3 / §7 / Wave 5). The control
/// panel mutates these in place each frame; `boid_forces` reads them. Separate
/// from [`SimParams`] so the Boids-mode controls are self-contained.
#[derive(Resource, Clone, Copy, Debug)]
pub struct BoidParams {
    /// Neighbor radius in world units: boids within this distance influence each
    /// other. Also the spatial-grid cell size (plan §6.4 / D11).
    pub radius: f32,
    /// Separation weight: strength of the push away from close neighbors.
    pub separation: f32,
    /// Alignment weight: strength of steering toward neighbors' average heading.
    pub alignment: f32,
    /// Cohesion weight: strength of steering toward neighbors' average position.
    pub cohesion: f32,
    /// Maximum boid speed in world units per second (velocity is clamped to it).
    pub max_speed: f32,
}

impl Default for BoidParams {
    fn default() -> Self {
        // Tuned for a few tens of thousands of boids in the +/-100 world box: a
        // radius small enough to keep the 3x3 grid neighborhood cheap, with the
        // classic separation > alignment ~ cohesion balance that reads as
        // flocking rather than clumping or scattering.
        Self {
            radius: 6.0,
            separation: 24.0,
            alignment: 6.0,
            cohesion: 4.0,
            max_speed: 60.0,
        }
    }
}

/// Pre-tick snapshot of one boid's state (plan D12). `#[repr(C)]` + `Copy` so the
/// snapshot buffer is a flat, cache-friendly array.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct BoidState {
    /// World position at the start of the step.
    pub pos: [f32; 2],
    /// World velocity at the start of the step.
    pub vel: [f32; 2],
}

/// Pre-tick `(position, velocity)` snapshot of every boid (plan D12).
///
/// The force pass reads neighbors' PREVIOUS-frame state from this snapshot while
/// writing the live `Velocity` column, so a worker never reads a row a sibling is
/// writing — that makes `boid_forces` a sound `par_iter` (snapshot + grid
/// read-only, each boid writes only its own row). Refreshed each step by
/// `snapshot_boids`; the `Vec` is sized once and refilled (no per-frame alloc).
#[derive(Resource, Default)]
pub struct BoidSnapshot {
    /// One entry per boid, in archetype row order — the same order the grid
    /// indexes and the force pass iterates.
    pub state: Vec<BoidState>,
}

impl BoidSnapshot {
    /// Builds a snapshot buffer pre-sized for up to `max_boids` (no later
    /// reallocation in steady state).
    pub fn with_capacity(max_boids: usize) -> Self {
        Self {
            state: Vec::with_capacity(max_boids),
        }
    }
}

/// Tunable physics (bouncing balls) parameters (plan §6.3 / §7 / Wave 6). The
/// control panel mutates these in place each frame; the physics systems read
/// them. Separate from [`SimParams`]/[`BoidParams`] so the Physics-mode controls
/// are self-contained.
#[derive(Resource, Clone, Copy, Debug)]
pub struct PhysicsParams {
    /// Downward gravitational acceleration in world units per second². `0`
    /// (the default) lets balls drift freely so the `Changed<Velocity>` tint
    /// flashes only genuine collisions/bounces (see `systems::physics` docs).
    pub gravity: f32,
    /// Coefficient of restitution `e ∈ [0, 1]` (1 = perfectly elastic) applied to
    /// both ball-ball impulses and wall bounces.
    pub restitution: f32,
    /// Rendered half-extent of a ball quad in world units. Read by the
    /// physics GPU sync so the size slider drives the dot size live.
    pub ball_size: f32,
}

impl Default for PhysicsParams {
    fn default() -> Self {
        // Gravity defaults to 0 so the change-detection tint isolates
        // collisions/bounces (raising it flashes every ball — a documented
        // footgun). A high restitution keeps the balls lively.
        Self {
            gravity: 0.0,
            restitution: 0.9,
            ball_size: 1.4,
        }
    }
}

/// Pre-tick snapshot of every ball plus the collision solver's scratch buffers
/// (plan D13 / G12).
///
/// The sequential collision + wall passes need random per-row access by archetype
/// row index, which a forward `iter_mut` cannot give; so `build_ball_grid`
/// snapshots the ball columns into these flat `Vec`s, the solver mutates them by
/// index on one thread, and `apply_ball_motion` writes the result back in the
/// same row order. Every `Vec` is cleared and refilled each frame, never
/// reallocated in steady state (plan §11.2). Row index `i` is consistent across
/// all of `pos`/`vel`/`radius`/`touched` and matches the archetype row order.
#[derive(Resource, Default)]
pub struct BallSnapshot {
    /// Ball centers, in archetype row order.
    pub pos: Vec<Position>,
    /// Ball velocities, parallel to [`pos`](Self::pos).
    pub vel: Vec<Velocity>,
    /// Ball radii, parallel to [`pos`](Self::pos).
    pub radius: Vec<f32>,
    /// `true` for rows whose velocity changed this frame (collision or wall
    /// bounce). Drives the per-row velocity write-back so only those rows bump
    /// their change tick (keeping `Changed<Velocity>` precise).
    pub touched: Vec<bool>,
    /// Reused scratch list of candidate neighbor row indices for the ball
    /// currently being resolved — a per-frame-reused buffer instead of a
    /// per-ball heap allocation in the collision loop (plan §11.2).
    pub candidates: Vec<usize>,
}

impl BallSnapshot {
    /// Builds snapshot buffers pre-sized for up to `max_balls` (no later
    /// reallocation in steady state).
    pub fn with_capacity(max_balls: usize) -> Self {
        Self {
            pos: Vec::with_capacity(max_balls),
            vel: Vec::with_capacity(max_balls),
            radius: Vec::with_capacity(max_balls),
            touched: Vec::with_capacity(max_balls),
            // A 3×3 neighbor block holds only a handful of balls at the chosen
            // cell size; a small reserve avoids growth in the common case.
            candidates: Vec::with_capacity(64),
        }
    }

    /// Clears every per-ball buffer for a fresh frame (reusing capacity). The
    /// `candidates` scratch is cleared per-ball by the solver, not here.
    #[inline]
    pub fn clear(&mut self) {
        self.pos.clear();
        self.vel.clear();
        self.radius.clear();
        self.touched.clear();
    }

    /// Appends one ball's snapshot row (position, velocity, radius), initially
    /// untouched.
    #[inline]
    pub fn push(&mut self, pos: Position, vel: Velocity, radius: f32) {
        self.pos.push(pos);
        self.vel.push(vel);
        self.radius.push(radius);
        self.touched.push(false);
    }
}

/// Capacity of the rolling frame-time history (plan §7 / §11.2: a fixed ring, no
/// per-frame allocation). At ~120 samples a 60 FPS plot shows the last ~2 s.
pub const FRAME_HISTORY_LEN: usize = 120;

/// Rolling per-frame statistics for the control panel's readouts and FPS plot
/// (plan §7 / §11.2).
///
/// Owned by the app shell (`DemoApp`), not registered as an ECS resource: the
/// stats are produced by the shell (frame timing, entity count) and consumed by
/// the shell (the egui panel), so routing them through the world would add
/// plumbing for no benefit.
///
/// The history is a **fixed-size ring** written in place — there is no per-frame
/// allocation on the hot path (plan principle 5 / §11.2). [`Self::push`] advances
/// a head cursor and overwrites the oldest sample; [`Self::iter_chronological`]
/// yields the samples oldest-first for plotting.
#[derive(Clone, Copy, Debug)]
pub struct FrameStats {
    /// Total wall time of the most recent frame, in milliseconds.
    pub frame_ms: f32,
    /// Time spent inside `SimRunner::step` (the schedule) last frame, in
    /// milliseconds. Always `<= frame_ms`.
    pub sim_ms: f32,
    /// Live entity count after the most recent step.
    pub entity_count: u32,
    /// Ring buffer of recent total frame times in milliseconds, oldest-first
    /// once filled. Indexing is via `head`; read it through
    /// [`Self::iter_chronological`].
    history: [f32; FRAME_HISTORY_LEN],
    /// Index of the next slot to write (the oldest sample once the ring is
    /// full). In `0..FRAME_HISTORY_LEN`.
    head: usize,
    /// Number of samples written so far, saturating at [`FRAME_HISTORY_LEN`].
    /// Lets the plot ignore the unwritten tail before the ring fills.
    filled: usize,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            frame_ms: 0.0,
            sim_ms: 0.0,
            entity_count: 0,
            history: [0.0; FRAME_HISTORY_LEN],
            head: 0,
            filled: 0,
        }
    }
}

impl FrameStats {
    /// Records one frame's stats: stores the scalar readouts and appends
    /// `frame_ms` to the ring.
    ///
    /// In-place write of a fixed array slot — no allocation (plan §11.2).
    #[inline]
    pub fn push(&mut self, frame_ms: f32, sim_ms: f32, entity_count: u32) {
        self.frame_ms = frame_ms;
        self.sim_ms = sim_ms;
        self.entity_count = entity_count;

        self.history[self.head] = frame_ms;
        self.head = (self.head + 1) % FRAME_HISTORY_LEN;
        if self.filled < FRAME_HISTORY_LEN {
            self.filled += 1;
        }
    }

    /// Number of valid samples currently in the ring (`<= FRAME_HISTORY_LEN`).
    #[inline]
    pub fn len(&self) -> usize {
        self.filled
    }

    /// Whether the ring holds no samples yet.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.filled == 0
    }

    /// Iterates the recorded frame times oldest-first.
    ///
    /// Before the ring fills, this yields only the `filled` written samples (in
    /// write order). Once full, it walks from the oldest slot (`head`) around.
    /// Borrows the ring — no allocation.
    pub fn iter_chronological(&self) -> impl Iterator<Item = f32> + '_ {
        // Before the ring is full the written samples occupy `0..filled` in
        // order; `head == filled` there, so starting at `head` and stepping
        // `filled` times still visits them oldest-first.
        let start = if self.filled < FRAME_HISTORY_LEN {
            0
        } else {
            self.head
        };
        (0..self.filled).map(move |i| self.history[(start + i) % FRAME_HISTORY_LEN])
    }
}
