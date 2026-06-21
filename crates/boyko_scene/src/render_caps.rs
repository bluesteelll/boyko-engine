//! Render-capability components (standard-library Phase S4).
//!
//! The renderer reads these alongside [`GlobalTransform`](crate::transform::GlobalTransform)
//! to decide WHAT to draw and HOW: a [`MeshHandle`] / [`MaterialHandle`] pair
//! selects the geometry + surface, and [`Visibility`] carries the authoring
//! intent. They are ordinary ECS component columns — no parallel render-data
//! system (Principle 0); the per-frame GPU instance pack reads the
//! `GlobalTransform` + `MaterialHandle` columns and writes the dense
//! `Gpu3dInstance` column in `boyko_render`.
//!
//! # The two visibility surfaces
//!
//! * [`Visibility`] is the **persisted authoring intent** (a per-row byte) — what
//!   the scene/editor sets. It survives serialization and is the source of truth.
//! * [`RenderEnabled`] is the **per-frame draw toggle** — an `EnableTag` bitset
//!   bit (no archetype migration, no per-row bytes). High-churn show/hide
//!   (culling, gameplay flicker) rides this O(1) path:
//!   `EntityCommands::enable::<RenderEnabled>()` / `disable::<RenderEnabled>()`.
//!   The instance-pack query filters on `Enabled<RenderEnabled>`, so a row whose
//!   bit is clear is skipped branch-free at iteration.
//!
//! The recommended bridge is: `Visibility::Hidden` ⇒ `disable::<RenderEnabled>()`;
//! `Visible` / `Inherited` ⇒ `enable::<RenderEnabled>()`. A `visibility_sync`
//! system that drives the bit from the byte is deferred (v1 documents the manual
//! toggle path); `InheritedVisibility` / `ViewVisibility` are not part of v1.

use boyko_macros::Component;

/// A mesh asset handle — an index into the renderer's mesh table.
///
/// `#[repr(transparent)]` so it is byte-identical to its `u32` (a future GPU
/// draw-indirect path can read the column as a raw `u32` array).
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshHandle(pub u32);

/// A material asset handle — an index into the renderer's material table.
///
/// `#[repr(transparent)]` over `u16`: the pack step widens it into the
/// `Gpu3dInstance::material` lane (low 16 bits).
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MaterialHandle(pub u16);

/// User-intent visibility — the persisted authoring state.
///
/// This is the durable per-row byte; high-churn show/hide rides the
/// [`RenderEnabled`] bitset instead (see the module docs). `#[repr(u8)]` pins the
/// discriminants (`Inherited = 0`, `Visible = 1`, `Hidden = 2`) so the byte is
/// stable across serialization.
#[repr(u8)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Visibility {
    /// Inherit the parent's effective visibility (the default; a root with
    /// `Inherited` is treated as visible).
    #[default]
    Inherited,
    /// Always visible regardless of the parent.
    Visible,
    /// Hidden — excluded from the draw. The recommended per-frame path is to
    /// also `disable::<RenderEnabled>()` so the pack step skips the row.
    Hidden,
}

/// The O(1) per-frame render toggle — an `EnableTag` bitset tag.
///
/// A bitset tag has NO `ComponentPool` and is NOT part of any archetype
/// signature: toggling it (`enable` / `disable`) is O(1) with no archetype
/// migration, no structural generation bump, and no per-row bytes. The 3D
/// instance-pack system
/// ([`sync_gpu_3d_instances`](../../boyko_render/gpu3d_system/fn.sync_gpu_3d_instances.html))
/// filters on `Enabled<RenderEnabled>`, so a row with the bit CLEAR is skipped
/// branch-free at iteration — the path by which `Visibility::Hidden` excludes a
/// row from the pack (the brief's "Hidden skipped branch-free via the EnableTag
/// gate").
#[derive(Component, Clone, Copy, Debug)]
#[component(storage = "bitset")]
pub struct RenderEnabled;

// Layout pins (house style): the renderer's GPU pack reads these widths, and a
// silent layout drift (an added field, a wider discriminant) must fail the build
// rather than corrupt the instance buffer.
const _: () = assert!(size_of::<MeshHandle>() == 4 && align_of::<MeshHandle>() == 4);
const _: () = assert!(size_of::<MaterialHandle>() == 2 && align_of::<MaterialHandle>() == 2);
const _: () = assert!(size_of::<Visibility>() == 1 && align_of::<Visibility>() == 1);
