//! World-space / diegetic UI (GUI P7a) — anchor a UI subtree to a 3D world point.
//!
//! A world-anchored UI subtree is a normal [`UiRoot`](crate::components::UiRoot)
//! carrying a [`UiWorldAnchor`]; its screen origin tracks a fixed
//! [`WorldTarget::WorldPos`] or a live [`WorldTarget::EntityAnchor`]'s
//! `GlobalTransform`, projected through the S3 camera each frame by
//! [`ui_world_project_system`]. A [`HoveredWorldEntity`] resource + the
//! [`ui_world_visibility_system`] give an O(1) show/hide path; P7b's
//! [`ui_world_pick_system`] populates the resource from the cursor ray (and
//! computes the [`UiWorldOccluded`] depth-test bit).
//!
//! P7a is the CPU-testable projection / visibility core. P7b (now implemented)
//! adds the CPU cursor-ray PICK ([`UiPickable`] bounds → [`HoveredWorldEntity`])
//! and a CPU occlusion PROXY ([`UiWorldOccluded`]). STILL OUT OF SCOPE (deferred):
//! a GPU depth-buffer occlusion test and the depth-test render pass (the
//! [`UiWorldProjection::depth`] field is stored for a future GPU-depth / z-order
//! consumer).
//!
//! # Module map
//!
//! * [`components`] — [`UiWorldAnchor`] / [`UiWorldProjection`] / [`WorldTarget`]
//!   / [`WorldScaleMode`] / [`HoveredWorldEntity`] / [`UiPickable`] /
//!   [`UiPickShape`] + the [`UiWorldCulled`] / [`UiWorldHidden`] /
//!   [`UiWorldOccluded`] EnableTags.
//! * [`project`] — [`project_world_to_screen`] (pure math) + the project system.
//! * [`visibility`] — the hover-driven show/hide system + its tracked state.
//! * [`pick`] — the cursor-ray pick + depth-test occlusion system (P7b).
//!
//! # Schedule contract (host-owned, like P1 / P5b)
//!
//! Register `ui_world_project_system` `.after(resolve_active_camera)` (fresh
//! `ViewUniform`) and `.after(propagate_transforms)` (fresh `GlobalTransform`),
//! and `.before(ui_layout_discovery)` (so the same-frame relayout sees the new
//! origin). Register `ui_world_pick_system` `.after(resolve_active_camera)` (fresh
//! `ViewUniform`) and `.after(propagate_transforms)` (fresh `GlobalTransform`),
//! and `.before(ui_world_visibility_system)` (which consumes `HoveredWorldEntity`)
//! — so it also precedes the layout pass transitively (the same-frame layout sees
//! the fresh `UiWorldOccluded`). `ui_world_pick_system` and `ui_world_project_system`
//! are independent (both read the same snapshots, neither writes the other's
//! outputs; `UiWorldOccluded` and `UiWorldCulled` are distinct bits) and may run in
//! either relative order. `ui_world_visibility_system` runs in the apply window (it
//! is exclusive). The layout pass skips a world root with [`UiWorldCulled`],
//! [`UiWorldHidden`], or [`UiWorldOccluded`] set.

pub mod components;
pub mod pick;
pub mod project;
pub mod visibility;

pub use components::{
    HoveredWorldEntity, UiPickShape, UiPickable, UiWorldAnchor, UiWorldCulled, UiWorldHidden,
    UiWorldOccluded, UiWorldProjection, WorldScaleMode, WorldTarget,
};
pub use pick::{ui_world_pick_system, UiWorldScratch};
pub use project::{project_world_to_screen, ui_world_project_system, ProjectedPoint};
pub use visibility::{ui_world_visibility_system, UiWorldHoverState};
