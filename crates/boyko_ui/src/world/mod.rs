//! World-space / diegetic UI (GUI P7a) — anchor a UI subtree to a 3D world point.
//!
//! A world-anchored UI subtree is a normal [`UiRoot`](crate::components::UiRoot)
//! carrying a [`UiWorldAnchor`]; its screen origin tracks a fixed
//! [`WorldTarget::WorldPos`] or a live [`WorldTarget::EntityAnchor`]'s
//! `GlobalTransform`, projected through the S3 camera each frame by
//! [`ui_world_project_system`]. A [`HoveredWorldEntity`] resource + the
//! [`ui_world_visibility_system`] give an O(1) show/hide path (the future P7b
//! GPU cursor-ray pick populates the resource).
//!
//! P7a is the CPU-testable core. OUT OF SCOPE (P7b, a GPU session): the cursor-ray
//! pick from the G-buffer depth/SDF, and depth-test render.
//!
//! # Module map
//!
//! * [`components`] — [`UiWorldAnchor`] / [`UiWorldProjection`] / [`WorldTarget`]
//!   / [`WorldScaleMode`] / [`HoveredWorldEntity`] + the [`UiWorldCulled`] /
//!   [`UiWorldHidden`] EnableTags.
//! * [`project`] — [`project_world_to_screen`] (pure math) + the project system.
//! * [`visibility`] — the hover-driven show/hide system + its tracked state.
//!
//! # Schedule contract (host-owned, like P1 / P5b)
//!
//! Register `ui_world_project_system` `.after(resolve_active_camera)` (fresh
//! `ViewUniform`) and `.after(propagate_transforms)` (fresh `GlobalTransform`),
//! and `.before(ui_layout_discovery)` (so the same-frame relayout sees the new
//! origin). `ui_world_visibility_system` runs in the apply window (it is
//! exclusive). The layout pass skips a world root with either [`UiWorldCulled`]
//! or [`UiWorldHidden`] set.

pub mod components;
pub mod project;
pub mod visibility;

pub use components::{
    HoveredWorldEntity, UiWorldAnchor, UiWorldCulled, UiWorldHidden, UiWorldProjection,
    WorldScaleMode, WorldTarget,
};
pub use project::{project_world_to_screen, ui_world_project_system, ProjectedPoint};
pub use visibility::{ui_world_visibility_system, UiWorldHoverState};
