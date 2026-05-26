use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::core::system::params::entity_counter::MAX_BATCH_HINT;
use crate::ecs::error::{EcsError, EcsResult};
use crate::ecs::identifiers::primitives::EntityId;

/// Manages entity lifecycle, recycling, and Phase 7 fast-path lookup.
///
/// Layout (post Phase-11 Wave A atomic-counter migration):
///
/// - `entities_inland`: sparse, indexed by `EntityId.0`. A slot with
///   `archetype_ptr.is_null()` is dead. The slot's `generation` survives
///   deallocation so the next `allocate_entity` for that recycled id
///   returns `Entity::new(id, current_gen)`.
/// - `sparse_to_active`: parallel to `entities_inland`. Holds the
///   dense index into `active_ids` for live ids, `u32::MAX` for dead.
/// - `active_ids`: dense list of currently-live entity ids, used by
///   `iter_entities` and updated via swap-remove on deallocation.
/// - `free_entity_ids`: LIFO recycling queue for ids. Dispatcher-only (EM2);
///   workers never pop. Pre-sized in `EcsMaster::new` to absorb the bulk
///   of recycling without reallocation.
/// - `next_entity_id`: monotonic atomic counter for fresh-id minting.
///   Phase 11 (EM1): workers call into this through the `EntityCounter<'s>`
///   newtype (worker-safe — atomic RMW only). The dispatcher reads/bumps
///   through `&mut self` on `allocate_entity`.
pub struct EntityMaster {
    /// Pool of free entity IDs for reuse.
    free_entity_ids: Vec<EntityId>,

    /// Phase 11 (EM1, EM6): atomic counter for fresh entity-id minting.
    ///
    /// Workers reach this field exclusively through the
    /// [`crate::ecs::core::system::params::entity_counter::EntityCounter<'s>`]
    /// newtype (`*const AtomicUsize`), which restricts the reachable surface
    /// to atomic RMW only. The dispatcher's `&mut self`-bound
    /// `allocate_entity` performs `fetch_add(1, Relaxed)` on the same atomic.
    next_entity_id: AtomicUsize,

    /// Phase 7: dense-indexed fast-path lookup record, sized by max-ever
    /// `EntityId`. Slots with `archetype_ptr.is_null()` represent dead /
    /// never-registered IDs. Written by `register_entity_with_ptr`; read
    /// by the hot `get_component_raw` path in `EcsMaster`.
    ///
    /// `pub(crate)` for direct access from `EcsMaster::get_component_raw`
    /// and the Phase 7 hot read path. Outside the crate, the layout is opaque.
    pub(crate) entities_inland: Vec<EntityInland>,

    /// Phase 7: sparse → dense map. `sparse_to_active[entity_id.0]` gives
    /// the index into `active_ids`, or `u32::MAX` for "not active".
    ///
    /// `pub(crate)` for direct access from the Phase 7 hot read path.
    /// Outside the crate, the layout is opaque.
    pub(crate) sparse_to_active: Vec<u32>,

    /// Phase 7: dense list of currently-active `EntityId`s. Drives
    /// `iter_entities`. Order is registration-order with swap-remove on
    /// deallocation.
    ///
    /// `pub(crate)` for direct access from the Phase 7 hot read path.
    /// Outside the crate, the layout is opaque.
    pub(crate) active_ids: Vec<EntityId>,
}

impl EntityMaster {
    /// Creates a new empty EntityMaster.
    #[inline]
    pub fn new() -> Self {
        Self {
            free_entity_ids: Vec::new(),
            next_entity_id: AtomicUsize::new(0),
            entities_inland: Vec::new(),
            sparse_to_active: Vec::new(),
            active_ids: Vec::new(),
        }
    }

