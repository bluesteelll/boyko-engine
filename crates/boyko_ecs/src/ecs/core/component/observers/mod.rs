//! Component lifecycle observers (Phase 14b).
//!
//! Observers are the runtime-mutable sibling of the Phase 14a `ComponentHooks`:
//! where a hook is a single fn-ptr bound per-type at registration (write-once
//! into the cold `HOOKS` table), an *observer* is one of an arbitrarily-long,
//! `add`/`remove`-able list of fn-ptrs keyed by `(kind, component)`. They fire
//! at the same six structural-op sites as hooks, after the per-component hook.
//!
//! # Why a field, not a global static
//!
//! Unlike `HOOKS` (a process-global `OnceLock` table, write-once per slot), the
//! observer registry is **per-world mutable** — it is stored as a field on
//! `ArchetypeMaster` so the single archetype-creation funnel can seed the
//! per-archetype `ON_*_OBSERVER` flag bits at construction, and so add/remove
//! stay one cohesive `&mut self` op that also walks the archetypes. Two
//! `EcsMaster`s therefore have independent observer sets (Bevy's per-`World`
//! `Observers` model, minus the parameter-threading).
//!
//! # Zero-cost when unused
//!
//! [`ObserverRegistry`] holds its 5×512 `Vec` headers (Feature 2 widened the
//! kind dimension 4→5 for `Despawn`) behind an `Option<Box<ObserverLists>>` that
//! stays `None` until the first `add_observer`. A world that registers no
//! observers pays no allocation and one `Option::is_none()` early-out on every
//! registry read.
//!
//! # Feature 2 additions
//!
//! [`entity_store`] holds per-entity observers (a `SparseMap<u32>` handle + a
//! side arena), [`trigger`] holds custom-trigger types + the global trigger
//! registry, [`traversal`] / [`propagate`] back propagation, and [`dispatch_key`]
//! unifies lifecycle-kind and custom-trigger keys.
//!
//! Waves 1-3 ship the data structures, the per-archetype flag bits, and the
//! `ArchetypeMaster` integration (seed + dynamic walk). The cold
//! `fire_*_observers` dispatch fns live in the [`dispatch`] sub-module and are
//! wired at the six structural-op fire sites by `EcsMaster` (Wave 5).

pub(crate) mod dispatch;
pub(crate) mod dispatch_key;
pub(crate) mod entity_store;
pub mod propagate;
pub mod traversal;
pub mod trigger;

use crate::ecs::core::component::component_registry::MAX_COMPONENTS;
use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::ComponentId;

/// Stable handle to a registered observer, returned by `add_observer` and
/// consumed by `remove_observer`.
///
/// Monotonic and never reused: a removed slot's id is retired, so a stale
/// handle can never alias a different observer. `u64` so wrap is unreachable in
/// practice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ObserverId(pub(crate) u64);

/// The five observer kinds — 1:1 with the lifecycle-hook kinds.
///
/// Feature 2 added `Despawn` (the Phase-14b note that there was "intentionally
/// no `Despawn` kind" is reversed): an entity despawn now fires per-component
/// `Despawn` observers FIRST (whole-entity cleanup), then the per-component
/// `Replace` + `Remove` observers — all pre-drop. The discriminants are the
/// dense `[kind]` index into [`ObserverLists::by_kind_component`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ObserverKind {
    /// Fired after a component is newly added to an entity.
    Add = 0,
    /// Fired after a component is inserted (newly or via bundle insert).
    Insert = 1,
    /// Fired before an existing component value is overwritten.
    Replace = 2,
    /// Fired before a component is removed (reads the dying value).
    Remove = 3,
    /// Fired once per dying entity at despawn, BEFORE components drop
    /// (Feature 2). Per the despawn fire site, this fires FIRST (before the
    /// per-component `Replace`/`Remove` passes), so a handler reads the
    /// fully-intact dying entity.
    Despawn = 4,
}

/// Number of [`ObserverKind`] variants — the dense first dimension of
/// [`ObserverLists::by_kind_component`].
pub(crate) const NUM_OBSERVER_KINDS: usize = 5;

