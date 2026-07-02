//! [`DenseStore`] — the global, non-fragmenting storage for one dense component
//! type (Dense plan, Decisions 1 / 3 / 8, D1).
//!
//! One `DenseStore` per dense `ComponentId` holds every instance of that type
//! across all archetypes in ONE contiguous column. Membership is keyed by
//! `EntityId` through a sparse map; deletion is tombstone + free-list, never
//! swap-remove, so **live slots never move** — the determinism contract the
//! colored physics solver depends on (Dense plan C3).
//!
//! D1 builds the data structure and its structural ops in isolation. Routing
//! the engine's spawn/insert/remove through it (D2), query integration (D3),
//! ticks + serde (D4), and the physics consumer (Stage P) land later and do
//! NOT touch this module's invariant.

use std::cell::UnsafeCell;

use boyko_utils::sparse_map::sparse_map::SparseMap;

use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, EntityId};
use crate::ecs::memory::component_pool::ComponentPool;

use super::live_bitmap::LiveBitmap;
use super::views::{DenseBuildView, DenseSolveView};

/// Reserved `EntityId` value stamped into `s2e` for tombstoned (freed) slots.
///
/// `usize::MAX` can never be a real entity index: `EntityMaster` mints ids from
/// a `fetch_add(1)` counter starting at 0, which would overflow `usize` long
/// before reaching this value. A tombstone marker lets `s2e`-driven iteration
/// recognise a freed slot even before consulting `live`, and serves as the
/// serde/debug "this slot is dead" sentinel (Dense plan, Data structures).
pub(crate) const TOMBSTONE: EntityId = EntityId(usize::MAX);

/// Global dense column for one component type + its `EntityId`-keyed bookkeeping.
///
/// # Invariant (debug-asserted + property-tested)
///
/// For every slot `s`:
/// * `s` is **live** ⟺ `live.test(s)` ⟺ `s ∉ free` ⟺ `s2e[s] != TOMBSTONE`;
/// * for every live `s`: `e2s.get(s2e[s]) == Some(s)` (the round-trip identity).
///
/// `column.count()` is the high-water mark (slots `0..count` have all been
/// appended at least once); `live_count()` is the number of currently-live
/// slots. Tombstoned slots stay within `[0, count)` but are dead.
///
/// `s2e` / `free` / `e2s` / `live` are the dense storage's own bookkeeping —
/// the legitimate `std::Vec` exception the Dense plan grants this module.
pub struct DenseStore {
    /// The one contiguous data column (Dense plan: `ComponentPool::new(id,
    /// reserve_rows)` directly — no arena, no synthetic id). Address-stable:
    /// `grow_rows` commits pages in place, the base never moves.
    column: ComponentPool,

    /// `EntityId -> slot`. The membership oracle (`contains` / `slot_of`).
    e2s: SparseMap<u32>,

    /// `slot -> EntityId` (deterministic order + serde key). `TOMBSTONE` marks
    /// a freed slot. Indexed by slot; grows alongside `column.count()`.
    s2e: Vec<EntityId>,

    /// Per-slot liveness (Dense plan W3). The O(1) oracle, the iteration skip,
    /// and the read-only source of `DenseSolveView::row_ptr`'s debug_assert.
    live: LiveBitmap,

    /// LIFO free list of tombstoned slots. `insert` pops here first so a freed
    /// slot is reused before the column grows (Dense plan Decision 3).
    free: Vec<u32>,

    /// Archetype-presence seed (Dense plan Data structures / D1-deferred → D2):
    /// a 1024-bit set marking every archetype that has at least once hosted an
    /// entity inserted into this store. D3's `seed_from_candidates` strides this
    /// to enumerate candidate archetypes for a mixed dense query without a full
    /// archetype sweep.
    ///
    /// CONSERVATIVE: a bit is SET on insert and never cleared on a single
    /// `remove` (an archetype may still host other dense members, and tracking
    /// per-archetype live counts would cost a SparseMap per store). D3's per-row
    /// membership filter (`e2s.contains(entity)`) is the exact oracle; this set
    /// only over-approximates the candidate archetypes (false positives are
    /// filtered per-row, never false negatives).
    arch_presence: ArchetypeBitSet,

