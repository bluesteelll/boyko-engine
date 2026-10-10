use std::ops::Range;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::core::entity::entity_reservoir::EntityReservoir;
use crate::ecs::core::entity::inland_store::InlandStore;
use crate::ecs::error::EcsResult;
use crate::ecs::identifiers::primitives::EntityId;

/// Manages entity lifecycle, recycling, and Phase 7 fast-path lookup.
///
/// Layout (three fields):
///
/// - `entities_inland`: sparse, indexed by `EntityId.0`. A slot with
///   `archetype_ptr.is_null()` is dead. The slot's `generation` survives
///   deallocation, and the recycled entry carries the bumped generation
///   verbatim. Read by the hot `get_component_raw` path in `EcsMaster`.
/// - `live_count`: count of currently-live entities, maintained under
///   `&mut self`.
/// - `reservoir`: the fresh-id counter and the recycled-entity stack
///   ([`EntityReservoir`]). EM2′: workers claim from the stack through
///   `EntityCounter` (one `fetch_sub`), so a population churned through
///   `Commands` recycles its ids instead of minting fresh ones forever; the
///   dispatcher pushes, pops and settles it under `&mut self`.
///
/// Phase X.D removed the `active_ids` (dense live list) and
/// `sparse_to_active` (sparse→dense map); their sole consumer was the cold
/// `iter_entities` API (zero hot callers), and the despawn swap-remove they
/// required is deleted with them. `iter_entities` now scans
/// `entities_inland` directly — O(capacity) instead of O(active); accepted
/// because real iteration goes through `Query`/archetype storage, never
/// through here.
// Cache-layout note (Phase X.G, re-laid by EM2′ plan D5): `#[repr(C)]` pins
// line 0 to `entities_inland` (48 B native: base/len/committed/reserve_request/
// vm) + `live_count` (8 B) — every create/delete/register touches it, and the
// hot pair (base, len) keeps offsets 0/8 (XG-B1). Line 1 is the reservoir
// (`align(64)`), holding BOTH RMW'd atomics: a worker spawn's `lock` RMW no
// longer invalidates the inland header every other worker's
// `get_component_raw` reads. Measured history: the repr(Rust) shuffle after
// the 24→32 B inland growth cost +6-10% on create/delete_entity_10k — this
// family is layout-sensitive, and `create/delete_entity_10k` is the bench that
// can overturn D5.
#[repr(C)]
pub struct EntityMaster {
    /// Phase 7: dense-indexed fast-path lookup record store — FIRST field:
    /// its hot pair (base, len) sits at offsets 0/8 of the master itself.
    ///
    /// Phase X.G: an [`InlandStore`] (address-stable reserve/commit growth)
    /// instead of a `Vec` — growth never reallocates, copies, or fills
    /// (deleting the g7b doubling-memcpy spike class); reads/indexed writes
    /// go through `Deref`/`DerefMut` with `Vec`-identical codegen.
    ///
    /// `pub(crate)` for direct access from `EcsMaster::get_component_raw`
    /// and the Phase 7 hot read path. Outside the crate, the layout is opaque.
    pub(crate) entities_inland: InlandStore,

    /// Count of currently-live entities. Maintained under `&mut self`
    /// (dispatcher, apply window SCH7). Replaces the removed `active_ids.len()`.
    live_count: usize,

    /// EM2′ / EM6′: the fresh-id counter and the claimable recycled-entity
    /// stack, on its own cache line.
    ///
    /// `pub(crate)` so [`crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::entity_counter`]
    /// can project `&raw const (*world).entity_master.reservoir` without an
    /// intermediate `&EntityMaster`. Workers reach it ONLY as the
    /// `*const EntityReservoir` inside
    /// [`crate::ecs::core::system::params::entity_counter::EntityCounter<'s>`]:
    /// no other `EntityMaster` field is reachable through that type.
    pub(crate) reservoir: EntityReservoir,
}

// Layout pins (plan D5 / O2). Native 64-bit only: under Miri `VmReservation`
// gains a `Layout` field (inland 64 B, reservoir at +128, size 256), and under
// loom the atomics are not 8 bytes.
#[cfg(all(not(miri), not(loom), target_pointer_width = "64"))]
const _: () = {
    assert!(core::mem::offset_of!(EntityMaster, entities_inland) == 0);
    assert!(core::mem::offset_of!(EntityMaster, live_count) == 48);
    assert!(core::mem::offset_of!(EntityMaster, reservoir) == 64);
    assert!(size_of::<EntityMaster>() == 192);
};

