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
//! The bridge is: `Visibility::Hidden` ⇒ `disable::<RenderEnabled>()`;
//! `Visible` / `Inherited` ⇒ `enable::<RenderEnabled>()`. The
//! [`visibility_sync`](crate::visibility_sync::visibility_sync) system drives the
//! bit from the byte automatically (a `Changed<Visibility>`-gated, deferred-command
//! bridge), so setting `Visibility::Hidden` alone now hides the row; high-churn
//! show/hide can still ride the manual `enable`/`disable` path directly.
//! `Inherited` is treated as visible at the entity level — true parent-effective
//! `InheritedVisibility` / `ViewVisibility` propagation down the `ChildOf` tree is
//! a separate, larger feature (deferred; see `visibility_sync`'s module docs).
//!
//! # Refcount hook wiring (asset-streaming plan F2 §1)
//!
//! [`MeshHandle`] / [`MaterialHandle`] are the STRONG-ref carriers the
//! asset-streaming refcount lifetime driver counts: attaching one to an
//! entity (`on_insert`) pushes `+1`; detaching it (`on_replace`) pushes `-1`.
//! Both push a [`RefDelta`](crate::asset_refs::RefDelta) into the
//! [`RefcountDeltas`](crate::asset_refs::RefcountDeltas) resource (via
//! `DeferredEcsMaster::resource_mut` — the sanctioned `on_remove`/`on_replace`
//! "decrement a counter" pattern); a `boyko_render` apply system drains the
//! buffer once per frame and folds each delta into the matching `Assets<T>`
//! table (`inc_ref`/`dec_ref`).
//!
//! ## Only `on_insert` + `on_replace` are wired — `on_remove` is NOT
//!
//! The kernel fires `on_replace` for EVERY value-departure event — an
//! in-place overwrite (a bundle-insert replacing an already-present
//! component, SAME archetype, no migration) **and** a genuine removal/
//! despawn — always reading the correct dying/old value (`migrate_entity_insert`
//! / `migrate_entity_remove` both trigger it pre-overwrite/pre-drop).
//! `on_remove` fires ADDITIONALLY, but ONLY for a genuine removal, reading
//! the SAME dying value `on_replace` already saw (confirmed by the kernel's
//! own `remove_fires_replace_then_remove_predrop_reading_dying_value` /
//! despawn tests). So `on_replace` alone already covers BOTH departure paths
//! with exactly one `-1` per departure; wiring an INDEPENDENT `-1` on
//! `on_remove` too would double-decrement every genuine removal whenever the
//! slot's refcount is still `> 1` at that point — the common shared-handle
//! case (many entities referencing one mesh/material slot; despawning ONE
//! of them must drop refcount by exactly 1, not 2). `Assets::dec_ref`'s
//! idempotent-on-`Retiring` guard only absorbs the duplicate when the FIRST
//! decrement already reached zero (the single-reference case) — it does not
//! help here. So this is a deliberate two-hook wiring, not the literal
//! three-hook (`on_insert` `+1`/`on_replace` `-1`/`on_remove` `-1`) shape a
//! first reading of the streaming plan might suggest.
//!
//! ## Known gap: a migrating multi-component bundle `insert` skips `on_replace`
//!
//! The "on_replace fires for EVERY departure" claim above has ONE exception:
//! `migrate_entity_insert` (a bundle `insert` that changes the entity's
//! ARCHETYPE — i.e. the bundle carries at least one component the entity did
//! not already have, alongside a re-supplied `MeshHandle`/`MaterialHandle`)
//! does NOT fire `on_replace` for a retained TABLE component the bundle
//! overlaps (`migration_helpers.rs`'s Step 2/3 fused overlap-write path,
//! ~line 628-670): it `drop_at`s + overwrites the OLD value directly, then
//! fires only `on_add`/`on_insert` for the NEW one in its Phase 2. The
//! SAME-archetype in-place-replace path (`insert_command.rs`, what
//! `MeshHandle`/`MaterialHandle`'s own doc above describes) is unaffected —
//! it always fires `on_replace` pre-overwrite; only the ARCHETYPE-MIGRATING
//! multi-component case has this gap. A `MeshHandle`/`MaterialHandle` rebind
//! that goes through THAT path would leak the old slot's ref (the old
//! decrement never fires).
//!
//! **Contract:** a `MeshHandle`/`MaterialHandle` may be rebound only via a
//! fresh spawn, or a SINGLE-component `insert` that does not change the
//! entity's archetype — never inside a migrating multi-component bundle
//! `insert` that re-supplies the handle alongside a new component. This is
//! NOT exercised anywhere in the engine today (no bundle re-supplies a
//! `MeshHandle`/`MaterialHandle` it does not already carry inside a
//! migrating multi-component insert — grep-confirmed), so it is a
//! documented streaming-era trap, not a live bug. Closing it (routing
//! `migrate_entity_insert`'s overlap path through `trigger_on_replace` too)
//! is kernel-level work out of this crate's scope.

use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_macros::Component;

use crate::asset_refs::{AssetRefKind, RefDelta, RefcountDeltas};
use crate::transform::{GlobalTransform, Transform};