    /// The component id this store serves (debug guards + diagnostics).
    id: ComponentId,
}

impl DenseStore {
    /// Creates an empty store for `component_id`, backing the data column with
    /// `ComponentPool::new(component_id, reserve_rows)` directly and sizing the
    /// `live` / `s2e` / `free` bookkeeping for `reserve_rows`.
    ///
    /// The component must already be registered in the `ComponentRegistry`
    /// (the `ComponentPool::new` contract); the layout's `drop_fn` is honored
    /// on `remove` / `compact` / store drop, so a `DenseStore` is correct for
    /// any dense component, not only POD.
    pub fn new(component_id: ComponentId, reserve_rows: usize) -> Self {
        Self {
            column: ComponentPool::new(component_id.get(), reserve_rows),
            e2s: SparseMap::with_capacity(reserve_rows),
            s2e: Vec::with_capacity(reserve_rows),
            live: LiveBitmap::with_capacity(reserve_rows),
            free: Vec::with_capacity(reserve_rows),
            arch_presence: ArchetypeBitSet::new(),
            id: component_id,
        }
    }

    /// Inserts `value_bytes` for `entity`, returning the assigned slot.
    ///
    /// O(1) amortized: pops a freed slot (LIFO) if one exists, else appends a
    /// fresh slot at the column frontier (growing the column in place if
    /// needed). `value_bytes` must be a valid byte representation of the
    /// store's registered type and exactly `stride` bytes long.
    ///
    /// Change detection (Dense plan D4 / Decision 5): a fresh dense component is
    /// `Added` this frame, so BOTH the slot's `added` and `changed` ticks are
    /// stamped with `current_tick`. Re-stamping on a reused (freed-then-popped)
    /// slot also clears any stale tick the slot carried from its prior tenant
    /// (the write-before-read property — a reused slot's history never leaks).
    ///
    /// # Panics
    /// * `value_bytes.len() != column stride` — debug-asserted in the column.
    /// * the entity is already present — debug-asserted (a re-insert without an
    ///   intervening `remove` is a caller bug).
    /// * the column's reserve ceiling is exhausted on a fresh-slot append.
    pub fn insert(&mut self, entity: EntityId, value_bytes: &[u8], current_tick: Tick) -> u32 {
        debug_assert!(
            !self.e2s.contains(entity.get()),
            "DenseStore::insert: entity {entity} already present (component {})",
            self.id
        );

        let slot = match self.free.pop() {
            Some(slot) => {
                // Reused slot: it lives within `[0, column.count())` (it was
                // appended before being tombstoned), so the column's
                // exclusive-access `write_at` rewrites the freed bytes in place.
                // SAFETY: `slot < column.count()` (a freed slot was previously
                // appended and is below the high-water mark); the slot is
                // logically uninitialised (its bytes were dropped by `remove`'s
                // `drop_at`), so no drop runs on the stale bytes; `&mut self`
                // gives the column exclusive access; `value_bytes` is a valid,
                // stride-sized representation per the caller contract.
                unsafe { self.column.write_at(slot as usize, value_bytes) };
                self.s2e[slot as usize] = entity;
                slot
            }
            None => {
                // Fresh slot: append at the frontier. `add` grows the column in
                // place (address-stable) and returns the new slot index.
                let slot = self
                    .column
                    .add(value_bytes)
                    .expect("invariant: DenseStore column reserve ceiling exhausted")
                    as u32;
                debug_assert_eq!(
                    slot as usize,
                    self.s2e.len(),
                    "DenseStore::insert: fresh slot must equal s2e.len()"
                );
                self.s2e.push(entity);
                slot
            }
        };

        self.live.set(slot as usize);
        self.e2s.insert(entity.get(), slot);

        // Change detection (D4): stamp both ticks — a fresh dense component is
        // Added (and trivially Changed) this frame.
        // SAFETY: `slot < column.count()` (it was just appended or written via a
        //   reused freed slot below the high-water mark), so the slot lies in the
        //   committed prefix of both tick sub-regions; `&mut self` ⇒ the column
        //   has exclusive access; no concurrent reader of this slot's tick exists
        //   (structural ops are single-threaded under `&mut DenseStore`).
        unsafe {
            self.column.write_added_tick(slot as usize, current_tick);
            self.column.write_changed_tick(slot as usize, current_tick);
        }

        debug_assert!(self.debug_check_slot(slot), "DenseStore::insert invariant");
        slot
    }

