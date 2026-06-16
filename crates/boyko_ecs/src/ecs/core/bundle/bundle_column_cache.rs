//! Phase 12.5 Opt-A3 — per-world `BundleTypeId` → resolved column-ids cache.
//!
//! See `docs/PHASE-12.5-SPAWN-OPTIMIZATIONS-PLAN.md` §6 for the design.
//!
//! # Why this exists
//!
//! Profiling (`docs/PHASE-12.5-PROFILE-SPAWN.md` finding #2) showed that
//! `Archetype::create_entity` performs **four SparseMap lookups per
//! component per entity** (input-mask build + `can_push` +
//! `push_entity_components` + per-row tick init). Bevy's `BundleInfo`
//! caches the per-bundle component-id → storage-index map once at
//! registration time and indexes directly thereafter; Opt-A3 ports that
//! optimisation onto the boyko spawn path.
//!
//! # Structure
//!
//! * [`BundleColumnRecord`] — per-`(BundleTypeId, ArchetypeId)` payload
//!   carrying the leaked `&'static [InlandPoolId]` (one slot per
//!   component in `B::component_ids()` order — B1/B2 canonical-sorted by
//!   `ComponentId.0`). 32 B `#[repr(C)]` for predictable layout.
//! * [`BundleColumnCache`] — boxed `[OnceLock<BundleColumnRecord>; MAX_BUNDLE_TYPES]`
//!   stored on `EcsMaster`. Eager allocation at `EcsMaster::new` keeps
//!   warm-path indexing branch-free (no `Option` unwrap, no allocation).
//!
//! # Hot path
//!
//! ```text
//! let record = world.bundle_column_cache
//!     .get_resolved::<B>()
//!     .unwrap_or_else(|| world.bundle_column_cache.resolve_and_cache::<B>(...));
//! let pool_ids = record.pool_ids;  // &'static [InlandPoolId], canonical-sorted
//! for (i, (id, bytes)) in bundle.iter() {
//!     archetype.pool_at_unchecked_mut(pool_ids[i]).write_at_unchecked_initialized(row, bytes);
//! }
//! ```
//!
//! Warm-path cost: one Acquire load on `OnceLock::get` (~2 ns) + one
//! `Box<[_]>` deref. Cold path runs once per `(B, world)` pair —
//! resolves `B::component_ids()` against `archetype.component_pools`,
//! collects `Vec<InlandPoolId>` in canonical order, leaks to `&'static`.
//!
//! # Invariants (SBO5, SBO6, SBO12, SBO-N, SBO-B2)
//!
//! * **SBO5**: cache writes happen under `&mut EcsMaster` (apply path
//!   only). Readers observe either `None` (cold) or a fully-published
//!   `BundleColumnRecord` via `OnceLock::get` Acquire ordering.
//! * **SBO6**: `&'static [InlandPoolId]` slices are leaked exactly once
//!   per `(BundleTypeId, ArchetypeId)`. Bounded by
//!   `MAX_BUNDLE_TYPES × MAX_BUNDLE_ARITY × 4 B = 32 KB` per world.
//! * **SBO12 (v1 binding)**: cache slot for `(B, A)` is valid for the
//!   world's lifetime. No archetype destruction in v1.
//! * **SBO-N (I-N3 detection-only)**: `ComponentPoolBundle::pools` Vec is
//!   push-only; `pools_len_at_install` snapshot allows the warm path to
//!   `debug_assert!` non-decrease.
//! * **SBO-B2**: `pool_ids` is in canonical-sorted `ComponentId.0` order,
//!   matching `B::component_ids()`. `debug_assert!(pool_ids.is_sorted_by_key(...))`
//!   runs at install time AND once per batch at apply time (W4 hoist).

#![allow(dead_code)]

use std::sync::OnceLock;

use static_assertions::assert_impl_all;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::bundle::bundle::Bundle;
use crate::ecs::core::bundle::bundle_type_registry::MAX_BUNDLE_TYPES;
use crate::ecs::core::component::component_registry::{self, RequiredEntry};
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, InlandPoolId};

