//! Light-object bundle presets (standard-library Phase S6).
//!
//! Each pairs `boyko_scene`'s spatial components with one of this crate's light
//! components. The layering is sound: `boyko_render` depends on `boyko_scene` (a
//! downward edge), so naming scene types here is cycle-free.
//!
//! # Three concrete bundles, not one generic
//!
//! The `Bundle` derive rejects generics (each non-generic impl owns the per-impl
//! `static OnceLock<BundleStaticInfo>` cache slot a warm spawn hits; a generic
//! `LightObject<L>` would monomorphize one static per `L` and would not compile
//! anyway). So the light objects are three near-identical named structs.
//!
//! The light components have no `Default` — construct the `light` field
//! explicitly. The pose ([`Transform`] / [`GlobalTransform`]) gives the light a
//! place in the hierarchy; a [`DirectionalLight`] / [`SpotLight`] also carries its
//! own direction (the light table reads that), with the pose available for
//! parenting / editor manipulation.

use boyko_macros::Bundle;
use boyko_scene::{GlobalTransform, Transform};

use crate::light::{DirectionalLight, PointLight, SpotLight};

/// A directional-light object: the sun as a placed, world-tracked entity (arity 3).
#[derive(Bundle)]
pub struct DirectionalLightObject {
    /// Local pose (designer-facing).
    pub transform: Transform,
    /// Cached world pose, filled by `propagate_transforms`.
    pub global: GlobalTransform,
    /// The directional light (direction / color / illuminance) — no `Default`.
    pub light: DirectionalLight,
}

/// A point-light object: an omnidirectional source as a placed entity (arity 3).
#[derive(Bundle)]
pub struct PointLightObject {
    /// Local pose (designer-facing).
    pub transform: Transform,
    /// Cached world pose, filled by `propagate_transforms`.
    pub global: GlobalTransform,
    /// The point light (position / color / power / range) — no `Default`.
    pub light: PointLight,
}

/// A spot-light object: a coned source as a placed entity (arity 3).
#[derive(Bundle)]
pub struct SpotLightObject {
    /// Local pose (designer-facing).
    pub transform: Transform,
    /// Cached world pose, filled by `propagate_transforms`.
    pub global: GlobalTransform,
    /// The spot light (position / axis / color / power / range / cone) — no `Default`.
    pub light: SpotLight,
}