    /// Inserts `value_bytes` for `entity` if absent, or REPLACES the existing
    /// value in place if present (dropping the old value via the column's
    /// `drop_fn` first). Returns `true` iff the entity was newly added (absent
    /// before).
    ///
    /// The insert-onto-an-existing-entity path the table `InsertCommand` replace
    /// semantics map onto for a dense component (Dense plan D2): a re-insert
    /// without an intervening `remove` is legal here (unlike [`Self::insert`],
    /// which debug-asserts absence) — it overwrites the slot in place, keeping
    /// the slot VALUE assignment stable (no churn, the determinism contract).
    ///
    /// Change detection (Dense plan D4 / Decision 5): a REPLACE stamps ONLY the
    /// slot's `changed` tick (the component was already present — it is Changed,
    /// not Added; its `added` tick is preserved). A fresh insert delegates to
    /// [`Self::insert`], which stamps both ticks.
    pub fn insert_or_replace(
        &mut self,
        entity: EntityId,
        value_bytes: &[u8],
        current_tick: Tick,
    ) -> bool {
        if let Some(slot) = self.e2s.get(entity.get()).copied() {
            // Present: drop the old value, overwrite in place at the SAME slot
            // (no free-list churn — the live slot never moves, C3 determinism).
            // SAFETY: `slot < column.count()` (`e2s` only maps to appended slots
            // below the high-water mark) and the slot holds a valid `T` written by
            // a prior insert; `&mut self` gives exclusive access. `drop_at` runs
            // the registered `drop_fn` exactly once, logically uninitialising the
            // slot; `write_at` then re-initialises it from `value_bytes` (a valid,
            // stride-sized representation per the caller contract). No double-drop:
            // the old value is dropped exactly here. `write_changed_tick` bumps the
            // slot's changed tick (the replace is a mutation); `added` is preserved.
            unsafe {
                self.column.drop_at(slot as usize);
                self.column.write_at(slot as usize, value_bytes);
                self.column.write_changed_tick(slot as usize, current_tick);
            }
            debug_assert!(self.debug_check_slot(slot), "DenseStore::insert_or_replace invariant");
            false
        } else {
            self.insert(entity, value_bytes, current_tick);
            true
        }
    }

    /// Removes `entity`, dropping its component bytes via the column's
    /// registered `drop_fn`. Returns `true` if the entity was present.
    ///
    /// O(1): tombstones the slot (the live slot is NOT moved — the determinism
    /// contract; swap-remove would reorder slots and break physics
    /// bit-determinism), pushes it onto the free list, and clears its liveness.
    pub fn remove(&mut self, entity: EntityId) -> bool {
        let Some(slot) = self.e2s.get(entity.get()).copied() else {
            return false;
        };

        // SAFETY: `slot < column.count()` — `e2s` only ever maps to a slot the
        // store appended (so it is below the high-water mark) and the live slot
        // holds a valid `T` written by `insert`. `&mut self` gives the column
        // exclusive access. `drop_at` runs the registered `drop_fn` exactly
        // once; the slot is then logically uninitialised and is guarded against
        // re-drop by the tombstone bookkeeping below (and by the store's own
        // `Drop`, which never re-drops a slot whose bit is clear).
        unsafe { self.column.drop_at(slot as usize) };

        self.live.clear(slot as usize);
        self.s2e[slot as usize] = TOMBSTONE;
        self.free.push(slot);
        self.e2s.swap_remove(entity.get());

        debug_assert!(
            !self.live.test(slot as usize),
            "DenseStore::remove: slot must be dead after remove"
        );
        true
    }

