//! Phase 18 — the [`App`] builder facade + [`Plugin`] composition layer;
//! Phase 20 adds the multi-schedule frame driver ([`CoreSchedule`],
//! fixed-timestep loop, [`EventUpdatePolicy`]).
//!
//! An additive, single-threaded-owned façade over the shipped `EcsMaster`,
//! `ScheduleBuilder`, `Schedule`, and `ThreadPool`. The frame driver lowers
//! to the `Schedule::run`s plus a declared, bench-bound additive envelope
//! (clock advance + three predictable branches — Phase 20 P20-B1); all
//! plugin / tuple machinery is cold setup-only code.

#[allow(clippy::module_inception)]
pub mod app;
pub mod app_exit;
pub mod plugin;
pub mod plugins;

pub use app::{App, CoreSchedule, EventUpdatePolicy};
pub use app_exit::AppExit;
pub use plugin::Plugin;
pub use plugins::{PluginMarker, Plugins};
