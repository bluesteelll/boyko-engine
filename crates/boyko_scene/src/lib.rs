//! `boyko_scene` — the engine's spatial vocabulary and transform propagation
//! (standard-library Phase S2).
//!
//! This crate sits one layer above the ECS kernel and owns the **spatial**
//! components every world-space subsystem (renderer, lights, camera, physics
//! sync) builds on:
//!
//! * [`Transform`] — the LOCAL, decomposed, designer-facing pose (relative to
//!   the parent).
//! * [`GlobalTransform`] — the cached WORLD pose, a packed
//!   [`Affine3A`](boyko_math::Affine3A), recomputed each frame.
//!
//! and the system that derives one from the other:
//!
//! * [`propagate_transforms`] — composes every entity's `GlobalTransform` from
//!   its `Transform` chain along the `ChildOf` / `Children` hierarchy, alloc-free
//!   and dirty-gated.
//!
//! # Principle 0 (no parallel pose data system)
//!
//! `Transform` and `GlobalTransform` are ordinary ECS component columns on the
//! kernel's own storage. The propagation system's only transient state lives in
//! a kernel-owned [`Resource`](boyko_ecs::ecs::core::resources::resource::Resource)
//! ([`TransformPropagationScratch`]) whose buffers are reused frame-to-frame —
//! there is no side `Vec` / `HashMap` pose store anywhere.
//!
//! # Quickstart
//!
//! ```ignore
//! use boyko_scene::prelude::*;
//!
//! app.add_plugins(TransformPlugin);
//! // spawn entities with `Transform` + `GlobalTransform`; parent them with
//! // `ChildOf`; `GlobalTransform` is filled in each frame by `propagate_transforms`.
//! ```
//!
//! [`Transform`]: crate::transform::Transform
//! [`GlobalTransform`]: crate::transform::GlobalTransform
//! [`propagate_transforms`]: crate::propagation::propagate_transforms
//! [`TransformPropagationScratch`]: crate::propagation::TransformPropagationScratch


// `missing_const_for_thread_local` is a FALSE POSITIVE on clippy 1.98.0
// (2026-09-01), and this allow is the repair rather than a suppression: a sweep
// of the workspace found 63 `thread_local!` statics across 12 crates and ALL 63
// already use the `const { … }` form the lint asks for, so it has no true
// positive here to hide.
//
// Two cures were tried and neither works. The 1.97.1 -> 1.98.1 toolchain update
// was taken specifically for this; it changed which crates report but did not
// remove the lint, so "wait for upstream" is not a live plan. The
// neighbouring-doc-comment confusion this lint has had before is not the cause
// either: stripping the `///` lines above a flagged static leaves the bare
// `const { … }` form and it still fires.
//
// Placement is crate-level because an `#[allow]` written OUTSIDE a
// `thread_local!` invocation is reported as an `unused attribute` while the lint
// fires anyway. Delete when clippy stops reporting the const form; the sweep
// above is the check that this is still safe to delete blind.
#![allow(clippy::missing_const_for_thread_local)]

pub mod asset_refs;
pub mod bundles;
pub mod camera;
pub mod camera_plugin;
pub mod identity;
pub mod plugin;
pub mod propagation;
pub mod render_caps;
pub mod sets;
pub mod transform;
pub mod visibility_sync;

pub use asset_refs::{AssetRefKind, DeferredFree, FreeEntry, RefDelta, RefcountDeltas};
pub use bundles::{CameraRig, FlyCameraBundle, SpatialBundle, StaticProp};
pub use camera::{
    ActiveCamera, Camera, FlyCamera, OrbitCamera, Projection, ViewUniform, Viewport,
    fly_camera_system, orbit_camera_system, resolve_active_camera,
};
pub use camera_plugin::CameraPlugin;
pub use identity::{Name, NameId, intern, resolve};
pub use plugin::TransformPlugin;
pub use propagation::{TransformPropagationScratch, compute_global_transform, propagate_transforms};
pub use render_caps::{MaterialHandle, MaterialRefGen, MeshHandle, MeshRefGen, RenderEnabled, Visibility};
pub use sets::{CameraSet, FixedSet};
pub use transform::{GlobalTransform, Transform};
pub use visibility_sync::visibility_sync;

/// Common `boyko_scene` imports.
///
/// Re-exports the spatial components, the propagation entry points, and the
/// plugin. The `#[derive(Component)]` / `#[derive(Resource)]` macros are NOT
/// re-exported (same boundary as `boyko_ecs::prelude`); import them from
/// `boyko_macros` directly.
pub mod prelude {
    pub use crate::asset_refs::{AssetRefKind, DeferredFree, FreeEntry, RefDelta, RefcountDeltas};
    pub use crate::bundles::{CameraRig, FlyCameraBundle, SpatialBundle, StaticProp};
    pub use crate::camera::{
        ActiveCamera, Camera, FlyCamera, OrbitCamera, Projection, ViewUniform, Viewport,
        fly_camera_system, orbit_camera_system, resolve_active_camera,
    };
    pub use crate::camera_plugin::CameraPlugin;
    pub use crate::identity::{Name, NameId, intern, resolve};
    pub use crate::plugin::TransformPlugin;
    pub use crate::propagation::{compute_global_transform, propagate_transforms};
    pub use crate::render_caps::{
        MaterialHandle, MaterialRefGen, MeshHandle, MeshRefGen, RenderEnabled, Visibility,
    };
    pub use crate::sets::{CameraSet, FixedSet};
    pub use crate::transform::{GlobalTransform, Transform};
    pub use crate::visibility_sync::visibility_sync;
}
