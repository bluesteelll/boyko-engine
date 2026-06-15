use crate::ecs::core::archetype::archetype_bundle::{ArchetypeBundle, ArchetypeBundleIterMut};
use crate::ecs::core::archetype::archetype_registry::ArchetypeRegistry;
use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::archetype::generation::ArchetypeGeneration;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::component::component_registry;
use crate::ecs::core::component::enable::enable_presence::EnablePresence;
use crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use crate::ecs::core::component::observers::{
    ObserverFn, ObserverId, ObserverKind, ObserverRegistry,
};
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use crate::ecs::core::iters::legacy_query::Query as LegacyQuery;

use core::sync::atomic::{AtomicU64, Ordering};

/// Master manager for archetypes, providing creation and lookup capabilities
/// Integrates ArchetypeBundle for storage and ArchetypeRegistry for efficient queries
pub struct ArchetypeMaster {
    /// Storage for archetypes with direct access by ID
    archetypes: ArchetypeBundle,

    /// Registry for efficient component-based lookups
    registry: ArchetypeRegistry,

    /// Next available archetype ID
    next_archetype_id: ArchetypeId,

    /// Monotonic counter bumped on every `create_archetype` call that mints a
    /// *new* archetype slot. Never reset — even after `clear()` — so a stale
    /// `QueryState` can detect cache invalidation by comparing against the
    /// saved value.
    ///
    /// This counter signals "the set of archetypes grew; classify the deltas".
    /// It does NOT cover removals — see [`structural_generation`].
    generation: ArchetypeGeneration,

    /// Monotonic counter bumped on every `remove_archetype` call that
    /// successfully tore down an archetype, AND on every `clear()` call.
    /// Never reset.
    ///
    /// This counter signals "the set of archetypes shrank, so the cached
    /// `matched_ids` may contain dead entries that must be evicted". A
    /// `QueryState` whose `last_structural_generation` differs from the
    /// master's must do a full rebuild instead of a delta-add — otherwise an
    /// ABA scenario can produce silently-wrong query results when a freshly
    /// minted archetype reuses a recycled `ArchetypeId` (e.g. after `clear()`
    /// resets `next_archetype_id` to 1).
    ///
    /// Separated from [`generation`] so the common "creates only, no removes"
    /// fast path keeps its ~21x warm-path speedup via delta-add. Only the
    /// rarer remove+create churn pays the full-rebuild cost.
    structural_generation: ArchetypeGeneration,

    /// Phase 14b: per-`(kind, component)` lifecycle observers. Co-located here
    /// (not on `EcsMaster`) so [`Self::create_archetype`] — the single
    /// archetype-creation funnel — can seed each new archetype's
    /// `ON_*_OBSERVER` flag bits at construction, and so
    /// [`Self::add_observer`] / [`Self::remove_observer`] can maintain those
    /// bits with one cohesive `&mut self` walk over `iter_archetypes_mut`.
    ///
    /// `pub(crate)` because the Wave-5 fire dispatch reads it cross-module
    /// (`&self.observer_registry`); the seed/walk sites are methods on
    /// `ArchetypeMaster` itself. Independently `Send + Sync` (fn-ptr-only
    /// entries) — no `unsafe impl` (SEND6).
    pub(crate) observer_registry: ObserverRegistry,

    /// EnableTag column-allocation epoch (Decision D1 sub-decision W2).
    ///
    /// Bumped exactly once per `EnableColumn` allocation (the first time a tag
    /// is toggled into an archetype). A `QueryState` that named an `Enabled`/
    /// `Disabled` term records this value when it last culled and re-checks it
    /// on the next `update`: a change means "an archetype gained an enable
    /// column" ⇒ re-run the presence cull (Decision D2 / O2). It is held
    /// **separate** from [`structural_generation`] on purpose — a structural
    /// bump force-rebuilds *every* cache (query_state delta-add path), whereas a
    /// per-toggle column-alloc must invalidate only enable-bearing caches; a
    /// full structural invalidation per toggle would be catastrophic.
    ///
    /// **W2 forward seam.** It is `AtomicU64` purely so the deferred D7
    /// worker-marking model (a `&self` toggle from a live worker) can bump it
    /// without an exclusive borrow. In v1 it is bumped only under `&mut self`
    /// (the structural/apply window) and read single-threaded in `update`, so
    /// `Relaxed` is sound *only because no concurrent access exists in v1*. When
    /// worker-marking lands (D8), the bump/read here gain real Acquire/Release
    /// plus a loom proof — do NOT loosen the v1 ordering assumption elsewhere.
    enable_generation: AtomicU64,

    /// EnableTag per-tag archetype-presence cull oracle (Decision D1 / D2).
    ///
    /// Records, for each EnableTag id, the set of THIS world's archetypes that
    /// own an allocated `EnableColumn`. The query cull (Wave 3) consults it as
    /// the O(1) `contains` oracle over the bounded matched set.
    ///
    /// **Per-WORLD, NOT process-global.** The Wave-1 module doc described it as
    /// "process-global", but it is keyed by [`ArchetypeId`], and `ArchetypeId`s
    /// are per-world (every world's `next_archetype_id` starts at 1). A
    /// process-global instance would conflate `ArchetypeId(1)` across worlds and
    /// trip `note_column_alloc`'s "genuine first column" assertion (verified by
    /// the Step-5 multi-world tests). Co-located with `enable_generation` so the
    /// two stay consistent: `note_enable_column_alloc` updates BOTH exactly once
    /// per column (Decision D1 inv 5).
    enable_presence: EnablePresence,
}

impl ArchetypeMaster {
    /// Creates a new ArchetypeMaster.
    ///
    /// (Phase X.J: the historical `arena_ptr` parameter — and the `unsafe`
    /// contract that came with it — were retired with the shared Arena;
    /// component pools own their backing reservations since Phase X.I.)
    pub fn new() -> Self {
        Self {
            archetypes: ArchetypeBundle::new(),
            registry: ArchetypeRegistry::with_capacity(64),
            next_archetype_id: ArchetypeId(1),
            generation: ArchetypeGeneration::FIRST,
            structural_generation: ArchetypeGeneration::FIRST,
            observer_registry: ObserverRegistry::new(),
            enable_generation: AtomicU64::new(0),
            enable_presence: EnablePresence::new(),
        }
    }

