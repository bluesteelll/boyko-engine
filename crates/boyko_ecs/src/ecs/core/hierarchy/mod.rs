//! Entity parent-child hierarchies (Phase 19, CORE).
//!
//! Two engine-defined components kept bidirectionally consistent by component
//! lifecycle hooks (the Phase 14a/14b substrate):
//!
//! * [`ChildOf`] — the foreign key on the **child** (source of truth). Inserting
//!   it links the child to a parent; overwriting it reparents; removing it
//!   unlinks.
//! * [`Children`] — the reverse collection on the **parent**, maintained
//!   reactively by [`ChildOf`]'s hooks. User code never writes `Children`
//!   directly.
//!
//! # Model
//!
//! This mirrors the Bevy-0.16 relationship model (research §1): `ChildOf`
//! registers `on_insert` (link) + `on_replace` (unlink); `Children` registers
//! `on_replace` (the recursive-despawn cascade). The whole relationship is
//! driven by `ChildOf` insertion / removal via the [`Commands`] /
//! `EntityCommands` ergonomics
//! (`commands.entity(parent).add_child(child)` etc.).
//!
//! # Consistency window
//!
//! `Children` becomes consistent with `ChildOf` only after the deferred-command
//! drain at the apply window (the hooks enqueue `LinkChildCommand` /
//! `UnlinkChildCommand`). This is the same same-frame staleness boyko already
//! accepts for observer-driven mutation — see [`commands`].
//!
//! # 0%-when-unused
//!
//! A program that never mints a `ChildOf` / `Children` component id leaves the
//! cold `HOOKS` slots unset, so the per-archetype `ArchetypeFlags` gate raises
//! no hierarchy bit and the hot iteration path pays nothing.
//!
//! [`Commands`]: crate::ecs::core::system::params::commands::Commands

use std::sync::OnceLock;

use crate::ecs::core::clone::map::EntityCloneMap;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry::{
    self, Cloneability, CloneFn, DeserializeFn, LoadMapEntitiesFn, SerializeFn, Serializability,
    WireBridge,
};
use crate::ecs::core::component::hooks::{ComponentHooks, HookFn};
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::relationship::{Relationship, RelationshipTarget};
use crate::ecs::core::serialize::{DecodeError, LoadEntityMap};
use crate::ecs::identifiers::primitives::ComponentId;

pub mod bundles;
pub mod commands;

/// Maximum number of children copied into the on-stack cascade buffer before
/// falling back to the per-turn re-derivation wide path (Phase 19 R2 Q3 / M2).
///
/// Sized so the common case (a handful of children) never touches the heap and
/// runs the branch-light inline path; parents with more children than this take
/// the slower-but-allocation-free wide path in [`Children::on_replace`]
/// (`commands.rs`).
pub(crate) const CASCADE_FANOUT_INLINE: usize = 32;

/// Foreign key on a **child** entity pointing at its parent (Phase 19).
///
/// `ChildOf` is the source of truth for the parent-child relationship:
///
/// * Inserting `ChildOf(parent)` links the child into `parent`'s [`Children`]
///   (via the `on_insert` hook).
/// * Overwriting `ChildOf` (reparenting) unlinks from the old parent then links
///   into the new one (`on_replace` then `on_insert`, applied in FIFO order).
/// * Removing `ChildOf` unlinks the child (`on_replace`).
///
/// Prefer the `EntityCommands` ergonomics
/// (`commands.entity(parent).add_child(child)` /
/// `commands.entity(child).set_parent(parent)`) over writing `ChildOf` by hand;
/// they all funnel through `ChildOf` insertion / removal.
///
/// # Guards
///
/// A self-referential `ChildOf(self)` and a `ChildOf` pointing at a
/// non-existent parent are both rejected reactively: the hook removes the bad
/// `ChildOf` and the parent's collection is never touched. Deeper cycles are a
/// documented footgun (only self-reference is guarded — research §1 pitfall 5).
///
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildOf(pub Entity);