/// Phase 12.5 Opt-A3 (§6.2 / §8.3): per-`(BundleTypeId, ArchetypeId)`
/// resolved column-ids record.
///
/// # Layout (64 B — deliberate 32→64 B trade-off)
///
/// Feature 1 (required components) grew this record from the original 32 B
/// (`archetype_id` + `pool_ids` + the two `u32`s) to **64 B = one full cache
/// line** by adding the two fat slices `required_missing` + `required_pool_ids`.
/// The trade-off is intentional: for a require-free bundle both new slices are
/// the empty `&'static []` (a null-len fat pointer, zero leaked bytes), so the
/// only cost is +32 B of zeroed inline storage per record. In exchange the
/// constructor pass reads the required plan as TWO plain indexed loads off the
/// SAME already-hot record — no extra pointer indirection, no second cache
/// lookup, no `OnceLock` chase. One-line-per-record keeps the warm spawn path
/// branch-free; padding the record to exactly one cache line (the `const _:`
/// assert below) also rules out a record straddling two lines.
///
/// `#[repr(C)]` pins the field order. Loaded once per batch at the top of
/// `SpawnBatchCommand::apply` and indexed inline thereafter.
///
/// ```text
/// +0  : archetype_id: ArchetypeId               (8 B)
/// +8  : pool_ids: &'static [InlandPoolId]        (16 B fat pointer)
/// +24 : required_missing: &'static [RequiredEntry] (16 B fat pointer)
/// +40 : required_pool_ids: &'static [InlandPoolId]  (16 B fat pointer)
/// +56 : pools_len_at_install: u32               (4 B)
/// +60 : _pad: u32                               (4 B)
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BundleColumnRecord {
    /// Resolved archetype id for the `(B, world)` pair this record
    /// represents. Matches `B::cached_archetype_id(world)` at install
    /// time; stable for the world's lifetime per SBO12.
    pub archetype_id: ArchetypeId,

    /// Canonical-sorted `&'static` slice of `InlandPoolId`s — one entry
    /// per component in `B::component_ids()` order (B1/B2). Leaked
    /// exactly once per `(BundleTypeId, ArchetypeId)` per world (SBO6).
    pub pool_ids: &'static [InlandPoolId],

    /// Required components (Feature 1, D4): the transitive required entries
    /// (ctor + id) that `B` does NOT supply directly and must be constructed
    /// at spawn/insert (`B::component_ids()` is the "supplied" set, present⇒skip).
    /// **Empty `&'static []` for a require-free bundle** — the load-bearing
    /// apply-time 0%-gate (an empty-slice check; the constructor pass runs zero
    /// iterations). Leaked once per `(BundleTypeId, ArchetypeId)` per world.
    pub required_missing: &'static [RequiredEntry],

    /// Required components (Feature 1, D4): the resolved `InlandPoolId` of each
    /// entry in [`Self::required_missing`], in the SAME order. The constructor
    /// pass indexes `required_pool_ids[i]` to find the column for
    /// `required_missing[i]`. Empty for a require-free bundle.
    pub required_pool_ids: &'static [InlandPoolId],

    /// SBO-N snapshot: `archetype.component_pools.pools_len()` at the
    /// moment this record was installed. The warm-path apply
    /// `debug_assert!`s `pools_len_at_install <= pools_len()` (push-only
    /// invariant) — detection-only per I-N3; v1 has no
    /// archetype-destruction path.
    pub pools_len_at_install: u32,

    /// Padding to round the struct up to 64 B. Reserved for future use
    /// (e.g. an explicit `flags` field).
    pub _pad: u32,
}

// SAFETY (SBC2 / SBO5):
//   - `ArchetypeId` is `#[repr(transparent)]` over `usize`; integers are
//     trivially `Send + Sync`.
//   - `&'static [InlandPoolId]` is an immutable shared slice into leaked
//     static memory; aliased reads from many threads are sound.
//   - The two `u32` fields are POD.
unsafe impl Send for BundleColumnRecord {}
// SAFETY: same composition as `Send`.
unsafe impl Sync for BundleColumnRecord {}