    /// Creates a new EntityMaster with pre-allocated capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            free_entity_ids: Vec::with_capacity(capacity / 4),
            next_entity_id: AtomicUsize::new(0),
            entities_inland: Vec::with_capacity(capacity),
            sparse_to_active: Vec::with_capacity(capacity),
            active_ids: Vec::with_capacity(capacity),
        }
    }

    /// Allocates a new entity or reuses a recycled one.
    ///
    /// Returns the allocated entity with the appropriate generation. For
    /// recycled ids the generation is read from the fast-store slot
    /// (`deallocate_entity` bumped it before nulling the archetype_ptr).
    /// Fresh ids start at generation 0.
    ///
    /// # Visibility (Phase 11 W2)
    ///
    /// `pub(crate)` — the only blessed public entrypoint for entity creation
    /// is [`crate::ecs::core::ecs_master::ecs_master::EcsMaster::create_entity`]
    /// (or `spawn_one` / `spawn_two`). Restricting privacy eliminates the
    /// risk that out-of-tree callers mint an `Entity` without registering
    /// it into the fast store, leaving a stranded `EntityId` and violating
    /// the EM2 invariant that recycling is dispatcher-exclusive.
    #[inline]
    pub(crate) fn allocate_entity(&mut self) -> Entity {
        if let Some(id) = self.free_entity_ids.pop() {
            // Recycled id: read its current generation from the fast store.
            // The slot was set to `is_null()` on deallocate_entity; the
            // generation field was bumped before nulling.
            debug_assert!(
                id.0 < self.entities_inland.len(),
                "Free entity ID out of bounds"
            );
            let current_gen = self.entities_inland[id.0].generation();
            Entity::new(id, current_gen)
        } else {
            // Phase 11 EM1: fresh-id minting through atomic fetch_add. We
            // hold `&mut self` so the load could be a plain read, but
            // routing through `fetch_add(Relaxed)` keeps a single source
            // of truth with the worker path (EntityCounter::reserve_entity)
            // and lets us share the counter without extra branches.
            let id_raw = self.next_entity_id.fetch_add(1, Ordering::Relaxed);
            let id = EntityId(id_raw);
            // Ensure the fast-store vectors have a slot for this id.
            if id.0 >= self.entities_inland.len() {
                self.entities_inland.resize(id.0 + 1, EntityInland::NULL);
            }
            if id.0 >= self.sparse_to_active.len() {
                self.sparse_to_active.resize(id.0 + 1, u32::MAX);
            }
            Entity::new(id, 0)
        }
    }

    /// Crate-internal accessor for `EntityCounter` construction inside
    /// [`crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::entity_counter`].
    ///
    /// Phase 11 Step 3 (Round 3 C-N1): the field-restriction invariant EM6
    /// is enforced by exposing ONLY the atomic counter — not the full
    /// `EntityMaster` — to worker code. Workers receive an
    /// [`crate::ecs::core::system::params::entity_counter::EntityCounter<'s>`]
    /// whose internal pointer type is `*const AtomicUsize`, so the type
    /// system rejects any attempt to project to a different `EntityMaster`
    /// field via this channel.
    #[inline]
    pub(crate) fn next_id_atomic(&self) -> &AtomicUsize {
        &self.next_entity_id
    }

    /// Reserves a fresh entity ID through an atomic increment — does NOT
    /// touch the free list (EM2).
    ///
    /// Used by dispatcher-side helpers that need a fresh ID without
    /// registering it into the fast store. Workers go through the
    /// [`crate::ecs::core::system::params::entity_counter::EntityCounter<'s>`]
    /// newtype which performs the same atomic RMW but cannot reach any
    /// other `EntityMaster` field (EM6).
    ///
    /// The current Phase 11 implementation routes worker reserves through
    /// `EntityCounter`; this dispatcher-facing alias is reserved for
    /// future opcode helpers (e.g. spawn-batch) that have a `&EntityMaster`
    /// in scope and skip the `UnsafeEcsCell` projection.
    ///
    /// # Atomic ordering
    ///
    /// `Ordering::Relaxed`: uniqueness only; happens-before for the
    /// returned id is established later by the apply-window barrier
    /// (every worker write is visible to the dispatcher via SCH7's join).
    #[inline]
    #[allow(dead_code)] // reserved for Phase 12 spawn-batch helpers (plan §15.2)
    pub(crate) fn reserve_entity(&self) -> Entity {
        let id = self.next_entity_id.fetch_add(1, Ordering::Relaxed);
        debug_assert!(id < usize::MAX / 2, "EntityId counter near exhaustion");
        Entity::new(EntityId(id), 0)
    }

    /// Phase 12.5 Opt-A2 (plan §5.7 / SBO14): atomically reserves a
    /// contiguous range of `n` fresh entity IDs through the world's
    /// counter — does NOT touch the free list (EM2).
    ///
    /// Validates `n ≤ MAX_BATCH_HINT` BEFORE any atomic operation. On
    /// overrun returns `Err(EcsError::SpawnBatchExceedsCapacity)`; **the
    /// counter is not advanced**.
    ///
    /// Routed through here (rather than poking `next_entity_id` directly)
    /// by the C-N2 lock-down: every `fetch_add` on the world counter goes
    /// through either this entry point or
    /// [`crate::ecs::core::system::params::entity_counter::EntityCounter::reserve_batch`].
    ///
    /// # Atomic ordering
    ///
    /// `Ordering::Relaxed` — uniqueness only. Happens-before for the
    /// returned IDs is established later by the apply-window barrier
    /// (SCH7).
    #[inline]
    pub(crate) fn reserve_batch(&self, n: usize) -> EcsResult<Range<usize>> {
        if n > MAX_BATCH_HINT {
            return Err(EcsError::SpawnBatchExceedsCapacity {
                requested: n,
                max: MAX_BATCH_HINT,
            });
        }
        let start = self.next_entity_id.fetch_add(n, Ordering::Relaxed);
        debug_assert!(
            start.checked_add(n).is_some_and(|end| end < usize::MAX / 2),
            "EntityId counter near exhaustion"
        );
        Ok(start..(start + n))
    }

    /// Phase 12.6 — grows the entity fast-store vectors so any index in
    /// `[0, capacity)` is in bounds.
    ///
    /// Called from dispatcher-only paths (`EcsMaster::spawn_batch` and
    /// `SpawnBatchCommand::apply`) BEFORE the apply window's per-row
    /// writes. The `&mut self` receiver enforces dispatcher exclusivity
    /// at the type-system level — workers cannot race a reallocation
    /// here against their `&self` reads (SEND5 / SBO16).
    ///
    /// Idempotent and cheap when already sized: only the `Vec::resize`
    /// branches do work. The extension memset is amortised O(n) across
    /// the world's lifetime (each slot is written once when promoted
    /// into the live range).
    ///
    /// Replaces the Phase 12.5 `EcsMaster::pre_sized_entity_master`
    /// eager pre-extension at world construction time. The 480 µs cost
    /// of that eager memset now happens lazily at the first batch
    /// dispatcher call; single-row spawns (which already grow lazily via
    /// `register_entity_with_ptr`) are unaffected.
    #[inline]
    pub(crate) fn ensure_capacity(&mut self, capacity: usize) {
        if self.entities_inland.len() < capacity {
            self.entities_inland.resize(capacity, EntityInland::NULL);
        }
        if self.sparse_to_active.len() < capacity {
            self.sparse_to_active.resize(capacity, u32::MAX);
        }
    }

    /// Phase 12.5 Opt-A2 (plan §5.7 / SBO15): registers a contiguous range
    /// of `n` entities in the Phase 7 fast store under dispatcher-only
    /// `&mut self` access.
    ///
    /// Writes `entities_inland`, `sparse_to_active`, and `active_ids` for
    /// every slot in `[start_entity.0, start_entity.0 + n)`. The slots
    /// MUST currently be NULL (caller contract: the range was just
    /// returned by `reserve_batch` and the IDs have not been used yet).
    ///
    /// All `n` entities share the same `archetype_ptr`; each receives
    /// `unit_index = start_row + i` (i.e. the rows landed contiguously in
    /// the archetype's pools by the batch path).
    ///
    /// # Preconditions (debug-asserted)
    ///
    /// * `start_entity.0 + n ≤ entities_inland.len()` (SBO16 — caller
    ///   pre-checks via SBO17 / SBO17b).
    /// * Every slot in the range is currently NULL.
    pub(crate) fn register_batch(
        &mut self,
        start_entity: EntityId,
        archetype_ptr: *mut Archetype,
        start_row: u32,
        n: usize,
    ) {
        if n == 0 {
            return;
        }
        let start = start_entity.0;
        let end = start.checked_add(n).expect(
            "register_batch: start_entity.0 + n overflows usize \
             (caller should have pre-checked via reserve_batch / SBO17)",
        );
        debug_assert!(
            end <= self.entities_inland.len(),
            "register_batch: range past entities_inland fast-store \
             (SBO16 violation — pre-check via SBO17/SBO17b should have caught this)"
        );
        debug_assert!(
            end <= self.sparse_to_active.len(),
            "register_batch: range past sparse_to_active fast-store"
        );

        // Hoist the per-slot NULL / sentinel sanity checks out of the
        // per-row loop. One scan in debug; zero cost in release.
        #[cfg(debug_assertions)]
        {
            for i in 0..n {
                let sparse_idx = start + i;
                debug_assert!(
                    self.entities_inland[sparse_idx].is_null(),
                    "register_batch: slot {} is already registered (SBO15 violation)",
                    sparse_idx
                );
                debug_assert!(
                    self.sparse_to_active[sparse_idx] == u32::MAX,
                    "register_batch: slot {} has stale sparse_to_active entry",
                    sparse_idx
                );
            }
        }

        // active_ids may grow by exactly `n`; reserve once outside the loop.
        self.active_ids.reserve(n);
        let dense_base = self.active_ids.len() as u32;
        debug_assert!(
            (dense_base as usize)
                .checked_add(n)
                .is_some_and(|v| v <= u32::MAX as usize),
            "register_batch: dense index range overflows u32"
        );

        // ── inland + sparse_to_active: tandem slice writes ──────────────
        // Acquire `&mut [T]` views over the disjoint slot ranges so the
        // compiler can hoist the bounds checks and vectorise the writes.
        let inland_slice = &mut self.entities_inland[start..end];
        let sparse_slice = &mut self.sparse_to_active[start..end];
        for (i, (inland, sparse)) in inland_slice
            .iter_mut()
            .zip(sparse_slice.iter_mut())
            .enumerate()
        {
            *inland = EntityInland::new(archetype_ptr, start_row + i as u32, 0);
            *sparse = dense_base + i as u32;
        }

        // ── active_ids: bulk extend from a TrustedLen Range<usize> ──────
        // `EntityId` is `#[repr(transparent)]` over `usize`, so the
        // `.map(EntityId)` closure compiles to a no-op and `Vec::extend`
        // fast-paths to `ptr::copy_nonoverlapping`.
        self.active_ids.extend((start..end).map(EntityId));
    }

    /// Phase 7 fast-path entity registration.
    ///
    /// Writes the entity into the fast store
    /// (`entities_inland` / `sparse_to_active` / `active_ids`).
    ///
    /// `archetype_ptr` MUST be obtained from
    /// `ArchetypeMaster::archetype_ptr_for` (write-capable provenance)
    /// and MUST be stable for the `EntityMaster`'s lifetime (plan
    /// invariants U1, U2). The pointer is stored verbatim; never
    /// dereferenced inside `EntityMaster`.
    ///
    /// `unit_index` is the row index in `Archetype.entity_ids` produced
    /// by the most recent `Archetype::create_entity` call.
    #[inline]
    pub fn register_entity_with_ptr(
        &mut self,
        entity: Entity,
        archetype_ptr: *mut Archetype,
        unit_index: u32,
    ) {
        let sparse_idx = entity.id().0;
        debug_assert!(
            sparse_idx < self.entities_inland.len(),
            "register_entity_with_ptr called before allocate_entity for this id"
        );
        debug_assert!(
            sparse_idx >= self.sparse_to_active.len()
                || self.sparse_to_active[sparse_idx] == u32::MAX,
            "Entity already present in Phase 7 fast store"
        );

        if sparse_idx >= self.entities_inland.len() {
            self.entities_inland.resize(sparse_idx + 1, EntityInland::NULL);
        }
        if sparse_idx >= self.sparse_to_active.len() {
            self.sparse_to_active.resize(sparse_idx + 1, u32::MAX);
        }

        self.entities_inland[sparse_idx] = EntityInland::new(
            archetype_ptr,
            unit_index,
            entity.generation(),
        );

        let dense_idx = self.active_ids.len() as u32;
        self.active_ids.push(entity.id());
        self.sparse_to_active[sparse_idx] = dense_idx;
    }

    /// Deallocates an entity, bumps its generation, and recycles its id.
    ///
    /// Returns `true` on success, `false` if the entity is stale or never
    /// registered. The generation is bumped IN PLACE on the fast-store slot
    /// before the slot's `archetype_ptr` is nulled — so the next
    /// `allocate_entity` for the same recycled id returns
    /// `Entity::new(id, bumped_gen)`.
    #[inline]
    pub fn deallocate_entity(&mut self, entity: Entity) -> bool {
        if !self.is_entity_valid(entity) {
            return false;
        }
        let entity_id = entity.id();
        let sparse_idx = entity_id.0;

        // Remove from active_ids dense list (swap-remove pattern).
        let dense_idx = self.sparse_to_active[sparse_idx];
        debug_assert!(dense_idx != u32::MAX, "is_entity_valid passed but sparse_to_active is u32::MAX");
        let last_dense = (self.active_ids.len() - 1) as u32;
        self.active_ids.swap_remove(dense_idx as usize);
        if dense_idx != last_dense {
            let moved_entity = self.active_ids[dense_idx as usize];
            self.sparse_to_active[moved_entity.0] = dense_idx;
        }
        self.sparse_to_active[sparse_idx] = u32::MAX;

        // Bump generation in place and null the archetype_ptr. The
        // generation must survive deallocation so the next allocate_entity
        // for this recycled id returns Entity::new(id, bumped_gen).
        let current_gen = self.entities_inland[sparse_idx].generation();
        let next_gen = current_gen.wrapping_add(1);
        self.entities_inland[sparse_idx] = EntityInland::new(
            std::ptr::null_mut(),
            0,
            next_gen,
        );

        self.free_entity_ids.push(entity_id);
        true
    }

    /// Checks if an entity handle is live (slot live + generation match).
    #[inline]
    pub fn is_entity_valid(&self, entity: Entity) -> bool {
        let Some(inland) = self.entities_inland.get(entity.id().0) else {
            return false;
        };
        !inland.is_null() && inland.generation() == entity.generation()
    }

    /// Resolves an entity by ID if it is currently active.
    ///
    /// Returns the `Entity` handle with the stored generation, or `None` if
    /// the id is out of bounds or its slot is dead.
    #[inline]
    pub fn get_entity(&self, entity_id: EntityId) -> Option<Entity> {
        let inland = self.entities_inland.get(entity_id.0)?;
        if inland.is_null() {
            return None;
        }
        Some(Entity::new(entity_id, inland.generation()))
    }

    /// Gets the total number of active entities.
    #[inline]
    pub fn entity_count(&self) -> usize {
        self.active_ids.len()
    }

    /// Gets the maximum-ever entity id (= capacity of the fast store).
    #[inline]
    pub fn capacity(&self) -> usize {
        self.entities_inland.len()
    }

    /// Gets the number of recycled entity IDs available for reuse.
    #[inline]
    pub fn recycled_entity_count(&self) -> usize {
        self.free_entity_ids.len()
    }

    /// Gets the next entity ID that would be allocated for a fresh slot.
    ///
    /// Reads the atomic counter with `Ordering::Relaxed` — observational
    /// only; no synchronization needed.
    #[inline]
    pub fn next_entity_id(&self) -> EntityId {
        EntityId(self.next_entity_id.load(Ordering::Relaxed))
    }

    /// Returns an iterator over all currently-active entities.
    ///
    /// Cost: O(active_count). Driven by `active_ids` (dense list of live
    /// entity IDs); the fast inland store is consulted once per active
    /// entity via direct index for the generation tag.
    pub fn iter_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.active_ids.iter().map(move |&id| {
            debug_assert!(
                id.0 < self.entities_inland.len(),
                "invariant: active id {} must be in entities_inland (len {})",
                id.0,
                self.entities_inland.len()
            );
            let inland = &self.entities_inland[id.0];
            debug_assert!(
                !inland.is_null(),
                "invariant: active id {} must point to a live fast-store slot",
                id.0
            );
            Entity::new(id, inland.generation())
        })
    }

    /// Clears all entities from the master.
    ///
    /// Resets the atomic counter under `&mut self` exclusivity — no
    /// synchronization needed; `Ordering::Relaxed` is sufficient.
    pub fn clear(&mut self) {
        self.free_entity_ids.clear();
        self.entities_inland.clear();
        self.sparse_to_active.clear();
        self.active_ids.clear();
        self.next_entity_id.store(0, Ordering::Relaxed);
    }

    /// Checks if the master is empty (no active entities).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.active_ids.is_empty()
    }

    /// Gets the total memory usage in bytes (approximate).
    pub fn memory_usage(&self) -> usize {
        self.free_entity_ids.capacity() * std::mem::size_of::<EntityId>()
            + self.entities_inland.capacity() * std::mem::size_of::<EntityInland>()
            + self.sparse_to_active.capacity() * std::mem::size_of::<u32>()
            + self.active_ids.capacity() * std::mem::size_of::<EntityId>()
    }

    /// Compacts the internal storage to minimize memory usage.
    pub fn compact(&mut self) {
        self.free_entity_ids.shrink_to_fit();

        // Note: we don't shrink the fast-store vectors because that would
        // require renumbering live ids. Instead, we sort the free list for
        // better cache usage on subsequent allocations.
        self.free_entity_ids.sort_unstable_by(|a, b| b.cmp(a)); // Reverse order for pop()
    }

    /// Rolls back the last `allocate_entity` call for a fresh ID (not a recycled one).
    ///
    /// # Invariant
    ///
    /// `rewind_allocate` must be called immediately after `allocate_entity` and
    /// before any other `EntityMaster` mutation, otherwise the
    /// `id == next_entity_id - 1` heuristic for fresh-ID rollback is unsound.
    /// The current single caller (`EcsMaster::create_entity` on guard failure)
    /// satisfies this contract by construction. If a second caller emerges,
    /// audit the contract or promote `rewind_allocate` to a token-based RAII
    /// guard.
    ///
    /// For recycled IDs (from `free_entity_ids`) this method has no effect and
    /// returns `false` — recycled IDs are returned to the free list by the
    /// caller (via `deallocate_entity`) if needed. In the single-caller context,
    /// `EcsMaster::create_entity` only calls this on the fresh-ID path (before
    /// `register_entity_with_ptr`), so the recycled case never occurs in practice.
    #[doc(hidden)]
    pub(crate) fn rewind_allocate(&mut self, entity: Entity) -> bool {
        let id = entity.id();
        // Fresh IDs are minted sequentially from next_entity_id; a fresh entity
        // is at `next_entity_id - 1` immediately after allocate_entity returns.
        // The fast-store length tracks the max-ever id, matching the role the
        // legacy `entities` Vec used to play.
        //
        // `&mut self` gives exclusive access to the counter, so the
        // load+store sequence below is race-free with worker
        // `EntityCounter::reserve_entity` calls (workers do not run during
        // the apply window per SCH7 — that is the only context in which
        // `rewind_allocate` is reachable).
        let current = self.next_entity_id.load(Ordering::Relaxed);
        if id.0 + 1 == current && id.0 < self.entities_inland.len() {
            // Verify it was never registered: a fresh id's slot starts as NULL
            // (just resized by allocate_entity). Anything else means a caller
            // registered it before calling rewind.
            debug_assert!(
                self.entities_inland[id.0].is_null(),
                "rewind_allocate called on a registered entity — invariant violated"
            );
            // Undo next_entity_id increment.
            self.next_entity_id.store(current - 1, Ordering::Relaxed);
            true
        } else {
            // Recycled ID path or stale call — caller must use deallocate_entity.
            false
        }
    }
}