/// What [`EntityMaster::allocate_entity_ticketed`] actually did (plan D9).
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AllocSource {
    /// Minted from the fresh counter at generation 0.
    Fresh,
    /// Popped from the recycled stack, carrying its bumped generation.
    Recycled,
}

/// An allocation together with the branch that produced it — the only input
/// [`EntityMaster::rewind_allocate`] accepts, so a rewind undoes exactly what
/// the allocation did instead of inferring it from id arithmetic (R1).
///
/// Neither `Copy` nor `Clone`: rewinding the same allocation twice does not
/// compile.
#[must_use]
#[derive(Debug)]
pub(crate) struct AllocTicket {
    entity: Entity,
    source: AllocSource,
}

impl AllocTicket {
    /// The allocated handle.
    #[inline]
    pub(crate) fn entity(&self) -> Entity {
        self.entity
    }
}

impl EntityMaster {
    /// Creates a new empty EntityMaster.
    ///
    /// Phase X.G: construction pays NO syscall (both reservations — the inland
    /// store's and the recycled stack's — materialize lazily), bounded by the
    /// XG-B4 `EcsMaster::new` gate.
    #[inline]
    pub fn new() -> Self {
        let entities_inland = InlandStore::new();
        let reservoir = EntityReservoir::new(entities_inland.ceiling_slots());
        Self {
            entities_inland,
            live_count: 0,
            reservoir,
        }
    }

