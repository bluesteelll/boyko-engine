//! [`FlyCameraPlugin`] — the R6 interactive-camera host bundle.
//!
//! Composes the input + fly-camera stack for a windowed scene:
//!
//! * the source-agnostic [`InputPlugin<FlyAction>`] (its `update_action_state`
//!   ingest is what populates [`PhysicalInput::mouse_delta`], so a
//!   [`FlyAction`]-typed `InputPlugin` is REQUIRED even though mouse-look has no
//!   action binding);
//! * [`fly_camera_system`](boyko_scene::fly_camera_system) joined to
//!   [`CameraSet::Control`] — the [`CameraPlugin`](boyko_scene::CameraPlugin)
//!   orders `Control.before(Resolve)`, so the fly write recomposes the view the
//!   SAME frame (no one-frame lag);
//! * [`quit_on_action`] — the ECS-native quit: it sets `AppExit(true)` when the
//!   rebindable [`FlyAction::Quit`] fires (Escape by default), so the runner
//!   stays input-agnostic (its existing step-3 `AppExit` check handles it).
//!
//! # Why the runner still has an inline Escape fallback
//!
//! A scene WITHOUT `FlyCameraPlugin` (`examples/room.rs`, `clear.rs`) inserts no
//! [`RawInputQueue`](boyko_input::RawInputQueue), so there is nothing for
//! `update_action_state` to fold and no [`ActionState<FlyAction>`] for
//! `quit_on_action` to read. The runner's step-1 keeps a lightweight inline
//! Escape scan of the drained OS events for exactly that case (see
//! `runner::frame_loop`). The two paths are non-redundant: the ECS path runs
//! only when the queue is present, the inline scan only when it is absent.
//!
//! # Direct `PhysicalInput` reads (the parity fork)
//!
//! `fly_camera_system` reads WASD / Space / Ctrl / Q / E + the mouse delta
//! DIRECTLY from `PhysicalInput` (parity-faithful with the reference viewer).
//! `FlyAction` exists ONLY to (a) satisfy `InputPlugin<A>` (populate the mouse
//! delta) and (b) provide a rebindable quit. A v2 migration would extend
//! `FlyAction` with an `Axis2D` move binding for rebindable movement; the
//! component/plugin shape is unchanged (only the system's read source flips).

use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::{App, AppExit, Plugin};
use boyko_input::{ActionState, Actionlike, BindSpec, GameplaySet, InputMap, InputPlugin, KeyCode};
use boyko_scene::{CameraSet, fly_camera_system};

/// The action set the fly camera binds (host plan R6).
///
/// A single [`Quit`](FlyAction::Quit) action, bound to Escape by default via
/// [`fly_default_map`]. Movement + look are NOT actions in v1 — they are direct
/// [`PhysicalInput`](boyko_input::PhysicalInput) reads in
/// [`fly_camera_system`](boyko_scene::fly_camera_system) (see the module docs).
/// `Quit` defaults to [`ActionKind::Button`](boyko_input::ActionKind::Button).
#[derive(Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlyAction {
    /// Request application exit. Bound to [`KeyCode::Escape`] by default;
    /// rebindable through the `.keys` override / `RebindSession`.
    Quit,
}

/// The default binding map for [`FlyAction`]: `Quit → Escape`.
///
/// Cold path (built once at plugin build). Extend the returned builder with
/// additional default bindings before shipping a game; a `.keys` override-delta
/// then rebinds them at runtime (plan §9.3).
#[inline]
pub fn fly_default_map() -> InputMap<FlyAction> {
    InputMap::builder()
        .bind(FlyAction::Quit, BindSpec::Key(KeyCode::Escape))
        .build()
}

/// Sets `AppExit(true)` when [`FlyAction::Quit`] is pressed — the ECS-native
/// quit (host plan R6, orchestrator resolution).
///
/// Reads the processed [`ActionState<FlyAction>`] (folded once per frame by
/// `InputPlugin<FlyAction>`'s ingest) rather than the raw key level, so the quit
/// respects rebinding + clash resolution. The runner's step-3 `AppExit` check
/// observes the flag after the frame and exits.
//
// `clippy::needless_pass_by_value`: `Res` / `ResMut` are by-value `SystemParam`s
// reborrowed internally — the same false-positive the engine's systems carry.
#[allow(clippy::needless_pass_by_value)]
fn quit_on_action(actions: Res<ActionState<FlyAction>>, mut exit: ResMut<AppExit>) {
    if actions.pressed(FlyAction::Quit) {
        exit.0 = true;
    }
}

/// The interactive fly-camera host plugin (host plan R6).
///
/// Add it ALONGSIDE `EnginePlugins` for an interactive windowed scene:
///
/// ```no_run
/// use boyko_app::prelude::*;
///
/// let mut app = App::new();
/// app.add_plugins(EnginePlugins::window("viewer", 800, 600));
/// app.add_plugin(FlyCameraPlugin);
/// // spawn a `FlyCameraBundle`, then `app.run()`.
/// ```
///
/// It composes [`InputPlugin<FlyAction>`] (the input ingest — do NOT add another
/// `InputPlugin<FlyAction>`, a duplicate plugin panics), registers
/// [`fly_camera_system`](boyko_scene::fly_camera_system) in
/// [`CameraSet::Control`], and registers [`quit_on_action`]. `EnginePlugins`
/// itself adds NO `InputPlugin` (it is action-type-agnostic), so an input-free
/// scene (`room.rs`) omits this plugin and keeps the runner's inline Escape
/// fallback.
#[derive(Default)]
pub struct FlyCameraPlugin;

impl Plugin for FlyCameraPlugin {
    fn build(&self, app: &mut App) {
        // The input ingest: inserts RawInputQueue + PhysicalInput +
        // ActionState<FlyAction> + InputMap<FlyAction>, and registers
        // `update_action_state::<FlyAction>` (which populates mouse_delta). The
        // presence of RawInputQueue is what flips the runner's step-1 bridge on.
        app.add_plugin(InputPlugin::<FlyAction>::new(fly_default_map()));

        // The fly controller + the quit both join `GameplaySet` so they run
        // AFTER `update_action_state::<FlyAction>` (which the `InputPlugin`
        // registers `.before_set(GameplaySet)`): the ingest fills THIS frame's
        // `PhysicalInput` + `ActionState` snapshot, so the fly reads the current
        // frame's input (no one-frame input lag) and the quit reads the current
        // frame's `Quit` edge. The fly ADDITIONALLY joins `CameraSet::Control`,
        // which `CameraPlugin` (composed by `EnginePlugins`) orders
        // `Control.before(Resolve)`, so the fly write is propagated + resolved the
        // SAME frame (no one-frame VIEW lag). A system may join multiple sets;
        // the two memberships give both edges (ingest → fly → propagate/resolve).
        app.add_systems_cfg(|b| {
            b.add_system(fly_camera_system)
                .in_set(CameraSet::Control)
                .in_set(GameplaySet);
            b.add_system(quit_on_action).in_set(GameplaySet);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_app::FlyCameraPlugin"
    }
}
