//! Dynamic-tag & enable-tag sub-registry (Phase 22 D3 / EnableTag plan D5).
//!
//! Split out of the former single-file `component_registry` (pure mechanical
//! move — every item keeps its exact `component_registry::…` path via the
//! `pub use tags::*` re-export in the parent `mod.rs`). Holds the filter-only
//! [`TagId`] / [`EnableTagId`] handles and the name-keyed dynamic-tag /
//! enable-tag mint (interned in the process-global `TAG_NAMES` table). Reaches
//! into the core registry (parent module) for the shared `NEXT_ID` counter, the
//! `try_register_dynamic` mint, `ComponentLayout::new_dynamic_tag`, and
//! `set_storage_kind`.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

use crate::ecs::identifiers::primitives::ComponentId;

use super::{
    ComponentLayout, MAX_COMPONENTS, NEXT_ID, StorageKind, set_storage_kind, try_register_dynamic,
};

/// Filter-only dynamic-tag handle (Phase 22 D3). `repr(transparent)` over
/// [`ComponentId`]: zero-cost, but type-distinct so data-fetch APIs cannot
/// accept it (a dynamic tag has no data by definition).
///
/// # One-way bridge (W3)
///
/// `TagId → ComponentId` is public — [`TagId::component_id`] and the
/// `From<TagId> for ComponentId` impl — because the id-keyed surfaces a
/// dynamic tag needs downstream ([`register_hooks_by_id`],
/// `EcsMaster::add_observer`) take [`ComponentId`]. The reverse direction has
/// NO constructor: a `TagId` is a proof that the id was minted as a size-0
/// dynamic tag, and only the `TAG_NAMES` mint path can issue one.
///
/// # Identity stability
///
/// Like every [`ComponentId`], the numeric value is first-call-order
/// process-unstable; the **name** is the stable serialization key
/// (`tag_by_name`).
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TagId(pub(crate) ComponentId);

impl TagId {
    /// Bridges to the shared [`ComponentId`] space (Phase 22 W3) — the
    /// vocabulary of the id-keyed hook/observer surfaces
    /// ([`register_hooks_by_id`], `EcsMaster::add_observer`).
    #[inline]
    pub const fn component_id(self) -> ComponentId {
        self.0
    }
}

impl From<TagId> for ComponentId {
    #[inline]
    fn from(tag: TagId) -> Self {
        tag.component_id()
    }
}

/// Filter-only enable-tag handle (EnableTag plan, D5). `repr(transparent)` over
/// [`ComponentId`]: zero-cost, but type-distinct so data-fetch APIs cannot
/// accept it (an enable tag has no `ComponentPool` by construction — D6).
///
/// An enable tag uses the **bitset** storage backend ([`StorageKind::Bitset`]):
/// its id is filtered out of every archetype signature and toggled with a
/// single per-row bit instead of triggering a migration. It is minted via
/// `try_register_enable_tag_by_name`, which sets the id's
/// `STORAGE_KIND` to [`StorageKind::Bitset`].
///
/// # One-way bridge
///
/// `EnableTagId → ComponentId` is public — [`EnableTagId::component_id`] and the
/// `From<EnableTagId> for ComponentId` impl — because the id-keyed enable
/// surfaces (`enable_id` / `disable_id` / `is_enabled_id`, `with_enabled`) take
/// a [`ComponentId`]. The reverse direction has NO constructor: an
/// `EnableTagId` is a proof that the id was minted as a bitset enable tag, and
/// only the mint path can issue one.
///
/// # Identity stability
///
/// Like every [`ComponentId`], the numeric value is first-call-order
/// process-unstable; the **name** is the stable serialization key.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EnableTagId(pub(crate) ComponentId);

impl EnableTagId {
    /// Bridges to the shared [`ComponentId`] space — the vocabulary of the
    /// id-keyed enable/disable surfaces and the `with_enabled` query terms.
    #[inline]
    pub const fn component_id(self) -> ComponentId {
        self.0
    }
}

impl From<EnableTagId> for ComponentId {
    #[inline]
    fn from(tag: EnableTagId) -> Self {
        tag.component_id()
    }
}