impl Default for EntityMaster {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY (SEND5 — Phase 9 §2.4, §9.1):
//
// `EntityMaster` becomes `Send + Sync` under the Phase 9 contract:
//
//   - Hot worker paths take `&self` access (`is_entity_valid`, `get_entity`,
//     `iter_entities`, plus the inline reads from `entities_inland` /
//     `sparse_to_active` driven by `EcsMaster::get_component_raw`); these are
//     race-free as long as no concurrent structural mutation occurs.
//   - Structural mutation (`allocate_entity`, `deallocate_entity`,
//     `register_entity_with_ptr`, `clear`, `rewind_allocate`) takes
//     `&mut self` and runs only on the dispatcher under the apply window
//     (SCH7). The plan's Round 2 W3 mitigation is enforced by
//     `EcsMaster::new` pre-allocating to `MAX_ENTITIES_HINT` (= 64 000), so
//     steady-state worker reads never observe a mid-flight `Vec` realloc.
//   - The `*mut Archetype` raw pointers stored inside `EntityInland` slots
//     point into the `ArchetypeMaster`'s stable-address slab (SEND6) and are
//     never dereferenced inside `EntityMaster` itself.
unsafe impl Send for EntityMaster {}
unsafe impl Sync for EntityMaster {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::archetype::archetype::Archetype;