/// Type-erased observer runner. Mirrors [`HookFn`](crate::ecs::core::component::hooks::HookFn)
/// exactly (Phase 14b D2).
///
/// A plain fn pointer — zero-alloc, monomorphised at registration, and
/// unconditionally `Send + Sync` (the property that lets [`ObserverRegistry`]
/// be thread-safe with no `unsafe impl`). `unsafe` because the dispatch site (a
/// Wave-5 `fire_*_observers` fn) guarantees the apply-window-only +
/// non-aliasing invariants the body relies on.
pub type ObserverFn = unsafe fn(DeferredEcsMaster<'_>, ObserverContext);

/// Context handed to every observer.
///
/// Same shape as [`HookContext`](crate::ecs::core::component::hooks::HookContext)
/// (entity + component) plus the [`ObserverKind`], so one runner can be
/// registered for several kinds and branch on `kind` internally if desired.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ObserverContext {
    /// The entity the structural op targets.
    pub entity: Entity,
    /// Which component triggered the observer.
    pub component_id: ComponentId,
    /// Which lifecycle kind fired.
    pub kind: ObserverKind,
}

/// One registered observer: its stable id + the runner fn-ptr.
///
/// POD, `Copy` — the fire loop copies a single `ObserverEntry` out of the
/// registry by value before minting the view (Phase 14b OBS-FIRE-LOOP), so the
/// type must be cheap to copy. fn-ptr-only ⇒ auto `Send + Sync` (the D2
/// property, same as `ComponentHooks`).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub(crate) struct ObserverEntry {
    /// Stable id assigned at registration (matched on removal).
    pub(crate) id: ObserverId,
    /// The type-erased runner fired by the Wave-5 dispatch.
    pub(crate) runner: ObserverFn,
}

/// The eager 4×512 `Vec` payload, allocated lazily behind
/// [`ObserverRegistry::lists`].
///
/// `[kind][component] -> Vec<ObserverEntry>` keeps the per-`(kind, cid)` lookup
/// a 2-multiply dense index (no hashing): the fire path and the dynamic
/// archetype-bit walk both index `by_kind_component[kind][cid.0]` directly.
struct ObserverLists {
    /// `[ObserverKind as usize][ComponentId.0]` → the observers for that pair.
    by_kind_component: [[Vec<ObserverEntry>; MAX_COMPONENTS]; NUM_OBSERVER_KINDS],
}

impl Default for ObserverLists {
    /// `Vec` is not `Copy`, so the array `Default` is not auto-derivable for
    /// `[[Vec<_>; 512]; 4]` — build it element-wise with `array::from_fn`.
    fn default() -> Self {
        Self {
            by_kind_component: core::array::from_fn(|_| core::array::from_fn(|_| Vec::new())),
        }
    }
}

/// Per-world, runtime-mutable registry of `(kind, component)` lifecycle
/// observers (Phase 14b).
///
/// Stored as a field on `ArchetypeMaster`. Mutated only under `&mut self`
/// (`add` / `remove`), read under `&self` (the fire loop, `has_observer`, and
/// `ArchetypeFlags::insert_from_observers`). The fn-ptr-only entries make the
/// whole type `Send + Sync` by construction — no `unsafe impl`.
pub struct ObserverRegistry {
    /// `None` until the first [`add`](Self::add) — the lazy gate that keeps a
    /// world with no observers at zero allocation and zero 48 KiB cost.
    ///
    /// [`add`]: ObserverRegistry::add
    lists: Option<Box<ObserverLists>>,
    /// Monotonic id source, outside the `Box` so it is valid before any
    /// allocation and is read without touching the lists. `0` is a valid first
    /// id.
    next_id: u64,
}

impl ObserverRegistry {
    /// Creates an empty registry — zero allocation (the [`Default`] /
    /// construction state on `ArchetypeMaster::new`).
    #[inline]
    pub fn new() -> Self {
        Self { lists: None, next_id: 0 }
    }

    /// Registers `runner` for `(kind, cid)`, returning its fresh
    /// [`ObserverId`] and whether the `(kind, cid)` list transitioned from
    /// empty to non-empty.
    ///
    /// The `became_nonempty` flag tells the caller (`ArchetypeMaster::add_observer`)
    /// whether the per-archetype `ON_{kind}_OBSERVER` bit must now be raised on
    /// the archetypes containing `cid` (the add-first dynamic walk). On a
    /// non-first add the bit is already set, so no walk is needed.
    ///
    /// Lazily allocates the 4×512 list array on the first call only.
    pub fn add(
        &mut self,
        kind: ObserverKind,
        cid: ComponentId,
        runner: ObserverFn,
    ) -> (ObserverId, bool) {
        let id = ObserverId(self.next_id);
        self.next_id += 1;
        let lists = self.lists.get_or_insert_with(|| Box::new(ObserverLists::default()));
        let list = &mut lists.by_kind_component[kind as usize][cid.0];
        let became_nonempty = list.is_empty();
        list.push(ObserverEntry { id, runner });
        (id, became_nonempty)
    }

