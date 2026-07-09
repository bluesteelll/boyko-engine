//! Generic Relations API — `Relationship` / `RelationshipTarget` (Relations v1).
//!
//! A monomorphized, trait-keyed generalization of the Phase-19 `ChildOf` /
//! `Children` machinery. ANY user struct can declare a one-to-many bidirectional
//! relation maintained by the existing component-hook substrate, with the SAME
//! cascade-soundness discipline and 0%-when-unused gating as the hierarchy.
//!
//! # The trait pair
//!
//! * [`Relationship`] — the writable source-of-truth foreign key on the SOURCE
//!   entity (one [`Entity`]). Inserting it links; overwriting it re-targets;
//!   removing it unlinks. Wires `on_insert` (link) + `on_replace` (unlink).
//! * [`RelationshipTarget`] — the derived reverse index on the TARGET entity (a
//!   [`RelationshipSourceCollection`] of source entities). Never written by user
//!   code; mutated only through the `*_risky` accessors inside the generic
//!   command applies. Wires ONLY `on_replace` (the cascade) — never
//!   `on_add`/`on_insert` (B7: no spurious first-source cascade).
//!
//! The associated types round-trip: `R::Target::Source = R`, so the link/unlink
//! /cascade bodies resolve the partner side without a runtime lookup.
//!
//! # Cascade soundness is STRUCTURAL (W2)
//!
//! A relationship-maintenance hook's only world handle is
//! [`DeferredEcsMaster`](crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster),
//! which exposes NO `&mut`-into-storage method (`get_component` returns `&T`
//! only). The `*_risky` mutators require `&mut Self`, obtainable ONLY inside a
//! [`Command::apply`] under `&mut EcsMaster` — NEVER inside a hook. So every
//! hook can ONLY enqueue into `deferred_hook_queue`, and the `apply_via_raw_twin`
//! disjoint-allocation drain (the BUG-P19-TB-1 fix) stays sound for ANY relation
//! — the same structural reason it is sound for `ChildOf`. This is a property of
//! the available API, not a rule a programmer must remember.
//!
//! # v1 scope
//!
//! v1 ships ONLY the `Vec<Entity>` one-to-many collection with
//! [`RETAIN_EMPTY`](RelationshipTarget::RETAIN_EMPTY) `= true` MANDATORY. The
//! `RETAIN_EMPTY = false` (remove-on-empty) branch and the 1:1 `Entity`
//! collection + eviction path are RESERVED for v1.1 (W1/O3): both are new
//! re-entrant edges that would double v1's Miri-TB audit surface.
//!
//! [`Entity`]: crate::ecs::core::entity::entity::Entity
//! [`Command::apply`]: crate::ecs::core::commands::command::Command::apply

use std::cell::Cell;

use crate::ecs::core::bundle::bundle::Bundle;
use crate::ecs::core::clone::map::EntityCloneMap;
use crate::ecs::core::commands::command::Command;
use crate::ecs::core::commands::migration_helpers::{merged_archetype_id, migrate_entity_insert};
use crate::ecs::core::commands::remove_command::RemoveCommand;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::hooks::HookContext;
use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;

pub mod collection;
pub mod edge_observers;
pub mod generic_hooks;

// ===========================================================================
// Clone-time link suppression (BUG-EDGE-CLONE-1) — relation-agnostic
// ===========================================================================

thread_local! {
    /// When set, [`relationship_on_insert`](generic_hooks::relationship_on_insert)
    /// does NOT enqueue its reverse-index [`LinkCommand`] — the verbatim-copied
    /// foreign key of a freshly-materialized clone must NOT link into its
    /// (un-remapped, still-original) target. The deep-clone relink pass
    /// ([`relationship_clone_relink`]) is the SOLE linker, establishing exactly one
    /// link toward the REMAPPED clone target after the FK has been remapped.
    ///
    /// Without this, the clone's `on_insert` (fired by `materialize_clone`) would
    /// enqueue a `LinkCommand` toward the ORIGINAL target, leaking the clone into
    /// the SOURCE subtree's reverse collection (`original_parent.LikedBy == [1, 3]`
    /// instead of `[1]`). The reactive self-ref / dangling-target guards in
    /// `relationship_on_insert` still run — only the (always-stale during clone)
    /// link enqueue is suppressed.
    ///
    /// Thread-local for the same reason as `CASCADE_SUPPRESS`
    /// (`hierarchy/commands.rs`): every hook fires on the single-threaded apply
    /// window, and a per-thread cell cannot be frozen by any `&mut EcsMaster`
    /// reborrow (the F2 invariant).
    static RELATIONSHIP_LINK_SUPPRESS: Cell<bool> = const { Cell::new(false) };
}