    /// Returns the slot `entity` occupies, or `None` if it is not present.
    #[inline]
    pub fn slot_of(&self, entity: EntityId) -> Option<u32> {
        self.e2s.get(entity.get()).copied()
    }

    /// Returns `true` iff `entity` is present in this store.
    #[inline]
    pub fn contains(&self, entity: EntityId) -> bool {
        self.e2s.contains(entity.get())
    }

    /// The column high-water mark (slots `0..len()` have been appended at least
    /// once; some may be tombstoned). NOT the live count — see
    /// [`Self::live_count`].
    #[inline]
    pub fn len(&self) -> usize {
        self.column.count()
    }

    /// `true` iff no slot has ever been appended.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.column.count() == 0
    }

    /// The number of currently-live slots (`high-water mark − tombstones`).
    #[inline]
    pub fn live_count(&self) -> usize {
        self.column.count() - self.free.len()
    }

    /// The component id this store serves.
    #[inline]
    pub fn component_id(&self) -> ComponentId {
        self.id
    }

    /// Marks `archetype_id` present in the [`arch_presence`](Self::arch_presence)
    /// seed (Dense plan D2). Called by the routing layer on every dense insert
    /// with the entity's current archetype, so D3 can enumerate candidate
    /// archetypes without a full sweep.
    ///
    /// Idempotent: a set bit stays set. Conservative — never cleared on a single
    /// remove (see the field doc).
    #[inline]
    pub fn mark_arch_present(&mut self, archetype_id: ArchetypeId) {
        self.arch_presence.insert(archetype_id.get());
    }

    /// Read-only access to the archetype-presence seed (Dense plan D2 → D3).
    #[inline]
    pub fn arch_presence(&self) -> &ArchetypeBitSet {
        &self.arch_presence
    }

    /// The `slot -> EntityId` table (Dense plan D3). `s2e[slot]` is the owning
    /// entity of a LIVE slot, or [`TOMBSTONE`] for a freed one. Indexed by slot,
    /// covering `0..len()`. Consumed by `DenseQueryIter` (the pure-dense fast
    /// path) to yield the per-slot entity in insertion order alongside the
    /// `live` skip.
    #[inline]
    pub fn s2e(&self) -> &[EntityId] {
        &self.s2e
    }

    /// The component stride (bytes per slot) — the public save-side accessor
    /// (Dense plan D4 serialization). Equal to the registered type's
    /// `size_of` (0 for a ZST dense tag).
    #[inline]
    pub fn stride_bytes(&self) -> usize {
        self.column.component_layout().size()
    }

    /// The registered component's `Layout` (size + align) — the load-side ViaFn
    /// decode accessor (Dense plan D4 serialization v1.1). The dense ViaFn loader
    /// (`load_dense_store_via_fn`) allocates one scratch buffer of this exact
    /// layout, decodes each member into it via the per-element `deserialize_fn`,
    /// then byte-moves the reconstructed value into a fresh slot via
    /// [`Self::insert`]. `align` (not just `size`) is needed so the scratch buffer
    /// satisfies the `DeserializeFn` contract (`dst` aligned to `align_of::<C>()`).
    #[inline]
    pub(crate) fn component_layout(&self) -> std::alloc::Layout {
        self.column.component_layout()
    }

    /// Read-only byte view of LIVE slot `slot` (Dense plan D4 serialization —
    /// the save-side per-member gather). Returns `&self`-borrowed `stride_bytes`
    /// bytes; the caller blits them into the file's dense column region.
    ///
    /// # Panics
    /// * `slot` is not live (debug-asserted) — the saver only passes slots
    ///   yielded by [`Self::for_each_live`].
    #[inline]
    pub fn row_bytes(&self, slot: u32) -> &[u8] {
        debug_assert!(
            self.live.test(slot as usize),
            "DenseStore::row_bytes: slot {slot} is not live"
        );
        let stride = self.column.component_layout().size();
        // SAFETY: `slot` is live (debug-asserted), so it is `< column.count()` and
        //   holds a valid `T`; `buffer_ptr().add(slot * stride)` lies inside the
        //   column's address-stable reservation and is valid for `stride` bytes.
        //   The `&self` borrow keeps the column alive; the slice is read-only.
        unsafe {
            let ptr = self.column.buffer_ptr().add(slot as usize * stride);
            core::slice::from_raw_parts(ptr, stride)
        }
    }

    /// Invokes `f(slot, entity)` for every live slot in slot order, skipping
    /// tombstones via the `live` oracle.
    ///
    /// Slot order is insertion order for the steady state, perturbed only by
    /// free-list reuse (a reused slot keeps its original, lower index). After a
    /// [`Self::compact`] this is exactly the canonical insertion order
    /// `0..live_count`.
    #[inline]
    pub fn for_each_live(&self, mut f: impl FnMut(u32, EntityId)) {
        let len = self.column.count();
        for slot in 0..len {
            if self.live.test(slot) {
                f(slot as u32, self.s2e[slot]);
            }
        }
    }

    /// Compacts the column so the live slots occupy `0..live_count` in their
    /// current slot (= insertion) order, dropping nothing-live tombstones,
    /// clearing the free list, and rebuilding `live` / `s2e` / `e2s`.
    ///
    /// COLD and deterministic: it reindexes by relocating the bytes of each
    /// live slot down to the next free canonical index (preserving relative
    /// order), so a fixed op-sequence always yields the same post-compact slot
    /// assignment.
    ///
    /// # Boundary (Dense plan C3d)
    /// `compact` mutates slot VALUES, so it MUST run only between physics steps
    /// (an exclusive-world point), never mid-step while any `DenseSolveView` is
    /// live. It is reachable through `DenseBuildView` (`!Send`) only, which the
    /// scheduler serializes against every solve view.
    pub fn compact(&mut self) {
        let len = self.column.count();
        let stride = self.column.component_layout().size();
        let mut write = 0usize;

        // Single forward pass: each live slot is moved down to `write` (its
        // canonical index) iff it is not already there. Relative order is
        // preserved (a stable compaction), so the result is deterministic.
        for read in 0..len {
            if !self.live.test(read) {
                continue;
            }
            if read != write {
                // SAFETY: both `read` and `write` are `< len <=
                // column.count() <= committed_rows`, so both row pointers are
                // valid committed slots within the column's address-stable
                // reservation. `write < read` always (the live count below
                // `read` never exceeds `read`), and the destination slot is a
                // tombstone (already dropped, logically uninitialised), so the
                // byte copy overwrites dead bytes without running drop and the
                // source bytes are moved (not copied-and-dropped). `&mut self`
                // gives exclusive access; the two slots are distinct so the
                // ranges do not overlap. `move_ticks` carries the slot's
                // change-detection ticks down with the data (D4: ticks are
                // slot-indexed, so the relocation must move them or the
                // compacted slot would read a stale tick).
                unsafe {
                    let base = self.column.buffer_ptr_mut();
                    core::ptr::copy_nonoverlapping(
                        base.add(read * stride),
                        base.add(write * stride),
                        stride,
                    );
                    self.column.move_ticks(read, write);
                }
            }
            // The entity at the moved slot now lives at `write`.
            self.s2e[write] = self.s2e[read];
            write += 1;
        }

        // Neutralise the column's tail: every slot `[write, len)` is now dead
        // (its bytes were either moved out or already dropped). `pop_*_no_drop`
        // decrements the column length without running `drop_fn`, so the
        // column's high-water mark becomes exactly the live count and its
        // terminal `Drop` will never touch a stale slot.
        for _ in write..len {
            self.column.pop_entity_no_drop();
        }
        self.s2e.truncate(write);

        // Rebuild the membership structures from the canonical order.
        self.free.clear();
        self.live.clear_all();
        self.e2s.clear();
        for (slot, &entity) in self.s2e.iter().enumerate() {
            debug_assert_ne!(
                entity, TOMBSTONE,
                "DenseStore::compact: a canonical slot must not be a tombstone"
            );
            self.live.set(slot);
            self.e2s.insert(entity.get(), slot as u32);
        }

        debug_assert_eq!(
            self.column.count(),
            self.live_count(),
            "DenseStore::compact: high-water mark must equal live count post-compact"
        );
        debug_assert!(self.free.is_empty(), "DenseStore::compact: free must be empty");
    }

    /// Borrows the store as a single-threaded structural view (Dense plan
    /// Decision 8). The `DenseBuildView` is `!Send` and is the ONLY surface
    /// exposing whole-buffer / structural ops (push / tombstone / compact).
    #[inline]
    pub fn build_view(&mut self) -> DenseBuildView<'_> {
        DenseBuildView::new(self)
    }

    /// Borrows the store as a `Copy + Send + Sync` solve view (Dense plan
    /// Decision 8). The `DenseSolveView` exposes per-slot `row_ptr` ONLY — no
    /// whole-buffer `&mut [T]` path exists, so the SP4 reborrow is un-typeable.
    ///
    /// The view caches the column's address-stable base, stride, length, and
    /// the `live` words pointer; it must not outlive any structural mutation
    /// of the store (enforced by `'a` borrowing `&self`).
    #[inline]
    pub fn solve_view(&self) -> DenseSolveView<'_> {
        DenseSolveView::new(
            self.column.buffer_ptr().cast_mut(),
            self.column.component_layout().size(),
            self.column.count(),
            self.live.words_ptr(),
            self.live.word_count(),
        )
    }

    // ── pub(crate) accessors for the views (no public column leakage) ───────

    /// The column's address-stable write-capable base. Used by
    /// [`DenseBuildView`] for the single-threaded whole-buffer slice.
    #[inline]
    pub(crate) fn column_base_mut(&mut self) -> *mut u8 {
        self.column.buffer_ptr_mut()
    }

    /// The column stride (component size in bytes).
    #[inline]
    pub(crate) fn stride(&self) -> usize {
        self.column.component_layout().size()
    }

    /// `true` iff slot `s` is live (the view's structural-side liveness oracle).
    #[inline]
    pub(crate) fn is_live(&self, slot: usize) -> bool {
        self.live.test(slot)
    }

    // ── pub(crate) per-slot tick accessors (Dense plan D4 — change detection) ──

    /// The base pointer of the column's per-slot `added` tick sub-region (Dense
    /// plan D4). The query fetch (`Added<Dense>` / `Mut<Dense>`) caches this once
    /// and reads `[slot]` per row; it is address-stable for the store's lifetime
    /// (write-once vm-reservation base of the column).
    #[inline]
    pub(crate) fn added_ticks_ptr(&self) -> *const UnsafeCell<Tick> {
        self.column.added_ticks_ptr()
    }

    /// The base pointer of the column's per-slot `changed` tick sub-region (Dense
    /// plan D4). Same address-stable contract as [`Self::added_ticks_ptr`];
    /// `Changed<Dense>` reads it and `Mut<Dense>`'s deref guard writes through it.
    #[inline]
    pub(crate) fn changed_ticks_ptr(&self) -> *const UnsafeCell<Tick> {
        self.column.changed_ticks_ptr()
    }

    /// Stamps both ticks at `slot` to `tick` (Dense plan D4 — the serde load
    /// path). Used by the dense loader to mark a freshly-restored membership
    /// Added at the load tick (mirroring the table blit's `fill_ticks`).
    ///
    /// # Safety
    /// * `slot < self.len()` — the slot must be live (a loaded membership).
    /// * Caller holds exclusive access via `&mut self`.
    #[inline]
    pub(crate) unsafe fn stamp_slot_ticks(&mut self, slot: usize, tick: Tick) {
        debug_assert!(slot < self.column.count(), "stamp_slot_ticks: slot out of range");
        // SAFETY: `slot < column.count() <= committed_rows` (caller contract),
        //   so the slot lies in the committed prefix of both tick sub-regions;
        //   `&mut self` ⇒ exclusive access.
        unsafe {
            self.column.write_added_tick(slot, tick);
            self.column.write_changed_tick(slot, tick);
        }
    }

    /// Debug-only round-trip check for a single live slot. Returns `true` when
    /// the invariant holds; called from `debug_assert!` in the structural ops.
    ///
    /// NOT `#[cfg(debug_assertions)]`: a `debug_assert!` condition is name-resolved
    /// in EVERY profile (only its *execution* is debug-gated), so the method must
    /// exist in release too or the lib fails to compile (E0599). The body is
    /// dead-code-eliminated in release (the `debug_assert!` lowers to `if false`).
    #[allow(dead_code)]
    fn debug_check_slot(&self, slot: u32) -> bool {
        let s = slot as usize;
        let entity = self.s2e[s];
        self.live.test(s)
            && entity != TOMBSTONE
            && self.e2s.get(entity.get()).copied() == Some(slot)
    }

    /// Full-structure invariant verifier (debug / property-test oracle):
    /// `e2s[s2e[s]] == s` for every live slot, and `!live(s) ⟺ s ∈ free` for
    /// every slot in `[0, len)`. Returns `true` iff the invariant holds.
    pub fn check_invariant(&self) -> bool {
        let len = self.column.count();
        let free: std::collections::HashSet<u32> = self.free.iter().copied().collect();

        // The free list must contain no duplicates (a duplicate would let one
        // slot be handed out twice) and every entry must be addressable.
        if free.len() != self.free.len() {
            return false;
        }
        if self.free.iter().any(|&s| (s as usize) >= len) {
            return false;
        }

        // Count live slots directly from the bitmap (independent of `free`),
        // then assert the partition `live + free == len` holds.
        let live_bits = (0..len).filter(|&s| self.live.test(s)).count();
        if live_bits + free.len() != len {
            return false;
        }

        for slot in 0..len {
            let live = self.live.test(slot);
            let in_free = free.contains(&(slot as u32));
            let is_tomb = self.s2e[slot] == TOMBSTONE;

            // !live ⟺ s ∈ free ⟺ s2e[s] == TOMBSTONE
            if live == in_free {
                return false;
            }
            if in_free != is_tomb {
                return false;
            }

            if live {
                let entity = self.s2e[slot];
                // e2s[s2e[slot]] == slot
                if self.e2s.get(entity.get()).copied() != Some(slot as u32) {
                    return false;
                }
            }
        }
        true
    }
}