    /// Creates a new ArchetypeMaster with the given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            archetypes: ArchetypeBundle::with_capacity(capacity),
            registry: ArchetypeRegistry::with_capacity(capacity),
            next_archetype_id: ArchetypeId(1),
            generation: ArchetypeGeneration::FIRST,
            structural_generation: ArchetypeGeneration::FIRST,
            observer_registry: ObserverRegistry::new(),
            enable_generation: AtomicU64::new(0),
            enable_presence: EnablePresence::new(),
        }
    }


    /// Creates a new archetype from a slice of component IDs
    /// Returns the ID of the created archetype
    pub fn create_archetype(&mut self, component_ids: &[ComponentId]) -> ArchetypeId {
        // First check if an archetype with exactly these components already
        // exists. EnableTag plan C1 premise (Decision D5): the registry-index
        // signature mask MUST filter out every `StorageKind::Bitset` id, exactly
        // as `Archetype::create_by_ids` does for the archetype's own signature.
        // Using the raw `ComponentMask::from_components` here would register the
        // archetype under a signature that INCLUDES the bitset bit while the
        // archetype's real signature EXCLUDES it — defeating `find_exact_match`
        // dedup and diverging the query-match path from the per-row enable cull.
        let mask = Archetype::filtered_signature_mask(component_ids);
        let existing = self.registry.find_exact_match(&mask);
        
        if let Some(first_id) = existing.first() {
            return *first_id; // Return existing archetype ID
        }
        
        // Allocate a new archetype ID
        let archetype_id = self.next_archetype_id;
        self.next_archetype_id.0 += 1;
        self.generation.bump();

        // Create a new archetype with these component IDs
        let _inland_id = self.archetypes.add_archetype_from_components(
            archetype_id,
            component_ids,
        );

        // Register the archetype with the registry
        self.registry.register_archetype(archetype_id, mask);

        // ── Phase 21 H1 — process-global "ever archetyped" mark ──
        // Set the global per-ComponentId bit for every component now living in
        // an archetype, so `register_component_hooks`'s staleness gate sees
        // placements in EVERY world, not just its own (the world-blind-hooks
        // hole). The dedup early-return above is exempt by construction: an
        // exact-match archetype set these bits when IT was first minted.
        //
        // EnableTag plan C1 premise (Decision D5): a `StorageKind::Bitset` id is
        // NEVER in this (or any) archetype's signature and has NO `ComponentPool`,
        // so it can never make an archetype's `ArchetypeFlags` stale w.r.t. a
        // hook — the precise staleness the H1 gate guards against is structurally
        // impossible for it (toggling fires no hooks/observers, Decision D3).
        // Marking it would add no protection and could mislead the gate into a
        // spurious "already archetyped" panic if a tag id were ever (mis)used
        // with `register_component_hooks`, so it is filtered out — keeping
        // EVER_ARCHETYPED's meaning exactly "this id entered a real signature".
        for &cid in component_ids {
            if component_registry::storage_kind(cid.0) == component_registry::StorageKind::Bitset {
                continue;
            }
            component_registry::mark_ever_archetyped(cid.0);
        }

        // ── Phase 14b OBS-SEED (C1, R2 §4) ──
        // Seed the new archetype's `ON_*_OBSERVER` bits from the registry. The
        // slab recipe (`add_archetype_from_components`) cannot reach the
        // registry, so we OR the observer bits in here, AFTER construction.
        //
        // Borrow-split: step 1 reads `&self.observer_registry` (shared) into the
        // `Copy` local `obs` — the shared borrow ends when `obs` is filled. Step
        // 2 writes through `&mut self.archetypes` (a disjoint field), so the two
        // borrows never overlap.
        let mut obs = ArchetypeFlags::empty();
        for &cid in component_ids {
            obs.insert_from_observers(cid, &self.observer_registry);
        }
        if !obs.is_empty() {
            let ptr = self
                .archetypes
                .get_archetype_ptr_mut(archetype_id)
                .expect("invariant: archetype just registered exists in bundle");
            // SAFETY (OBS-SEED): `ptr` is write-capable stable slab provenance
            //   (bundle invariants U1/U2), minted under `&mut self`. No other
            //   borrow into this slot is live — the `&mut Archetype` taken by
            //   `add_archetype_from_components` was dropped inside that call, and
            //   the `&self.observer_registry` read above has ended (copied into
            //   `obs`). `flags` is a `Copy` u16 read-modify-write, so the OR
            //   touches only this archetype and aliases nothing.
            unsafe {
                (*ptr).flags.insert_observer_bits(obs);
            }
        }

        #[cfg(debug_assertions)]
        self.debug_assert_observer_flags_consistent();

        archetype_id
    }
    
    /// Returns `true` if an archetype with the given ID is registered.
    #[inline]
    pub fn has_archetype(&self, archetype_id: ArchetypeId) -> bool {
        self.archetypes.get_archetype(archetype_id).is_some()
    }

    /// Gets a reference to an archetype by ID
    pub fn get_archetype(&self, archetype_id: ArchetypeId) -> Option<&Archetype> {
        self.archetypes.get_archetype(archetype_id)
    }

    /// Gets a mutable reference to an archetype by ID
    pub fn get_archetype_mut(&mut self, archetype_id: ArchetypeId) -> Option<&mut Archetype> {
        self.archetypes.get_archetype_mut(archetype_id)
    }

    /// Returns a read-only raw pointer to the archetype with `archetype_id`,
    /// or `None` if no archetype is registered for that id.
    ///
    /// Used by the Phase 7 fast read path in `EcsMaster` (Step 7+):
    /// `EntityInland` stores `*mut Archetype` directly, and the read
    /// fast-path dereferences through this pointer under `&EcsMaster`.
    ///
    /// # Provenance contract
    /// The returned pointer carries read-only provenance under Tree Borrows
    /// (minted via `&self` flavour of [`ArchetypeBundle::get_archetype_ptr`]).
    /// Callers may dereference for reads (`&*ptr`) as long as the
    /// `ArchetypeMaster` is borrowed at least immutably; the pointer is
    /// stable for the master's lifetime by bundle invariant U1 (slab base
    /// is heap-stable, slot addresses never move).
    ///
    /// The returned `*const Archetype` MUST NOT be cast to `*mut Archetype`
    /// and dereferenced for writing — under Tree Borrows the cast does
    /// not grant write capability and the child-write traps as retag UB.
    /// Use [`Self::archetype_ptr_for`] for write access.
    #[inline]
    pub fn get_archetype_ptr(&self, archetype_id: ArchetypeId) -> Option<*const Archetype> {
        self.archetypes.get_archetype_ptr(archetype_id)
    }

    /// Returns a write-capable raw pointer to the archetype with
    /// `archetype_id`, or `None` if no archetype is registered for that id.
    ///
    /// Used by `EcsMaster::create_entity` (Step 7 W7 choreography): the
    /// caller obtains the pointer under `&mut self`, re-borrows it as
    /// `&mut Archetype` to fill in the new entity's row, then later stores
    /// the same pointer inside `EntityInland` for fast random-access reads.
    ///
    /// # Provenance contract
    /// The returned pointer carries write-capable provenance (minted via
    /// `&mut self` flavour of [`ArchetypeBundle::get_archetype_ptr_mut`]).
    /// The pointer is stable for the master's lifetime by bundle invariants
    /// U1 (slab base stability) and U2 (slot lifetime ⊇ master lifetime).
    /// Subsequent `create_archetype` calls do not invalidate previously-
    /// minted pointers — the slab is a single fixed-size allocation, never
    /// reallocated or moved.
    #[inline]
    pub fn archetype_ptr_for(&mut self, archetype_id: ArchetypeId) -> Option<*mut Archetype> {
        self.archetypes.get_archetype_ptr_mut(archetype_id)
    }

    /// Registers (or reuses) an archetype with the given `component_ids` and
    /// returns both the `ArchetypeId` and a write-capable raw pointer to the
    /// slab slot, without performing a second lookup.
    ///
    /// Used by `EcsMaster::create_archetype` to atomically obtain both
    /// pieces in one call (Step 7 W7 choreography). When
    /// [`Self::create_archetype`] dedups against an existing archetype with
    /// the same component set (via `ArchetypeRegistry::find_exact_match`),
    /// the returned `ArchetypeId` and pointer reference that **existing**
    /// archetype — not a freshly-created one. Callers relying on the
    /// pointer pointing to a fresh slot must verify against
    /// [`Self::has_archetype`] beforehand.
    ///
    /// # Provenance contract
    /// Same as [`Self::archetype_ptr_for`]: write-capable provenance,
    /// stable for the master's lifetime via bundle invariants U1 + U2.
    pub fn add_archetype_and_get_ptr(
        &mut self,
        component_ids: &[ComponentId],
    ) -> (ArchetypeId, *mut Archetype) {
        let archetype_id = self.create_archetype(component_ids);
        let ptr = self
            .archetypes
            .get_archetype_ptr_mut(archetype_id)
            .expect("invariant: archetype just registered exists in bundle");
        (archetype_id, ptr)
    }
    
    /// Finds all archetypes that contain the specified components.
    ///
    /// Thin wrapper around [`find_archetypes_with_components_into`] for backward compatibility.
    #[inline]
    pub fn find_archetypes_with_components(&self, component_ids: &[ComponentId]) -> Vec<ArchetypeId> {
        self.registry.find_archetypes_with_components(component_ids)
    }

    /// Writes matching archetype IDs into `out`.
    ///
    /// # API contract
    /// `out` is **cleared at function entry**. Any existing contents are
    /// discarded. The caller's `Vec` is reused only for capacity, not data —
    /// this enables zero-allocation reuse across calls.
    #[inline]
    pub fn find_archetypes_with_components_into(
        &self,
        component_ids: &[ComponentId],
        out: &mut Vec<ArchetypeId>,
    ) {
        self.registry.find_archetypes_with_components_into(component_ids, out);
    }

    /// Finds all archetypes containing all components in the specified mask.
    ///
    /// Thin wrapper around [`find_matching_archetypes_into`] for backward compatibility.
    #[inline]
    pub fn find_matching_archetypes(&self, mask: &ComponentMask) -> Vec<ArchetypeId> {
        self.registry.find_matching_archetypes(mask)
    }

    /// Writes archetypes matching `mask` into `out`.
    ///
    /// # API contract
    /// `out` is **cleared at function entry**. Any existing contents are
    /// discarded. The caller's `Vec` is reused only for capacity, not data —
    /// this enables zero-allocation reuse across calls.
    #[inline]
    pub fn find_matching_archetypes_into(&self, mask: &ComponentMask, out: &mut Vec<ArchetypeId>) {
        self.registry.find_matching_archetypes_into(mask, out);
    }

    /// Find archetypes with complex filtering criteria (include, exclude, optional components).
    ///
    /// Thin wrapper around [`find_archetypes_with_filter_into`] for backward compatibility.
    #[inline]
    pub fn find_archetypes_with_filter(
        &self,
        include_mask: &ComponentMask,
        exclude_mask: &ComponentMask,
        optional_mask: &ComponentMask,
    ) -> Vec<ArchetypeId> {
        self.registry.find_with_filter(include_mask, exclude_mask, optional_mask)
    }

    /// Writes matching archetype IDs into `out` using include/exclude/optional masks.
    ///
    /// # API contract
    /// `out` is **cleared at function entry**. Any existing contents are
    /// discarded. The caller's `Vec` is reused only for capacity, not data —
    /// this enables zero-allocation reuse across calls.
    #[inline]
    pub fn find_archetypes_with_filter_into(
        &self,
        include_mask: &ComponentMask,
        exclude_mask: &ComponentMask,
        optional_mask: &ComponentMask,
        out: &mut Vec<ArchetypeId>,
    ) {
        self.registry
            .find_with_filter_into(include_mask, exclude_mask, optional_mask, out);
    }

    /// Find archetypes with components that can be included, excluded, or optional.
    ///
    /// Thin wrapper around [`find_archetypes_with_component_filter_into`] for backward compatibility.
    #[inline]
    pub fn find_archetypes_with_component_filter(
        &self,
        include_components: &[ComponentId],
        exclude_components: &[ComponentId],
        optional_components: &[ComponentId],
    ) -> Vec<ArchetypeId> {
        self.registry.find_with_component_filter(
            include_components,
            exclude_components,
            optional_components,
        )
    }

    /// Writes matching archetype IDs into `out` using component-array filters.
    ///
    /// # API contract
    /// `out` is **cleared at function entry**. Any existing contents are
    /// discarded. The caller's `Vec` is reused only for capacity, not data —
    /// this enables zero-allocation reuse across calls.
    #[inline]
    pub fn find_archetypes_with_component_filter_into(
        &self,
        include_components: &[ComponentId],
        exclude_components: &[ComponentId],
        optional_components: &[ComponentId],
        out: &mut Vec<ArchetypeId>,
    ) {
        self.registry.find_with_component_filter_into(
            include_components,
            exclude_components,
            optional_components,
            out,
        );
    }
    
    /// Get references to archetypes with complex filtering
    /// Returns direct references to archetypes for faster access
    pub fn get_archetypes_with_filter(
        &self,
        include_mask: &ComponentMask,
        exclude_mask: &ComponentMask,
        optional_mask: &ComponentMask
    ) -> Vec<&Archetype> {
        let archetype_ids = self.find_archetypes_with_filter(include_mask, exclude_mask, optional_mask);
        archetype_ids.into_iter()
            .filter_map(|id| self.get_archetype(id))
            .collect()
    }
    
    /// Get references to archetypes with component filtering
    /// Returns direct references to archetypes for faster access
    pub fn get_archetypes_with_component_filter(
        &self,
        include_components: &[ComponentId],
        exclude_components: &[ComponentId],
        optional_components: &[ComponentId]
    ) -> Vec<&Archetype> {
        let archetype_ids = self.find_archetypes_with_component_filter(
            include_components,
            exclude_components,
            optional_components
        );
        archetype_ids.into_iter()
            .filter_map(|id| self.get_archetype(id))
            .collect()
    }
    
    /// Get references to archetypes matching a simple component set
    /// Returns direct references to archetypes for faster access
    pub fn get_archetypes_with_components(&self, component_ids: &[ComponentId]) -> Vec<&Archetype> {
        let archetype_ids = self.find_archetypes_with_components(component_ids);
        archetype_ids.into_iter()
            .filter_map(|id| self.get_archetype(id))
            .collect()
    }
    
    /// Returns the number of registered archetypes
    #[inline]
    pub fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }
    
    /// Adds an existing archetype to the master.
    /// This is used when loading archetypes from external sources or for cloning.
    pub fn add_existing_archetype(&mut self, archetype: Archetype) -> ArchetypeId {
        let archetype_id = archetype.id();

        // Extract component IDs before moving the archetype
        let component_ids = archetype.component_ids().to_vec();

        // Register with the bundle
        self.archetypes.add_archetype(archetype);

        // Create the registry-index mask from the component IDs. EnableTag plan
        // C1 premise (Decision D5): this third archetype-mint funnel must apply
        // the SAME `StorageKind::Bitset` filter as `create_archetype` /
        // `get_or_create_archetype` / `Archetype::create_by_ids`, otherwise a
        // pre-built archetype carrying a bitset id in its raw `component_ids`
        // would be registered under a signature that includes the bitset bit
        // (the archetype's own signature already excludes it via `create_by_ids`),
        // breaking dedup against the table-only twin.
        let mask = Archetype::filtered_signature_mask(&component_ids);

        // Register with the registry
        self.registry.register_archetype(archetype_id, mask);

        // ── Phase 21 H1 — process-global "ever archetyped" mark (bypass arm) ──
        // Same mark as `create_archetype`: this is the second archetype-mint
        // funnel (the OBS-SEED2 bypass below exists for the same reason), so
        // components placed through it must also raise the global staleness
        // bit for `register_component_hooks`. The same `StorageKind::Bitset`
        // filter applies (see `create_archetype`'s H1 loop): a bitset id can
        // never make an archetype's flags stale, so it is excluded.
        for &cid in &component_ids {
            if component_registry::storage_kind(cid.0) == component_registry::StorageKind::Bitset {
                continue;
            }
            component_registry::mark_ever_archetyped(cid.0);
        }

        // ── Phase 14b OBS-SEED2 (C1, R2 §4 — the one bypass) ──
        // `add_existing_archetype` inserts a pre-built `Archetype` whose `flags`
        // were computed elsewhere, so it does NOT funnel through
        // `create_archetype`'s seed. Seed observer bits here too (same
        // borrow-split + write-pointer as OBS-SEED).
        let mut obs = ArchetypeFlags::empty();
        for &cid in &component_ids {
            obs.insert_from_observers(cid, &self.observer_registry);
        }
        if !obs.is_empty() {
            let ptr = self
                .archetypes
                .get_archetype_ptr_mut(archetype_id)
                .expect("invariant: archetype just registered exists in bundle");
            // SAFETY (OBS-SEED2): identical to OBS-SEED — `ptr` is write-capable
            //   stable slab provenance for the just-stored slot, minted under
            //   `&mut self`; no other borrow into the slot is live; the
            //   `&self.observer_registry` read above ended (copied into `obs`);
            //   `flags` is a `Copy` u16 RMW touching only this archetype.
            unsafe {
                (*ptr).flags.insert_observer_bits(obs);
            }
        }

        // Update next ID if necessary and bump generation for each new archetype slot minted
        if archetype_id >= self.next_archetype_id {
            self.next_archetype_id = ArchetypeId(archetype_id.0 + 1);
            self.generation.bump();
        }

        #[cfg(debug_assertions)]
        self.debug_assert_observer_flags_consistent();

        archetype_id
    }

    /// Returns the current archetype generation, monotonic across the master's
    /// entire lifetime including `clear()` calls. Used by `QueryState` to detect
    /// cache invalidation due to new archetype creations.
    #[inline]
    pub fn archetype_generation(&self) -> ArchetypeGeneration {
        self.generation
    }

    /// Returns the current structural generation — bumped on every archetype
    /// removal and on `clear()`. `QueryState` compares against the saved value;
    /// any change forces a full rebuild instead of a delta-add, eliminating
    /// the ArchetypeId-ABA hazard (cached `matched_ids` holding an ID that was
    /// freed and later reused by an unrelated archetype).
    #[inline]
    pub fn structural_generation(&self) -> ArchetypeGeneration {
        self.structural_generation
    }

    /// Returns the current EnableTag column-allocation epoch (Decision D1 / W2).
    ///
    /// Bumped once per `EnableColumn` allocation (the first toggle of a tag into
    /// an archetype). A `QueryState` carrying an `Enabled`/`Disabled` term
    /// compares against the saved value on `update`; any change re-runs the
    /// presence cull (Decision D2 / O2). Distinct from
    /// [`Self::structural_generation`]: a column-alloc must invalidate only
    /// enable-bearing caches, never force the full structural rebuild.
    ///
    /// `Relaxed` is sound in v1 because the value is bumped only under
    /// `&mut self` and read single-threaded in `update` — see the field's
    /// W2 forward-seam note. (D7 worker-marking is the seam that upgrades this
    /// to Acquire/Release with a loom proof.)
    #[inline]
    pub fn enable_generation(&self) -> u64 {
        self.enable_generation.load(Ordering::Relaxed)
    }

    /// Records the FIRST `EnableColumn` allocation for `(tag, archetype)`
    /// (Decision D1 inv 5 / O2). Updates the cull oracle AND bumps the epoch
    /// **atomically together**, exactly once per column — the toggle path (Step
    /// 5) calls this iff `Archetype::set_enable_bit` reported a fresh column.
    ///
    /// Co-locating the two updates here (instead of two separate calls from the
    /// toggle path) is what keeps the D1 inv-5 pairing impossible to desync:
    /// the presence bit and the generation bump can never get out of step.
    ///
    /// `&mut self` in v1: the toggle runs in the structural/apply window with
    /// exclusive world access, so the `Relaxed` epoch bump cannot race (W2
    /// forward-seam note on the field). D7 worker-marking is where the receiver
    /// relaxes to `&self` under real Acquire/Release.
    #[inline]
    pub(crate) fn note_enable_column_alloc(&mut self, tag: ComponentId, arch: ArchetypeId) {
        // Presence bit first, then the epoch bump — `note_column_alloc`'s own
        // Release pairs the bit publish with the epoch's Acquire reader (the
        // ordering is the forward seam; v1 is `&mut self`-exclusive).
        self.enable_presence.note_column_alloc(tag, arch);
        // Relaxed: sound only because no concurrent access exists in v1 (the
        // `&mut self` receiver proves exclusivity). Forward seam W2 / D7.
        self.enable_generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns this world's EnableTag presence cull oracle (Decision D2).
    ///
    /// The query cull (Wave 3) consults [`EnablePresence::contains`] /
    /// [`EnablePresence::epoch`] over the bounded matched set. Per-world (keyed
    /// by this world's `ArchetypeId`s), co-located with `enable_generation`.
    // Production consumer (Wave 3 / Step 7a): the `cull_enable_archetypes`
    // presence cull and the candidate-seeded global scan in
    // `QueryDataState::new` / `update` consult this oracle over the bounded
    // matched set / candidate snapshot.
    #[inline]
    pub(crate) fn enable_presence(&self) -> &EnablePresence {
        &self.enable_presence
    }

    /// Removes an archetype by ID. Returns true if the archetype was found
    /// and removed.
    ///
    /// On success, bumps `structural_generation` so any live `QueryState`
    /// referencing this master is forced to do a full rebuild on its next
    /// `iter()` — this is the load-bearing piece of the ArchetypeId-ABA fix:
    /// without the bump, a stale `matched_ids` could retain the just-freed ID
    /// and silently surface a future archetype reusing that same numeric ID
    /// (after `clear()` resets `next_archetype_id` to 1) as if it matched the
    /// original filter.
    pub fn remove_archetype(&mut self, archetype_id: ArchetypeId) -> bool {
        // First unregister from the registry
        let registry_success = self.registry.unregister_archetype(archetype_id);

        // If registry removal failed, the archetype wasn't registered
        if !registry_success {
            return false;
        }

        // Now remove from the archetype bundle
        let bundle_success = self.archetypes.remove_archetype(archetype_id);

        debug_assert!(bundle_success, "Registry and bundle are out of sync");

        if bundle_success {
            // EnableTag amendment A4.4 / Step 4: drop this archetype's presence
            // bit across every tag so a recycled id never falsely persists as a
            // candidate for the candidate-seeded global scan. Runs only on this
            // structural-removal path (rare), off the hot path.
            self.enable_presence.clear_archetype(archetype_id);
            // Signal cache invalidation to every outstanding QueryState.
            // See struct-level doc on `structural_generation`.
            self.structural_generation.bump();
        }

        bundle_success
    }
    
    /// Finds or creates an archetype with the specified component IDs
    /// This is an optimized version that first tries to find an existing archetype
    pub fn get_or_create_archetype(&mut self, component_ids: &[ComponentId]) -> ArchetypeId {
        // First try to find an existing archetype with the exact components.
        // Same C1-premise filter as `create_archetype`: the lookup mask must
        // exclude `StorageKind::Bitset` ids so it keys on the SAME filtered
        // signature the archetype was registered under (else the spawn funnel
        // `cold_register_bundle_archetype` would miss the dedup and mint a
        // duplicate registry entry for a structurally-identical archetype).
        let mask = Archetype::filtered_signature_mask(component_ids);
        let existing = self.registry.find_exact_match(&mask);
        
        if let Some(first_id) = existing.first() {
            *first_id // Return existing archetype ID
        } else {
            // Create a new archetype
            self.create_archetype(component_ids)
        }
    }
    
    /// Adds a component type to an existing archetype
    /// Returns the ID of the new archetype containing the added component
    pub fn add_component_to_archetype(
        &mut self, 
        source_archetype_id: ArchetypeId, 
        component_id: ComponentId
    ) -> Option<ArchetypeId> {
        // EnableTag plan C1 premise (Decision D5): the appended id must NOT be a
        // bitset enable tag — those never enter a signature and route to
        // `enable`/`disable`, not a structural add. The registry signature is
        // already bitset-safe (`get_or_create_archetype` filters), so a misrouted
        // bitset id cannot corrupt the registry; this debug_assert surfaces the
        // misuse instead of silently producing a structurally-identical archetype.
        debug_assert_ne!(
            component_registry::storage_kind(component_id.0),
            component_registry::StorageKind::Bitset,
            "add_component_to_archetype: id {} is a bitset enable tag; toggle it \
             via enable/disable, do not migrate it into a signature",
            component_id.0
        );

        // Get the source archetype
        let source_archetype = self.get_archetype(source_archetype_id)?;

        // Get all component IDs from the source archetype
        let mut new_components = source_archetype.component_ids().to_vec();

        // Check if the component already exists in the archetype
        if new_components.contains(&component_id) {
            return Some(source_archetype_id); // No change needed
        }

        // Add the new component ID
        new_components.push(component_id);

        // Create or get the new archetype
        Some(self.get_or_create_archetype(&new_components))
    }
    
    /// Removes a component type from an existing archetype
    /// Returns the ID of the new archetype without the component
    pub fn remove_component_from_archetype(
        &mut self, 
        source_archetype_id: ArchetypeId, 
        component_id: ComponentId
    ) -> Option<ArchetypeId> {
        // Get the source archetype
        let source_archetype = self.get_archetype(source_archetype_id)?;
        
        // Get all component IDs from the source archetype
        let new_components: Vec<ComponentId> = source_archetype.component_ids()
            .iter()
            .filter(|&&c| c != component_id)
            .copied()
            .collect();
        
        // If no components were removed, return the source archetype
        if new_components.len() == source_archetype.component_ids().len() {
            return Some(source_archetype_id);
        }
        
        // Create or get the new archetype
        Some(self.get_or_create_archetype(&new_components))
    }
    
    /// Returns a reference to the internal archetype bundle
    pub fn archetype_bundle(&self) -> &ArchetypeBundle {
        &self.archetypes
    }
    
    /// Returns a mutable reference to the internal archetype bundle.
    ///
    /// `pub(crate)` (Phase 14b §1, C1): a `&mut ArchetypeBundle` exposes the
    /// bundle bit-setters that mint archetype slots, which would bypass the
    /// `create_archetype` observer-flag seed and corrupt the registry/id
    /// bookkeeping. Narrowed to crate-internal to close that path (verified
    /// zero callers in src/tests/benches). The read-only
    /// [`Self::archetype_bundle`] stays `pub` — a `&ArchetypeBundle` has no
    /// bit-setter.
    // Retained crate-internal capability (no current caller after the C1
    // narrowing); kept rather than deleted so future crate code has a guarded
    // mutable bundle handle without re-widening the public surface.
    #[allow(dead_code)]
    pub(crate) fn archetype_bundle_mut(&mut self) -> &mut ArchetypeBundle {
        &mut self.archetypes
    }
    
    /// Returns a reference to the internal archetype registry
    pub fn archetype_registry(&self) -> &ArchetypeRegistry {
        &self.registry
    }
    
    /// Returns a mutable reference to the internal archetype registry
    pub fn archetype_registry_mut(&mut self) -> &mut ArchetypeRegistry {
        &mut self.registry
    }
    
    /// Creates a new query for archetypes containing all specified component IDs
    pub fn query_with_components<'a>(&'a self, component_ids: &[ComponentId]) -> LegacyQuery<'a> {
        LegacyQuery::with_component_ids(self, component_ids)
    }

    /// Creates a new query for archetypes matching the component mask
    pub fn query_with_mask<'a>(&'a self, mask: &ComponentMask) -> LegacyQuery<'a> {
        LegacyQuery::with_mask(self, mask)
    }

    /// Creates a new query for archetypes exactly matching the component mask
    pub fn query_with_exact_mask<'a>(&'a self, mask: &ComponentMask) -> LegacyQuery<'a> {
        LegacyQuery::with_exact_mask(self, mask)
    }

    /// Creates a type-safe query for archetypes containing the specified components
    /// Example: master.query::<(Position, Velocity)>()
    pub fn query<'a, T: crate::ecs::core::iters::component_set::ComponentSet>(&'a self) -> LegacyQuery<'a> {
        LegacyQuery::with::<T>(self)
    }

    /// Creates a query with complex filtering criteria
    /// - include_mask: Components that must be present (AND)
    /// - exclude_mask: Components that must not be present (NOT)
    /// - optional_mask: Components that are optional (at least one must be present)
    pub fn query_with_filters<'a>(
        &'a self,
        include_mask: &ComponentMask,
        exclude_mask: &ComponentMask,
        optional_mask: &ComponentMask
    ) -> LegacyQuery<'a> {
        LegacyQuery::with_filters(self, include_mask, exclude_mask, optional_mask)
    }

    /// Creates a type-safe query with complex filtering
    pub fn query_with_type_filters<'a, Inc: crate::ecs::core::iters::component_set::ComponentSet,
                                     Exc: crate::ecs::core::iters::component_set::ComponentSet,
                                     Opt: crate::ecs::core::iters::component_set::ComponentSet>(
        &'a self
    ) -> LegacyQuery<'a> {
        LegacyQuery::with_type_filters::<Inc, Exc, Opt>(self)
    }
    
    /// Returns an iterator over all archetypes
    pub fn iter_archetypes(&self) -> impl Iterator<Item = &Archetype> {
        self.archetypes.iter()
    }

    /// Mutable iterator over every archetype.
    ///
    /// Mirror of [`Self::iter_archetypes`]; required by Phase 10
    /// `run_check_ticks_scan` (plan §4.6, §9.6, §15.7) for in-place clamping
    /// of per-row `added`/`changed` ticks via [`ComponentPool::write_added_tick`]
    /// / [`ComponentPool::write_changed_tick`] under `&mut Archetype`.
    ///
    /// Delegates to [`ArchetypeBundle::iter_mut`]; disjoint `&mut Archetype`
    /// yields are guaranteed by the bundle's bitset-driven iteration
    /// (each slot visited at most once).
    #[inline]
    pub(crate) fn iter_archetypes_mut(&mut self) -> ArchetypeBundleIterMut<'_> {
        self.archetypes.iter_mut()
    }

    /// Registers `runner` as a `kind` observer for component `cid`, returning a
    /// stable [`ObserverId`] for later [`Self::remove_observer`] (Phase 14b).
    ///
    /// On the FIRST observer for `(kind, cid)` (the `(kind, cid)` list goes
    /// empty → non-empty), this walks every archetype containing `cid` and
    /// raises its `ON_{kind}_OBSERVER` bit so the structural-op fire sites
    /// dispatch to the new observer (Bevy's `Archetypes::update_flags`). On a
    /// non-first add the bit is already set, so no walk runs (O(1)).
    pub fn add_observer(
        &mut self,
        kind: ObserverKind,
        cid: ComponentId,
        runner: ObserverFn,
    ) -> ObserverId {
        let (id, became_nonempty) = self.observer_registry.add(kind, cid, runner);
        if became_nonempty {
            // Add-first walk: raise the bit on every archetype containing `cid`.
            // Idempotent OR — preserves the hook bit and every other kind's bit.
            // `iter_archetypes_mut` borrows `self.archetypes`; the registry
            // mutation above has already ended, so no registry borrow is live.
            let bit = Self::observer_bit(kind);
            for archetype in self.iter_archetypes_mut() {
                if archetype.has_component_id(cid) {
                    archetype.flags.insert(bit);
                }
            }
            #[cfg(debug_assertions)]
            self.debug_assert_observer_flags_consistent();
        }
        id
    }

    /// Removes the observer with `id`, returning `true` if it was registered
    /// (Phase 14b).
    ///
    /// On removal of the LAST observer for its `(kind, cid)` pair, this
    /// recomputes the `ON_{kind}_OBSERVER` bit on every archetype containing
    /// `cid`: the bit stays set iff some *sibling* component in that archetype
    /// still has a `kind` observer; otherwise it is cleared (the hook bit and
    /// the other kinds' bits are preserved by the masked write). On a non-last
    /// removal no walk runs (the bit stays correct, O(1)).
    pub fn remove_observer(&mut self, id: ObserverId) -> bool {
        let Some((kind, cid, became_empty)) = self.observer_registry.remove(id) else {
            return false;
        };
        if became_empty {
            // Remove-last recompute walk. Disjoint-field borrows: `&mut
            // self.archetypes` (the walk) and `&self.observer_registry` (the
            // sibling read) are different fields, so they may be live together.
            let bit = Self::observer_bit(kind);
            let reg = &self.observer_registry;
            for archetype in self.archetypes.iter_mut() {
                if archetype.has_component_id(cid) {
                    // `cid`'s list is now empty (removed above), so this is true
                    // iff some OTHER component in the archetype still observes
                    // `kind`.
                    let any_sibling = archetype
                        .component_ids()
                        .iter()
                        .any(|&sib| reg.has_observer(kind, sib));
                    if any_sibling {
                        archetype.flags.insert(bit);
                    } else {
                        archetype.flags.clear(bit);
                    }
                }
            }
            #[cfg(debug_assertions)]
            self.debug_assert_observer_flags_consistent();
        }
        true
    }

    /// Maps an [`ObserverKind`] to its `ON_{kind}_OBSERVER` flag bit.
    #[inline]
    const fn observer_bit(kind: ObserverKind) -> u16 {
        match kind {
            ObserverKind::Add => ArchetypeFlags::ON_ADD_OBSERVER,
            ObserverKind::Insert => ArchetypeFlags::ON_INSERT_OBSERVER,
            ObserverKind::Replace => ArchetypeFlags::ON_REPLACE_OBSERVER,
            ObserverKind::Remove => ArchetypeFlags::ON_REMOVE_OBSERVER,
        }
    }

    /// Debug-only tripwire (Phase 14b §1): asserts that every archetype's
    /// `ON_*_OBSERVER` bits exactly reflect the registry — a bit is set iff some
    /// component in the archetype has ≥1 observer of that kind.
    ///
    /// Walks the SHARED `iter_archetypes()` iterator (read-only). Called at the
    /// three sites that can change a bit: both seed sites (`create_archetype`,
    /// `add_existing_archetype`) and both dynamic walks (the `add_observer`
    /// add-first and the `remove_observer` remove-last). Compiles to nothing in
    /// release.
    #[cfg(debug_assertions)]
    fn debug_assert_observer_flags_consistent(&self) {
        const KINDS: [(ObserverKind, u16); 4] = [
            (ObserverKind::Add, ArchetypeFlags::ON_ADD_OBSERVER),
            (ObserverKind::Insert, ArchetypeFlags::ON_INSERT_OBSERVER),
            (ObserverKind::Replace, ArchetypeFlags::ON_REPLACE_OBSERVER),
            (ObserverKind::Remove, ArchetypeFlags::ON_REMOVE_OBSERVER),
        ];
        for archetype in self.iter_archetypes() {
            for (kind, bit) in KINDS {
                let expected = archetype
                    .component_ids()
                    .iter()
                    .any(|&cid| self.observer_registry.has_observer(kind, cid));
                debug_assert_eq!(
                    archetype.flags.contains(bit),
                    expected,
                    "observer flag bit out of sync with registry for archetype {:?}",
                    archetype.id()
                );
            }
        }
    }

    /// Removes all archetypes and resets `next_archetype_id` to 1.
    ///
    /// # Interaction with `QueryState`
    /// Safe across outstanding `QueryState`s. `clear()` bumps
    /// `structural_generation`, which on the next `QueryState::iter()` triggers
    /// a full cache rebuild (the dedup bitset is dropped + every live archetype
    /// is reclassified against the filter). This eliminates the
    /// ArchetypeId-ABA hazard that the pre-fix code documented as a caller
    /// burden — a freshly created archetype reusing a recycled id is now
    /// classified correctly against the stale `QueryState`'s filter.
    ///
    /// `generation` is intentionally NOT reset (kept monotonic across the
    /// master's entire lifetime — `QueryState::generation` stays `<=` master's,
    /// preserving the debug_assert invariant). The structural counter is what
    /// signals invalidation; the creation counter signals the delta-add path.
    pub fn clear(&mut self) {
        self.archetypes = ArchetypeBundle::new();
        self.registry.clear();
        self.next_archetype_id = ArchetypeId(1);
        // EnableTag amendment A4.4 / Step 4: `clear()` recycles every
        // ArchetypeId from 1, so the presence oracle's per-tag bits are wholly
        // stale. Replace it with a fresh oracle (`&mut self` makes this a plain
        // field swap — no atomics needed) so no recycled id is a stale
        // candidate. `enable_generation` is intentionally left monotonic: the
        // structural bump below already forces the candidate path's full
        // re-seed (a fresh oracle yields an empty candidate set until the first
        // post-clear column alloc bumps it again).
        self.enable_presence = EnablePresence::new();
        // `generation` is NOT reset — see doc comment above.
        // `structural_generation` IS bumped: `clear()` is the maximally-
        // structural change possible. A subsequent `QueryState::iter()` will
        // observe the mismatch and do a full rebuild, which is the only path
        // that correctly handles `next_archetype_id` rollback (otherwise the
        // dedup bitset would treat ID=1 as "already seen").
        self.structural_generation.bump();
    }
}