/// Reads the clone-time link-suppress flag for the current thread. Read by the
/// generic link hook
/// ([`relationship_on_insert`](generic_hooks::relationship_on_insert)).
#[inline]
pub(crate) fn relationship_link_suppressed() -> bool {
    RELATIONSHIP_LINK_SUPPRESS.with(|s| s.get())
}

/// RAII guard that suppresses clone-time reverse-index linking for its scope and
/// restores the previous value on every exit path (`Ok` / panic). Mirrors
/// [`CascadeSuppressGuard`](crate::ecs::core::hierarchy::commands) — it touches
/// only the thread-local, never any field of `EcsMaster`, so a bracketed
/// `&mut self` body cannot freeze it.
///
/// Held by the deep-clone walk across each node's `materialize_clone` (whose
/// `on_insert` fire would otherwise enqueue a stale `LinkCommand` toward the
/// original target); dropped BEFORE the remap + relink pass, so
/// [`relationship_clone_relink`] establishes the one correct link unimpeded.
pub(crate) struct LinkSuppressGuard {
    /// Restores the previous flag value on drop (supports nesting; in practice the
    /// depth is one — a deep clone is not re-entrant under itself).
    prev: bool,
}

impl LinkSuppressGuard {
    /// Enters a clone-time link-suppressed scope.
    #[inline]
    pub(crate) fn enter() -> Self {
        let prev = RELATIONSHIP_LINK_SUPPRESS.with(|s| {
            let p = s.get();
            s.set(true);
            p
        });
        Self { prev }
    }
}

impl Drop for LinkSuppressGuard {
    #[inline]
    fn drop(&mut self) {
        RELATIONSHIP_LINK_SUPPRESS.with(|s| s.set(self.prev));
    }
}

// ===========================================================================
// Clone-time eviction suppression (Relations v1.1) — relation-agnostic
// ===========================================================================

thread_local! {
    /// When set, the 1:1 eviction branch in [`LinkCommand::apply`] does NOT steal a
    /// real external incumbent (C3): a freshly-materialized clone whose verbatim FK
    /// still points at an external 1:1 target must NOT evict that target's existing
    /// source. Under suppression the clone DETACHES instead — its own (dangling) FK
    /// is dropped via a deferred [`RemoveCommand`], and the external incumbent +
    /// reverse slot are left byte-for-byte untouched.
    ///
    /// Mirrors [`RELATIONSHIP_LINK_SUPPRESS`]: a thread-local because every hook /
    /// command applies on the single-threaded apply window and a per-thread cell
    /// cannot be frozen by any `&mut EcsMaster` reborrow (the F2 invariant).
    static RELATIONSHIP_EVICTION_SUPPRESS: Cell<bool> = const { Cell::new(false) };
}

/// Reads the clone-time eviction-suppress flag for the current thread. Read by the
/// 1:1 eviction branch in [`LinkCommand::apply`].
#[inline]
pub(crate) fn relationship_eviction_suppressed() -> bool {
    RELATIONSHIP_EVICTION_SUPPRESS.with(|s| s.get())
}

/// RAII guard that suppresses clone-time 1:1 eviction for its scope and restores
/// the previous value on every exit path (`Ok` / panic). Mirrors
/// [`LinkSuppressGuard`] — it touches only the thread-local, never any field of
/// `EcsMaster`, so a bracketed `&mut self` body cannot freeze it.
///
/// Held by the deep-clone / prefab relink pass: during that pass a cloned source's
/// FK toward an EXTERNAL 1:1 target must not evict the external incumbent (the
/// clone is unrelated); under the guard the clone detaches instead.
pub(crate) struct EvictionSuppressGuard {
    /// Restores the previous flag value on drop (supports nesting; in practice the
    /// depth is one — a deep clone is not re-entrant under itself).
    prev: bool,
}

impl EvictionSuppressGuard {
    /// Enters a clone-time eviction-suppressed scope.
    #[inline]
    pub(crate) fn enter() -> Self {
        let prev = RELATIONSHIP_EVICTION_SUPPRESS.with(|s| {
            let p = s.get();
            s.set(true);
            p
        });
        Self { prev }
    }
}

impl Drop for EvictionSuppressGuard {
    #[inline]
    fn drop(&mut self) {
        RELATIONSHIP_EVICTION_SUPPRESS.with(|s| s.set(self.prev));
    }
}