// `BundleColumnRecord` holds an `ArchetypeId` (wraps `usize`), three
// pointer-width fat slices, and two `u32`s, so the 64-byte size encodes the
// 64-bit ABI (one cache line). Gated to 64-bit (the engine's supported
// platform) — see CLAUDE.md target platform.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<BundleColumnRecord>() == 64);

/// Phase 12.5 Opt-A3 (§6.2): per-world cache of resolved
/// `(BundleTypeId, ArchetypeId, &'static [InlandPoolId])` records.
///
/// # Memory footprint
///
/// `Box<[OnceLock<BundleColumnRecord>; MAX_BUNDLE_TYPES]>`. With
/// `MAX_BUNDLE_TYPES = 1024` and `size_of::<OnceLock<BundleColumnRecord>>()`
/// at most ~48 B (the OnceLock wraps a 32-B payload plus a 4-B state
/// word plus padding), the cache occupies ≤ 48 KB per world. Eagerly
/// allocated at `EcsMaster::new` to keep the warm-path branch-free (no
/// `Option` unwrap of a `OnceLock<Box<...>>` wrapper).
///
/// # Concurrency
///
/// Slots are populated under `&mut EcsMaster` (apply path only — SBO5).
/// Cold-path racers would observe `Err` from `OnceLock::set` and read
/// back the winner's value; in v1 there is no Phase-9-scheduler
/// concurrency on `&mut EcsMaster`, so racing is theoretical.
pub struct BundleColumnCache {
    slots: Box<[OnceLock<BundleColumnRecord>]>,
}

assert_impl_all!(BundleColumnCache: Send, Sync);
assert_impl_all!(BundleColumnRecord: Send, Sync);

impl BundleColumnCache {
    /// Phase 12.5 Opt-A3 (I3): eagerly allocates the per-world slot array.
    ///
    /// Called once from `EcsMaster::new` / `EcsMaster::with_capacity`.
    /// Every slot starts in the `None` state; `resolve_and_cache::<B>`
    /// installs a record on the first warm-up per bundle type.
    pub fn new() -> Self {
        let slots: Box<[OnceLock<BundleColumnRecord>]> = (0..MAX_BUNDLE_TYPES)
            .map(|_| OnceLock::new())
            .collect();
        Self { slots }
    }

    /// Phase 12.5 Opt-A3 (§6.2 hot path): returns the cached
    /// `&BundleColumnRecord` for bundle type `B`, or `None` if no
    /// resolve has happened yet for this world.
    ///
    /// Cost: one Acquire load on `OnceLock::get` (~2 ns) + one boxed
    /// slice deref. The `B::bundle_type_id()` lookup itself hits another
    /// Acquire load on the per-impl `OnceLock<BundleStaticInfo>` (~2 ns,
    /// shared across all worlds for the same `B`).
    #[inline]
    pub fn get_resolved<B: Bundle>(&self) -> Option<&BundleColumnRecord> {
        let bundle_type_id = B::bundle_type_id();
        debug_assert!(
            bundle_type_id.0 < MAX_BUNDLE_TYPES,
            "BundleTypeId out of bounds — saturation guard should have prevented this"
        );
        debug_assert_eq!(
            self.slots.len(),
            MAX_BUNDLE_TYPES,
            "BundleColumnCache::new must allocate exactly MAX_BUNDLE_TYPES slots"
        );
        // SAFETY (Phase 12.5 P4):
        //   * `bundle_type_id.0 < MAX_BUNDLE_TYPES` is enforced by
        //     `bundle_type_registry::register_new`: the counter saturates
        //     at `MAX_BUNDLE_TYPES` and panics before returning an
        //     out-of-range value. By the time any `B::bundle_type_id()`
        //     load can complete successfully, the id is in [0, MAX_BUNDLE_TYPES).
        //   * `self.slots.len() == MAX_BUNDLE_TYPES` is established at
        //     `BundleColumnCache::new` time and is invariant (the Box is
        //     never re-allocated). Debug-asserted above.
        //   * Therefore `bundle_type_id.0 < self.slots.len()` and
        //     `get_unchecked` is in-bounds.
        let slot = unsafe { self.slots.get_unchecked(bundle_type_id.0) };
        slot.get()
    }