// SAFETY (SEND6 — Phase 9 §2.4, §9.1):
//
// `ArchetypeMaster` becomes `Send + Sync` under the Phase 9 contract:
//
//   - Internal `ArchetypeBundle` uses a stable-address slab
//     (`Box<[MaybeUninit<Archetype>; _]>`); slot addresses do not move once
//     written. Worker reads via `archetype_ptr_for` / `get_archetype` are
//     sound because pointer-stable archetypes are immutable to workers
//     (mutations are gated on `&mut self`).
//   - Archetype creation (`create_archetype`, `get_or_create_archetype`,
//     `remove_archetype`, `clear`) takes `&mut self` and runs only on the
//     dispatcher under the apply window (SCH7); no worker holds a live
//     cell view that aliases the slab while a new slot is being written.
//   - `observer_registry: ObserverRegistry` (Phase 14b) is independently
//     `Send + Sync` with NO `unsafe impl`: it holds only
//     `Option<Box<[[Vec<ObserverEntry>; MAX_COMPONENTS]; 4]>>` + a `u64`, and
//     `ObserverEntry` is `{ ObserverId(u64), fn-ptr }` POD — fn-pointers are
//     unconditionally `Send + Sync`. Mutated only via `&mut self`
//     (`add_observer` / `remove_observer`) on the dispatcher under the apply
//     window; never touched by a worker (SCH7).
unsafe impl Send for ArchetypeMaster {}
unsafe impl Sync for ArchetypeMaster {}