    /// Helper: mint a non-null, never-dereferenced `*mut Archetype` for tests
    /// that only exercise the entity-management bookkeeping (not the actual
    /// archetype storage). Phase 7 M1 migration recipe.
    fn dummy_archetype_ptr() -> *mut Archetype {
        core::ptr::NonNull::<Archetype>::dangling().as_ptr()
    }

    #[test]
    fn test_entity_allocation_fresh() {
        let mut master = EntityMaster::new();

        // Allocate first entity (fresh).
        let entity1 = master.allocate_entity();
        assert_eq!(entity1.id(), EntityId(0));
        assert_eq!(entity1.generation(), 0);

        // Allocate second entity (fresh).
        let entity2 = master.allocate_entity();
        assert_eq!(entity2.id(), EntityId(1));
        assert_eq!(entity2.generation(), 0);
    }

    // --- Phase 7 Step 10 (M1): tests rebuilt on the new `register_entity_with_ptr` API ---

    /// `register_entity_with_ptr` round-trip: after allocate + register the
    /// entity is reported live by `is_entity_valid`, the entity count tracks
    /// the dense active list, and `get_entity` resolves to the same handle.
    /// Rebuilt from the deleted `test_entity_registration`.
    #[test]
    fn register_entity_with_ptr_round_trip() {
        let mut em = EntityMaster::new();
        let entity = em.allocate_entity();

        em.register_entity_with_ptr(entity, dummy_archetype_ptr(), 7);

        assert!(em.is_entity_valid(entity), "registered entity must be valid");
        assert_eq!(em.entity_count(), 1, "active count must be 1 after one register");
        assert_eq!(em.get_entity(entity.id()), Some(entity),
            "get_entity must resolve to the same handle");
        // The fast-store inland reflects the registration parameters.
        let inland = em.entities_inland[entity.id().0];
        assert!(!inland.is_null(), "inland slot must be live after register");
        assert_eq!(inland.unit_index(), 7);
        assert_eq!(inland.generation(), entity.generation());
    }