    /// Creates a new EntityMaster with pre-allocated capacity.
    ///
    /// Phase X.G: `capacity` slots are PRECOMMITTED (no growth events for the
    /// first `capacity` entities); a capacity above the default 67 M-slot
    /// ceiling sizes the reservation up instead of failing (R2-W2 option b —
    /// `Vec::with_capacity`'s "never refuse a satisfiable request" contract).
    /// The recycled stack precommits `capacity / 4` entries, as the former
    /// `Vec::with_capacity(capacity / 4)` did.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        let entities_inland = InlandStore::with_capacity(capacity);
        let mut reservoir = EntityReservoir::new(entities_inland.ceiling_slots());
        reservoir.precommit(capacity / 4);
        Self {
            entities_inland,
            live_count: 0,
            reservoir,
        }
    }

    /// Allocates a new entity or reuses a recycled one, and records which.
    ///
    /// A recycled entity comes off the stack with the generation
    /// `deallocate_entity` bumped (settling the previous phase's worker claims
    /// first, plan D3). Fresh ids start at generation 0.
    ///
    /// # Visibility (Phase 11 W2)
    ///
    /// `pub(crate)` — the only blessed public entrypoint for entity creation
    /// is [`crate::ecs::core::ecs_master::ecs_master::EcsMaster::create_entity`]
    /// (or `spawn_one` / `spawn_two`). Restricting privacy eliminates the
    /// risk that out-of-tree callers mint an `Entity` without registering
    /// it into the fast store, leaving a stranded `EntityId`.
    #[inline]
    pub(crate) fn allocate_entity_ticketed(&mut self) -> AllocTicket {
        if let Some(entity) = self.reservoir.pop_free() {
            self.debug_check_free_entry(entity, "pop_free");
            AllocTicket {
                entity,
                source: AllocSource::Recycled,
            }
        } else {
            // Phase 11 EM1: fresh-id minting through the SAME atomic the worker
            // claim uses — one source of truth, no extra branch.
            let entity = self.reservoir.mint_fresh();
            // Ensure the fast store has a slot for this id (X.G: no copy, no
            // fill — `ensure` self-gates on `len`).
            self.entities_inland.ensure(entity.id().0 + 1);
            AllocTicket {
                entity,
                source: AllocSource::Fresh,
            }
        }
    }

    /// [`allocate_entity_ticketed`](Self::allocate_entity_ticketed) for callers
    /// that never rewind.
    #[inline]
    pub(crate) fn allocate_entity(&mut self) -> Entity {
        self.allocate_entity_ticketed().entity
    }

    /// Claims an entity id through the reservoir — a recycled entity if one is
    /// claimable, otherwise a fresh id at generation 0 — without registering
    /// it into the fast store (EM2′).
    ///
    /// The UNGATED claim: each call stands alone. Used by hooks'
    /// `DeferredCommands::spawn` on the dispatcher; workers go through
    /// [`crate::ecs::core::system::params::entity_counter::EntityCounter<'s>`]'s
    /// gated claim, which reaches only the reservoir (EM6′).
    ///
    /// # Atomic ordering
    ///
    /// `Ordering::Relaxed`: uniqueness only; happens-before for the
    /// returned id is established later by the apply-window barrier
    /// (every worker write is visible to the dispatcher via SCH7's join).
    #[inline]
    pub(crate) fn reserve_entity(&self) -> Entity {
        self.reservoir.claim()
    }

    /// Phase 12.5 Opt-A2 (plan §5.7 / SBO14): atomically reserves a
    /// contiguous range of `n` FRESH entity IDs — batches stay fresh and
    /// contiguous in Stage A (EM2′ plan D7), so this never claims from the
    /// recycled stack.
    ///
    /// Validates `n ≤ MAX_BATCH_HINT` BEFORE any atomic operation. On
    /// overrun returns `Err(EcsError::SpawnBatchExceedsCapacity)`; **the
    /// counter is not advanced**.
    ///
    /// # Atomic ordering
    ///
    /// `Ordering::Relaxed` — uniqueness only. Happens-before for the
    /// returned IDs is established later by the apply-window barrier
    /// (SCH7).
    #[inline]
    pub(crate) fn reserve_batch(&self, n: usize) -> EcsResult<Range<usize>> {
        self.reservoir.mint_fresh_batch(n)
    }

    /// Phase 12.6 — grows the entity fast store so any index in
    /// `[0, capacity)` is in bounds.
    ///
    /// Called from dispatcher-only paths (`EcsMaster::spawn_batch` and
    /// `SpawnBatchCommand::apply`) BEFORE the apply window's per-row
    /// writes. The `&mut self` receiver enforces dispatcher exclusivity
    /// at the type-system level (SEND5 / SBO16) — and since Phase X.G the
    /// store's base address is additionally write-once (no reallocation
    /// exists at all).
    ///
    /// Phase X.G: `InlandStore::ensure` — idempotent, O(1) warm; growth is a
    /// rare cold frontier commit with ZERO copies and ZERO fills (newly
    /// exposed slots read NULL via the invariant-J zero-tail; there is no
    /// extension memset anymore).
    #[inline]
    pub(crate) fn ensure_capacity(&mut self, capacity: usize) {
        self.entities_inland.ensure(capacity);
    }

    /// Phase 12.5 Opt-A2 (plan §5.7 / SBO15): registers a contiguous range
    /// of `n` entities in the Phase 7 fast store under dispatcher-only
    /// `&mut self` access.
    ///
    /// Writes `entities_inland` for every slot in
    /// `[start_entity.0, start_entity.0 + n)` and bumps `live_count` by `n`.
    /// The slots MUST currently be NULL (caller contract: the range was just
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

        // Hoist the per-slot NULL sanity check out of the per-row loop.
        // One scan in debug; zero cost in release.
        #[cfg(debug_assertions)]
        {
            for i in 0..n {
                let sparse_idx = start + i;
                debug_assert!(
                    self.entities_inland[sparse_idx].is_null(),
                    "register_batch: slot {} is already registered (SBO15 violation)",
                    sparse_idx
                );
            }
        }

        // ── inland: slice write ─────────────────────────────────────────
        // Acquire a `&mut [T]` view over the slot range so the compiler can
        // hoist the bounds checks and vectorise the writes.
        let inland_slice = &mut self.entities_inland[start..end];
        for (i, inland) in inland_slice.iter_mut().enumerate() {
            *inland = EntityInland::new(archetype_ptr, start_row + i as u32, 0);
        }

        self.live_count += n;
    }

    /// Phase 7 fast-path entity registration.
    ///
    /// Writes the entity into the fast store (`entities_inland`) and bumps
    /// `live_count`.
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
            self.entities_inland.get(sparse_idx).is_none_or(|i| i.is_null()),
            "Entity already present in Phase 7 fast store"
        );

        // Defensive growth (unreachable in practice — the debug_assert above
        // requires allocate_entity to have sized the slot already).
        self.entities_inland.ensure(sparse_idx + 1);

        self.entities_inland[sparse_idx] = EntityInland::new(
            archetype_ptr,
            unit_index,
            entity.generation(),
        );

        self.live_count += 1;
    }

    /// Deallocates an entity, bumps its generation, and recycles it.
    ///
    /// Returns `true` on success, `false` if the entity is stale or never
    /// registered. The generation is bumped IN PLACE on the fast-store slot
    /// before the slot's `archetype_ptr` is nulled, and the recycled entry
    /// carries the bumped generation — so whoever claims or pops the id next
    /// (a worker's `Commands::spawn` or the dispatcher's `allocate_entity`)
    /// receives `Entity::new(id, bumped_gen)`.
    #[inline]
    pub fn deallocate_entity(&mut self, entity: Entity) -> bool {
        if !self.is_entity_valid(entity) {
            return false;
        }
        let entity_id = entity.id();
        let sparse_idx = entity_id.0;

        // Bump generation in place and null the archetype_ptr. The
        // generation must survive deallocation: the dead slot and the
        // recycled entry agree on it (F3).
        let current_gen = self.entities_inland[sparse_idx].generation();
        let next_gen = current_gen.wrapping_add(1);
        self.entities_inland[sparse_idx] = EntityInland::new(
            std::ptr::null_mut(),
            0,
            next_gen,
        );

        self.reservoir.push_free(Entity::new(entity_id, next_gen));

        // Decrement only on the success path: the `is_entity_valid` early
        // return above skips never-registered recycled ids, for which
        // `register*` never incremented `live_count`.
        debug_assert!(
            self.live_count > 0,
            "deallocate_entity: live_count underflow (unbalanced register/deallocate accounting)"
        );
        self.live_count -= 1;
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
        self.live_count
    }

    /// Gets the maximum-ever entity id (= capacity of the fast store).
    #[inline]
    pub fn capacity(&self) -> usize {
        self.entities_inland.len()
    }

    /// Gets the number of recycled entity IDs available for reuse — the
    /// claimable length of the recycled stack, which equals its length at rest
    /// (between frames). Inside a phase it already excludes what workers have
    /// claimed.
    #[inline]
    pub fn recycled_entity_count(&self) -> usize {
        self.reservoir.claimable()
    }

    /// Gets the next entity ID that would be allocated for a fresh slot.
    ///
    /// Reads the atomic counter with `Ordering::Relaxed` — observational
    /// only; no synchronization needed.
    #[inline]
    pub fn next_entity_id(&self) -> EntityId {
        EntityId(self.reservoir.next_fresh())
    }

    /// Raw `free_top` of the recycled stack, negative drift included.
    ///
    /// Test / loom probe, not API (precedent: the `term_list` gate shims): its
    /// drift below zero counts exactly the claims that found the stack empty
    /// since the last settle, so the EXHAUSTED bit's RMW saving is pinned as an
    /// integer rather than a timing.
    #[doc(hidden)]
    #[inline]
    pub fn free_top_raw(&self) -> isize {
        self.reservoir.free_top_raw()
    }

    /// Returns an iterator over all currently-active entities.
    ///
    /// Cost: O(capacity) — scans the Phase-7 fast store and skips dead
    /// (`is_null`) slots. Yields live entities in ascending `EntityId`
    /// order. Real entity iteration goes through `Query`/archetype storage,
    /// never through here; this is a cold inspection/test API.
    pub fn iter_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entities_inland
            .iter()
            .enumerate()
            .filter(|(_, inland)| !inland.is_null())
            .map(|(i, inland)| Entity::new(EntityId(i), inland.generation()))
    }

    /// Clears all entities from the master.
    ///
    /// Resets the counter and the recycled stack under `&mut self`
    /// exclusivity — no synchronization needed.
    pub fn clear(&mut self) {
        self.reservoir.clear();
        self.entities_inland.clear();
        self.live_count = 0;
    }

    /// Checks if the master is empty (no active entities).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.live_count == 0
    }

    /// Gets the total memory usage in bytes (approximate).
    ///
    /// Phase X.G: reports RESIDENT truth — the committed frontiers of the
    /// entity store and of the recycled stack, not their (multi-GB,
    /// cost-free) address reservations.
    pub fn memory_usage(&self) -> usize {
        self.reservoir.free_committed_bytes()
            + self.entities_inland.committed_slots() * std::mem::size_of::<EntityInland>()
    }

    /// Commit frontier of the entity fast store, in slots (diagnostics —
    /// mirror of `ComponentPool::committed_rows`).
    #[inline]
    pub fn committed_slots(&self) -> usize {
        self.entities_inland.committed_slots()
    }

    /// Orders the recycled stack so the lowest ids are reused first.
    ///
    /// No storage is shrunk. The fast store cannot be: that would require
    /// renumbering live ids, and Phase X.G adds a second reason — no decommit
    /// of `[0, len)` is EVER legal, because recycled-dead slots are non-zero
    /// (bumped generations), so re-zeroing them would alias entities
    /// (invariant J's caveat). The recycled stack keeps its commit frontier
    /// for the same reuse reason every other `VmColumn` does.
    pub fn compact(&mut self) {
        self.reservoir.sort_free_low_ids_first();
    }

    /// Undoes an allocation that was never registered, following the branch
    /// the ticket records (R1 / plan D9) — never inferring it from the id.
    ///
    /// * **Fresh** — `id + 1 == next_entity_id` rolls the counter back. Any
    ///   other state means an id was minted after this one; rolling back would
    ///   re-issue it, so the id is LEAKED instead.
    /// * **Recycled** — the slot must still be null with the entry's
    ///   generation (release check); the entity then goes back on the recycled
    ///   stack. Otherwise the id is LEAKED.
    ///
    /// Returns `true` when the id was restored and `false` when it was leaked.
    /// A refusal is a bug in the caller (debug builds panic on it); release
    /// builds leak because a leaked id is bounded and safe, while a wrong
    /// restore is generation ABA.
    ///
    /// The ticket is consumed, so a second rewind of one allocation does not
    /// compile. The only caller is `EcsMaster::create_entity`'s rejection path,
    /// which runs between `allocate_entity_ticketed` and any other entity
    /// mutation.
    #[cold]
    pub(crate) fn rewind_allocate(&mut self, ticket: AllocTicket) -> bool {
        let AllocTicket { entity, source } = ticket;
        let id = entity.id().0;
        match source {
            AllocSource::Fresh => {
                if id + 1 == self.reservoir.next_fresh_mut() {
                    debug_assert!(
                        self.entities_inland.get(id).is_some_and(|slot| slot.is_null()),
                        "rewind_allocate: fresh id {id} was registered before the rewind"
                    );
                    self.reservoir.unmint_fresh_mut();
                    true
                } else {
                    Self::rewind_refused(entity, source)
                }
            }
            AllocSource::Recycled => {
                let restorable = self
                    .entities_inland
                    .get(id)
                    .is_some_and(|slot| slot.is_null() && slot.generation() == entity.generation());
                if restorable {
                    self.reservoir.unpop(entity);
                    true
                } else {
                    Self::rewind_refused(entity, source)
                }
            }
        }
    }

    /// The refusal arm of [`rewind_allocate`](Self::rewind_allocate): leak,
    /// and in a debug build say so.
    #[cold]
    #[inline(never)]
    fn rewind_refused(entity: Entity, source: AllocSource) -> bool {
        if cfg!(debug_assertions) {
            panic!(
                "rewind_allocate refused the {source:?} ticket for {entity:?}: the entity store \
                 changed since the allocation, so the id is leaked rather than re-issued (R1)"
            );
        }
        false
    }

    /// F3 for one entry: its slot is dead and carries its generation.
    #[inline]
    fn debug_check_free_entry(&self, e: Entity, site: &'static str) {
        debug_assert!(
            self.entities_inland
                .get(e.id().0)
                .is_some_and(|slot| slot.is_null() && slot.generation() == e.generation()),
            "F3 violated at {site}: recycled entry {e:?} does not match its dead slot"
        );
    }

    /// Walks F1–F3 over the whole recycled stack, O(n), and panics on the
    /// first violation. Test / proptest surface, not API.
    ///
    /// Settles first (F1 is the post-settle statement), so it is `&mut`.
    #[doc(hidden)]
    pub fn check_invariants(&mut self) {
        let claimable = self.reservoir.claimable();
        let entries = self.reservoir.settled_entries();
        assert_eq!(entries.len(), claimable, "F1: settled length != claimable length");
        let mut seen = crate::ecs::memory::vm_column::VmColumn::<EntityId>::new(
            "EntityMaster::check_invariants",
            entries.len().max(1),
        );
        for (i, e) in entries.iter().enumerate() {
            let slot = self.entities_inland.get(e.id().0).unwrap_or_else(|| {
                panic!("F3: entry {i} ({e:?}) is past the fast store (len {})", self.entities_inland.len())
            });
            assert!(slot.is_null(), "F3: entry {i} ({e:?}) names a LIVE slot");
            assert_eq!(slot.generation(), e.generation(), "F3: entry {i} ({e:?}) generation drift");
            seen.push(e.id());
        }
        let ids = seen.as_mut_slice();
        ids.sort_unstable();
        assert!(
            ids.windows(2).all(|w| w[0] != w[1]),
            "F3: the recycled stack holds a duplicate id"
        );
    }
}

