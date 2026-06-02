//! Phase 18 — the [`App`] builder facade + [`Plugin`] composition layer.
//!
//! An additive, single-threaded-owned façade over the shipped `EcsMaster`,
//! `ScheduleBuilder`, `Schedule`, and `ThreadPool`. It adds no per-frame
//! overhead: the runner lowers to `Schedule::run`, and all plugin / tuple
//! machinery is cold setup-only code.

#[allow(clippy::module_inception)]
pub mod app;
pub mod app_exit;
pub mod plugin;
pub mod plugins;

pub use app::App;
pub use app_exit::AppExit;
pub use plugin::Plugin;
pub use plugins::{PluginMarker, Plugins};