    /// Deallocate then allocate: the ID is recycled and its generation is
    /// bumped. The pre-dealloc handle becomes stale and is rejected by
    /// `is_entity_valid`. Rebuilt from the deleted
    /// `test_entity_deallocation_and_reuse`.
    #[test]
    fn deallocate_then_allocate_recycles_id_with_bumped_generation() {
        let mut em = EntityMaster::new();
        let e0 = em.allocate_entity();
        em.register_entity_with_ptr(e0, dummy_archetype_ptr(), 0);
        assert!(em.is_entity_valid(e0));

        assert!(em.deallocate_entity(e0), "deallocate of a live entity must return true");
        assert!(!em.is_entity_valid(e0), "stale handle must be rejected after dealloc");
        assert_eq!(em.entity_count(), 0);
        assert_eq!(em.recycled_entity_count(), 1, "id must be on the free list");

        let e1 = em.allocate_entity();
        assert_eq!(e1.id(), e0.id(), "ID must be recycled (LIFO free list)");
        assert_eq!(
            e1.generation(),
            e0.generation().wrapping_add(1),
            "generation must bump on dealloc"
        );
        // Re-using e0 (stale) must still report invalid even after recycle.
        assert!(!em.is_entity_valid(e0),
            "stale pre-dealloc handle must remain invalid after id is recycled");
    }