/// Reverse collection on a **parent** entity listing its children (Phase 19).
///
/// Maintained reactively by [`ChildOf`]'s hooks — **user code never writes
/// `Children` directly**; mutation is exposed only to the crate-internal
/// Link/Unlink command applies.
///
/// # Sibling order
///
/// Sibling order is **unspecified** and changes on removal: a child is removed
/// with `Vec::swap_remove` (O(1), the last child fills the gap), so the order is
/// not a stable contract. Sort at the consumer if a deterministic order is
/// required.
///
/// # Retained when empty
///
/// Removing the last child does **not** remove the `Children` component — an
/// ex-parent keeps an empty `Children` (a 24 B header over a zero-capacity
/// `Vec` — no heap allocation until the next push). Rationale: a child-count
/// `0↔1↔0` oscillation under remove-on-empty would migrate the parent's
/// archetype on every transition (~590 ns full byte-copy) versus a pure
/// in-place `swap_remove` (~90 ns class). Archetype-gated iteration skips an
/// empty `Children` row at zero cost.
///
/// # Cycles
///
/// Deep `ChildOf` cycles (A→B→…→A, not a direct self-reference) are **not**
/// detected — only the one-compare self-reference guard exists. A cycle is a
/// documented footgun: a recursive despawn over a cycle would re-enter
/// indefinitely. Do not build `ChildOf` cycles.
#[repr(transparent)]
#[derive(Debug, Default)]
pub struct Children(Vec<Entity>);

impl Children {
    /// Returns the children as a slice. Sibling order is unspecified (see the
    /// type docs).
    #[inline]
    pub fn as_slice(&self) -> &[Entity] {
        &self.0
    }

    /// Number of children.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// `true` when this parent currently has no children. An emptied `Children`
    /// is retained, not removed (see the type docs).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `true` when `entity` is one of this parent's children.
    #[inline]
    pub fn contains(&self, entity: Entity) -> bool {
        self.0.contains(&entity)
    }

    /// Constructs a `Children` holding exactly one child — the first-child
    /// insert path (crate-internal). Used by the deep-clone reverse-index rebuild
    /// (`clone/deep.rs`, which mirrors `LinkCommand::apply`'s first-source migrate
    /// without going through the deferred queue). The generic `LinkCommand` /
    /// `UnlinkCommand` mutate the collection through
    /// [`RelationshipTarget::collection_mut_risky`] (the `RelationshipSourceCollection`
    /// `add` / `remove`), so the former bespoke `push` / `swap_remove_entity`
    /// mutators are superseded by the generic `Vec<Entity>` collection impl.
    #[inline]
    pub(crate) fn with_one(child: Entity) -> Self {
        Self(vec![child])
    }
}

// Relations Option A (D4): `ChildOf` / `Children` ARE the generic machinery. The
// hand-written `Relationship` / `RelationshipTarget` impls below are the in-crate
// MIRROR of `#[derive(Relationship)]` / `#[derive(RelationshipTarget)]` output
// (the dev-dep cycle precludes the derives in library `src/`, exactly like
// `impl_self_bundle!` mirrors `#[derive(Bundle)]`). They wire the generic
// monomorphized hooks (`relationship_on_insert::<ChildOf>` etc.) into
// `register_hooks` below, replacing the deleted bespoke `child_of_on_*` /
// `children_on_replace` bodies.

impl Relationship for ChildOf {
    type Target = Children;

    #[inline]
    fn target(&self) -> Entity {
        self.0
    }

    #[inline]
    fn from_target(target: Entity) -> Self {
        ChildOf(target)
    }
    // `ALLOW_SELF_REFERENTIAL` keeps the trait default (`false`): a
    // `ChildOf(self)` is reactively removed by the generic `on_insert` guard
    // (Phase-19 self-ref behavior, B3).

    /// A `ChildOf` parent forest is structurally acyclic (the Phase-19
    /// self-ref + cascade discipline forbids a cycle), so the transitive
    /// `ancestors` / `descendants` walks const-fold away the revisit guard and
    /// rely on the depth cap alone.
    const ACYCLIC: bool = true;
}

impl RelationshipTarget for Children {
    type Source = ChildOf;
    type Collection = Vec<Entity>;

    /// `Children` recursively despawns its children on the parent's despawn
    /// (Phase-19 B9 / B13 — `LINKED_DESPAWN`).
    const LINKED_DESPAWN: bool = true;