    /// Phase 12.5 Opt-A3 (§6.2 cold path): resolves the per-component
    /// `InlandPoolId`s for `B` against the supplied archetype, leaks the
    /// canonical-sorted result to `&'static`, and CAS-installs the
    /// record into this world's cache slot for `B::bundle_type_id()`.
    ///
    /// Returns the **published** `&BundleColumnRecord` (either ours if
    /// we won the race, or the racer's if we lost). The leaked slice
    /// from the loser is dropped on `OnceLock::set`'s `Err` return —
    /// since we leaked it via `Box::leak`, the bytes stay alive (memory
    /// leak by design; bounded by SBO6).
    ///
    /// # Cost
    ///
    /// Runs once per `(BundleTypeId, world)` pair across the world's
    /// lifetime — `~250 ns` worst case (Vec alloc plus N SparseMap
    /// lookups plus leak plus OnceLock CAS). `#[cold]` +
    /// `#[inline(never)]` to keep the warm path's instruction cache
    /// tight.
    #[cold]
    #[inline(never)]
    pub fn resolve_and_cache<B: Bundle>(
        &self,
        archetype_id: ArchetypeId,
        archetype: &Archetype,
    ) -> &BundleColumnRecord {
        let bundle_type_id = B::bundle_type_id();
        debug_assert!(
            bundle_type_id.0 < MAX_BUNDLE_TYPES,
            "BundleTypeId out of bounds"
        );

        let component_ids = B::component_ids();

        // Resolve every ComponentId to its InlandPoolId via the bundle's
        // SparseMap. Canonical order is preserved because B1 guarantees
        // `B::component_ids()` is already sorted by `ComponentId.0`.
        let mut pool_ids_owned: Vec<InlandPoolId> = Vec::with_capacity(component_ids.len());
        for &cid in component_ids {
            let inland = archetype
                .component_pools()
                .pool_id_for(cid)
                .expect(
                    "invariant: B::cached_archetype_id returned an archetype that hosts \
                     every component in B::component_ids() (Bundle / ArchetypeMaster \
                     registration contract)",
                );
            pool_ids_owned.push(inland);
        }

        // SBO-B2 install-time canonical-order assertion (debug only).
        debug_assert!(
            component_ids.is_sorted_by_key(|id| id.0),
            "B1 violation: B::component_ids() must be sorted by ComponentId.0"
        );

        // SBO-N snapshot: capture `pools_len()` at install so the warm
        // path can `debug_assert!` non-decrease. v1 has no archetype-
        // destruction path, so this is detection-only (I-N3).
        let pools_len_at_install = archetype.component_pools().pools_len() as u32;

        // Leak the canonical-sorted slice to `&'static`. Bounded by SBO6
        // (one slice per (BundleTypeId, ArchetypeId) per world; ≤ 32 KB
        // worst case per world at MAX_BUNDLE_TYPES * MAX_BUNDLE_ARITY * 4 B).
        let pool_ids_boxed: Box<[InlandPoolId]> = pool_ids_owned.into_boxed_slice();
        let pool_ids: &'static [InlandPoolId] = Box::leak(pool_ids_boxed);

        // Required components (Feature 1, D4 / Step 5): compute the entries the
        // bundle does NOT supply directly (`B::component_ids()` is the supplied
        // set, present⇒skip) and resolve each to its `InlandPoolId` in the
        // resolved archetype. Empty `&'static []` for a require-free bundle — the
        // apply-time 0%-gate. Leaked once per `(BundleTypeId, ArchetypeId)`.
        let (required_missing, required_pool_ids) =
            Self::resolve_required_missing(component_ids, archetype);

        let record = BundleColumnRecord {
            archetype_id,
            pool_ids,
            required_missing,
            required_pool_ids,
            pools_len_at_install,
            _pad: 0,
        };

        // Install. In v1, no racer exists — the `set` always succeeds
        // because the cache is touched only under `&mut EcsMaster`
        // (single-dispatcher per SBO5). The `Err` arm is defensive
        // coverage in case future code allows concurrent first-access
        // (Phase 13). If a racer ever wins, the rejected record's leaked
        // `&'static` slice stays alive (bounded by SBO6) and the winner's
        // value is read back below.
        let slot = self
            .slots
            .get(bundle_type_id.0)
            .expect("invariant: bundle_type_id.0 < MAX_BUNDLE_TYPES");
        let _ = slot.set(record);
        slot.get()
            .expect("invariant: OnceLock populated by self or racer in cold path")
    }

