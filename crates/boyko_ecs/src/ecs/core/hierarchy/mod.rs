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

use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry;
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

/// Private 1-field bundle newtype used to route the first-child `Children`
/// insert through the audited `migrate_entity_insert` machinery (Phase 19 R2
/// C1). `Bundle` is sealed and all insert machinery is `B: Bundle`-bound, so a
/// bare `Children` cannot reuse it — a 1-field newtype can.
///
/// `boyko-macros` is a dev-dependency only (it cannot be a normal dependency
/// without an architectural change), so `#[derive(Bundle)]` is unavailable in
/// library source. The `Bundle` impl is therefore hand-written to mirror the
/// derive output exactly — the established codebase pattern (see the hand-impl
/// `Component`s at `ecs_master.rs` test module / `component_pool.rs`). See
/// [`bundles`] for the impl + the SAFETY accounting of the reproduced
/// `for_each_component_bytes` byte-erasure.
pub(crate) struct ChildrenBundle(pub(crate) Children);

/// Private 1-field bundle newtype for inserting `ChildOf` through the audited
/// insert machinery (symmetry with [`ChildrenBundle`]; Phase 19 R2 C1).
///
/// `Bundle` impl is hand-written for the same dev-only-macros reason as
/// [`ChildrenBundle`] — see [`bundles`].
pub(crate) struct ChildOfBundle(pub(crate) ChildOf);

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
            ComponentId(raw)
        })
    }

    const HAS_HOOKS: bool = true;

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
    use crate::ecs::core::component::component_registry::get_hooks;

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