impl Default for ArchetypeMaster {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each test module owns its own ComponentId range to avoid `OnceLock`
    // collisions across tests (see audit C-003 / Phase 1b). `archetype_master`
    // uses 300-309. All mock components share the same backing type (`u32`)
    // because the tests only exercise mask logic, never byte-level layout.
    const MOCK_ID_BASE: ComponentId = ComponentId(300);

    /// Translate a test-local "logical" usize ID (1..=8) into the actual
    /// `ComponentId` used in the registry.
    #[inline]
    fn mock(local: usize) -> ComponentId {
        ComponentId(MOCK_ID_BASE.0 + local)
    }

    /// Translate a slice of local usize IDs into actual ComponentIds — keeps the
    /// test bodies readable (`master.create_archetype(&mocks([1, 2, 3]))`).
    fn mocks<const N: usize>(local: [usize; N]) -> [ComponentId; N] {
        local.map(mock)
    }

    /// Register mock components for testing
    fn register_mock_components() {
        // Register `u32` under each test-local ID. `OnceLock::set` is
        // idempotent: re-registration after the first call is a no-op, so
        // running this from every test is safe.
        for local in 1..=8 {
            component_registry::register_layout::<u32>(mock(local).0);
        }
    }
    
    #[test]
    fn test_create_archetype() {
        register_mock_components();
        let mut master = ArchetypeMaster::new();

        // Create a new archetype
        let id1 = master.create_archetype(&mocks([1, 2, 3]));
        assert_eq!(id1, ArchetypeId(1));

        // Create another archetype
        let id2 = master.create_archetype(&mocks([1, 2]));
        assert_eq!(id2, ArchetypeId(2));

        // Try to create an archetype with the same components - should return existing ID
        let id3 = master.create_archetype(&mocks([1, 2, 3]));
        assert_eq!(id3, id1); // Should return the ID of the first archetype

        // Verify both archetypes exist
        assert!(master.get_archetype(id1).is_some());
        assert!(master.get_archetype(id2).is_some());
    }