/// A mesh asset handle — an index into the renderer's mesh table.
///
/// `#[repr(transparent)]` so it is byte-identical to its `u32` (a future GPU
/// draw-indirect path can read the column as a raw `u32` array).
///
/// # Required components (S8)
///
/// `#[require(Transform, GlobalTransform)]` enforces the invariant *a renderable
/// can never exist without a pose*: inserting a `MeshHandle` on an entity that
/// lacks a [`Transform`] / [`GlobalTransform`] auto-inserts the missing ones
/// (each via its `Default`), so the renderer's `GlobalTransform`-driven pack
/// always has a pose to read — even without the [`StaticProp`](crate::bundles::StaticProp)
/// bundle. Supplying either component explicitly suppresses its auto-insert (no
/// double-insert). [`MaterialHandle`] / [`Visibility`] ride alongside `MeshHandle`
/// and carry no require of their own.
///
/// # Refcount lifetime hooks (asset-streaming plan F2 §1)
///
/// `on_insert` (`mesh_handle_on_insert`) pushes `+1` for the NEW slot;
/// `on_replace` (`mesh_handle_on_replace`) pushes `-1` for the OLD slot — see
/// the module doc's "Refcount hook wiring" section for why `on_remove` is
/// deliberately NOT also wired.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[require(Transform, GlobalTransform)]
#[component(on_insert = mesh_handle_on_insert, on_replace = mesh_handle_on_replace)]
pub struct MeshHandle(pub u32);

/// A material asset handle — an index into the renderer's material table.
///
/// `#[repr(transparent)]` over `u16`: the pack step widens it into the
/// `Gpu3dInstance::material` lane (low 16 bits).
///
/// # Refcount lifetime hooks (asset-streaming plan F2 §1)
///
/// Mirrors [`MeshHandle`]'s hook wiring — see the module doc's "Refcount hook
/// wiring" section.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[component(on_insert = material_handle_on_insert, on_replace = material_handle_on_replace)]
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

// ---- Refcount lifetime hooks (asset-streaming plan F2 §1) --------------------------
//
// See the module doc's "Refcount hook wiring" section for the on_insert/on_replace
// (not on_remove) design. Declared `unsafe fn` only to match the `HookFn` signature;
// each body calls ONLY the safe `get_component`/`resource_mut` — no `unsafe` block.

/// `MeshHandle::on_insert`: the carrier just attached (fresh add, or the NEW value
/// of an in-place bundle-replace) — pushes `+1` for the NEW slot.
///
/// # Safety
///
/// The caller is always a `trigger_on_insert` dispatch firing synchronously under
/// the outermost apply's `&mut EcsMaster` (the single-threaded apply window, POST
/// the row write — `get_component` reads the NEW value). `resource_mut` returns a
/// `&mut RefcountDeltas` into resource storage, disjoint from every archetype/pool
/// buffer, so this never aliases the apply's component reborrows — the canonical
/// `on_insert`/`on_remove` resource-mutate pattern (mirrors `evict_light` in
/// `boyko_render::light_system`).
unsafe fn mesh_handle_on_insert(mut dm: DeferredEcsMaster<'_>, ctx: HookContext) {
    let Some(&MeshHandle(slot)) = dm.get_component::<MeshHandle>(ctx.entity) else {
        return;
    };
    if let Some(deltas) = dm.resource_mut::<RefcountDeltas>() {
        deltas.push(RefDelta::new(AssetRefKind::Mesh, slot, 1));
    }
}

/// `MeshHandle::on_replace`: the carrier's current value is about to depart (an
/// in-place overwrite OR a genuine removal/despawn) — pushes `-1` for the OLD
/// (still-live, pre-overwrite/pre-drop) slot.
///
/// # Safety
///
/// Same contract as [`mesh_handle_on_insert`], reading the row PRE-overwrite (the
/// kernel's `on_replace` dispatch point) — `get_component` therefore resolves the
/// OLD/dying value, not the incoming one.
unsafe fn mesh_handle_on_replace(mut dm: DeferredEcsMaster<'_>, ctx: HookContext) {
    let Some(&MeshHandle(slot)) = dm.get_component::<MeshHandle>(ctx.entity) else {
        return;
    };
    if let Some(deltas) = dm.resource_mut::<RefcountDeltas>() {
        deltas.push(RefDelta::new(AssetRefKind::Mesh, slot, -1));
    }
}

/// `MaterialHandle::on_insert`: mirrors [`mesh_handle_on_insert`] for the material
/// store.
///
/// # Safety
///
/// Same contract as [`mesh_handle_on_insert`].
unsafe fn material_handle_on_insert(mut dm: DeferredEcsMaster<'_>, ctx: HookContext) {
    let Some(&MaterialHandle(slot)) = dm.get_component::<MaterialHandle>(ctx.entity) else {
        return;
    };
    if let Some(deltas) = dm.resource_mut::<RefcountDeltas>() {
        deltas.push(RefDelta::new(AssetRefKind::Material, u32::from(slot), 1));
    }
}

/// `MaterialHandle::on_replace`: mirrors [`mesh_handle_on_replace`] for the
/// material store.
///
/// # Safety
///
/// Same contract as [`mesh_handle_on_replace`].
unsafe fn material_handle_on_replace(mut dm: DeferredEcsMaster<'_>, ctx: HookContext) {
    let Some(&MaterialHandle(slot)) = dm.get_component::<MaterialHandle>(ctx.entity) else {
        return;
    };
    if let Some(deltas) = dm.resource_mut::<RefcountDeltas>() {
        deltas.push(RefDelta::new(AssetRefKind::Material, u32::from(slot), -1));
    }
}