    /// `iter_entities` must reflect only currently-live entities. After
    /// churning (allocate three, deallocate one), the iterator yields exactly
    /// the surviving two — order is registration-order with swap-remove.
    /// Rebuilt from the deleted `t_iter_entities_skips_recycled_slots` and
    /// `t_iter_entities_yields_correct_set_after_recycle`.
    #[test]
    fn iter_entities_yields_only_live_entities_after_churn() {
        let mut em = EntityMaster::new();
        let ptr = dummy_archetype_ptr();
        let e0 = em.allocate_entity();
        em.register_entity_with_ptr(e0, ptr, 0);
        let e1 = em.allocate_entity();
        em.register_entity_with_ptr(e1, ptr, 1);
        let e2 = em.allocate_entity();
        em.register_entity_with_ptr(e2, ptr, 2);

        assert!(em.deallocate_entity(e1), "dealloc of the middle entity must succeed");

        let live: Vec<_> = em.iter_entities().collect();
        assert_eq!(live.len(), 2, "exactly 2 entities must remain after one dealloc");
        assert!(live.contains(&e0), "e0 must still be reported by iter_entities");
        assert!(live.contains(&e2), "e2 must still be reported by iter_entities");
        assert!(!live.contains(&e1), "deallocated e1 must NOT appear in iter_entities");
    }