    #[test]
    fn test_remove_archetype() {
        register_mock_components();
        let mut master = ArchetypeMaster::new();

        // Create a new archetype
        let id = master.create_archetype(&mocks([1, 2, 3]));
        assert!(master.get_archetype(id).is_some());

        // Remove the archetype
        let result = master.remove_archetype(id);
        assert!(result);

        // Verify the archetype doesn't exist anymore
        assert!(master.get_archetype(id).is_none());

        // Try to remove a non-existent archetype
        let result = master.remove_archetype(ArchetypeId(999));
        assert!(!result);
    }

    #[test]
    fn test_find_archetypes() {
        register_mock_components();
        let mut master = ArchetypeMaster::new();

        // Create different archetypes
        let id1 = master.create_archetype(&mocks([1, 2, 3]));
        let id2 = master.create_archetype(&mocks([1, 2]));
        let id3 = master.create_archetype(&mocks([2, 3]));

        // Find archetypes with component 1
        let results = master.find_archetypes_with_components(&mocks([1]));
        assert_eq!(results.len(), 2);
        assert!(results.contains(&id1));
        assert!(results.contains(&id2));

        // Find archetypes with components 2 and 3
        let results = master.find_archetypes_with_components(&mocks([2, 3]));
        assert_eq!(results.len(), 2);
        assert!(results.contains(&id1));
        assert!(results.contains(&id3));

        // Find archetypes with components 1, 2, and 3
        let results = master.find_archetypes_with_components(&mocks([1, 2, 3]));
        assert_eq!(results.len(), 1);
        assert!(results.contains(&id1));
    }

