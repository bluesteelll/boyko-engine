//! [`EnginePlugins`] — the host composition plugin (host plan D1/D6, R2
//! subset): installs the windowed clear-color runner.

use boyko_ecs::{App, Plugin};

use crate::runner::{self, WindowDesc};

/// The engine host plugin: opens a window and installs the windowed runner
/// (device-singleton boot, frame loop, D2 teardown) via `App::set_runner`.
///
/// R2 keeps the surface minimal — title + client size; `present_mode` and the
/// rest of the windowing knobs arrive with later rungs.
///
/// ```no_run
/// use boyko_app::prelude::*;
///
/// let mut app = App::new();
/// app.add_plugins(EnginePlugins::window("my game", 800, 600));
/// app.run();
/// ```
pub struct EnginePlugins {
    /// The window caption.
    title: &'static str,
    /// Requested client-area width in pixels.
    width: u32,
    /// Requested client-area height in pixels.
    height: u32,
}

impl EnginePlugins {
    /// A windowed host with the given caption and requested client size.
    #[inline]
    pub fn window(title: &'static str, width: u32, height: u32) -> Self {
        Self {
            title,
            width,
            height,
        }
    }
}

impl Plugin for EnginePlugins {
    /// Installs the windowed runner. `App::run` hands it control BEFORE
    /// `finish()`; the runner owns the app lifecycle from there (its own
    /// `finish()` call, `AppExit` policy, and teardown — see `runner.rs`).
    fn build(&self, app: &mut App) {
        let desc = WindowDesc {
            title: self.title,
            width: self.width,
            height: self.height,
        };
        app.set_runner(Box::new(move |app: &mut App| {
            runner::run_windowed(app, desc)
        }));
    }
}
