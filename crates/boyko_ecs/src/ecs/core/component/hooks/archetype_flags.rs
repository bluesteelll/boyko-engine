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
use crate::ecs::core::component::observers::{ObserverKind, ObserverRegistry};
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
    // bit 4 stays RESERVED (forward-compat with a future `on_despawn` hook;
    // do NOT renumber — keeps parity with Bevy's bit-4 = ON_DESPAWN_HOOK slot).

    /// Set iff some component in the archetype has ≥1 registered `on_add`
    /// observer (Phase 14b).
    pub const ON_ADD_OBSERVER: u16 = 1 << 5;
    /// Set iff some component in the archetype has ≥1 registered `on_insert`
    /// observer (Phase 14b).
    pub const ON_INSERT_OBSERVER: u16 = 1 << 6;
    /// Set iff some component in the archetype has ≥1 registered `on_replace`
    /// observer (Phase 14b).
    pub const ON_REPLACE_OBSERVER: u16 = 1 << 7;
    /// Set iff some component in the archetype has ≥1 registered `on_remove`
    /// observer (Phase 14b).
    pub const ON_REMOVE_OBSERVER: u16 = 1 << 8;

    /// `on_add` gate mask: set iff the archetype has an `on_add` hook OR
    /// observer. The structural-op fire site widens its inner test from
    /// `ON_ADD_HOOK` to this (Phase 14b §5) — same instruction count, a
    /// different immediate, so the no-op hot path stays byte-identical.
    pub const ON_ADD_ANY: u16 = Self::ON_ADD_HOOK | Self::ON_ADD_OBSERVER;
    /// `on_insert` gate mask (hook OR observer); see [`Self::ON_ADD_ANY`].
    pub const ON_INSERT_ANY: u16 = Self::ON_INSERT_HOOK | Self::ON_INSERT_OBSERVER;
    /// `on_replace` gate mask (hook OR observer); see [`Self::ON_ADD_ANY`].
    pub const ON_REPLACE_ANY: u16 = Self::ON_REPLACE_HOOK | Self::ON_REPLACE_OBSERVER;
    /// `on_remove` gate mask (hook OR observer); see [`Self::ON_ADD_ANY`].
    pub const ON_REMOVE_ANY: u16 = Self::ON_REMOVE_HOOK | Self::ON_REMOVE_OBSERVER;

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

    /// Clears `bit` from the set, leaving every other bit untouched (Phase
    /// 14b).
    ///
    /// Used by the `remove_observer` remove-last recompute to drop an
    /// `ON_{kind}_OBSERVER` bit while preserving the hook bit and the other
    /// kinds' bits.
    #[inline]
    pub fn clear(&mut self, bit: u16) {
        self.0 &= !bit;
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

    /// ORs every set bit of `other` into `self` (Phase 14b).
    ///
    /// Used by the `create_archetype` / `add_existing_archetype` seed step to
    /// merge the observer bits computed from the registry into a freshly-built
    /// archetype's flags WITHOUT disturbing the hook bits the slab recipe
    /// already seeded.
    #[inline]
    pub fn insert_observer_bits(&mut self, other: ArchetypeFlags) {
        self.0 |= other.0;
    }

    /// ORs into `self` the `ON_*_OBSERVER` bits the registry has registered for
    /// component `cid` (Phase 14b).
    ///
    /// A no-op when the registry has no lists allocated (no observer anywhere —
    /// the common case, one `Option::is_none()` early-out per kind) or none for
    /// `cid`. Called per component in the archetype-construction seed step
    /// (§4); symmetric to [`Self::insert_from_hooks`] but reads the per-world
    /// [`ObserverRegistry`] instead of the global `HOOKS` table.
    #[inline]
    pub(crate) fn insert_from_observers(&mut self, cid: ComponentId, reg: &ObserverRegistry) {
        if reg.has_observer(ObserverKind::Add, cid) {
            self.insert(Self::ON_ADD_OBSERVER);
        }
        if reg.has_observer(ObserverKind::Insert, cid) {
            self.insert(Self::ON_INSERT_OBSERVER);
        }
        if reg.has_observer(ObserverKind::Replace, cid) {
            self.insert(Self::ON_REPLACE_OBSERVER);
        }
        if reg.has_observer(ObserverKind::Remove, cid) {
            self.insert(Self::ON_REMOVE_OBSERVER);
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
        let add_only = ComponentHooks {
            on_add: Some(dummy as super::super::HookFn),
            ..ComponentHooks::default()
        };
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
        let insert_only = ComponentHooks {
            on_insert: Some(dummy as super::super::HookFn),
            ..ComponentHooks::default()
        };
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
        // gate size (plan §3.3 / O3). Bit 8 (the highest observer bit) fits.
        assert_eq!(std::mem::size_of::<ArchetypeFlags>(), 2);
    }

    // ----- Phase 14b: observer bits -----

    #[test]
    fn observer_bits_are_distinct_powers_of_two() {
        // Forward-compat guard: observer bits stay 1<<5..1<<8 (bit 4 RESERVED
        // for a future on_despawn hook; do NOT renumber).
        assert_eq!(ArchetypeFlags::ON_ADD_OBSERVER, 1 << 5);
        assert_eq!(ArchetypeFlags::ON_INSERT_OBSERVER, 1 << 6);
        assert_eq!(ArchetypeFlags::ON_REPLACE_OBSERVER, 1 << 7);
        assert_eq!(ArchetypeFlags::ON_REMOVE_OBSERVER, 1 << 8);
    }

    #[test]
    fn observer_bits_do_not_overlap_hook_bits_or_reserved_bit_four() {
        let hook_bits = ArchetypeFlags::ON_ADD_HOOK
            | ArchetypeFlags::ON_INSERT_HOOK
            | ArchetypeFlags::ON_REPLACE_HOOK
            | ArchetypeFlags::ON_REMOVE_HOOK;
        let observer_bits = ArchetypeFlags::ON_ADD_OBSERVER
            | ArchetypeFlags::ON_INSERT_OBSERVER
            | ArchetypeFlags::ON_REPLACE_OBSERVER
            | ArchetypeFlags::ON_REMOVE_OBSERVER;
        assert_eq!(hook_bits & observer_bits, 0, "hook and observer bits are disjoint");
        assert_eq!(observer_bits & (1 << 4), 0, "bit 4 stays reserved (unused by observers)");
    }

    #[test]
    fn any_masks_are_the_hook_or_observer_union() {
        assert_eq!(
            ArchetypeFlags::ON_ADD_ANY,
            ArchetypeFlags::ON_ADD_HOOK | ArchetypeFlags::ON_ADD_OBSERVER
        );
        assert_eq!(
            ArchetypeFlags::ON_INSERT_ANY,
            ArchetypeFlags::ON_INSERT_HOOK | ArchetypeFlags::ON_INSERT_OBSERVER
        );
        assert_eq!(
            ArchetypeFlags::ON_REPLACE_ANY,
            ArchetypeFlags::ON_REPLACE_HOOK | ArchetypeFlags::ON_REPLACE_OBSERVER
        );
        assert_eq!(
            ArchetypeFlags::ON_REMOVE_ANY,
            ArchetypeFlags::ON_REMOVE_HOOK | ArchetypeFlags::ON_REMOVE_OBSERVER
        );
    }

    #[test]
    fn insert_observer_bits_ors_without_disturbing_hook_bits() {
        let mut f = ArchetypeFlags::empty();
        f.insert(ArchetypeFlags::ON_ADD_HOOK); // pretend the slab recipe seeded a hook bit
        let mut obs = ArchetypeFlags::empty();
        obs.insert(ArchetypeFlags::ON_REMOVE_OBSERVER);
        f.insert_observer_bits(obs);
        assert!(f.contains(ArchetypeFlags::ON_ADD_HOOK), "hook bit preserved");
        assert!(f.contains(ArchetypeFlags::ON_REMOVE_OBSERVER), "observer bit merged in");
        assert!(!f.contains(ArchetypeFlags::ON_ADD_OBSERVER), "unrelated observer bit stays clear");
    }

    #[test]
    fn insert_from_observers_reads_the_registry() {
        use crate::ecs::core::component::observers::{ObserverContext, ObserverFn, ObserverKind, ObserverRegistry};
        use crate::ecs::identifiers::primitives::ComponentId;

        unsafe fn dummy(
            _w: crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster<'_>,
            _c: ObserverContext,
        ) {
        }
        let runner = dummy as ObserverFn;

        let mut reg = ObserverRegistry::new();
        // Empty registry: no bit raised (the zero-cost early-out path).
        let mut f = ArchetypeFlags::empty();
        f.insert_from_observers(ComponentId(20), &reg);
        assert!(f.is_empty(), "no observer => no bit");

        // Register an on_add and an on_remove observer for ComponentId(20).
        reg.add(ObserverKind::Add, ComponentId(20), runner);
        reg.add(ObserverKind::Remove, ComponentId(20), runner);
        let mut g = ArchetypeFlags::empty();
        g.insert_from_observers(ComponentId(20), &reg);
        assert!(g.contains(ArchetypeFlags::ON_ADD_OBSERVER), "on_add bit raised from registry");
        assert!(g.contains(ArchetypeFlags::ON_REMOVE_OBSERVER), "on_remove bit raised from registry");
        assert!(!g.contains(ArchetypeFlags::ON_INSERT_OBSERVER), "on_insert NOT raised");
        assert!(!g.contains(ArchetypeFlags::ON_REPLACE_OBSERVER), "on_replace NOT raised");

        // A different component is unaffected.
        let mut h = ArchetypeFlags::empty();
        h.insert_from_observers(ComponentId(21), &reg);
        assert!(h.is_empty(), "an unobserved component raises no bit");
    }
}