    #[test]
    fn test_add_component_to_archetype() {
        register_mock_components();
        let mut master = ArchetypeMaster::new();

        // Create an archetype with components 1 and 2
        let id1 = master.create_archetype(&mocks([1, 2]));

        // Add component 3 to the archetype
        let id2 = master.add_component_to_archetype(id1, mock(3)).unwrap();

        // The new archetype should have components 1, 2, and 3
        let archetype = master.get_archetype(id2).unwrap();
        assert!(archetype.has_component_id(mock(1)));
        assert!(archetype.has_component_id(mock(2)));
        assert!(archetype.has_component_id(mock(3)));

        // Adding a component that already exists should return the same archetype
        let id3 = master.add_component_to_archetype(id2, mock(2)).unwrap();
        assert_eq!(id3, id2);
    }

    #[test]
    fn test_remove_component_from_archetype() {
        register_mock_components();
        let mut master = ArchetypeMaster::new();

        // Create an archetype with components 1, 2, and 3
        let id1 = master.create_archetype(&mocks([1, 2, 3]));

        // Remove component 3 from the archetype
        let id2 = master.remove_component_from_archetype(id1, mock(3)).unwrap();

        // The new archetype should have only components 1 and 2
        let archetype = master.get_archetype(id2).unwrap();
        assert!(archetype.has_component_id(mock(1)));
        assert!(archetype.has_component_id(mock(2)));
        assert!(!archetype.has_component_id(mock(3)));

        // Removing a component that doesn't exist should return the same archetype
        let id3 = master.remove_component_from_archetype(id2, mock(3)).unwrap();
        assert_eq!(id3, id2);
    }