    /// `Children` retains an emptied collection to dodge `0↔1↔0` archetype thrash
    /// (Phase-19 B8 — the perf rule, now an explicit policy const; v1 mandatory).
    const RETAIN_EMPTY: bool = true;

    #[inline]
    fn collection(&self) -> &Self::Collection {
        &self.0
    }

    #[inline]
    fn collection_mut_risky(&mut self) -> &mut Self::Collection {
        &mut self.0
    }

    #[inline]
    fn from_collection_risky(collection: Self::Collection) -> Self {
        Children(collection)
    }
}

// Phase 22 D7: the Phase-19 `ChildrenBundle` / `ChildOfBundle` 1-field
// newtypes were deleted — `ChildOf` / `Children` implement `Bundle` directly
// via `impl_self_bundle!` (see [`bundles`]) and ride the audited insert
// machinery as themselves.

/// Remaps a cloned `ChildOf`'s parent through the deep-clone source→clone map
/// (Feature 3, D5). Installed ONLY for `ChildOf` (the single relationship remap in
/// v1) via [`install_map_entities_fn`](component_registry::install_map_entities_fn)
/// in `ChildOf::component_id()`. A parent inside the cloned subtree is rewritten to
/// the cloned parent; a parent OUTSIDE the subtree (the cloned root's external
/// parent) is left verbatim.
///
/// # Safety (the [`crate::ecs::core::component::component_registry::MapEntitiesFn`] contract)
/// `dst` points at a live, initialized `ChildOf` (a `#[repr(transparent)]` over
/// `Entity`); `map` is a shared, non-aliased reference for the call's duration.
unsafe fn child_of_map_entities(dst: *mut u8, map: &EntityCloneMap) {
    // SAFETY: `dst` is a live, aligned, initialized `ChildOf` row (the deep-clone
    //   remap pass resolves it through the fast store for an archetype that hosts
    //   `ChildOf`). We form `&mut ChildOf` to rewrite its inner `Entity` in place;
    //   no other reference aliases it (single-threaded `&mut EcsMaster`).
    let child_of: &mut ChildOf = unsafe { &mut *dst.cast::<ChildOf>() };
    if let Some(mapped) = map.get(child_of.0) {
        child_of.0 = mapped; // clone points at the cloned parent
    }
    // else: parent is outside the cloned subtree → keep verbatim (shared sibling).
}

/// Remaps a LOADED `ChildOf`'s parent through the load-direction saved→fresh map
/// (Serialization S2.5, plan §3.11 step 5 / C4). Installed for `ChildOf` (the
/// hand-written v1 relationship remap) via
/// [`install_serialize_fn`](component_registry::install_serialize_fn) reading the
/// `map_entities_fn()` override below, and invoked by the loader's
/// [`remap_loaded_entities`](crate::ecs::core::serialize::remap_loaded_entities)
/// pass. Mirrors the clone-side [`child_of_map_entities`] but errors (rather than
/// keeping verbatim) on an unmapped saved id — a load references only entities that
/// were themselves saved, so a missing mapping is a corrupt / dangling reference,
/// not an external sibling (C4: loud, never silent dangling-ref corruption).
///
/// # Safety (the [`LoadMapEntitiesFn`] contract)
/// `dst` points at a live, initialized `ChildOf` (a `#[repr(transparent)]` over
/// `Entity`); `map` is a shared, non-aliased reference for the call's duration.
unsafe fn child_of_load_map_entities(
    dst: *mut u8,
    map: &LoadEntityMap,
) -> Result<(), DecodeError> {
    // SAFETY: `dst` is a live, aligned, initialized `ChildOf` row (the load remap
    //   pass derives it from the pool's live-row pointer for an archetype that hosts
    //   `ChildOf`). We form `&mut ChildOf` to rewrite its inner `Entity` in place; no
    //   other reference aliases it (single-threaded `&mut EcsMaster`).
    let child_of: &mut ChildOf = unsafe { &mut *dst.cast::<ChildOf>() };
    match map.get(child_of.0.id().0) {
        Some(fresh) => {
            child_of.0 = fresh; // the loaded child points at the loaded parent
            Ok(())
        }
        // C4: the saved parent id is not in this load's map — a dangling reference.
        // Surface it loudly; the loader turns it into a `LoadError::Decode`.
        None => Err(DecodeError::UnmappedEntity),
    }
}

