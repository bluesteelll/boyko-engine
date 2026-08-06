//! VG R3 piece 2 step P2-1 — the structural OCCLUSION-CULLING capability marker.
//!
//! Principle 0 / capability-is-structural: participation in the HZB occlusion test is the
//! PRESENCE of [`OcclusionCulling`] on an entity, not a field on a config and not a runtime
//! bool. The marker lands ahead of every consumer so that each later step of
//! `docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md` is a small diff against a capability surface that
//! is already reviewed — the [`hzb_config`](crate::hzb_config) P1-1 shape (the knob before the
//! machinery).
//!
//! # Read by nothing ON THE DEVICE, deliberately
//!
//! Piece 2 lands the capability and the visibility-buffer raster split INERT.
//!
//! * **P2-1** minted the marker with zero call sites.
//! * **P2-2** (landed) gave it two non-filtering readers on the HOST — the main gather
//!   ([`gather_mesh_draws`](crate::mesh_draw::gather_mesh_draws), both `cfg` variants) and the
//!   caster gather ([`gather_shadow_casters`](crate::csm_caster::gather_shadow_casters)) — which
//!   scatter [`VB_INST_FLAG_OCCLUSION_CULLING`] into
//!   [`MeshRenderScratch::inst_flags`](crate::mesh_draw::MeshRenderScratch::inst_flags), fold
//!   [`MeshRenderScratch::occlusion_instances()`](crate::mesh_draw::MeshRenderScratch::occlusion_instances),
//!   and pack the word into [`VbInstanceRow::flags`](crate::instance_model::VbInstanceRow::flags).
//!   **Nothing on the device reads that word**: the HLSL mirrors still spell offsets 52..64
//!   `uint3 _pad`, which is layout-identical, and no shader loads it.
//! * **P2-3** adds the frame-level predicate `GBufferScene::path_vb_occlusion_split()`, and
//!   **P2-5** the late raster scope — which draws nothing.
//!
//! Because no entity in the tree carries the marker, every scattered flags word is `0` — bit for
//! bit what the retired `_pad[0]` carried — so the uploaded instance ring is byte-UNCHANGED.
//!
//! # Axis-1 (structural capability), beside — not instead of — Axis-2 (runtime on/off)
//!
//! The engine keeps two independent axes, and this marker is the first one. The caster gather
//! carries both in ONE query line
//! ([`gather_shadow_casters`](crate::csm_caster::gather_shadow_casters)):
//!
//! ```text
//! Query<(&MeshHandle, &InstanceModelCol, Option<&OcclusionCulling>),
//!       (Enabled<RenderEnabled>, With<ShadowCaster>)>
//!        ^^^^ Axis-2: runtime  ^^^^ Axis-1: capability
//! ```
//!
//! (The data tuple's third term is P2-2's own Axis-1 read — non-filtering, so it sits in the
//! DATA position, never in the filter position the two axes above occupy.)
//!
//! Occlusion-culling participation is a property of the object KIND — a skybox, a first-person
//! weapon and a UI proxy never participate — and that is decided at spawn, never toggled per
//! frame. Axis-1 ⇒ TABLE storage, verbatim the
//! [`ShadowCaster`](crate::csm_marker::ShadowCaster) shape.
//!
//! # Why table storage, and why `#[component(storage = "bitset")]` is a BUILD ERROR here
//!
//! `storage = "bitset"` IS this kernel's EnableTag backend, so it cannot express PRESENCE at
//! all: the id is stripped from every archetype signature (`With<T>` then matches zero
//! archetypes), it owns no `ComponentPool` (so `Option<&T>` cannot resolve — only
//! `IsEnabled<T>`), and a row that was never toggled reads `false`, which makes "unmarked" and
//! "marked, then disabled" the same datum. It also suppresses the derive's `Bundle` impl
//! (`crates/boyko_macros/src/component.rs`), so the marker could not be spawned in a bundle or
//! `insert`ed at all, and it declares NO scheduler access, so the gather's read of it would be
//! invisible to conflict detection. The two non-signature backends are pinned out by the
//! const-asserts below rather than merely left un-written.
//!
//! The cost of table storage is real and accepted: a marked SUBSET fragments the mesh archetype
//! in two, shortening the per-archetype runs in the gather. It is bounded at 2× archetypes for
//! the mesh family, the gather is already per-archetype chunked, and a game that marks all its
//! meshes collapses back to one archetype.
//!
//! # `Has<T>` is not a query term in this kernel
//!
//! The non-filtering per-row read is `Option<&OcclusionCulling>`: its `matches_component_set`
//! is unconditionally true and its `aggregate_include` is a no-op, so it never drops and never
//! reorders a row — which is what the lock-step scatter of P2-2 requires. `Enabled<T>` and
//! `With<T>` both FILTER, and a filtered gather would silently renumber the instance ring.
//!
//! # Opt-IN, because the error direction of the alternative deletes geometry
//!
//! Absence of the marker means "never occlusion-culled", i.e. always drawn. An entity type
//! added in two years by an author who never heard of this feature cannot be silently deleted
//! from the frame. The inverse spelling (a `NoOcclusionCulling` opt-out) would make a new
//! component's failure mode INVISIBLE GEOMETRY — the same asymmetry the geometry table already
//! states for unknown bounds ("absence of bounds is not evidence of invisibility"). The price is
//! that a scene must opt in; it is payable per object kind through `#[require(OcclusionCulling)]`,
//! which is reachable precisely because the storage is table storage.
//!
//! # Mark AT SPAWN — an `insert` arms the split one frame late
//!
//! A table-storage insert triggers an archetype migration that is applied at the next command
//! flush, so an entity marked inside a system is invisible to
//! [`gather_mesh_draws`](crate::mesh_draw::gather_mesh_draws) until that flush: the split arms
//! ONE FRAME LATE. That is the safe direction (one frame of extra draws, never missing
//! geometry), but it makes "which frame does a gate read?" ambiguous — so every fixture in this
//! campaign marks in the SAME COMMAND FLUSH as the spawn, never from a later frame.
//!
//! ⚠️ **Not by a tuple spawn.** An earlier draft of this doc wrote
//! `spawn((MeshBundle { .. }, OcclusionCulling))`, and that does not compile: this kernel has **no
//! tuple `Bundle` impl** — `Bundle` is sealed and per-type (`bundle/self_bundle.rs`), and
//! `system/params/commands.rs` records the tuple impl's deletion at Phase 8.5. The route is an
//! `insert(OcclusionCulling)` queued into the same flush as the spawn, which is what
//! `MaterialHandle` already does in every one of these fixtures. One flush applies both before any
//! gather runs, so the one-frame-late hazard — an insert issued in a LATER frame — does not arise.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_macros::Component;