    #[test]
    fn test_reuse_existing_archetype() {
        register_mock_components();
        let mut master = ArchetypeMaster::new();

        // Create an archetype with components 1, 2, and 3
        let id1 = master.create_archetype(&mocks([1, 2, 3]));

        // Create an archetype with components 1 and 2
        let id2 = master.create_archetype(&mocks([1, 2]));

        // Add component 3 to the second archetype, which should result in
        // reusing the first archetype
        let id3 = master.add_component_to_archetype(id2, mock(3)).unwrap();
        assert_eq!(id3, id1);
    }

    #[test]
    fn test_get_archetypes_with_filter() {
        register_mock_components();
        let mut master = ArchetypeMaster::new();

        // Create different archetypes
        master.create_archetype(&mocks([1, 2]));          // Position, Velocity
        master.create_archetype(&mocks([1, 3]));          // Position, Health
        master.create_archetype(&mocks([2, 4]));          // Velocity, Damage
        master.create_archetype(&mocks([1, 2, 3]));       // Position, Velocity, Health
        master.create_archetype(&mocks([1, 2, 4]));       // Position, Velocity, Damage

        // Filter: Position AND Velocity, but NOT Damage
        let mut include_mask = ComponentMask::new();
        include_mask.set(mock(1));  // Position
        include_mask.set(mock(2));  // Velocity

        let mut exclude_mask = ComponentMask::new();
        exclude_mask.set(mock(4));  // Damage

        let optional_mask = ComponentMask::new();

        // Get archetypes with references
        let archetypes = master.get_archetypes_with_filter(
            &include_mask,
            &exclude_mask,
            &optional_mask
        );

        // Should match [Position, Velocity] and [Position, Velocity, Health]
        assert_eq!(archetypes.len(), 2);

        // Verify components
        for archetype in archetypes {
            assert!(archetype.has_component_id(mock(1)));  // Position
            assert!(archetype.has_component_id(mock(2)));  // Velocity
            assert!(!archetype.has_component_id(mock(4))); // Not Damage
        }
    }

    #[test]
    fn test_get_archetypes_with_component_filter() {
        register_mock_components();
        let mut master = ArchetypeMaster::new();

        // Create different archetypes
        master.create_archetype(&mocks([1, 2]));          // Position, Velocity
        master.create_archetype(&mocks([1, 3]));          // Position, Health
        master.create_archetype(&mocks([2, 4]));          // Velocity, Damage
        master.create_archetype(&mocks([1, 2, 3]));       // Position, Velocity, Health
        master.create_archetype(&mocks([1, 2, 4]));       // Position, Velocity, Damage

        // Filter: Position AND at least one of [Health, Damage]
        let include = mocks([1]);                // Position
        let exclude: [ComponentId; 0] = [];
        let optional = mocks([3, 4]);           // Health or Damage

        // Get archetypes with references
        let archetypes = master.get_archetypes_with_component_filter(
            &include,
            &exclude,
            &optional
        );

        // Should match [Position, Health], [Position, Velocity, Health], [Position, Velocity, Damage]
        assert_eq!(archetypes.len(), 3);

        // Verify components
        for archetype in archetypes {
            assert!(archetype.has_component_id(mock(1)));  // Position

            // At least one of Health or Damage
            assert!(archetype.has_component_id(mock(3)) || archetype.has_component_id(mock(4)));
        }
    }

    // --- ABA-prevention via dual-generation counters ---

    /// `remove_archetype` must bump `structural_generation` so any live
    /// `QueryState` is forced to invalidate its cache on the next iter.
    /// Without this, a `QueryState` whose `matched_ids` still contains the
    /// just-freed ID can leak it into iteration after a future `clear()` +
    /// `create_archetype()` cycle reuses the same numeric ID.
    #[test]
    fn remove_archetype_bumps_structural_generation() {
        register_mock_components();
        let mut master = ArchetypeMaster::new();

        let id = master.create_archetype(&mocks([1, 2]));
        let struct_before = master.structural_generation();

        // No-op removal of an unknown ID must NOT bump.
        assert!(!master.remove_archetype(ArchetypeId(9999)));
        assert_eq!(
            master.structural_generation(),
            struct_before,
            "structural_generation must not bump on failed removal"
        );

        // Successful removal MUST bump.
        assert!(master.remove_archetype(id));
        assert!(
            master.structural_generation() > struct_before,
            "structural_generation must bump on successful removal"
        );
    }

