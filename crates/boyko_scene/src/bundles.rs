//! Object-category bundle presets that live entirely within `boyko_scene`
//! (standard-library Phase S6).
//!
//! Each is a named `#[derive(Bundle)]` over the crate's own S2–S4 component parts
//! ([`Transform`] / [`GlobalTransform`] / [`MeshHandle`] / [`MaterialHandle`] /
//! [`Visibility`] / [`Camera`] / [`Projection`]). They reference NO physics or
//! render-crate types, so the scene crate stays at the bottom of the dependency
//! DAG (physics and render depend on scene — referencing them here would cycle).
//! Domain bundles that need physics or light components live in those crates'
//! own `bundles` modules.
//!
//! # Why named structs (not tuples / generics)
//!
//! The `Bundle` derive rejects generics and tuple/unit shapes (Phase 8.5 scope),
//! because each non-generic impl owns one `static OnceLock<BundleStaticInfo>` —
//! the per-impl cache a warm spawn hits. Every bundle here is a plain named
//! struct; spawning one repeatedly hits that static cache (no per-spawn archetype
//! rebuild).
//!
//! # GPU instance + per-frame enable bit are NOT bundle fields
//!
//! Neither `RenderEnabled` (an `EnableTag` bitset — no `ComponentPool`, so it is
//! not a poolable bundle field) nor the renderer's dense `Gpu3dInstance` column is
//! a bundle field: the renderer mints those (a CPU-only / headless scene must not
//! be forced into a GPU-resident archetype at spawn). Attach them via the render
//! layer after spawning.

use boyko_macros::Bundle;

use crate::camera::{Camera, FlyCamera, Projection};
use crate::render_caps::{MaterialHandle, MeshHandle, Visibility};
use crate::transform::{GlobalTransform, Transform};

/// The minimal spatial preset: a placed, world-tracked, visibility-carrying
/// entity with no geometry of its own (arity 3).
///
/// Use it for a parent/anchor node, a logical pivot, or any entity that needs a
/// pose + a `GlobalTransform` slot + an authoring [`Visibility`] but draws nothing
/// itself.
#[derive(Bundle)]
pub struct SpatialBundle {
    /// Local pose (designer-facing).
    pub transform: Transform,
    /// Cached world pose, filled by `propagate_transforms`.
    pub global: GlobalTransform,
    /// Persisted authoring visibility.
    pub visibility: Visibility,
}

/// A static, drawable prop: a placed mesh + material with authoring visibility
/// (arity 5).
///
/// The canonical "scenery" preset — a non-simulated object the renderer draws.
/// The GPU instance column + the per-frame `RenderEnabled` bit are attached by the
/// render layer, not by this bundle (see the module docs).
#[derive(Bundle)]
pub struct StaticProp {
    /// Local pose (designer-facing).
    pub transform: Transform,
    /// Cached world pose, filled by `propagate_transforms`.
    pub global: GlobalTransform,
    /// Mesh asset handle.
    pub mesh: MeshHandle,
    /// Material asset handle.
    pub material: MaterialHandle,
    /// Persisted authoring visibility.
    pub visibility: Visibility,
}

/// A camera rig: a placed, world-tracked camera with a projection (arity 4).
///
/// [`Camera`] has a `Default`, but [`Projection`] does NOT — construct the
/// `projection` field explicitly (a perspective or orthographic preset).
#[derive(Bundle)]
pub struct CameraRig {
    /// Local pose (designer-facing).
    pub transform: Transform,
    /// Cached world pose, filled by `propagate_transforms`.
    pub global: GlobalTransform,
    /// The view component (order / active / viewport).
    pub camera: Camera,
    /// The projection (perspective or orthographic) — no `Default`, fill it.
    pub projection: Projection,
}

/// An interactive FLY camera rig: a placed, world-tracked camera with a
/// projection and a [`FlyCamera`] controller (arity 5).
///
/// The R6 interactive counterpart of [`CameraRig`]:
/// [`fly_camera_system`](crate::camera::fly_camera_system) drives the
/// `transform` from the per-frame input snapshot. Wire the controller +
/// the OS→ECS input bridge with `boyko_app::FlyCameraPlugin`.
///
/// [`Camera`] has a `Default`, but [`Projection`] does NOT — construct the
/// `projection` field explicitly. Seed the initial view by setting the
/// `fly` field's `yaw` / `pitch` and the `transform`'s `translation` (the eye);
/// `fly_camera_system` overwrites the `rotation` from `yaw` / `pitch` on the
/// first frame.
#[derive(Bundle)]
pub struct FlyCameraBundle {
    /// Local pose (designer-facing); the `translation` is the initial eye.
    pub transform: Transform,
    /// Cached world pose, filled by `propagate_transforms`.
    pub global: GlobalTransform,
    /// The view component (order / active / viewport).
    pub camera: Camera,
    /// The projection (perspective or orthographic) — no `Default`, fill it.
    pub projection: Projection,
    /// The fly controller (yaw / pitch accumulators + speed / sensitivity).
    pub fly: FlyCamera,
}