pub use collection::{Exclusive, RelationshipSourceCollection};
pub use edge_observers::{OnLink, OnUnlink};

/// The source-of-truth side of a relation: a foreign key on the source entity
/// pointing at one target (Relations v1, Decision 1).
///
/// Implemented (by `#[derive(Relationship)]` for user types, by hand for the
/// in-crate `ChildOf` — the dev-dep cycle precludes the derive) on the component
/// the user writes. The bidirectional sync is done entirely by the generic
/// component-hook bodies keyed on this trait
/// ([`generic_hooks`]) — no bespoke per-relation code.
///
/// # Safety (the cascade-soundness contract, W2)
///
/// The `on_insert` / `on_replace` hook bodies MUST only ENQUEUE deferred
/// commands — they must never mutate storage inline. This holds STRUCTURALLY for
/// the generic bodies (a hook's [`DeferredEcsMaster`] has no `&mut`-into-storage
/// path), so it is automatic for any conforming implementation; the note records
/// the invariant the disjoint-allocation drain depends on.
pub trait Relationship: Component + Sized {
    /// The reverse-index component on the target entity. The round-trip
    /// `Self::Target::Source = Self` is enforced by the bound on
    /// [`RelationshipTarget`].
    type Target: RelationshipTarget<Source = Self>;

    /// Reads the target [`Entity`] out of `self`. Monomorphizes to a single
    /// field load.
    fn target(&self) -> Entity;

    /// Constructs `Self` from a target [`Entity`] (other fields via `Default`).
    /// Used by the relate ergonomics.
    fn from_target(target: Entity) -> Self;

    /// `true` permits a self-referential relation (`R(self)`); `false` (the
    /// default) makes the generic `on_insert` reactively remove a self-link. The
    /// `#[relationship(allow_self_referential)]` flag flips it; `ChildOf` keeps
    /// the default `false`.
    const ALLOW_SELF_REFERENTIAL: bool = false;

    /// `true` iff the relation graph is structurally acyclic (a forest/DAG of
    /// single-target edges, e.g. `ChildOf`). When `true`, the transitive
    /// traversal iterators ([`AncestorsIter`](crate::ecs::core::iters::query::relation::AncestorsIter)
    /// / [`DescendantsIter`](crate::ecs::core::iters::query::relation::DescendantsIter))
    /// const-fold away the per-walk revisit-guard `VisitedSet` — the depth cap
    /// (`MAX_PROPAGATION_DEPTH`) alone bounds the walk. When `false` (the
    /// default, the conservative choice for an arbitrary user relation), the
    /// traversal allocates a `#[cold]` function-local visited set to terminate
    /// on cycles. Shared with the relation-aware observer-broadcast phase.
    const ACYCLIC: bool = false;

    /// Generic LINK hook (`on_insert`): pushes `source` into the target's
    /// collection. Wired into `hooks.on_insert` by the derive / hand-mirror; not
    /// overridable (the relation owns the slot).
    ///
    /// # Safety
    ///
    /// Same `HookFn` contract as every lifecycle hook — invoked only inside the
    /// single-threaded apply window with a view that withholds every structural +
    /// `&mut`-into-storage method.
    unsafe fn on_insert(view: DeferredEcsMaster<'_>, ctx: HookContext) {
        // SAFETY: forwards the `HookFn` contract to the generic body verbatim.
        unsafe { generic_hooks::relationship_on_insert::<Self>(view, ctx) }
    }

    /// Generic UNLINK hook (`on_replace`): removes `source` from the OLD target's
    /// collection. Wired into `hooks.on_replace` by the derive / hand-mirror.
    ///
    /// # Safety
    ///
    /// See [`Self::on_insert`].
    unsafe fn on_replace(view: DeferredEcsMaster<'_>, ctx: HookContext) {
        // SAFETY: forwards the `HookFn` contract to the generic body verbatim.
        unsafe { generic_hooks::relationship_on_replace::<Self>(view, ctx) }
    }
}

/// The derived reverse-index side of a relation (Relations v1, Decision 1).
///
/// User code must NEVER write it directly; the `*_risky` methods are the only
/// mutators and exist solely for the generic command applies / ergonomics. The
/// round-trip `Self::Source::Target = Self` is enforced by the bound on
/// [`Relationship`].
///
/// # Safety (the `*_risky` privacy fence, W2)
///
/// [`collection_mut_risky`](Self::collection_mut_risky) /
/// [`from_collection_risky`](Self::from_collection_risky) bypass the
/// source-of-truth invariant. They require `&mut Self`, obtainable ONLY inside a
/// [`Command::apply`] under `&mut EcsMaster` — never from a hook's
/// [`DeferredEcsMaster`] (which has no `&mut`-into-storage path). A
/// `RelationshipTarget` collection is therefore unreachable for mutation from any
/// hook body; the cascade-soundness argument rests on this missing capability.
///
/// [`Command::apply`]: crate::ecs::core::commands::command::Command::apply
pub trait RelationshipTarget: Component + Default + Bundle + Sized {
    /// The source-of-truth component. The round-trip `Self::Source::Target =
    /// Self` ties the pair.
    type Source: Relationship<Target = Self>;

