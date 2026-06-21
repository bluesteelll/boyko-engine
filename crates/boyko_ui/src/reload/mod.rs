//! Hot-reload of a `.ui` document: mtime+size poll watch + diff-by-`UiName`
//! reconcile that preserves transient runtime state (P3).
//!
//! * [`state`] — the [`UiHotReload`] watch `Resource` + the [`SmallRoots`]
//!   document-scope anchor.
//! * [`system`] — the throttled poll watch system ([`ui_hot_reload_system`]),
//!   zero-alloc / zero-tree-read on the no-change path (Decision 7 / 8).
//! * [`reconcile`] — the scoped, soundness-fixed reconciler
//!   ([`reconcile_ui`], Decision 9 / 10 / 11 / 13 / 14).
//! * [`tree_view`] — the owned read snapshot ([`UiTreeView`]) the reconcile +
//!   serializer read.

pub mod reconcile;
pub mod state;
pub mod system;
pub mod tree_view;

pub use reconcile::{apply_despawns, reconcile_ui, DespawnPlan};
pub use state::{SmallRoots, UiHotReload, DEFAULT_POLL_INTERVAL};
pub use system::ui_hot_reload_system;
pub use tree_view::{LiveNode, UiTreeView};
