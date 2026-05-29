//! `ArchetypeFlags` — per-archetype "which hook kinds does ANY component in
//! this archetype declare?" bitset (Phase 14a, plan §3.3 / §2.4).
//!
//! OR-computed once at archetype construction from the cold `HOOKS` table
//! (plan §4.6), then read as a single `u16` load + `test`/`jz` on every
//! structural-op dispatch site. When no component in the archetype declares
//! any hook, [`ArchetypeFlags::is_empty`] is `true` and the branch predicts
//! not-taken — the Phase 10 "0% when unused" mechanism applied to structural
//! ops.
//!
//! Mirrors Bevy's `ArchetypeFlags: u32` at boyko's bit count: 4 hook bits now,
//! 12 spare for 14b observer/despawn flags.

use crate::ecs::core::component::component_registry;
use crate::ecs::identifiers::primitives::ComponentId;

/// Per-archetype hook-presence bitset.
///
/// `#[repr(transparent)]` over `u16`; trivially `Send + Sync` (a plain
/// integer). The bit constants name the four lifecycle-hook kinds; bits 4..16
/// are reserved for Phase 14b observer/despawn flags.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct ArchetypeFlags(u16);

impl ArchetypeFlags {
    /// Set iff some component in the archetype declares an `on_add` hook.
    pub const ON_ADD_HOOK: u16 = 1 << 0;
    /// Set iff some component in the archetype declares an `on_insert` hook.
    pub const ON_INSERT_HOOK: u16 = 1 << 1;
    /// Set iff some component in the archetype declares an `on_replace` hook.
    pub const ON_REPLACE_HOOK: u16 = 1 << 2;
    /// Set iff some component in the archetype declares an `on_remove` hook.
    pub const ON_REMOVE_HOOK: u16 = 1 << 3;
    // bit 4 RESERVED + bits 5..16 reserved for 14b observer/despawn flags
    // (do NOT renumber the four bits above; the layout is forward-compatible).

    /// Returns an empty flag set (no hook bits raised).
    #[inline]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Returns `true` if `bit` is set.
    ///
    /// `bit` is one of the `ON_*_HOOK` associated constants.
    #[inline]
    pub const fn contains(self, bit: u16) -> bool {
        self.0 & bit != 0
    }

    /// Raises `bit` in the set.
    #[inline]
    pub fn insert(&mut self, bit: u16) {
        self.0 |= bit;
    }