    /// The container of source entities (v1: `Vec<Entity>`).
    type Collection: RelationshipSourceCollection;

    /// `true` → despawning the target recursively despawns all sources (Bevy's
    /// `linked_spawn`). `Children` sets this `true`. The generic cascade hook
    /// const-folds to the non-cascading (unlink-only) body when `false`.
    const LINKED_DESPAWN: bool;

    /// `true` → keep an emptied collection (do NOT remove the target component)
    /// to dodge `0↔1↔0` archetype-migration thrash — the Phase-19 perf rule.
    ///
    /// **v1: MUST be `true`.** `RETAIN_EMPTY = false` (remove-on-empty, the Bevy
    /// default) is a NEW re-entrant edge (it fires the target's own `on_replace`
    /// on emptying) DEFERRED to v1.1 (W1/O3). The const stays in the trait so
    /// v1.1 lifts the restriction without an API break.
    const RETAIN_EMPTY: bool;

    /// Reads the collection.
    fn collection(&self) -> &Self::Collection;

    /// `_risky`: bypasses the source-of-truth invariant — command applies /
    /// ergonomics ONLY (see the trait-level safety note).
    fn collection_mut_risky(&mut self) -> &mut Self::Collection;

    /// `_risky`: constructs the target from a collection — command applies ONLY.
    fn from_collection_risky(collection: Self::Collection) -> Self;

    /// Generic CASCADE hook (`on_replace`): drives source cleanup / recursive
    /// despawn. Wired into `hooks.on_replace` by the derive / hand-mirror,
    /// matching boyko's pre-remove despawn site (NOT Bevy's `on_despawn`).
    ///
    /// # Safety
    ///
    /// Same `HookFn` contract as [`Relationship::on_insert`].
    unsafe fn on_replace(view: DeferredEcsMaster<'_>, ctx: HookContext) {
        // SAFETY: forwards the `HookFn` contract to the generic body verbatim.
        unsafe { generic_hooks::relationship_target_on_replace::<Self>(view, ctx) }
    }

    /// Constructs a target whose collection has room for `cap` sources.
    #[inline]
    fn with_capacity(cap: usize) -> Self {
        Self::from_collection_risky(<Self::Collection>::with_capacity(cap))
    }

    /// Number of sources currently pointing at this target.
    #[inline]
    fn len(&self) -> usize {
        self.collection().len()
    }

    /// `true` when no source points at this target. An emptied collection is
    /// retained, not removed (v1: `RETAIN_EMPTY = true`).
    #[inline]
    fn is_empty(&self) -> bool {
        self.collection().is_empty()
    }
}