    /// `rewind_allocate` undoes a fresh `allocate_entity` (the C-007 guard
    /// path used by `EcsMaster::create_entity` on post-allocate failure).
    /// Calling it without a prior allocation must report `false` (the
    /// fresh-id heuristic doesn't fire). Rebuilt as the test-migration
    /// recipe equivalent of `test_entity_inland_update` (the legacy
    /// `update_entity_inland` is replaced by `EntityInland::set_unit_index`
    /// exercised inline).
    #[test]
    fn rewind_allocate_decrements_next_id_on_fresh_path() {
        let mut em = EntityMaster::new();
        assert_eq!(em.next_entity_id(), EntityId(0));

        let e = em.allocate_entity();
        assert_eq!(em.next_entity_id(), EntityId(1));

        // Rewind must succeed and restore `next_entity_id`.
        let rewound = em.rewind_allocate(e);
        assert!(rewound, "fresh-id rewind must succeed");
        assert_eq!(em.next_entity_id(), EntityId(0),
            "next_entity_id must roll back after rewind");
        assert_eq!(em.entity_count(), 0);

        // A second rewind on a stale entity must NOT decrement again.
        let rewound_again = em.rewind_allocate(e);
        assert!(!rewound_again,
            "rewind on a stale entity must report false (heuristic doesn't fire)");
        assert_eq!(em.next_entity_id(), EntityId(0),
            "next_entity_id must not be touched by a no-op rewind");
    }

