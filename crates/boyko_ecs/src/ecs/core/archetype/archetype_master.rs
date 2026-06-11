use crate::ecs::core::archetype::archetype_bundle::{ArchetypeBundle, ArchetypeBundleIterMut};
use crate::ecs::core::archetype::archetype_registry::ArchetypeRegistry;
use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::archetype::generation::ArchetypeGeneration;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::component::component_registry;
use crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use crate::ecs::core::component::observers::{
    ObserverFn, ObserverId, ObserverKind, ObserverRegistry,
};
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use crate::ecs::core::iters::legacy_query::Query as LegacyQuery;

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
        }
    }
    
    
    /// Creates a new archetype from a slice of component IDs
    /// Returns the ID of the created archetype
    pub fn create_archetype(&mut self, component_ids: &[ComponentId]) -> ArchetypeId {
        // First check if an archetype with exactly these components already exists
        let mask = ComponentMask::from_components(component_ids);
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
        for &cid in component_ids {
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

        // Create a mask from the component IDs
        let mask = ComponentMask::from_components(&component_ids);

        // Register with the registry
        self.registry.register_archetype(archetype_id, mask);

        // ── Phase 21 H1 — process-global "ever archetyped" mark (bypass arm) ──
        // Same mark as `create_archetype`: this is the second archetype-mint
        // funnel (the OBS-SEED2 bypass below exists for the same reason), so
        // components placed through it must also raise the global staleness
        // bit for `register_component_hooks`.
        for &cid in &component_ids {
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
            // Signal cache invalidation to every outstanding QueryState.
            // See struct-level doc on `structural_generation`.
            self.structural_generation.bump();
        }

        bundle_success
    }
    
    /// Finds or creates an archetype with the specified component IDs
    /// This is an optimized version that first tries to find an existing archetype
    pub fn get_or_create_archetype(&mut self, component_ids: &[ComponentId]) -> ArchetypeId {
        // First try to find an existing archetype with the exact components
        let mask = ComponentMask::from_components(component_ids);
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
}