/// Generic clone-direction foreign-key remap for a relationship source `R`
/// (BUG-RELATIONS-CLONE-1) — the monomorphized generalization of the hand-mirrored
/// `child_of_map_entities`. Reads `R`'s target via [`Relationship::target`], and if
/// that target was part of the cloned subtree, rewrites the FK to its clone via
/// [`Relationship::from_target`]; a target OUTSIDE the subtree (e.g. the root's
/// external parent) is kept verbatim — exactly the `ChildOf` semantics.
///
/// Installed for every `#[derive(Relationship)]` source (and the hand-mirrored
/// `ChildOf`) through
/// [`install_relationship_clone_remap`](crate::ecs::core::component::component_registry::install_relationship_clone_remap),
/// and read by the deep-clone remap pass via
/// [`get_map_entities_fn`](crate::ecs::core::component::component_registry::get_map_entities_fn).
/// Monomorphizes to one bare [`MapEntitiesFn`](crate::ecs::core::component::component_registry::MapEntitiesFn)
/// per relation type — no `dyn`.
///
/// For a single-FK source (the v1 shape) this rewrites the foreign key in place. For a
/// multi-field source it RECONSTRUCTS `R` via `from_target` — per the trait contract
/// (`from_target` builds `R` from the target alone), any non-FK fields are reset to
/// their `Default`, NOT preserved from the cloned row. v1 sources are single-FK, so
/// this is an exact FK rewrite; a future multi-field relation author must account for
/// the default-reset.
///
/// # Safety
///
/// The [`MapEntitiesFn`](crate::ecs::core::component::component_registry::MapEntitiesFn)
/// contract: `dst` points at a live, aligned, initialized `R` (the deep-clone remap
/// pass resolves it through the fast store for an archetype that hosts `R`); `map` is a
/// shared, non-aliased reference for the call's duration.
pub unsafe fn relationship_clone_map_entities<R: Relationship>(
    dst: *mut u8,
    map: &EntityCloneMap,
) {
    // SAFETY: `dst` is a live, aligned, initialized `R` row (the deep-clone remap
    //   pass resolves it through the fast store for an archetype that hosts `R`).
    //   We form `&mut R` to rewrite its foreign key in place via the trait
    //   round-trip; no other reference aliases it (single-threaded `&mut EcsMaster`
    //   drives the deep clone). Provenance + initialization are the caller's
    //   `MapEntitiesFn` contract, identical to the `child_of_map_entities` invariant.
    let source: &mut R = unsafe { &mut *dst.cast::<R>() };
    if let Some(mapped) = map.get(source.target()) {
        // The target was cloned — point the clone's FK at the cloned target.
        // `from_target` rebuilds `R` (non-FK fields via `Default`, matching the
        // derive's `from_target` codegen); the surrounding clone already wrote the
        // full row, so this is a FK-only rewrite for a single-field source.
        *source = R::from_target(mapped);
    }
    // else: the target is outside the cloned subtree → keep the FK verbatim (a
    // shared external reference, e.g. the root's parent), matching `ChildOf`.
}

/// Type-erased "relink a cloned relationship source into its target's reverse
/// index" fn (BUG-RELATIONS-CLONE-1). The deep-clone remap pass — which works in
/// `ComponentId` space, not types — calls this AFTER
/// [`relationship_clone_map_entities`] has remapped a cloned source's foreign key, to
/// rebuild the (cloner-denied) reverse index on the clone side. The generalization of
/// the hierarchy-specific `link_child`.
///
/// `source` is the CLONED source entity (its FK already remapped). `map` is the
/// deep-clone source→clone map. The clone-time `on_insert` link is unconditionally
/// suppressed by the [`LinkSuppressGuard`] (BUG-EDGE-CLONE-1), so this relink is the
/// SOLE linker for every cloned source — it links toward whatever the FK CURRENTLY
/// points at: the remapped clone target for an in-subtree FK, or the verbatim external
/// target for an external FK (BUG-EDGE-CLONE-2 — restoring the kept FK's reverse entry
/// so FK↔reverse consistency holds).
pub(crate) type RelationshipRelinkFn =
    fn(world: &mut EcsMaster, source: Entity, map: &EntityCloneMap);

/// Generic relink for a relationship source `R` (BUG-RELATIONS-CLONE-1 /
/// BUG-EDGE-CLONE-2): reads the cloned source's now-remapped `R` foreign key and links
/// the source into THAT target's [`RelationshipTarget`] collection, reusing
/// [`LinkCommand`]'s apply logic verbatim (the audited migrate-or-push path — first
/// source migrate-inserts the reverse index, subsequent sources push in place).
/// Monomorphizes to one bare [`RelationshipRelinkFn`] per relation type — no `dyn`.
///
/// Because the clone-time `on_insert` link is unconditionally suppressed by the
/// [`LinkSuppressGuard`] (BUG-EDGE-CLONE-1), this relink is the SOLE establisher of the
/// clone's reverse-index membership and therefore must run for EVERY cloned FK,
/// regardless of whether the target was part of the cloned subtree:
/// - IN-SUBTREE FK (target was cloned): the FK was remapped to the cloned target; the
///   relink links the clone into the CLONE target's reverse index.
/// - EXTERNAL FK (target kept verbatim, `map.is_clone == false`): the FK still points
///   at the external entity `E`; the relink links the clone into `E`'s reverse index,
///   restoring `clone ∈ E.Reverse` for the kept `clone.fk == R(E)` (BUG-EDGE-CLONE-2).
///   This restores internal FK↔reverse CONSISTENCY for the kept external FK; it is NOT
///   the Bevy-parity "detach the clone-root's external subtree FK" semantic choice
///   (that would change `relationship_clone_map_entities` to drop the FK, which v1 does
///   not do).
///
/// Links ONLY into the TARGET's reverse collection (`LinkCommand::apply` mutates
/// `R::Target` on `target`, never on `source`), so it can never re-introduce the
/// BUG-EDGE-CLONE-1 source-side leak — a clone is never its own FK target.
///
/// A no-op when the source carries no `R`, or when the target was despawned
/// (`LinkCommand::apply`'s dangling-target guard).
pub(crate) fn relationship_clone_relink<R: Relationship>(
    world: &mut EcsMaster,
    source: Entity,
    map: &EntityCloneMap,
) {
    let _ = map; // `map` is the relink-pass signature; the FK was already remapped.
    let Some(target) = world.get_component::<R>(source).map(|r| r.target()) else {
        return;
    };
    // Reuse the audited link path verbatim — the same machinery `LinkCommand::apply`
    // and the hierarchy `link_child` route through (migrate-insert the reverse index
    // for the first source, in-place push thereafter). The link goes into `target`'s
    // reverse collection only (never `source`'s), so an external `target` gets its
    // kept FK's reverse entry restored without ever touching the source subtree's
    // reverse index. A dangling target is a no-op.
    LinkCommand::<R> {
        target,
        source,
        _marker: core::marker::PhantomData,
    }
    .apply(world);
}