    /// Removes the observer with `id`, returning its `(kind, component)` and
    /// whether the `(kind, component)` list became empty as a result, or `None`
    /// if no observer with `id` is registered.
    ///
    /// `became_empty` tells `ArchetypeMaster::remove_observer` whether to run
    /// the remove-last recompute walk. Uses `swap_remove` (order within a list
    /// is irrelevant — entries are matched by id, never by position).
    pub fn remove(&mut self, id: ObserverId) -> Option<(ObserverKind, ComponentId, bool)> {
        let lists = self.lists.as_mut()?;
        const KINDS: [ObserverKind; NUM_OBSERVER_KINDS] = [
            ObserverKind::Add,
            ObserverKind::Insert,
            ObserverKind::Replace,
            ObserverKind::Remove,
            ObserverKind::Despawn,
        ];
        for kind in KINDS {
            let per_component = &mut lists.by_kind_component[kind as usize];
            for (cid_raw, list) in per_component.iter_mut().enumerate() {
                if let Some(pos) = list.iter().position(|entry| entry.id == id) {
                    list.swap_remove(pos);
                    return Some((kind, ComponentId(cid_raw), list.is_empty()));
                }
            }
        }
        None
    }

    /// Returns `true` if at least one observer is registered for `(kind, cid)`.
    ///
    /// The remove-last recompute walk uses this to decide, per affected
    /// archetype, whether any *sibling* component still observes `kind`.
    #[inline]
    pub fn has_observer(&self, kind: ObserverKind, cid: ComponentId) -> bool {
        self.lists
            .as_ref()
            .is_some_and(|l| !l.by_kind_component[kind as usize][cid.0].is_empty())
    }

    /// Returns the `(kind, cid)` observer list as a slice, or `None` when the
    /// registry has never allocated its lists (the zero-observer fast path).
    ///
    /// The Wave-5 `fire_*_observers` dispatch indexes this slice per iteration
    /// (re-deriving a fresh `&` each turn so no registry borrow spans the runner
    /// call — OBS-FIRE-LOOP). Keeping the indexing arithmetic behind one
    /// accessor lets the dispatch loop read the list without reaching into the
    /// private `ObserverLists` layout. An empty slice and `None` are equivalent
    /// to the caller (both terminate the walk on the first `len()` check).
    #[inline]
    pub(crate) fn fire_list(&self, kind: ObserverKind, cid: ComponentId) -> Option<&[ObserverEntry]> {
        self.lists
            .as_ref()
            .map(|l| l.by_kind_component[kind as usize][cid.0].as_slice())
    }
}

impl Default for ObserverRegistry {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::entity::entity::Entity;
    use crate::ecs::identifiers::primitives::EntityId;

