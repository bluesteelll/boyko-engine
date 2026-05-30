//! ECS resources for the simulation (plan §6.3).
//!
//! Resources are the per-frame inputs and tunables systems read. The app owns
//! `&mut EcsMaster`, so the UI and the input pump write these directly
//! (`world.resource_mut::<_>()`) and the next `Schedule::run` picks them up with
//! zero plumbing. `DeltaTime` and `InputState` cover gaps the engine leaves to
//! the application (plan §9 G8/G9: no built-in `Time`/input resource).

use boyko_macros::Resource;

/// Fixed simulation timestep for the current run, in seconds (plan §9 G8).
///
/// The engine has no built-in `Time`; the runner writes this before each
/// `Schedule::run` so systems integrate against a stable `dt`.
#[derive(Resource, Clone, Copy, Debug)]
pub struct DeltaTime(pub f32);

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