// ===========================================================================
// Generic deferred commands (the generalized Phase-19 Link/Unlink commands)
// ===========================================================================

/// Deferred "link `source` into `target`'s [`RelationshipTarget`]" command —
/// the generic form of the Phase-19 `LinkChildCommand` (Relations v1).
///
/// Enqueued by [`relationship_on_insert`](generic_hooks::relationship_on_insert)
/// after the new `R` is written; applied under `&mut EcsMaster` at the apply
/// window. Monomorphized per relation type `R`.
#[repr(C)]
pub(crate) struct LinkCommand<R: Relationship> {
    pub(crate) target: Entity,
    pub(crate) source: Entity,
    pub(crate) _marker: core::marker::PhantomData<R>,
}

/// Deferred "unlink `source` from `target`'s [`RelationshipTarget`]" command —
/// the generic form of the Phase-19 `UnlinkChildCommand` (Relations v1).
///
/// Enqueued by [`relationship_on_replace`](generic_hooks::relationship_on_replace)
/// reading the OLD target; a no-op if the link is not present. Monomorphized per
/// relation type `R`.
#[repr(C)]
pub(crate) struct UnlinkCommand<R: Relationship> {
    pub(crate) target: Entity,
    pub(crate) source: Entity,
    pub(crate) _marker: core::marker::PhantomData<R>,
}

// SAFETY (mirrors the Phase-19 `LinkChildCommand` / B3): the carried payloads
//   are plain `Entity` PODs plus a ZST `PhantomData<R>` (`R: 'static`), so moving
//   them across threads is sound. The explicit impls document the intent for the
//   `Command: Send + 'static` queue bound (the `R: Relationship` bound implies
//   `R: 'static`).
unsafe impl<R: Relationship> Send for LinkCommand<R> {}
unsafe impl<R: Relationship> Sync for LinkCommand<R> {}
unsafe impl<R: Relationship> Send for UnlinkCommand<R> {}
unsafe impl<R: Relationship> Sync for UnlinkCommand<R> {}

/// The decision the in-place arm of [`LinkCommand::apply`] reaches WHILE the
/// `Mut<R::Target>` borrow is live, so the world-touching follow-up (`fire_*` /
/// deferred push) runs only AFTER that borrow ends (F2). Carries `Copy` scalars
/// only — no borrow escapes the inner block.
enum InPlaceOutcome {
    /// Clone-relink detach (C3): skip the add + the `OnLink` fire; defer a remove of
    /// the clone's own dangling FK.
    Detach,
    /// 1:1 production eviction: the slot was overwritten to the new source; fire
    /// `OnUnlink{incumbent}` and defer a remove of the incumbent's dangling FK.
    Evicted(Entity),
    /// Plain add (empty slot, `Vec` push, or identical 1:1 re-link). `changed` gates
    /// the `OnLink` fire (always `true` for `Vec` → byte-identical).
    Added { changed: bool },
}

