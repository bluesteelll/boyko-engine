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

pub mod bundles;
pub mod camera;
pub mod camera_plugin;
pub mod identity;
pub mod plugin;
pub mod propagation;
pub mod render_caps;
pub mod transform;

pub use bundles::{CameraRig, SpatialBundle, StaticProp};
pub use camera::{
    ActiveCamera, Camera, Projection, ViewUniform, Viewport, resolve_active_camera,
};
pub use camera_plugin::CameraPlugin;
pub use identity::{Name, NameId, intern, resolve};
pub use plugin::TransformPlugin;
pub use propagation::{TransformPropagationScratch, compute_global_transform, propagate_transforms};
pub use render_caps::{MaterialHandle, MeshHandle, RenderEnabled, Visibility};
pub use transform::{GlobalTransform, Transform};

/// Common `boyko_scene` imports.
///
/// Re-exports the spatial components, the propagation entry points, and the
/// plugin. The `#[derive(Component)]` / `#[derive(Resource)]` macros are NOT
/// re-exported (same boundary as `boyko_ecs::prelude`); import them from
/// `boyko_macros` directly.
pub mod prelude {
    pub use crate::bundles::{CameraRig, SpatialBundle, StaticProp};
    pub use crate::camera::{
        ActiveCamera, Camera, Projection, ViewUniform, Viewport, resolve_active_camera,
    };
    pub use crate::camera_plugin::CameraPlugin;
    pub use crate::identity::{Name, NameId, intern, resolve};
    pub use crate::plugin::TransformPlugin;
    pub use crate::propagation::{compute_global_transform, propagate_transforms};
    pub use crate::render_caps::{MaterialHandle, MeshHandle, RenderEnabled, Visibility};
    pub use crate::transform::{GlobalTransform, Transform};
}