/// Name-keyed enable-tag mint (EnableTag plan, D5). Mirrors
/// [`try_register_tag_by_name`] — idempotent per name, returns `None` when the
/// shared `MAX_COMPONENTS` budget is exhausted and `name` was never minted —
/// but additionally classifies the minted id as [`StorageKind::Bitset`] so it
/// is filtered out of every archetype signature.
///
/// A name already minted as a *non-bitset* dynamic tag is returned as-is in
/// debug only after the same-kind reclassification assertion fires (storage
/// kind is write-once); the two name spaces are otherwise independent
/// (`tag_by_name` vs the enable-tag name).
///
/// Lookup, capacity check, mint, and name-intern run under the shared
/// name-intern lock (inside [`try_register_tag_by_name`]), so two threads racing
/// the same name cannot double-mint. The kind classification
/// ([`set_storage_kind`]) runs AFTER that lock is released — this is sound
/// because storage kind is write-once-idempotent and registration completes
/// before any archetype can read the kind (the same registration-before-
/// construction ordering the `STORAGE_KIND` table relies on). See
/// [`try_register_tag_by_name`] for the leak bound (identical; no extra alloc).
// Step 5 (Wave 2) wired the live caller: `EcsMaster::register_enable_tag` /
// `try_register_enable_tag` delegate here, mirroring how `EcsMaster::register_tag`
// delegates to `try_register_tag_by_name`. The Step-1 forward-seam
// `#[allow(dead_code)]` is therefore removed.
pub(crate) fn try_register_enable_tag_by_name(name: &str) -> Option<EnableTagId> {
    let tag = try_register_tag_by_name(name)?;
    // Classify the minted id as bitset storage. Write-once: a re-mint of the
    // same name re-runs this with the already-bitset id, which is the idempotent
    // same-kind no-op above.
    set_storage_kind(tag.0.0, StorageKind::Bitset);
    Some(EnableTagId(tag.0))
}

/// Phase 22 (D3): process-global name → id intern for dynamic tags. COLD:
/// touched at mint/lookup only (setup time) — never on the per-frame hot path.
///
/// `Mutex + HashMap` is justified per the Phase-12.5 `QueryTypeId`-intern
/// precedent; one concrete global (not a generic-fn-body static) avoids the
/// monomorphization-collapse trap. Capacity + idempotency are atomic under
/// this lock; idempotency is NAME-keyed, never TypeId-keyed (all dynamic tags
/// share [`DynamicTagMarker`]'s TypeId — plan O2). Names are leaked once per
/// successfully minted unique tag (bounded ≤ [`MAX_COMPONENTS`], the #53
/// bounded-leak precedent).
static TAG_NAMES: OnceLock<Mutex<HashMap<Box<str>, TagId>>> = OnceLock::new();

/// Lazily initializes and returns the [`TAG_NAMES`] intern table.
fn tag_names() -> &'static Mutex<HashMap<Box<str>, TagId>> {
    TAG_NAMES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Name-keyed dynamic-tag mint (Phase 22 D3 mint protocol). Idempotent per
/// name: a present name returns its existing [`TagId`] (success), even after
/// the budget is exhausted. `None` means the shared `MAX_COMPONENTS` budget is
/// exhausted and `name` was never minted.
///
/// The whole decision — lookup, capacity check, mint, intern — runs under the
/// [`TAG_NAMES`] lock, so two threads racing the same name cannot double-mint.
///
/// # Leak bound
///
/// The name is leaked (`Box::leak`) only on the mint path, after the
/// early-return budget check. One residual window exists: a concurrent typed
/// [`register_new`] (whose `fetch_add` is not under this lock) can claim the
/// last slot between our budget peek and the CAS in [`try_register_dynamic`],
/// leaking one name copy. Past exhaustion `NEXT_ID` never decreases, so every
/// later failing call returns at the peek, allocation-free — the leak is
/// bounded by the number of distinct names racing the exact exhaustion
/// boundary, in practice zero.
pub(crate) fn try_register_tag_by_name(name: &str) -> Option<TagId> {
    let mut names = tag_names()
        .lock()
        .expect("invariant: TAG_NAMES lock poisoned only after a registry-invariant panic (dynamic_slot_occupied_panic under the guard) — process already condemned");
    if let Some(&tag) = names.get(name) {
        return Some(tag);
    }
    if NEXT_ID.load(Ordering::Relaxed) >= MAX_COMPONENTS {
        return None;
    }
    let leaked: &'static str = Box::leak(Box::<str>::from(name));
    let id = try_register_dynamic(ComponentLayout::new_dynamic_tag(leaked))?;
    let tag = TagId(id);
    names.insert(Box::from(name), tag);
    Some(tag)
}

/// Cold name → [`TagId`] lookup (Phase 22 D3). `None` if `name` was never
/// successfully minted. Never mints.
pub(crate) fn tag_by_name(name: &str) -> Option<TagId> {
    tag_names()
        .lock()
        .expect("invariant: TAG_NAMES lock poisoned only after a registry-invariant panic (dynamic_slot_occupied_panic under the guard) — process already condemned")
        .get(name)
        .copied()
}