    /// Dummy runner used only to populate an `ObserverFn` slot; never invoked in
    /// these data-structure tests.
    unsafe fn dummy(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {}

    const RUNNER: ObserverFn = dummy as ObserverFn;

    #[test]
    fn new_registry_is_lazy_and_empty() {
        let reg = ObserverRegistry::new();
        // No list allocated until the first add — the zero-cost gate.
        assert!(reg.lists.is_none(), "fresh registry allocates nothing");
        assert!(!reg.has_observer(ObserverKind::Add, ComponentId(0)));
        assert!(!reg.has_observer(ObserverKind::Remove, ComponentId(511)));
    }

    #[test]
    fn first_add_reports_became_nonempty_and_allocates() {
        let mut reg = ObserverRegistry::new();
        let (id, became) = reg.add(ObserverKind::Add, ComponentId(3), RUNNER);
        assert_eq!(id, ObserverId(0), "first id is 0");
        assert!(became, "empty -> non-empty on the first add for (kind, cid)");
        assert!(reg.lists.is_some(), "lists allocated on first add");
        assert!(reg.has_observer(ObserverKind::Add, ComponentId(3)));
        // A different kind / component is unaffected.
        assert!(!reg.has_observer(ObserverKind::Insert, ComponentId(3)));
        assert!(!reg.has_observer(ObserverKind::Add, ComponentId(4)));
    }

    #[test]
    fn second_add_same_pair_does_not_report_became_nonempty() {
        let mut reg = ObserverRegistry::new();
        let (_id0, became0) = reg.add(ObserverKind::Replace, ComponentId(7), RUNNER);
        let (id1, became1) = reg.add(ObserverKind::Replace, ComponentId(7), RUNNER);
        assert!(became0, "first add transitions empty -> non-empty");
        assert!(!became1, "second add for the same (kind, cid) is not a transition");
        assert_eq!(id1, ObserverId(1), "ids are monotonic");
    }

    #[test]
    fn ids_are_monotonic_across_kinds_and_components() {
        let mut reg = ObserverRegistry::new();
        let (a, _) = reg.add(ObserverKind::Add, ComponentId(0), RUNNER);
        let (b, _) = reg.add(ObserverKind::Remove, ComponentId(1), RUNNER);
        let (c, _) = reg.add(ObserverKind::Add, ComponentId(0), RUNNER);
        assert_eq!(a, ObserverId(0));
        assert_eq!(b, ObserverId(1));
        assert_eq!(c, ObserverId(2));
    }

    #[test]
    fn remove_unknown_id_returns_none() {
        let mut reg = ObserverRegistry::new();
        // Never allocated -> None.
        assert!(reg.remove(ObserverId(0)).is_none());
        // Allocated but id absent -> None.
        let _ = reg.add(ObserverKind::Add, ComponentId(0), RUNNER);
        assert!(reg.remove(ObserverId(999)).is_none());
    }

    #[test]
    fn remove_last_reports_became_empty_with_kind_and_component() {
        let mut reg = ObserverRegistry::new();
        let (id, _) = reg.add(ObserverKind::Insert, ComponentId(42), RUNNER);
        let removed = reg.remove(id).expect("registered id must be removable");
        assert_eq!(removed.0, ObserverKind::Insert, "kind is reported back");
        assert_eq!(removed.1, ComponentId(42), "component is reported back");
        assert!(removed.2, "removing the only entry empties the (kind, cid) list");
        assert!(!reg.has_observer(ObserverKind::Insert, ComponentId(42)));
    }

    #[test]
    fn remove_non_last_does_not_report_became_empty() {
        let mut reg = ObserverRegistry::new();
        let (id0, _) = reg.add(ObserverKind::Remove, ComponentId(5), RUNNER);
        let (_id1, _) = reg.add(ObserverKind::Remove, ComponentId(5), RUNNER);
        let removed = reg.remove(id0).expect("registered id must be removable");
        assert!(!removed.2, "one of two entries removed -> list still non-empty");
        assert!(
            reg.has_observer(ObserverKind::Remove, ComponentId(5)),
            "the surviving entry keeps the pair non-empty"
        );
    }

    #[test]
    fn ids_are_not_reused_after_removal() {
        let mut reg = ObserverRegistry::new();
        let (id0, _) = reg.add(ObserverKind::Add, ComponentId(1), RUNNER);
        assert!(reg.remove(id0).is_some());
        let (id1, _) = reg.add(ObserverKind::Add, ComponentId(1), RUNNER);
        assert_ne!(id0, id1, "a retired id is never minted again");
        assert_eq!(id1, ObserverId(1));
    }

    #[test]
    fn observer_entry_carries_id_and_runner() {
        // The fire loop copies an `ObserverEntry` by value; confirm it is the
        // small POD pair it must be (id + fn-ptr).
        let entry = ObserverEntry { id: ObserverId(3), runner: RUNNER };
        assert_eq!(entry.id, ObserverId(3));
        // fn-ptr equality is well-defined for the same monomorphisation.
        assert!(entry.runner as usize == RUNNER as usize);
    }

    #[test]
    fn observer_context_carries_entity_component_and_kind() {
        let ctx = ObserverContext {
            entity: Entity::new(EntityId(9), 2),
            component_id: ComponentId(13),
            kind: ObserverKind::Replace,
        };
        assert_eq!(ctx.entity.id().0, 9);
        assert_eq!(ctx.component_id.0, 13);
        assert_eq!(ctx.kind, ObserverKind::Replace);
    }

    /// `ObserverRegistry` and its POD members are `Send + Sync` (fn-ptr-only) —
    /// the property that lets it sit inside the `Send + Sync` `ArchetypeMaster`
    /// with no `unsafe impl` (Phase 14b §8 / SEND6).
    #[test]
    fn observer_types_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ObserverRegistry>();
        assert_send_sync::<ObserverEntry>();
        assert_send_sync::<ObserverContext>();
        assert_send_sync::<ObserverId>();
    }
}
