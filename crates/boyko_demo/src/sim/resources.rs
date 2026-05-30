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

/// Tunable simulation parameters (plan §6.3). The Wave-3 MVP exposes the subset
/// the particle integrator reads; later waves extend this with boid/physics
/// fields and the UI sliders that write them.
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
            paused: false,
        }
    }
}