/// Serialization S2.5 — the bound-free `WireBridge` for `ChildOf` (the hand-written
/// equivalent of what `#[derive(Component)]` emits for a one-field struct). Maps
/// `ChildOf` to its single `Entity` field tuple so the generic
/// [`serialize_via_wire`](component_registry::serialize_via_wire) /
/// [`deserialize_via_wire`](component_registry::deserialize_via_wire) glue can
/// encode it through the [`Wire`](crate::ecs::core::serialize::Wire) `Entity` codec
/// (the raw saved id; the remap above rewrites it on load).
impl WireBridge for ChildOf {
    type Owned = (Entity,);
    type Refs<'a> = (&'a Entity,);

    #[inline]
    fn as_refs(&self) -> Self::Refs<'_> {
        (&self.0,)
    }

    #[inline]
    fn from_owned(owned: Self::Owned) -> Self {
        ChildOf(owned.0)
    }
}

impl Component for ChildOf {
    #[inline]
    fn component_id() -> ComponentId {
        static ID: OnceLock<ComponentId> = OnceLock::new();
        *ID.get_or_init(|| {
            let raw = component_registry::register_new::<Self>();
            // C2: a hand-written `component_id()` MUST trigger `install_hooks`
            // here, exactly like the derive (`boyko_macros::lib.rs:111`). Without
            // it `HAS_HOOKS == true` but the cold `HOOKS` slot stays unset, so the
            // link/unlink/cascade hooks would silently never fire.
            if Self::HAS_HOOKS {
                component_registry::install_hooks::<Self>(raw);
            }
            // Feature 3: a hand-written `component_id()` MUST install the clone
            // metadata too (the derive does this ungated for derived types). `ChildOf`
            // is `Copy`-with-an-`Entity`-field, so it is classified `CloneViaFn` (NOT
            // `TriviallyCopyable`) so the deep-clone `ChildOf` remap can run.
            component_registry::install_clone_fn::<Self>(raw);
            // Feature 3 D5: install the SINGLE relationship remap fn (ChildOf only).
            component_registry::install_map_entities_fn(
                raw,
                child_of_map_entities as component_registry::MapEntitiesFn,
            );
            // Serialization S2.5: a hand-written `component_id()` MUST install the
            // serialize metadata too (the derive does this ungated for derived
            // types). `ChildOf` is `SerializeViaFn` (it carries an `Entity`, never
            // blittable) with a load-direction `map_entities_fn` — without these
            // installs the saver would classify `ChildOf` as `Ignore` and skip it,
            // and the loader would have no remap fn (the C4 boundary).
            component_registry::install_serialize_fn::<Self>(raw);
            component_registry::register_stable_name::<Self>(raw);
            ComponentId(raw)
        })
    }

    const HAS_HOOKS: bool = true;

    /// Feature 3: `ChildOf` is `Copy`-with-an-`Entity`-field → `CloneViaFn` (NOT
    /// trivially copyable) so the deep-clone remap pass runs.
    const CLONE_BEHAVIOR: Cloneability = Cloneability::CloneViaFn;

    #[inline]
    fn clone_fn() -> Option<CloneFn> {
        Some(component_registry::clone_via_clone::<Self> as CloneFn)
    }

    /// Serialization S2.5: `ChildOf` carries an `Entity`, so it is NEVER blittable
    /// (the saved id must be remapped on load) — `SerializeViaFn`, encoded through
    /// the `WireBridge` glue below (mirrors the derive's classification for an
    /// Entity-bearing component, C3 / C4).
    const SERIALIZABILITY: Serializability = Serializability::SerializeViaFn;

