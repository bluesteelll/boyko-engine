//! Render-object bundle presets: the light objects (standard-library Phase S6)
//! and the drawable [`MeshBundle`] (host plan R3).
//!
//! Each pairs `boyko_scene`'s spatial components with this crate's render
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
use boyko_scene::render_caps::{MaterialHandle, MeshHandle, Visibility};
use boyko_scene::{GlobalTransform, Transform};

use crate::instance_model::InstanceModelCol;
use crate::light::{DirectionalLight, PointLight, SpotLight};

/// A DRAWABLE mesh instance (host plan R3): `boyko_scene`'s `StaticProp` spatial
/// preset PLUS this crate's per-entity 48-byte [`InstanceModelCol`] — the exact
/// component set the instanced G-buffer path reads
/// ([`sync_instance_model_cols`](crate::instance_model::sync_instance_model_cols)
/// packs `GlobalTransform` → `InstanceModelCol`;
/// [`gather_mesh_draws`](crate::mesh_draw::gather_mesh_draws) buckets
/// `(MeshHandle, InstanceModelCol)` rows filtered on `Enabled<RenderEnabled>`).
///
/// The per-frame `RenderEnabled` draw BIT is an `EnableTag` (no `ComponentPool`,
/// not a poolable bundle field): it is driven from the [`Visibility`] byte by
/// `boyko_scene`'s `visibility_sync` bridge — a freshly-spawned bundle's
/// `Visibility` counts as `Changed`, so the bit is enabled at the first
/// command-apply window after spawn (the entity draws from the next gather).
#[derive(Bundle)]
pub struct MeshBundle {
    /// Local pose (designer-facing).
    pub transform: Transform,
    /// Cached world pose, filled by `propagate_transforms`.
    pub global: GlobalTransform,
    /// Mesh asset handle (a dense index into the world's `Assets<MeshGpu>` table —
    /// see [`MeshAssetsExt`](crate::MeshAssetsExt)).
    pub mesh: MeshHandle,
    /// Material asset handle (table slot; `0` = the engine default material).
    pub material: MaterialHandle,
    /// Persisted authoring visibility (drives the `RenderEnabled` draw bit).
    pub visibility: Visibility,
    /// The 48-byte per-entity model affine the gbuffer VS reads; kept fresh from
    /// `GlobalTransform` each frame by `sync_instance_model_cols`.
    pub instance: InstanceModelCol,
}

impl MeshBundle {
    /// A drawable mesh at `transform`: default material (slot 0), default
    /// (`Inherited` ⇒ visible) visibility. `global` and `instance` are seeded
    /// from `transform` so the entity is valid BEFORE the first propagation /
    /// pack run (no one-frame garbage pose).
    #[inline]
    pub fn new(mesh: MeshHandle, transform: Transform) -> Self {
        let global = GlobalTransform(transform.to_affine());
        Self {
            transform,
            global,
            mesh,
            material: MaterialHandle(0),
            visibility: Visibility::default(),
            instance: InstanceModelCol::from_global(&global),
        }
    }
}

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
    /// NOTE: `light.direction` is a seed; with a `GlobalTransform` present,
    /// `light_reconcile` overwrites it with the pose's world `-Z`. Aim the spot via
    /// `transform` (`look_at`), not `light.direction`.
    pub light: SpotLight,
}