    /// `clear()` bumps the structural counter — it is the maximally-structural
    /// change possible (everything is gone + next_archetype_id resets to 1).
    #[test]
    fn clear_bumps_structural_generation() {
        register_mock_components();
        let mut master = ArchetypeMaster::new();
        master.create_archetype(&mocks([1]));
        master.create_archetype(&mocks([2]));

        let struct_before = master.structural_generation();
        master.clear();
        assert!(
            master.structural_generation() > struct_before,
            "structural_generation must bump on clear()"
        );
    }

    // --- EnableTag column-allocation epoch (W2 forward seam) ---

    /// A fresh master starts at `enable_generation == 0`.
    #[test]
    fn enable_generation_starts_at_zero() {
        let master = ArchetypeMaster::new();
        assert_eq!(master.enable_generation(), 0);
        let master = ArchetypeMaster::with_capacity(8);
        assert_eq!(master.enable_generation(), 0);
    }

    /// Each `note_enable_column_alloc` advances the epoch by exactly one — the
    /// "bumps once per column alloc" invariant the toggle path relies on
    /// (Decision D1 inv 5). Distinct `(tag, arch)` pairs satisfy the
    /// "genuine first column" assertion in `note_column_alloc`.
    #[test]
    fn enable_generation_bumps_exactly_once_per_call() {
        let mut master = ArchetypeMaster::new();
        let before = master.enable_generation();
        master.note_enable_column_alloc(ComponentId(20), ArchetypeId(1));
        assert_eq!(
            master.enable_generation(),
            before + 1,
            "one column alloc must advance the epoch by exactly one"
        );
        master.note_enable_column_alloc(ComponentId(20), ArchetypeId(2));
        master.note_enable_column_alloc(ComponentId(21), ArchetypeId(1));
        assert_eq!(master.enable_generation(), before + 3);
        // The presence oracle reflects every recorded column.
        assert!(master.enable_presence().contains(ComponentId(20), ArchetypeId(1)));
        assert!(master.enable_presence().contains(ComponentId(20), ArchetypeId(2)));
        assert!(master.enable_presence().contains(ComponentId(21), ArchetypeId(1)));
        assert!(!master.enable_presence().contains(ComponentId(21), ArchetypeId(2)));
    }

    /// `enable_generation` is independent of the structural generation: a column
    /// alloc must not move the structural generation (Decision D1 — a column
    /// alloc must not force a full structural rebuild).
    #[test]
    fn enable_generation_independent_of_structural_generation() {
        register_mock_components();
        let mut master = ArchetypeMaster::new();

        let struct_before = master.structural_generation();
        master.note_enable_column_alloc(ComponentId(22), ArchetypeId(7));
        assert_eq!(
            master.structural_generation(),
            struct_before,
            "a column alloc must not touch structural_generation"
        );

        // Conversely, a structural op must not move enable_generation.
        let enable_before = master.enable_generation();
        let id = master.create_archetype(&mocks([1, 2]));
        assert!(master.remove_archetype(id));
        assert_eq!(
            master.enable_generation(),
            enable_before,
            "structural ops must not touch enable_generation"
        );
    }

    // --- EnableTag W1: registry-mask bitset filter (C1-premise completeness) ---
    //
    // ID range 325-327 reserved for these tests. Grep-verified free within
    // `src/` (320-322 = archetype Step-4, 323-324 = enable_tag_api, the
    // archetype_master mock block is 300-308). 325-329 appear only in the
    // separate `tests/miri_phase8_5.rs` integration binary, which is a distinct
    // process from the lib unit-test binary this module compiles into, so there
    // is no `OnceLock`/`STORAGE_KIND` collision.
    const W1_TABLE: ComponentId = ComponentId(325);
    const W1_TAG: ComponentId = ComponentId(326);

    /// Registers a table component and classifies a sibling id as a bitset
    /// enable tag (write-once / idempotent across repeated test runs).
    fn register_w1_components() {
        component_registry::register_layout::<u32>(W1_TABLE.0);
        component_registry::register_layout::<u32>(W1_TAG.0);
        component_registry::set_storage_kind(W1_TAG.0, component_registry::StorageKind::Bitset);
    }

    /// W1 (C1-premise completeness): the REGISTRY-index mask built at
    /// `create_archetype` must filter out `StorageKind::Bitset` ids exactly as
    /// the archetype's own signature does. Without it, `create_archetype(&[T,
    /// Tag])` and `create_archetype(&[T])` register two distinct registry
    /// signatures for the structurally-identical archetype `{T}`, defeating
    /// `find_exact_match` dedup, and the registry signature carries a bitset bit
    /// the archetype's real signature lacks.
    ///
    /// This gap was invisible to every Wave-2 test because they all created
    /// table-only archetypes and reached the tag only by toggling (Phase-14b
    /// "behavioral coverage catches what per-plan APPROVED misses").
    #[test]
    fn create_archetype_dedups_table_only_against_table_plus_bitset_tag() {
        register_w1_components();
        let mut master = ArchetypeMaster::new();

        // (a) The table-only archetype and the table+bitset-tag archetype must
        //     dedup to the SAME ArchetypeId (structurally identical signature).
        let id_table_only = master.create_archetype(&[W1_TABLE]);
        let id_with_tag = master.create_archetype(&[W1_TABLE, W1_TAG]);
        assert_eq!(
            id_with_tag, id_table_only,
            "create_archetype(&[T, Tag]) must dedup against create_archetype(&[T]) — \
             the bitset tag is filtered out of the registry signature"
        );
        // Order-independent: tag-first also dedups (the filter precedes the lookup).
        let id_tag_first = master.create_archetype(&[W1_TAG, W1_TABLE]);
        assert_eq!(id_tag_first, id_table_only);
        // Exactly one archetype was minted.
        assert_eq!(master.archetype_count(), 1);

        // (b) The registry signature for that archetype contains NO bitset bit.
        let sig = master
            .archetype_registry()
            .get_archetype_signature(id_table_only)
            .expect("invariant: the archetype is registered");
        assert!(
            sig.mask().contains(W1_TABLE),
            "registry signature must contain the table id"
        );
        assert!(
            !sig.mask().contains(W1_TAG),
            "registry signature must NOT contain the bitset tag bit (C1 premise)"
        );

        // The archetype's own signature agrees, and it holds no pool for the tag.
        let arch = master
            .get_archetype(id_table_only)
            .expect("invariant: the archetype exists in the bundle");
        assert!(arch.has_component_id(W1_TABLE));
        assert!(
            !arch.has_component_id(W1_TAG),
            "archetype signature must exclude the bitset tag"
        );
        assert!(
            arch.component_pools().get_pool(W1_TAG).is_none(),
            "the bitset tag must have NO ComponentPool (C1 premise)"
        );
    }

    /// `get_or_create_archetype` applies the same registry-mask filter (the spawn
    /// funnel `cold_register_bundle_archetype` routes through it), so it returns
    /// the existing table-only archetype when handed the table+tag id set.
    #[test]
    fn get_or_create_archetype_dedups_through_bitset_filter() {
        register_w1_components();
        let mut master = ArchetypeMaster::new();

        let id_table_only = master.create_archetype(&[W1_TABLE]);
        let id_via_goc = master.get_or_create_archetype(&[W1_TABLE, W1_TAG]);
        assert_eq!(
            id_via_goc, id_table_only,
            "get_or_create_archetype(&[T, Tag]) must reuse the table-only archetype"
        );
        assert_eq!(master.archetype_count(), 1);
    }
}