    /// Required components (Feature 1, D4 / Step 5): computes the transitive
    /// required entries the bundle does NOT supply directly (present⇒skip
    /// against `supplied_ids = B::component_ids()`) and resolves each to its
    /// `InlandPoolId` in the resolved archetype.
    ///
    /// Returns `(required_missing, required_pool_ids)` — two parallel leaked
    /// `&'static` slices in the SAME order. Both are empty `&'static []` for a
    /// require-free bundle (the apply-time 0%-gate). The archetype hosts every
    /// required id by construction (the expansion union ran at
    /// `cold_register_bundle_archetype` / `merged_archetype_id` BEFORE the
    /// archetype was resolved), so `pool_id_for` always succeeds.
    ///
    /// Cold path only — runs once per `(BundleTypeId, ArchetypeId)` per world.
    fn resolve_required_missing(
        supplied_ids: &[ComponentId],
        archetype: &Archetype,
    ) -> (&'static [RequiredEntry], &'static [InlandPoolId]) {
        // 0%-gate: a require-free bundle leaks nothing and returns empty slices.
        if !component_registry::any_requires(supplied_ids) {
            return (&[], &[]);
        }

        // Build the missing set: each transitively-required id paired with the
        // ctor the closure resolved for it (W1 conflict rule already applied).
        // present⇒skip is enforced by `for_each_required_id_excluding`, which
        // never emits an id already in `supplied_ids`.
        let mut missing: Vec<RequiredEntry> = Vec::new();
        let mut missing_pools: Vec<InlandPoolId> = Vec::new();
        for &supplied in supplied_ids {
            for &entry in component_registry::get_required_plan(supplied.0).entries {
                // Skip ids the bundle supplies directly (present⇒skip) and ids
                // already collected (diamond dedup).
                if supplied_ids.contains(&entry.component_id)
                    || missing.iter().any(|e| e.component_id == entry.component_id)
                {
                    continue;
                }
                let inland = archetype
                    .component_pools()
                    .pool_id_for(entry.component_id)
                    .expect(
                        "invariant: the archetype was expanded with every required id at \
                         cold_register_bundle_archetype / merged_archetype_id, so it hosts \
                         every transitively-required component",
                    );
                missing.push(entry);
                missing_pools.push(inland);
            }
        }

        // Leak both parallel slices to `&'static` (bounded by SBO6-class: one
        // pair per (BundleTypeId, ArchetypeId) per world).
        let required_missing: &'static [RequiredEntry] = Box::leak(missing.into_boxed_slice());
        let required_pool_ids: &'static [InlandPoolId] =
            Box::leak(missing_pools.into_boxed_slice());
        (required_missing, required_pool_ids)
    }
}

impl Default for BundleColumnCache {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_column_record_layout() {
        // Locked in by `const _: () = assert!(size_of::<...>() == 64)` at
        // module scope. Repeat here as a human-visible smoke check.
        assert_eq!(std::mem::size_of::<BundleColumnRecord>(), 64);
    }

    #[test]
    fn cache_new_creates_max_bundle_types_slots() {
        let cache = BundleColumnCache::new();
        assert_eq!(cache.slots.len(), MAX_BUNDLE_TYPES);
        // Every slot starts uninitialised.
        for slot in cache.slots.iter() {
            assert!(slot.get().is_none());
        }
    }
}