impl Drop for DenseStore {
    fn drop(&mut self) {
        // The column's own `Drop` runs `drop_fn` over `[0, count)` blindly — it
        // cannot tell a tombstoned slot from a live one and would double-drop
        // (or drop logically-uninitialised bytes — UB) on every freed slot.
        //
        // So the store drops each LIVE slot itself, then neutralises the
        // column: `drop_at(slot)` runs `drop_fn` exactly once per live slot,
        // and `pop_entity_no_drop` walks the high-water mark down to 0 WITHOUT
        // dropping, so the column's terminal `Drop` loop over `[0, count)` is
        // empty. Net effect: every component is dropped exactly once.
        let len = self.column.count();
        for slot in 0..len {
            if self.live.test(slot) {
                // SAFETY: `slot < column.count()`, the slot is live (its bit is
                // set), so it holds a valid `T` written by `insert`; `&mut self`
                // (Drop receives `&mut self`) gives exclusive access. `drop_at`
                // runs `drop_fn` exactly once on this live slot.
                unsafe { self.column.drop_at(slot) };
            }
        }
        // Walk the column length down to 0 without dropping; the now-empty
        // column's terminal Drop is a no-op and never re-touches a slot.
        while self.column.count() != 0 {
            self.column.pop_entity_no_drop();
        }
    }
}