impl<R: Relationship> Command for LinkCommand<R> {
    fn apply(self, world: &mut EcsMaster) {
        let target = self.target;
        let source = self.source;

        // Dangling-target guard: the target may have been despawned between the
        // hook firing and this apply. A no-op keeps the invariant rather than
        // resurrecting a dead collection (Phase-19 `LinkChildCommand` verbatim).
        // Critic W2/(iv): an edge that never establishes (dead target) does NOT
        // fire `OnLink` — the fire below is reached only past this guard.
        if !world.has_entity(target) {
            return;
        }

        // Threaded out of the in-place arm: the migrate / first-source arm and the
        // eviction arm always commit a genuine new edge → fire `OnLink`
        // unconditionally; only an identical 1:1 re-link in-place clears this so the
        // `OnLink` fire below is suppressed (W2/Q3). For `Vec`, `add()` is always
        // `true`, so this stays `true` → byte-identical to the v1 unconditional fire.
        let mut should_fire_link = true;

        match world.get_component_mut::<R::Target>(target) {
            // Target already hosts its `RelationshipTarget` — pure in-place push
            // (no archetype change). `DerefMut` stamps the changed tick; harmless
            // for a structural relationship op.
            Some(mut reverse) => {
                // The in-place arm computes its outcome WHILE the `reverse` borrow is
                // live (the inner block), yields the decision, and only THEN touches
                // `world` again — so no `world`-derived `&mut` (the `Mut<R::Target>`,
                // which is a non-`Drop` lifetime holder) spans the post-block
                // `fire_*` / `deferred_hook_queue.push` (F2; clippy `drop_non_drop`).
                let outcome = {
                    // 1:1 eviction probe. `Vec` returns the trait default `None`, so
                    // the whole eviction/detach block const-folds away (byte-identical
                    // v1 path); only `Exclusive` can yield `Some(incumbent)`.
                    let evict = reverse.collection().source_to_evict_before_add();
                    match evict {
                        Some(incumbent) if incumbent != source => {
                            if relationship_eviction_suppressed() {
                                // CLONE-RELINK DETACH (C3): do NOT steal a real
                                // external incumbent during a clone. Do NOT add; the
                                // clone is unrelated to this external 1:1 target, which
                                // is left byte-for-byte untouched (incumbent + slot).
                                InPlaceOutcome::Detach
                            } else {
                                // PRODUCTION EVICTION: overwrite the slot to the new
                                // source `B` now (under the live borrow); fire
                                // `OnUnlink` + defer the incumbent's FK clear AFTER the
                                // borrow ends.
                                let changed = reverse.collection_mut_risky().add(source);
                                debug_assert!(
                                    changed,
                                    "eviction add of a distinct source must change the slot"
                                );
                                debug_assert_eq!(
                                    reverse.collection().get(0),
                                    Some(source),
                                    "post-eviction slot must hold the new source"
                                );
                                InPlaceOutcome::Evicted(incumbent)
                            }
                        }
                        _ => {
                            // Empty slot, identical 1:1 re-link (incumbent == source),
                            // or the `Vec` const-`None` path: plain add. For `Vec`,
                            // `add()` is always `true` (byte-identical); an `Exclusive`
                            // identical re-link returns `false`.
                            let changed = reverse.collection_mut_risky().add(source);
                            InPlaceOutcome::Added { changed }
                        }
                    }
                    // <-- `reverse` (the `Mut<R::Target>` borrow) drops here, BEFORE
                    //     any `world.fire_*` / `world.deferred_hook_queue.push` (F2).
                };

                match outcome {
                    InPlaceOutcome::Detach => {
                        // Drop the clone's own dangling FK next drain turn; skip the
                        // add (already skipped) AND the `OnLink` fire.
                        debug_assert!(relationship_eviction_suppressed());
                        world
                            .deferred_hook_queue
                            .push(RemoveCommand::<R>::new(source));
                        return;
                    }
                    InPlaceOutcome::Evicted(incumbent) => {
                        // Q2 order: `OnUnlink` BEFORE `OnLink`; `OnUnlink` BEFORE the
                        // `RemoveCommand` push. Fires `OnUnlink{incumbent, target}`
                        // exactly once.
                        world.fire_on_unlink::<R>(incumbent, target);
                        // Clears the evicted source's now-dangling FK next drain turn
                        // (the C2 deferred-remove pattern). W3 KEYSTONE: this remove
                        // fires `incumbent`'s `on_replace` → an `UnlinkCommand`, whose
                        // `Exclusive::remove(incumbent)` returns `false` (the slot now
                        // holds `B != incumbent`), so the `if removed` gate suppresses
                        // a second `OnUnlink`. Exactly one `OnUnlink`, one `OnLink`.
                        world
                            .deferred_hook_queue
                            .push(RemoveCommand::<R>::new(incumbent));
                        // Falls through to the unconditional `fire_on_link` below
                        // (genuine new edge → `OnLink{B, target}` after `OnUnlink`).
                    }
                    InPlaceOutcome::Added { changed } => {
                        // Gate the `OnLink` fire on `changed` so an identical 1:1
                        // re-link does NOT spuriously re-fire `OnLink`. For `Vec`,
                        // `changed` is always `true` → byte-identical.
                        should_fire_link = changed;
                    }
                }
            }
            None => {
                // First source: route the insert through the audited migration
                // machinery — `R::Target` is a `Bundle` itself (`impl_self_bundle!`
                // / `#[derive(Component)]` Bundle emission). This fires `on_add` +
                // `on_insert` only — `RelationshipTarget` registers neither, so no
                // spurious cascade (B7).
                //
                // `has_entity(target)` above proved the slot is non-null and
                // generation-matched; the sequential exclusive borrows hold
                // nothing live across the migrate.
                let inland = world.entity_master.entities_inland[target.id().0];
                // SAFETY (verbatim copy of the audited Phase-19 / `insert_command.rs`
                //   F1 pattern, U2): `archetype_ptr` is write-capable, stable,
                //   interior-mutable (`SharedReadWrite`, F4-rooted) slab
                //   provenance — it survives sibling structural writes under
                //   TB/SB (the whole slab element is `UnsafeCell`-wrapped).
                //   Non-null + generation-matched by the preceding `has_entity`,
                //   so the slot is live.
                // BUG-MIGRATE-TB-1: raw projection of `id` — a `.id()` method call
                // auto-refs `&Archetype` (a foreign read that freezes a sibling
                // structural write to `current_index`/`entity_ids`).
                let src = unsafe { core::ptr::addr_of!((*inland.archetype_ptr()).id).read() };
                let tgt = merged_archetype_id::<R::Target>(world, src);
                migrate_entity_insert::<R::Target>(
                    world,
                    target,
                    src,
                    tgt,
                    R::Target::with_capacity(1).seed_with(source),
                );
            }
        }

        // Critic W2 (i)/(v): the `R` edge `source -> target` is now COMMITTED in
        // every reachable branch (in-place push, first-source migrate, eviction
        // overwrite). Fire `OnLink<R>` on the source, gated behind the cold 0%-probe
        // so a world with no edge observers pays ~nothing. Firing here (after the
        // guard, inside the committed-edge apply) covers every committed edge exactly
        // once and excludes the never-established (dead-target) edge for free —
        // including a deep-clone subtree relink, which routes through this same
        // `apply` and INTENTIONALLY fires `OnLink` per re-established edge.
        //
        // `should_fire_link` is `true` in every arm EXCEPT an identical 1:1 re-link
        // (W2/Q3); for `Vec` and the migrate / eviction arms it is always `true`, so
        // the v1 `Vec` path stays byte-identical to the unconditional fire.
        if should_fire_link {
            world.fire_on_link::<R>(source, target);
        }
    }
}