/// The structural occlusion-culling capability: an entity carrying [`OcclusionCulling`] MAY be
/// rejected by the HZB occlusion test (from piece 3 on); an entity WITHOUT it is always drawn.
/// A zero-sized marker (`#[derive(Component)]`, table storage — no `#[component(storage = ...)]`
/// attribute) — its PRESENCE is the whole datum, exactly as
/// [`ShadowCaster`](crate::csm_marker::ShadowCaster) and
/// [`CastsPunctualShadow`](crate::shadow_marker::CastsPunctualShadow) are.
///
/// Opt-IN: the failure mode of a forgotten marker is a wasted draw, never vanished geometry.
/// Apply it in the SAME COMMAND FLUSH as the spawn — an `insert` issued in a LATER frame migrates
/// the archetype at that frame's flush and therefore arms the split one frame late. There is no
/// tuple `Bundle` in this kernel, so "inside the bundle" is not the route; see the module doc.
///
/// The derived `Default` is load-bearing rather than cosmetic: `#[require(OcclusionCulling)]`,
/// the route by which a game marks an object KIND once instead of once per spawn, auto-inserts
/// the required component through its `Default`.
///
/// Read as `Option<&OcclusionCulling>` — non-filtering, so the gather's per-row lock-step with
/// the instance ring is preserved. Read by nothing on the DEVICE (see the module doc).
#[derive(Component, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct OcclusionCulling;

// Layout pin: a zero-sized marker carries no data — its presence is the datum.
const _: () = assert!(size_of::<OcclusionCulling>() == 0);