impl Default for EntityMaster {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY (SEND5 — Phase 9 §2.4, §9.1; updated Phase X.D and EM2′):
//
// `EntityMaster` is `Send + Sync` under the Phase 9 contract. The struct
// holds: `entities_inland`, `live_count`, `reservoir`.
//
//   - Hot worker paths take `&self` (`is_entity_valid`, `get_entity`, plus the
//     inline `entities_inland` reads driven by `EcsMaster::get_component_raw`).
//     These are race-free as long as no concurrent structural mutation runs.
//   - Structural mutation (`allocate_entity*`, `deallocate_entity`,
//     `register_entity_with_ptr`, `register_batch`, `ensure_capacity`,
//     `clear`, `compact`, `rewind_allocate`) takes `&mut self` and runs only
//     on the dispatcher inside the apply window (SCH7); no worker is in
//     flight, so a worker `&self` read can never race a structural mutation.
//     Phase X.G makes the no-mid-flight-reallocation clause STRUCTURAL as
//     well: `entities_inland` and the recycled stack both live on write-once
//     reservations (growth commits fresh pages at the frontier — no pointer is
//     ever invalidated). This is defense-in-depth, NOT a relaxation: `len`
//     fields are plain non-atomic usizes, so a concurrent dispatcher-grow vs
//     worker-read would still be a data race — SCH7 exclusivity (and, for the
//     recycled stack, EM2′-K) remains the normative argument.
//   - `reservoir` is the ONLY field a worker mutates, and only through its two
//     atomics, reached solely as `*const EntityReservoir` through
//     `EntityCounter<'s>` (EM6′). Its plain parts (`free.base`, the entries)
//     are read-only in a phase; `EntityReservoir`'s own `Sync` impl carries
//     that argument.
//   - `live_count: usize` is dispatcher-only (`&mut self`); no worker reaches it.
//   - The `*mut Archetype` raw pointers inside `EntityInland` slots point into
//     the `ArchetypeMaster`'s stable-address slab (SEND6) and are never
//     dereferenced inside `EntityMaster` itself.
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
    /// the surviving two — order is ascending `EntityId` (post Phase-X.D the
    /// scan walks `entities_inland`). This test uses `.contains()`, so it is
    /// order-insensitive. Rebuilt from the deleted
    /// `t_iter_entities_skips_recycled_slots` and
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

