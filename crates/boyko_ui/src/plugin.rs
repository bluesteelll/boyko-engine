//! [`UiPlugin`] — the Phase-18 [`Plugin`] that loads a `.ui` document into an
//! [`App`] and (optionally) hot-reloads it (P3 §Public API).
//!
//! Mirrors the `boyko_input` `InputPlugin` precedent: a builder configures a
//! path + options, and [`build`](Plugin::build) reads the file, lowers it via
//! `Commands` at startup (so the spawned tree is byte-identical to a `ui!`
//! invocation), and — when hot-reload is enabled — inserts the [`UiHotReload`]
//! watch resource and registers [`ui_hot_reload_system`] on
//! [`CoreSchedule::Main`].
//!
//! A missing / unreadable `.ui` file is a graceful no-op (an empty tree), never
//! a panic — the `.keys` graceful-fallback discipline.
//!
//! [`Plugin`]: boyko_ecs::ecs::core::app::Plugin
//! [`App`]: boyko_ecs::ecs::core::app::App
//! [`CoreSchedule`]: boyko_ecs::ecs::core::app::CoreSchedule

use std::time::{Duration, SystemTime};

use boyko_ecs::ecs::core::app::{App, CoreSchedule, Plugin};
use boyko_ecs::ecs::core::system::{Commands, ResMut};

use crate::reload::state::{UiHotReload, DEFAULT_POLL_INTERVAL};
use crate::reload::system::ui_hot_reload_system;
use crate::text::lower::spawn_ui_tree;
use crate::text::parser::parse_ui;

/// Loads a `.ui` file at build and (optionally) hot-reloads it.
///
/// Build it with [`UiPlugin::new`], set the path with
/// [`with_ui_path`](Self::with_ui_path), and add it with
/// `app.add_plugin(UiPlugin::new().with_ui_path("ui/main.ui"))`. Without a path,
/// the plugin is a no-op (nothing is spawned, no system is registered).
#[derive(Clone, Debug)]
pub struct UiPlugin {
    /// The `.ui` document path; `None` ⇒ the plugin does nothing.
    path: Option<&'static str>,
    /// Whether to register the poll watch system for hot-reload.
    hot_reload: bool,
    /// The watch poll interval (default 250 ms).
    poll_interval: Duration,
}

impl UiPlugin {
    /// Creates a plugin with no path and hot-reload enabled by default.
    #[inline]
    pub fn new() -> Self {
        Self {
            path: None,
            hot_reload: true,
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }

    /// Sets the `.ui` document path (the analog of `with_keys_path`).
    #[inline]
    pub fn with_ui_path(mut self, path: &'static str) -> Self {
        self.path = Some(path);
        self
    }

    /// Enables or disables hot-reload (the poll watch system).
    #[inline]
    pub fn with_hot_reload(mut self, enabled: bool) -> Self {
        self.hot_reload = enabled;
        self
    }

    /// Overrides the hot-reload poll interval.
    #[inline]
    pub fn with_poll_interval(mut self, d: Duration) -> Self {
        self.poll_interval = d;
        self
    }
}

impl Default for UiPlugin {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        let Some(path) = self.path else {
            return; // no path: a no-op plugin
        };

        // Read the file gracefully: a missing/unreadable file → empty source, so
        // the initial spawn is a no-op (never a panic; a fresh install may have
        // no `.ui` yet).
        let src = std::fs::read_to_string(path).unwrap_or_default();
        let signature = read_signature(path);

        if self.hot_reload {
            // Insert the watch resource so the startup load can record the
            // document's roots into it.
            app.insert_resource(UiHotReload::with_poll_interval(path, self.poll_interval));

            // Startup: parse + lower + capture doc_roots + seed the signature so
            // the first poll does not re-load the unchanged file. Lowering reuses
            // the parse report so lower-time re-parse errors stay reachable
            // (recorded on `UiHotReload::last_report`).
            app.add_startup_system(move |mut cmds: Commands, mut hot: ResMut<UiHotReload>| {
                let tree = parse_ui(&src);
                let mut report = tree.report.clone();
                let roots = spawn_ui_tree(&tree, &mut cmds, &mut report);
                hot.set_doc_roots(roots);
                hot.last_report = report;
                hot.seed_signature(signature.map(|s| s.0), signature.map(|s| s.1).unwrap_or(0));
            });

            // Register the poll watch system on the Main step.
            app.add_systems_in(CoreSchedule::Main, ui_hot_reload_system);
        } else {
            // No hot-reload: just lower the tree once at startup. The lower-time
            // report has nowhere to land (no watch resource), so it is dropped
            // after lowering — a clean `parse_ui` implies a clean lowering by the
            // identical-value-grammar invariant.
            app.add_startup_system(move |mut cmds: Commands| {
                let tree = parse_ui(&src);
                let mut report = tree.report.clone();
                let _ = spawn_ui_tree(&tree, &mut cmds, &mut report);
            });
        }
    }
}

/// Reads `(mtime, size)` for `path`, or `None` on any I/O error.
fn read_signature(path: &str) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    Some((mtime, meta.len()))
}