// The storage decision (plan D1), mechanically enforced instead of merely documented: `bitset`
// and `dense` are the two NON-signature backends — each drops the id from every archetype
// signature, leaves it without a per-archetype `ComponentPool`, and suppresses the derive's
// `Bundle` impl. Adding either attribute is therefore a BUILD ERROR here rather than a silent
// reversal of the capability semantics into enable-bit semantics.
const _: () = assert!(
    !<OcclusionCulling as Component>::STORAGE_IS_BITSET,
    "OcclusionCulling is a structural capability (presence IS the datum), not an EnableTag: a \
     bitset id has no ComponentPool, is stripped from every archetype signature, reads `false` \
     on every never-toggled row, and gets no Bundle impl"
);
const _: () = assert!(
    !<OcclusionCulling as Component>::STORAGE_IS_DENSE,
    "OcclusionCulling must stay signature storage: a dense id is excluded from every archetype \
     signature and gets no Bundle impl, so it cannot be spawned in a bundle nor matched by \
     With<OcclusionCulling>"
);

/// Bit 0 of the per-instance flags word: "this instance's entity carries
/// [`OcclusionCulling`]".
///
/// The lane is a WORD rather than a bool so that piece 3 adds a BIT instead of a column; bits
/// 1..31 are reserved and written zero.
///
/// Written by P2-2 into
/// [`VbInstanceRow::flags`](crate::instance_model::VbInstanceRow::flags) — offset 52, formerly
/// `_pad[0]`, the same 16-byte lane as `mesh_id`, which the batch cull's existing per-candidate
/// instance load already brings into cache, so the flag costs zero extra device fetches. READ
/// by nothing on the device: piece 3 is the first code to load the bit.
pub const VB_INST_FLAG_OCCLUSION_CULLING: u32 = 1 << 0;

// Exactly one bit, and bit 0. P2-2's scatter encodes presence branchlessly as
// `u32::from(present) * VB_INST_FLAG_OCCLUSION_CULLING`, so a multi-bit or shifted constant
// would quietly claim reserved bits that piece 3 hands to its own per-instance datum.
const _: () = assert!(VB_INST_FLAG_OCCLUSION_CULLING.count_ones() == 1);
const _: () = assert!(VB_INST_FLAG_OCCLUSION_CULLING.trailing_zeros() == 0);

#[cfg(test)]
mod tests {
    use super::*;

    use boyko_ecs::ecs::core::bundle::Bundle;
    use boyko_ecs::ecs::core::component::component_registry::{self, StorageKind};
    use boyko_ecs::ecs::core::iters::query::QueryData;

    #[test]
    fn marker_id_registers_as_table_storage() {
        // `component_id()` is what runs the derive's one-shot registration closure, so the
        // classification is only observable after it — minting first is load-bearing, not
        // ceremony.
        let id = OcclusionCulling::component_id();
        let kind = component_registry::storage_kind(id.0);

        // ⚠️ What this CANNOT claim: `Table` is discriminant 0 and the registry's default, so
        // this does not distinguish "classified as Table" from "no classification ran". What it
        // CAN fail on is the one edit that matters — a `#[component(storage = "bitset"/"dense")]`
        // attribute appearing on the marker, which installs a non-Table kind for this id.
        assert_eq!(
            kind,
            StorageKind::Table,
            "the capability must be signature storage; a bitset/dense id has no ComponentPool"
        );
        // Named in the kernel's own vocabulary: signature storage is exactly "the id is part of
        // the archetype signature and owns a per-archetype pool" — the property that makes
        // `With<OcclusionCulling>` match archetypes and `Option<&OcclusionCulling>` resolve.
        assert!(
            component_registry::is_signature_storage(kind),
            "OcclusionCulling must participate in the archetype signature"
        );
    }

    #[test]
    fn marker_is_bundle_spawnable_and_readable_as_optional_data() {
        // Empty-bodied shims: instantiating one proves its bound at COMPILE time and asserts
        // nothing at runtime (the idiom `boyko_ecs`'s own `SystemParam` tests use).
        fn assert_bundle<B: Bundle>() {}
        fn assert_query_data<D: QueryData>() {}

        // The two rows of D1 that the rejected bitset storage would break: a bitset id gets no
        // `Bundle` impl (so the mark-at-spawn fixtures would not compile) and no column (so the
        // non-filtering `Option<&T>` read would not resolve).
        assert_bundle::<OcclusionCulling>();
        assert_query_data::<Option<&'static OcclusionCulling>>();
    }
}