    /// `rewind_allocate` undoes a fresh `allocate_entity_ticketed` (the C-007
    /// guard path used by `EcsMaster::create_entity` on post-allocate
    /// failure). The former "a second rewind of the same entity reports
    /// false" leg is gone with the id-arithmetic heuristic it tested: the
    /// ticket is consumed by the first rewind and is neither `Copy` nor
    /// `Clone`, so a second rewind of one allocation no longer compiles
    /// (plan D9).
    #[test]
    fn rewind_allocate_decrements_next_id_on_fresh_path() {
        let mut em = EntityMaster::new();
        assert_eq!(em.next_entity_id(), EntityId(0));

        let ticket = em.allocate_entity_ticketed();
        assert_eq!(ticket.source, AllocSource::Fresh);
        assert_eq!(em.next_entity_id(), EntityId(1));

        // Rewind must succeed and restore `next_entity_id`.
        let rewound = em.rewind_allocate(ticket);
        assert!(rewound, "fresh-id rewind must succeed");
        assert_eq!(em.next_entity_id(), EntityId(0),
            "next_entity_id must roll back after rewind");
        assert_eq!(em.entity_count(), 0);
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

    /// EM2′ (inverts Phase 11's `reserve_entity_skips_free_list`, the test
    /// that pinned the deferred-route leak's cause): `reserve_entity` claims
    /// the recycled entity — with its bumped generation — before it mints a
    /// fresh id, and the next `&mut` op settles the claim off the stack.
    #[test]
    fn reserve_entity_claims_free_list_before_minting() {
        let mut em = EntityMaster::new();
        // Allocate then deallocate to populate the free list.
        let e0 = em.allocate_entity();
        em.register_entity_with_ptr(e0, dummy_archetype_ptr(), 0);
        assert!(em.deallocate_entity(e0));
        assert_eq!(em.recycled_entity_count(), 1, "free list now has one ID");

        let e1 = em.reserve_entity();
        assert_eq!(e1, Entity::new(e0.id(), e0.generation() + 1),
            "the claim must return the recycled id with its bumped generation");
        assert_eq!(em.recycled_entity_count(), 0, "the claim consumed the entry");

        let e2 = em.reserve_entity();
        assert_eq!(e2, Entity::new(EntityId(1), 0), "an empty stack mints fresh");

        // The dispatcher's next allocation must not re-issue the claimed id.
        let e3 = em.allocate_entity();
        assert_eq!(e3, Entity::new(EntityId(2), 0),
            "allocate_entity after the claims must mint, not re-pop the claimed entry");
        assert_eq!(em.free_top_raw(), 0, "settle clamped the overshoot back to 0");
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

    // --- Phase X.D: live_count accounting + inland-scan iter_entities ---

    /// `live_count` (via `entity_count`) tracks register/deallocate balance.
    #[test]
    fn live_count_tracks_register_and_deallocate() {
        let mut em = EntityMaster::new();
        let ptr = dummy_archetype_ptr();
        let e0 = em.allocate_entity();
        em.register_entity_with_ptr(e0, ptr, 0);
        let e1 = em.allocate_entity();
        em.register_entity_with_ptr(e1, ptr, 1);
        let e2 = em.allocate_entity();
        em.register_entity_with_ptr(e2, ptr, 2);
        assert_eq!(em.entity_count(), 3, "three registers must yield live_count == 3");

        assert!(em.deallocate_entity(e1));
        assert_eq!(em.entity_count(), 2, "one dealloc must drop live_count to 2");

        assert!(em.deallocate_entity(e0));
        assert!(em.deallocate_entity(e2));
        assert_eq!(em.entity_count(), 0, "all dealloc'd → live_count == 0");
        assert!(em.is_empty(), "is_empty must agree with live_count == 0");
    }

    /// `register_batch` bumps `live_count` by `n` and writes the inland
    /// slots for the contiguous range.
    #[test]
    fn register_batch_sets_live_count() {
        let mut em = EntityMaster::new();
        let n = 5usize;
        em.ensure_capacity(n);
        em.register_batch(EntityId(0), dummy_archetype_ptr(), 0, n);

        assert_eq!(em.entity_count(), 5, "register_batch must set live_count to n");
        for i in 0..n {
            let inland = em.entities_inland[i];
            assert!(!inland.is_null(), "slot {} must be live after register_batch", i);
            assert_eq!(
                inland.unit_index(),
                i as u32,
                "slot {} must carry the contiguous unit index",
                i
            );
        }
    }

    /// Locks the Phase-X.D ordering contract: after churn, `iter_entities`
    /// yields the survivors in ascending `EntityId` order.
    #[test]
    fn iter_entities_after_sparse_churn_yields_survivors_ascending() {
        let mut em = EntityMaster::new();
        let ptr = dummy_archetype_ptr();
        let mut handles = Vec::with_capacity(5);
        for i in 0..5 {
            let e = em.allocate_entity();
            em.register_entity_with_ptr(e, ptr, i);
            handles.push(e);
        }

        // Deallocate the even-id entities (0, 2, 4).
        assert!(em.deallocate_entity(handles[0]));
        assert!(em.deallocate_entity(handles[2]));
        assert!(em.deallocate_entity(handles[4]));

        let survivors: Vec<EntityId> = em.iter_entities().map(|e| e.id()).collect();
        assert_eq!(
            survivors,
            vec![EntityId(1), EntityId(3)],
            "iter_entities must yield the odd survivors in ascending id order"
        );
    }

    /// `clear` resets `live_count`, emptiness, and capacity.
    #[test]
    fn clear_resets_live_count() {
        let mut em = EntityMaster::new();
        let ptr = dummy_archetype_ptr();
        for i in 0..3 {
            let e = em.allocate_entity();
            em.register_entity_with_ptr(e, ptr, i);
        }
        assert_eq!(em.entity_count(), 3);

        em.clear();
        assert_eq!(em.entity_count(), 0, "clear must reset live_count");
        assert!(em.is_empty(), "clear must leave the master empty");
        assert_eq!(em.capacity(), 0, "clear must drop the fast-store capacity");
    }

    /// CRITIC C1 regression: deallocating a recycled-but-never-registered id
    /// is a no-op that must NOT decrement `live_count` (the decrement sits
    /// after the `is_entity_valid` guard).
    #[test]
    fn deallocate_unregistered_recycled_id_is_noop_and_preserves_live_count() {
        let mut em = EntityMaster::new();
        let e0 = em.allocate_entity();
        em.register_entity_with_ptr(e0, dummy_archetype_ptr(), 0);
        assert!(em.deallocate_entity(e0), "first dealloc must succeed");
        assert_eq!(em.entity_count(), 0, "live_count back to 0 after dealloc");

        // Recycle id 0: pops the free list, slot stays NULL, NOT registered.
        let e_recycled = em.allocate_entity();
        assert_eq!(e_recycled.id(), e0.id(), "id must be recycled");
        assert!(
            !em.is_entity_valid(e_recycled),
            "recycled-but-unregistered id must be invalid (NULL slot)"
        );

        // Dealloc on the never-registered recycled id is a no-op.
        assert!(
            !em.deallocate_entity(e_recycled),
            "dealloc of an unregistered recycled id must return false"
        );
        assert_eq!(
            em.entity_count(),
            0,
            "live_count must NOT be decremented on the never-registered no-op path"
        );
    }

    /// CRITIC W1 tripwire: `live_count` must equal the number of non-null
    /// inland slots after arbitrary churn.
    #[test]
    fn live_count_equals_non_null_inland_count_after_churn() {
        let mut em = EntityMaster::new();
        let ptr = dummy_archetype_ptr();
        let mut handles = Vec::with_capacity(6);
        for i in 0..6 {
            let e = em.allocate_entity();
            em.register_entity_with_ptr(e, ptr, i);
            handles.push(e);
        }

        assert!(em.deallocate_entity(handles[1]));
        assert!(em.deallocate_entity(handles[4]));

        let non_null = em.entities_inland.iter().filter(|i| !i.is_null()).count();
        assert_eq!(
            em.entity_count(),
            non_null,
            "live_count must match the non-null inland slot count after churn"
        );
    }

    /// Phase X.G (XG-B6, EntityMaster level) — the slot ADDRESS of an early
    /// entity is stable while the store grows across several commit slabs.
    /// Impossible with the pre-X.G `Vec` (every doubling relocated the
    /// buffer).
    #[test]
    fn xg_b6_slot_address_stable_across_growth() {
        let mut em = EntityMaster::new();
        let first = em.allocate_entity();
        em.register_entity_with_ptr(first, EntityInland::dangling_for_test(1, 0).archetype_ptr(), 0);
        let addr_before = &em.entities_inland[first.id().0] as *const EntityInland as usize;

        // Grow far past the first 256 KiB slab (16,384 slots).
        em.ensure_capacity(100_000);

        let addr_after = &em.entities_inland[first.id().0] as *const EntityInland as usize;
        assert_eq!(
            addr_before, addr_after,
            "entity slot address moved across growth — X.G no-realloc contract broken"
        );
        assert!(em.committed_slots() >= 100_000);
        // Never-written tail reads NULL (invariant J through the public type).
        assert!(em.entities_inland[99_999].is_null());
    }
}