    /// Serialization S2.5: the same value the derive would fold for a
    /// `#[repr(transparent)]` one-`Entity`-field struct — a layout-change guard (C2).
    /// Hand-mirrored so a `ChildOf` shape change is detected on load.
    const LAYOUT_FINGERPRINT: u64 = {
        const fn push(buf: &mut [u8; 48], len: &mut usize, value: u64) {
            let bytes = value.to_le_bytes();
            let mut i = 0;
            while i < 8 {
                buf[*len] = bytes[i];
                *len += 1;
                i += 1;
            }
        }
        let mut buf = [0u8; 8 * 6];
        let mut len = 0usize;
        push(&mut buf, &mut len, size_of::<ChildOf>() as u64);
        push(&mut buf, &mut len, align_of::<ChildOf>() as u64);
        // repr tag 2 == transparent (matches the derive's `ReprKind::Transparent`).
        push(&mut buf, &mut len, 2);
        push(&mut buf, &mut len, 1); // field_count
        push(&mut buf, &mut len, 0); // offset_of the single field (transparent)
        push(&mut buf, &mut len, size_of::<Entity>() as u64);
        let slice: &[u8] = buf.split_at(len).0;
        component_registry::fnv1a_64(slice)
    };

    /// Serialization S2.5: the per-element encoder, through the generic
    /// `serialize_via_wire` glue + the `WireBridge` above (the `Entity` field is
    /// encoded as its raw saved id via the `Wire` codec).
    #[inline]
    fn serialize_fn() -> Option<SerializeFn> {
        Some(component_registry::serialize_via_wire::<Self> as SerializeFn)
    }

    /// Serialization S2.5: the per-element decoder (the inverse of `serialize_fn`).
    #[inline]
    fn deserialize_fn() -> Option<DeserializeFn> {
        Some(component_registry::deserialize_via_wire::<Self> as DeserializeFn)
    }

    /// Serialization S2.5 (C4): the load-direction entity-remap fn — the v1
    /// relationship remap that rewrites the saved parent id to the loaded parent's
    /// fresh `Entity`. Installed via `install_serialize_fn` reading this method.
    #[inline]
    fn map_entities_fn() -> Option<LoadMapEntitiesFn> {
        Some(child_of_load_map_entities as LoadMapEntitiesFn)
    }

    /// `ChildOf` links on insert and unlinks on replace (Phase 19 §3). It does
    /// NOT register `on_add` / `on_remove`: `on_add` would double-fire alongside
    /// the migrate-insert `on_insert`, and unlink-on-removal already rides
    /// `on_replace` (which the remove-migration fires before the value leaves).
    ///
    /// Relations Option A: the slots are the GENERIC monomorphizations
    /// `<ChildOf as Relationship>::on_insert` / `::on_replace` (which forward to
    /// `relationship_on_insert::<ChildOf>` / `relationship_on_replace::<ChildOf>`),
    /// replacing the deleted hand-written `child_of_on_*` bodies. The fn pointers
    /// monomorphize to the same machine code.
    fn register_hooks(hooks: &mut ComponentHooks) {
        hooks.on_insert = Some(<Self as Relationship>::on_insert as HookFn);
        hooks.on_replace = Some(<Self as Relationship>::on_replace as HookFn);
    }
}

impl Component for Children {
    #[inline]
    fn component_id() -> ComponentId {
        static ID: OnceLock<ComponentId> = OnceLock::new();
        *ID.get_or_init(|| {
            let raw = component_registry::register_new::<Self>();
            // C2: see `ChildOf::component_id` — the install is mandatory.
            if Self::HAS_HOOKS {
                component_registry::install_hooks::<Self>(raw);
            }
            // Feature 3: populate the clone slot (ungated, like the derive).
            // `Children` keeps the default `Cloneability::Ignore` / `clone_fn ==
            // None`: it is a derived reverse index, ALWAYS cloner-denied (a deep
            // clone rebuilds it via `LinkChildCommand`, never byte-copies it).
            component_registry::install_clone_fn::<Self>(raw);
            // BUG-RELATIONS-CLONE-1: flag `Children` as a relationship-target reverse
            // index so the GENERIC clone-deny (`select_clone_ids` via
            // `is_relationship_target`) denies it — replacing the old literal
            // `children_id` special-case with the same predicate every derived target
            // (`LikedBy`, …) uses.
            component_registry::set_relationship_target(raw);
            ComponentId(raw)
        })
    }