impl<R: Relationship> Command for UnlinkCommand<R> {
    fn apply(self, world: &mut EcsMaster) {
        let target = self.target;
        let source = self.source;
        // No remove-on-empty (v1, W1): an emptied collection is retained to avoid
        // archetype thrash on `0↔1↔0` oscillation. A missing target or an absent
        // source are both harmless no-ops (the spurious-unlink path from the
        // self-ref / dangling guards lands here). Phase-19 `UnlinkChildCommand`
        // verbatim, generalized over `R::Target`.
        let removed = {
            let Some(mut reverse) = world.get_component_mut::<R::Target>(target) else {
                return;
            };
            // `remove` returns `true` iff the source was actually present — the
            // COMMITTED-edge test for `OnUnlink` (critic W2 (ii)/(iii)/(iv)). A
            // spurious unlink (missing target / absent source — the self-ref /
            // dangling / double-unlink paths) returns `false` and fires nothing.
            reverse.collection_mut_risky().remove(source)
            // <-- the `reverse` borrow ends here, BEFORE `fire_on_unlink` re-mints
            //     a `world` borrow (no `world`-derived `&mut` spans the fire).
        };
        if removed {
            world.fire_on_unlink::<R>(source, target);
        }
    }
}

/// Internal extension constructing a single-source target — the first-source
/// migrate path. Kept as a private helper so `LinkCommand::apply` reads cleanly;
/// it routes through the `*_risky` constructor (the only target mutator).
trait SeedWith: RelationshipTarget {
    /// Seeds `self`'s collection with one `source` (the first-source path).
    fn seed_with(self, source: Entity) -> Self;
}

impl<T: RelationshipTarget> SeedWith for T {
    #[inline]
    fn seed_with(mut self, source: Entity) -> Self {
        self.collection_mut_risky().add(source);
        self
    }
}