    /// `EntityInland::set_unit_index` (used by `EcsMaster::delete_entity` on
    /// `RemoveOutcome::Swapped`) must update the live slot in place so that
    /// subsequent fast-path lookups see the new index. Reflects the test
    /// migration recipe — replacing the deleted legacy
    /// `update_entity_unit_index` path.
    #[test]
    fn set_unit_index_on_inland_updates_fast_store() {
        let mut em = EntityMaster::new();
        let e = em.allocate_entity();
        em.register_entity_with_ptr(e, dummy_archetype_ptr(), 5);

        // Direct field mutation via the pub(crate) accessor — mirrors what
        // `EcsMaster::delete_entity` does on the swap-remove path.
        em.entities_inland[e.id().0].set_unit_index(2);

        let stored = em.entities_inland[e.id().0];
        assert!(!stored.is_null(), "slot must remain live after unit_index update");
        assert_eq!(stored.unit_index(), 2, "unit_index must reflect the in-place update");
        assert_eq!(stored.generation(), e.generation(),
            "generation must NOT change as a side effect of set_unit_index");
    }

    /// Phase 11 EM4 / §4.6: `next_entity_id` is now `AtomicUsize`.
    /// Repeated calls to `allocate_entity` must yield strictly increasing
    /// ids on the fresh-path (proof that `fetch_add(1)` is wired
    /// correctly).
    #[test]
    fn atomic_counter_advances_on_fresh_allocation() {
        let mut em = EntityMaster::new();
        let e0 = em.allocate_entity();
        let e1 = em.allocate_entity();
        let e2 = em.allocate_entity();
        assert_eq!(e0.id(), EntityId(0));
        assert_eq!(e1.id(), EntityId(1));
        assert_eq!(e2.id(), EntityId(2));
        assert_eq!(em.next_entity_id(), EntityId(3));
    }

    /// Phase 11 §4.7: `reserve_entity` is the atomic-counter path used by
    /// `EntityCounter::reserve_entity`. Verify that it advances the same
    /// counter shared with `allocate_entity` (the two paths cannot
    /// produce duplicates).
    #[test]
    fn reserve_entity_shares_counter_with_allocate_entity() {
        let mut em = EntityMaster::new();
        let e0 = em.allocate_entity();
        let e1 = em.reserve_entity();
        let e2 = em.allocate_entity();
        // EM4 monotonicity: counter advanced by exactly one per call.
        assert_eq!(e0.id(), EntityId(0));
        assert_eq!(e1.id(), EntityId(1));
        assert_eq!(e2.id(), EntityId(2));
    }

    /// Phase 11 EM2: `reserve_entity` MUST NOT pop from the free list —
    /// only `allocate_entity` may. Workers calling reserve_entity from
    /// EntityCounter therefore never observe a recycled ID with a bumped
    /// generation (the bumped generation always travels through the
    /// dispatcher's `allocate_entity` path).
    #[test]
    fn reserve_entity_skips_free_list() {
        let mut em = EntityMaster::new();
        // Allocate then deallocate to populate the free list.
        let e0 = em.allocate_entity();
        em.register_entity_with_ptr(e0, dummy_archetype_ptr(), 0);
        assert!(em.deallocate_entity(e0));
        assert_eq!(em.recycled_entity_count(), 1, "free list now has one ID");

        // reserve_entity must NOT consume the recycled ID — it must
        // hand out a strictly fresh one.
        let e1 = em.reserve_entity();
        assert_eq!(e1.id(), EntityId(1), "fresh ID, not the recycled 0");
        assert_eq!(em.recycled_entity_count(), 1, "free list intact");
    }

    /// Test that `deallocate_entity` on a stale handle (generation mismatch)
    /// returns false and does not corrupt the active set. Covers the
    /// generation-mismatch leg of the deleted
    /// `test_entity_deallocation_and_reuse`.
    #[test]
    fn deallocate_entity_rejects_stale_generation_handle() {
        let mut em = EntityMaster::new();
        let e0 = em.allocate_entity();
        em.register_entity_with_ptr(e0, dummy_archetype_ptr(), 0);

        // Dealloc + recycle to bump the generation.
        assert!(em.deallocate_entity(e0));
        let e1 = em.allocate_entity();
        em.register_entity_with_ptr(e1, dummy_archetype_ptr(), 0);
        assert_ne!(e0.generation(), e1.generation(),
            "recycled entity must have a different generation than its predecessor");

        // Pre-recycle handle is stale; dealloc must reject it.
        assert!(!em.deallocate_entity(e0),
            "dealloc of stale handle must return false");
        // Live entity still alive.
        assert!(em.is_entity_valid(e1),
            "stale dealloc attempt must not invalidate the live recycled entity");
        assert_eq!(em.entity_count(), 1);
    }
}