    const HAS_HOOKS: bool = true;

    /// `Children` registers ONLY `on_replace` — the recursive-despawn cascade
    /// (Phase 19 §3 / W4 / B7). It must NOT register `on_add` / `on_insert`: the
    /// first-child insert fires those, and a cascade there would despawn the
    /// brand-new (single-child) collection. It fires `on_replace` from
    /// `delete_entity` (the per-component pre-remove order), reading the CURRENT
    /// children.
    ///
    /// Relations Option A: the slot is the GENERIC monomorphization
    /// `<Children as RelationshipTarget>::on_replace` (forwarding to
    /// `relationship_target_on_replace::<Children>`), replacing the deleted
    /// hand-written `children_on_replace` body. Same machine code.
    fn register_hooks(hooks: &mut ComponentHooks) {
        hooks.on_replace = Some(<Self as RelationshipTarget>::on_replace as HookFn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component_registry::{
        get_clone_info, get_hooks, get_map_entities_fn,
    };

    /// Feature 3 install-probe (mirrors `hooks_install_for_child_of_and_children`).
    ///
    /// A hand-written `component_id()` that omits the `install_clone_fn` /
    /// `install_map_entities_fn` calls would leave `CLONE_BEHAVIOR == CloneViaFn`
    /// but the cold `CLONE` / `MAP_ENTITIES` slots unset — silently breaking deep
    /// clone (the `ChildOf` remap would never run). This asserts both installs
    /// fired with the expected shape.
    #[test]
    fn clone_install_for_child_of_and_children() {
        let child_of = get_clone_info(ChildOf::component_id().0)
            .expect("ChildOf clone info must be installed by component_id()");
        assert_eq!(
            child_of.cloneability,
            Cloneability::CloneViaFn,
            "ChildOf is Copy-with-Entity ⇒ CloneViaFn (so deep-clone remap runs)"
        );
        assert!(
            child_of.clone_fn.is_some(),
            "ChildOf installs Some(clone_via_clone::<ChildOf>)"
        );
        assert!(
            get_map_entities_fn(ChildOf::component_id().0).is_some(),
            "ChildOf installs its map_entities_fn (the v1 relationship remap)"
        );

        let children = get_clone_info(Children::component_id().0)
            .expect("Children clone info must be installed by component_id()");
        assert_eq!(
            children.cloneability,
            Cloneability::Ignore,
            "Children is always cloner-denied (derived reverse index)"
        );
        assert!(
            children.clone_fn.is_none(),
            "Children installs no clone fn (never byte-copied)"
        );
        assert!(
            get_map_entities_fn(Children::component_id().0).is_none(),
            "Children installs no remap fn (only ChildOf does in v1)"
        );
    }

    /// C2 install-probe (Phase 19 R2 §C2) — the foundation tripwire.
    ///
    /// A hand-written `component_id()` that omits the `install_hooks` call would
    /// leave `HAS_HOOKS == true` but the cold `HOOKS` slot unset, silently
    /// disabling every downstream link/unlink/cascade hook. This asserts the
    /// install fired and registered EXACTLY the expected hook kinds (the
    /// negative asserts guard against over-registration that would double-fire).
    #[test]
    fn hooks_install_for_child_of_and_children() {
        let child_of = get_hooks(ChildOf::component_id().0)
            .expect("ChildOf hooks must be installed by component_id()");
        assert!(child_of.on_insert.is_some(), "ChildOf registers on_insert (link)");
        assert!(child_of.on_replace.is_some(), "ChildOf registers on_replace (unlink)");
        assert!(child_of.on_add.is_none(), "ChildOf must NOT register on_add");
        assert!(child_of.on_remove.is_none(), "ChildOf must NOT register on_remove");

        let children = get_hooks(Children::component_id().0)
            .expect("Children hooks must be installed by component_id()");
        assert!(children.on_replace.is_some(), "Children registers on_replace (cascade)");
        assert!(children.on_add.is_none(), "Children must NOT register on_add");
        assert!(children.on_insert.is_none(), "Children must NOT register on_insert");
        assert!(children.on_remove.is_none(), "Children must NOT register on_remove");
    }
}
