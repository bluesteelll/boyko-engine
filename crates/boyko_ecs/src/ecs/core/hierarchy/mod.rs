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
//! [`EntityCommands`] ergonomics
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
//! [`EntityCommands`]: crate::ecs::core::system::params::entity_commands::EntityCommands

use std::sync::OnceLock;

use crate::ecs::core::clone::map::EntityCloneMap;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry::{self, Cloneability, CloneFn};
use crate::ecs::core::component::hooks::{ComponentHooks, HookFn};
use crate::ecs::core::entity::entity::Entity;
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
/// Prefer the [`EntityCommands`] ergonomics
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
/// [`EntityCommands`]: crate::ecs::core::system::params::entity_commands::EntityCommands
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
    /// insert path (crate-internal; only Link/Unlink applies mutate `Children`).
    #[inline]
    pub(crate) fn with_one(child: Entity) -> Self {
        Self(vec![child])
    }

    /// Appends `child`. Crate-internal: only `LinkChildCommand::apply` calls it.
    #[inline]
    pub(crate) fn push(&mut self, child: Entity) {
        self.0.push(child);
    }

    /// Removes `child` via `Vec::swap_remove` (O(1), order-perturbing), returning
    /// whether it was present. Crate-internal: only `UnlinkChildCommand::apply`
    /// calls it.
    #[inline]
    pub(crate) fn swap_remove_entity(&mut self, child: Entity) -> bool {
        if let Some(idx) = self.0.iter().position(|&c| c == child) {
            self.0.swap_remove(idx);
            true
        } else {
            false
        }
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

    /// `ChildOf` links on insert and unlinks on replace (Phase 19 §3). It does
    /// NOT register `on_add` / `on_remove`: `on_add` would double-fire alongside
    /// the migrate-insert `on_insert`, and unlink-on-removal already rides
    /// `on_replace` (which the remove-migration fires before the value leaves).
    fn register_hooks(hooks: &mut ComponentHooks) {
        hooks.on_insert = Some(commands::child_of_on_insert as HookFn);
        hooks.on_replace = Some(commands::child_of_on_replace as HookFn);
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
            ComponentId(raw)
        })
    }

    const HAS_HOOKS: bool = true;

    /// `Children` registers ONLY `on_replace` — the recursive-despawn cascade
    /// (Phase 19 §3 / W4). It must NOT register `on_add` / `on_insert`: the
    /// first-child insert fires those, and a cascade there would despawn the
    /// brand-new (single-child) collection. It fires `on_replace` from
    /// `delete_entity` (the per-component pre-remove order), reading the CURRENT
    /// children.
    fn register_hooks(hooks: &mut ComponentHooks) {
        hooks.on_replace = Some(commands::children_on_replace as HookFn);
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
