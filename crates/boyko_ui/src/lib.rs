//! boyko_ui — ECS-native UI. P1: layout components + the in-house layout systems.
//!
//! Widgets are entities; layout inputs/outputs are components; the tree is
//! `ChildOf`/`Children` (Phase 19); layout is two systems over the ECS. There is
//! no parallel data system — props/outputs are ECS columns and the per-frame
//! scratch is a `Resource`-owned engine buffer (frame-transient, reset every
//! frame).
//!
//! # The two systems
//!
//! * [`ui_layout_discovery`](layout::ui_layout_discovery) — a normal scheduled
//!   `FunctionSystem`. SystemParams supply the `(last_run, this_run]` window that
//!   `Changed`/`Added` require, so this is where change detection lives. It sets a
//!   per-frame `dirty` flag in [`LayoutScratch`](resources::LayoutScratch).
//! * [`ui_layout_apply`](layout::ui_layout_apply) — an exclusive system
//!   (`&mut EcsMaster`, the only form that expresses nested parent↔child mutable
//!   row access without unsafe aliasing). When `dirty` (or the viewport resized)
//!   it re-lays-out the root subtrees.
//!
//! Schedule them in that order, after all structural/prop-mutation systems (the
//! `Children`/`ChildOf` consistency window).

pub mod bundles;
pub mod components;
pub mod layout;
pub mod plugin;
pub mod reload;
pub mod resources;
pub mod text;
pub mod units;

/// The `ui!` authoring macro (P2): write a UI entity tree as a literal nested
/// block. See [`boyko_macros::ui`] for the grammar and expansion contract.
pub use boyko_macros::ui;

/// The `.ui` text format (P3): [`parse_ui`](text::parse_ui),
/// [`spawn_ui_tree`](text::spawn_ui_tree), [`serialize_ui`](text::serialize_ui),
/// and the runtime hot-reload [`UiPlugin`](plugin::UiPlugin).
pub use plugin::UiPlugin;
pub use text::{parse_ui, serialize_ui, spawn_ui_tree, UI_FORMAT_VERSION};

/// Crate-local convenience re-exports (no engine-wide prelude).
pub mod prelude {
    pub use crate::bundles::UiNodeBundle;
    pub use crate::components::{
        ComputedClip, ComputedRect, ContentSize, StackIndex, UiAbsolute, UiAlign, UiLayout, UiName,
        UiRoot, UiSpacing,
    };
    pub use crate::layout::{ui_layout_apply, ui_layout_discovery};
    pub use crate::plugin::UiPlugin;
    pub use crate::resources::{LayoutScratch, UiViewport};
    pub use crate::text::{parse_ui, serialize_ui, spawn_ui_tree, UI_FORMAT_VERSION};
    pub use crate::units::{AlignCross, AlignMain, LayoutType, PositionType, Unit};

    pub use boyko_macros::ui;
}
