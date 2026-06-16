//! `DispatchKey` — the unified per-observer key (Feature 2 D2).
//!
//! An entity-targeted observer (see [`entity_store`]) fires for EITHER a
//! lifecycle [`ObserverKind`] (`on_add`..`on_despawn`) OR a user-defined custom
//! trigger (keyed by a dense `TriggerId`). One packed `u32` carries both so the
//! per-entity observer list, the custom-trigger walk, and the entity-fire loop
//! share a single key type and a single linear scan.
//!
//! [`entity_store`]: crate::ecs::core::component::observers::entity_store

use crate::ecs::core::component::observers::ObserverKind;

/// What an entity-targeted observer listens for.
///
/// Packs the lifecycle-kind OR a dense custom-trigger id into one `u32`:
///
/// * high bit clear ⇒ the low bits are an [`ObserverKind`] discriminant
///   (`0..=4`);
/// * high bit set ⇒ the low 31 bits are a custom-trigger id (`TriggerId`,
///   minted dense by the trigger registry — far below `2^31`).
///
/// POD, `Copy` — fits the fire-loop copy-out discipline (the entity-fire loop
/// copies the key + entry out by value before minting the view).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub(crate) struct DispatchKey {
    raw: u32,
}

// FIX O2: the entire trigger-id space (`0..MAX_TRIGGERS`) must fit below the
// custom-flag high bit, so packing a `TriggerId` via `DispatchKey::custom` can
// never overflow into `CUSTOM_FLAG`. Compile-time guard against a future
// `MAX_TRIGGERS` bump silently colliding with the flag.
const _: () = assert!(
    crate::ecs::core::component::observers::trigger::MAX_TRIGGERS <= DispatchKey::CUSTOM_FLAG as usize,
    "MAX_TRIGGERS must fit below DispatchKey::CUSTOM_FLAG (the trigger id high bit)"
);

impl DispatchKey {
    /// High bit: set ⇒ custom trigger, clear ⇒ lifecycle kind.
    const CUSTOM_FLAG: u32 = 1 << 31;

    /// Builds a key for a lifecycle observer kind.
    #[inline]
    pub(crate) const fn lifecycle(kind: ObserverKind) -> Self {
        Self { raw: kind as u32 }
    }

    /// Builds a key for a custom trigger by its dense id.
    ///
    /// `trigger_id` must be `< 2^31` (the registry mints far below this — the
    /// dispatch id space is tiny). Debug-asserted.
    #[inline]
    pub(crate) const fn custom(trigger_id: u32) -> Self {
        debug_assert!(trigger_id < Self::CUSTOM_FLAG, "TriggerId overflows 31 bits");
        Self { raw: Self::CUSTOM_FLAG | trigger_id }
    }

    /// `true` iff this key is a custom trigger (high bit set), `false` for a
    /// lifecycle kind. Selects which runner variant an entity observer stores.
    #[inline]
    pub(crate) const fn is_custom(self) -> bool {
        self.raw & Self::CUSTOM_FLAG != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_keys_are_distinct() {
        let add = DispatchKey::lifecycle(ObserverKind::Add);
        let despawn = DispatchKey::lifecycle(ObserverKind::Despawn);
        assert_ne!(add, despawn);
    }

    #[test]
    fn custom_keys_are_disjoint_from_each_other_and_lifecycle() {
        let c0 = DispatchKey::custom(0);
        let c7 = DispatchKey::custom(7);
        assert_ne!(c0, c7);
        // A custom id of 0 must NOT collide with `ObserverKind::Add` (raw 0):
        // the high custom-flag bit keeps the spaces disjoint.
        assert_ne!(c0, DispatchKey::lifecycle(ObserverKind::Add));
        assert_ne!(c7, DispatchKey::lifecycle(ObserverKind::Despawn));
    }

    #[test]
    fn dispatch_key_is_four_bytes_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<DispatchKey>();
        assert_eq!(core::mem::size_of::<DispatchKey>(), 4);
    }
}