    /// Returns `true` if no hook bit is raised (the no-hook hot path).
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// ORs into `self` the hook bits declared by component `cid`, reading the
    /// cold `HOOKS` table once (plan §4.6). A no-op when `cid` has no hooks
    /// (the common case). Called per component in the archetype-construction
    /// compute loops (`create_by_ids` / `register_component_inplace`).
    #[inline]
    pub fn insert_from_hooks(&mut self, cid: ComponentId) {
        if let Some(hooks) = component_registry::get_hooks(cid.0) {
            if hooks.on_add.is_some() {
                self.insert(Self::ON_ADD_HOOK);
            }
            if hooks.on_insert.is_some() {
                self.insert(Self::ON_INSERT_HOOK);
            }
            if hooks.on_replace.is_some() {
                self.insert(Self::ON_REPLACE_HOOK);
            }
            if hooks.on_remove.is_some() {
                self.insert(Self::ON_REMOVE_HOOK);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::hooks::ComponentHooks;

    // Hook IDs for the `insert_from_hooks` tests. The cold `HOOKS` table is a
    // global `[OnceLock; MAX_COMPONENTS]`, so each slot is write-once for the
    // life of the test binary — these IDs must be disjoint from every other
    // test's component slots. 508..=511 sit at the very top of the id space,
    // away from the integration-test allocations (≤ 619) and the unit-test
    // register_layout slots (≤ 465).
    const HOOK_ADD_ID: ComponentId = ComponentId(508);
    const HOOK_INSERT_ID: ComponentId = ComponentId(509);
    const HOOK_ALL_ID: ComponentId = ComponentId(510);
    const NO_HOOK_ID: ComponentId = ComponentId(511);

    /// Dummy hook used only to populate an `Option<HookFn>` field as `Some`.
    unsafe fn dummy(
        _w: crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster<'_>,
        _c: crate::ecs::core::component::hooks::HookContext,
    ) {
    }

    #[test]
    fn empty_has_no_bits_and_is_empty() {
        let f = ArchetypeFlags::empty();
        assert!(f.is_empty(), "empty() yields no raised bits");
        assert!(!f.contains(ArchetypeFlags::ON_ADD_HOOK), "no on_add bit");
        assert!(!f.contains(ArchetypeFlags::ON_REMOVE_HOOK), "no on_remove bit");
    }

    #[test]
    fn default_equals_empty() {
        // `ArchetypeFlags` derives `PartialEq` but not `Debug`, so compare with
        // `==` (the production type intentionally omits `Debug`).
        assert!(
            ArchetypeFlags::default() == ArchetypeFlags::empty(),
            "Default is the all-clear no-hook value"
        );
    }

    #[test]
    fn insert_raises_only_the_named_bit() {
        let mut f = ArchetypeFlags::empty();
        f.insert(ArchetypeFlags::ON_REPLACE_HOOK);
        assert!(f.contains(ArchetypeFlags::ON_REPLACE_HOOK), "on_replace raised");
        assert!(!f.contains(ArchetypeFlags::ON_ADD_HOOK), "on_add untouched");
        assert!(!f.is_empty(), "a raised bit means non-empty");
    }

    #[test]
    fn insert_is_idempotent_and_ors() {
        let mut f = ArchetypeFlags::empty();
        f.insert(ArchetypeFlags::ON_ADD_HOOK);
        f.insert(ArchetypeFlags::ON_ADD_HOOK); // repeat — no change
        f.insert(ArchetypeFlags::ON_INSERT_HOOK);
        assert!(f.contains(ArchetypeFlags::ON_ADD_HOOK));
        assert!(f.contains(ArchetypeFlags::ON_INSERT_HOOK));
        assert!(!f.contains(ArchetypeFlags::ON_REMOVE_HOOK));
    }

    #[test]
    fn four_bits_are_distinct_powers_of_two() {
        // Forward-compat guard: the four hook bits must stay 1,2,4,8 (do NOT
        // renumber — 14b reserves bits 4..16).
        assert_eq!(ArchetypeFlags::ON_ADD_HOOK, 1 << 0);
        assert_eq!(ArchetypeFlags::ON_INSERT_HOOK, 1 << 1);
        assert_eq!(ArchetypeFlags::ON_REPLACE_HOOK, 1 << 2);
        assert_eq!(ArchetypeFlags::ON_REMOVE_HOOK, 1 << 3);
    }

    #[test]
    fn insert_from_hooks_with_no_registered_hooks_is_noop() {
        // NO_HOOK_ID is never installed into the HOOKS table.
        let mut f = ArchetypeFlags::empty();
        f.insert_from_hooks(NO_HOOK_ID);
        assert!(f.is_empty(), "a component with no hooks raises no bit (0-cost path)");
    }

    #[test]
    fn insert_from_hooks_raises_only_declared_kinds() {
        // Install an on_add-only hook set for HOOK_ADD_ID.
        let mut add_only = ComponentHooks::default();
        add_only.on_add = Some(dummy as super::super::HookFn);
        assert!(
            component_registry::try_set_hooks(HOOK_ADD_ID.0, add_only),
            "first install into a fresh slot must succeed"
        );

        let mut f = ArchetypeFlags::empty();
        f.insert_from_hooks(HOOK_ADD_ID);
        assert!(f.contains(ArchetypeFlags::ON_ADD_HOOK), "on_add bit raised from table");
        assert!(!f.contains(ArchetypeFlags::ON_INSERT_HOOK), "on_insert NOT raised");
        assert!(!f.contains(ArchetypeFlags::ON_REPLACE_HOOK), "on_replace NOT raised");
        assert!(!f.contains(ArchetypeFlags::ON_REMOVE_HOOK), "on_remove NOT raised");
    }

    #[test]
    fn insert_from_hooks_with_insert_only_raises_insert_bit() {
        let mut insert_only = ComponentHooks::default();
        insert_only.on_insert = Some(dummy as super::super::HookFn);
        assert!(component_registry::try_set_hooks(HOOK_INSERT_ID.0, insert_only));

        let mut f = ArchetypeFlags::empty();
        f.insert_from_hooks(HOOK_INSERT_ID);
        assert!(f.contains(ArchetypeFlags::ON_INSERT_HOOK), "on_insert raised");
        assert!(!f.contains(ArchetypeFlags::ON_ADD_HOOK), "on_add NOT raised");
    }

    #[test]
    fn insert_from_hooks_all_four_raises_all_bits() {
        let f = dummy as super::super::HookFn;
        let all = ComponentHooks {
            on_add: Some(f),
            on_insert: Some(f),
            on_replace: Some(f),
            on_remove: Some(f),
        };
        assert!(component_registry::try_set_hooks(HOOK_ALL_ID.0, all));

        let mut flags = ArchetypeFlags::empty();
        flags.insert_from_hooks(HOOK_ALL_ID);
        assert!(flags.contains(ArchetypeFlags::ON_ADD_HOOK));
        assert!(flags.contains(ArchetypeFlags::ON_INSERT_HOOK));
        assert!(flags.contains(ArchetypeFlags::ON_REPLACE_HOOK));
        assert!(flags.contains(ArchetypeFlags::ON_REMOVE_HOOK));
    }

    #[test]
    fn archetype_flags_is_two_bytes() {
        // `#[repr(transparent)]` over u16 — the load-bearing "one u16 load"
        // gate size (plan §3.3 / O3).
        assert_eq!(std::mem::size_of::<ArchetypeFlags>(), 2);
    }
}
