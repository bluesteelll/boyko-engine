//! [`Assets<T>`] — the world-global, per-asset-type storage table
//! (asset-streaming plan F1: unified store on a standalone [`ComponentPool`]).
//!
//! # Storage — the shipped `DenseStore` recipe, not a `Vec`-slotmap
//!
//! `slot.rs`'s former rationale — "a hand-rolled `SlotColumn` over a raw byte
//! column is unsound because dropping a `T: !Copy` through it reintroduces
//! double-free / drop-uninit UB" — is **retracted**. `DenseStore` already
//! performs exactly this: an occupancy-tracked (`LiveBitmap`), exactly-once
//! drop (`ComponentPool::drop_at`, gated on the `live` bit) over a
//! store-owned, standalone `ComponentPool` (`ComponentPool::new(id,
//! reserve_rows)` directly, no archetype). `Assets<T>` now reuses that
//! identical recipe: [`col`](Assets::col) is the store-owned data column,
//! [`live`](Assets::live) is the occupancy oracle the terminal [`Drop`]
//! consults, and [`free`](Assets::free) is the LIFO free-list — the same
//! three primitives, applied to an asset table instead of a dense component.
//!
//! # `slot_word` — a packed `{generation, state}` per row
//!
//! Each row's [`Handle`] generation and lifecycle state
//! ([`AssetLoadState`]-ish, plus the F2+ `Retiring` state) are packed into
//! one `u32`: the low 3 bits are the state, the high 29 bits are the
//! generation (see [`pack_slot_word`]). `Vacant`/`Retiring` are not part of
//! the public [`AssetLoadState`] enum — a row in either state simply does not
//! resolve through [`Assets::state`] (mirrors the old `resolve_index`
//! contract exactly).
//!
//! A `Loading`/`Failed` row carries NO real `T` value — [`Assets::reserve`]
//! must still make its data-column row EXIST (every minted slot index is
//! dense-indexed 1:1 across `col` / `slot_word` / `live` / `free`), so it
//! writes inert all-zero scratch bytes that are never read or dropped as a
//! `T` while the packed state stays non-`Loaded` (the `live` bitmap gates
//! every read/drop path in this file).
//!
//! # Streaming fields land inert (F2+ wires them)
//!
//! [`refcount`](Assets::refcount), [`free_epoch`](Assets::free_epoch),
//! [`dirty`](Assets::dirty), and [`pinned`](Assets::pinned) exist as of this
//! rung but are not yet consulted by any lifecycle decision.
//!
//! Removal (see [`Assets::remove`]) stays FULLY SYNCHRONOUS (immediate
//! `take_at` + move-out + generation bump + free-list push) — the
//! deferred/fence-gated `Retiring` path is a later rung (F2/F6). This is
//! what lets the existing unit tests and the `assets_matches_hashmap_oracle`
//! proptest port verbatim: a POD test type with no device teardown is
//! correctly served by an immediate remove.

use std::marker::PhantomData;

use crate::ecs::constants::pool_reserve_rows;
use crate::ecs::core::asset::asset::AssetLoadState;
use crate::ecs::core::asset::backing::AssetBacking;
use crate::ecs::core::asset::error::AssetError;
use crate::ecs::core::asset::handle::Handle;
use crate::ecs::core::component::dense::live_bitmap::LiveBitmap;
use crate::ecs::core::resources::resource::{NonSendResource, Resource};
use crate::ecs::core::resources::resource_type_registry::resource_id_for;
use crate::ecs::identifiers::primitives::{ComponentId, ResourceId};
use crate::ecs::memory::component_pool::ComponentPool;
use crate::ecs::memory::vm_column::VmColumn;

/// Number of low bits `slot_word` dedicates to the packed lifecycle state —
/// 3 bits (room for up to 8 states; 5 are named today).
const STATE_BITS: u32 = 3;
const STATE_MASK: u32 = (1 << STATE_BITS) - 1;

/// Never minted, or minted then freed via [`Assets::remove`] — occupies the
/// LIFO [`Assets::free`] list, awaiting reuse.
const STATE_VACANT: u32 = 0;
/// Minted by [`Assets::reserve`]; no value yet.
const STATE_LOADING: u32 = 1;
/// A live value, resolvable via [`Assets::get`] / [`Assets::get_mut`] /
/// [`Assets::iter`].
const STATE_LOADED: u32 = 2;
/// [`Assets::reserve`]'d then [`Assets::fail`]'d — no value, like `Loading`.
const STATE_FAILED: u32 = 3;
/// Reserved for F6 (fence-gated deferred free): occupied-but-unreusable,
/// awaiting a future `retire_deferred_frees` pass. Never written in F1.
#[allow(dead_code)]
const STATE_RETIRING: u32 = 4;

/// Packs `generation` (high 29 bits) and `state` (low 3 bits) into one `u32`.
///
/// A generation that overflows the 29-bit space silently wraps (the high
/// bits are dropped by the left shift) — the same "practically unreachable,
/// and harmless if it ever happens" acceptance the pre-rewrite plain `u32`
/// generation counter already made for `u32::wrapping_add`, just at a
/// smaller (still ~536 million-deep) wrap boundary.
#[inline]
fn pack_slot_word(generation: u32, state: u32) -> u32 {
    debug_assert!(state <= STATE_MASK, "pack_slot_word: state must fit in STATE_BITS");
    (generation << STATE_BITS) | state
}

#[inline]
fn unpack_state(word: u32) -> u32 {
    word & STATE_MASK
}

#[inline]
fn unpack_generation(word: u32) -> u32 {
    word >> STATE_BITS
}

/// Routing tag for the future fence-gated deferred-free queue (F6) — names
/// which `Assets<T>` table a queued `FreeEntry` belongs to. Wraps the type's
/// own [`ComponentId`] (already unique per asset type via
/// [`AssetBacking::register_layout`]) rather than minting a second registry.
/// Inert in F1 — nothing reads it yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AssetKind(#[allow(dead_code)] ComponentId);

/// A world-global, per-asset-type storage table — registered as a
/// [`Resource`] (or [`NonSendResource`] for a `!Send` element type) via the
/// same generic-registry pattern [`State<S>`](crate::ecs::core::state::State)
/// uses.
///
/// # Storage
///
/// - [`col`](Self::col) — the store-owned [`ComponentPool`] data column
///   (stride = `size_of::<T>()`), address-stable.
/// - [`slot_word`](Self::slot_word) — packed `{generation, state}` per row
///   (see the module doc).
/// - [`live`](Self::live) — occupancy oracle: exactly the rows holding a real,
///   droppable `T`. The terminal [`Drop`] consults this, never `slot_word`'s
///   state directly (`Retiring`, once F6 lands, stays `live` while its packed
///   state has already moved off `Loaded`).
/// - [`free`](Self::free) — LIFO free-list; `add`/`reserve` pop it before
///   growing `col`/`slot_word`.
/// - [`refcount`](Self::refcount) / [`free_epoch`](Self::free_epoch) /
///   [`dirty`](Self::dirty) / [`pinned`](Self::pinned) — streaming lifetime
///   plumbing that lands inert in this rung (F2+ wires the readers).
pub struct Assets<T: AssetBacking> {
    col: ComponentPool,
    slot_word: VmColumn<u32>,
    refcount: VmColumn<u32>,
    live: LiveBitmap,
    free: Vec<u32>,
    pinned: LiveBitmap,
    dirty: LiveBitmap,
    live_count: usize,
    dirty_gen: u64,
    free_epoch: u64,
    // Read starting F6 (the deferred-free queue's routing key). Inert in F1.
    #[allow(dead_code)]
    id: AssetKind,
    _t: PhantomData<T>,
}

impl<T: AssetBacking> Assets<T> {
    /// Creates an empty table, registering `T`'s [`ComponentId`] layout on
    /// first call (memoized — see [`AssetBacking::register_layout`]).
    ///
    /// `cap` is a soft pre-touch hint, not a hard ceiling: the backing
    /// `ComponentPool`/`VmColumn` reserve a large, practically-unbounded
    /// virtual-address ceiling (`pool_reserve_rows`, the same byte-targeted,
    /// row-clamped formula every table/dense column uses) — VA is free and
    /// commit stays lazy, so this preserves the old `Vec`'s "grow forever, no
    /// real ceiling" behavior rather than hard-capping the table at `cap`
    /// rows. A caller requesting MORE than the default ceiling is still
    /// honored (`.max(cap)`).
    #[inline]
    pub fn with_reserved(cap: usize) -> Self {
        let component_id = T::register_layout();
        let reserve_rows = pool_reserve_rows(std::mem::size_of::<T>()).max(cap);
        Self {
            col: ComponentPool::new(component_id.get(), reserve_rows),
            slot_word: VmColumn::new("Assets.slot_word", reserve_rows),
            refcount: VmColumn::new("Assets.refcount", reserve_rows),
            live: LiveBitmap::with_capacity(cap),
            free: Vec::new(),
            pinned: LiveBitmap::with_capacity(cap),
            dirty: LiveBitmap::with_capacity(cap),
            live_count: 0,
            dirty_gen: 0,
            free_epoch: 0,
            id: AssetKind(component_id),
            _t: PhantomData,
        }
    }

    /// Moves `value` into a FRESH row at the data column's frontier (a row
    /// never written before), returning the assigned index.
    ///
    /// Mirrors `ComponentPool::add_typed` for a type this pool cannot
    /// type-check via the `Component` bound (`AssetBacking` types are not ECS
    /// components) — the pool takes an independent byte copy of `value`'s
    /// representation (`ComponentPool::add`'s `copy_nonoverlapping`
    /// contract), so forgetting `value` afterwards transfers ownership
    /// without a double-drop.
    fn push_value(&mut self, value: T) -> usize {
        // SAFETY: `&value` points at a valid, aligned, fully-initialized `T`
        // of exactly `size_of::<T>()` bytes — the pool's registered type
        // (built from `T::register_layout()` in `with_reserved`).
        let bytes = unsafe {
            core::slice::from_raw_parts((&value as *const T).cast::<u8>(), core::mem::size_of::<T>())
        };
        let idx = self
            .col
            .add(bytes)
            .expect("invariant: Assets<T> reserve ceiling exhausted (pool_reserve_rows-class row cap)");
        core::mem::forget(value);
        idx
    }

    /// Overwrites the logically-uninitialized row `idx` with `value`, moving
    /// ownership without invoking `T::drop` on the source binding.
    ///
    /// # Safety
    /// `idx` must be logically uninitialized from this store's own
    /// perspective: either a `reserve()`'d row's inert scratch bytes (never
    /// filled), or a row freed via [`Self::remove`] (whose `take_at` already
    /// moved the prior value out). Caller holds exclusive access via
    /// `&mut self`.
    unsafe fn write_value_at(&mut self, idx: usize, value: T) {
        // SAFETY: see `push_value` for why `&value`'s bytes are a valid,
        // stride-sized representation of the pool's registered type.
        let bytes = unsafe {
            core::slice::from_raw_parts((&value as *const T).cast::<u8>(), core::mem::size_of::<T>())
        };
        // SAFETY: the caller's contract on `idx` (logically uninitialized)
        // satisfies `ComponentPool::write_at`'s own precondition; `bytes` is
        // valid + stride-sized (above).
        unsafe { self.col.write_at(idx, bytes) };
        core::mem::forget(value);
    }

    /// Inserts `value`, reusing a freed row (LIFO) if one exists, otherwise
    /// appending a fresh row. Returns the [`Handle`] addressing it.
    pub fn add(&mut self, value: T) -> Handle<T> {
        if let Some(reused) = self.free.pop() {
            let idx = reused as usize;
            let word = self.slot_word.get(idx).expect("invariant: freed slot must be addressable");
            debug_assert_eq!(
                unpack_state(word),
                STATE_VACANT,
                "invariant: a row popped off `free` must be Vacant"
            );
            let generation = unpack_generation(word);
            // SAFETY: `idx` is Vacant (checked above) — its data-column row
            // is logically dead (either never filled — a `reserve()`'d row's
            // inert scratch bytes — or freed via `remove`'s `take_at`);
            // `write_value_at` overwrites it with `value` without touching
            // the stale content.
            unsafe { self.write_value_at(idx, value) };
            self.slot_word.set(idx, pack_slot_word(generation, STATE_LOADED));
            self.refcount.set(idx, 0);
            self.live.set(idx);
            self.live_count += 1;
            return Handle::new(reused, generation);
        }

        let idx = self.slot_word.len();
        debug_assert!(
            idx < u32::MAX as usize,
            "invariant: Assets<T> row count exceeds u32 range (the render carrier is a 32-bit index)"
        );
        let col_idx = self.push_value(value);
        debug_assert_eq!(
            col_idx, idx,
            "invariant: a fresh slot's data-column row must equal its slot index"
        );
        self.slot_word.push(pack_slot_word(0, STATE_LOADED));
        self.refcount.push(0);
        self.live.set(idx);
        self.live_count += 1;
        Handle::new(idx as u32, 0)
    }

    /// Reserves a fresh row without a value, in state
    /// [`AssetLoadState::Loading`] — the target of an in-flight load. Reuses
    /// a freed row (LIFO) exactly like [`Self::add`], but does NOT bump
    /// [`len`](Self::len): a `Reserved` row is counted in
    /// [`high_water`](Self::high_water) but not in `len` until
    /// [`fill`](Self::fill) succeeds.
    pub fn reserve(&mut self) -> Handle<T> {
        if let Some(reused) = self.free.pop() {
            let idx = reused as usize;
            let word = self.slot_word.get(idx).expect("invariant: freed slot must be addressable");
            debug_assert_eq!(
                unpack_state(word),
                STATE_VACANT,
                "invariant: a row popped off `free` must be Vacant"
            );
            let generation = unpack_generation(word);
            // No `col` write: a reused row's stale bytes (from a prior
            // Loaded value's move-out, or a prior Loading scratch write) are
            // never read while the state stays non-Loaded.
            self.slot_word.set(idx, pack_slot_word(generation, STATE_LOADING));
            self.refcount.set(idx, 0);
            return Handle::new(reused, generation);
        }

        let idx = self.slot_word.len();
        debug_assert!(
            idx < u32::MAX as usize,
            "invariant: Assets<T> row count exceeds u32 range (the render carrier is a 32-bit index)"
        );
        // Fresh slot: the data column's dense row space must grow in
        // lockstep with `slot_word` (a later `fill()` targets this exact row
        // via `write_at`) — append inert scratch bytes now. They are never
        // read or dropped as a live `T` while the packed state stays
        // Loading/Failed (the `live` bitmap gates every read/drop path). A
        // heap `vec!` (not a `[0u8; size_of::<T>()]` stack array — Rust
        // rejects a generic-parameter-sized array length on stable) is fine
        // here: `reserve()`'s fresh-slot path is a cold, once-per-in-flight-
        // load event, never a per-frame hot path.
        let scratch = vec![0u8; core::mem::size_of::<T>()];
        let col_idx = self
            .col
            .add(&scratch)
            .expect("invariant: Assets<T> reserve ceiling exhausted (pool_reserve_rows-class row cap)");
        debug_assert_eq!(
            col_idx, idx,
            "invariant: a fresh slot's data-column row must equal its slot index"
        );
        self.slot_word.push(pack_slot_word(0, STATE_LOADING));
        self.refcount.push(0);
        Handle::new(idx as u32, 0)
    }

    /// Validates `handle` against the current row count + generation,
    /// returning the row index on success. Does NOT check the row's
    /// lifecycle state.
    #[inline]
    fn resolve_index(&self, handle: Handle<T>) -> Option<usize> {
        let idx = handle.index() as usize;
        if idx >= self.slot_word.len() {
            return None;
        }
        let word = self.slot_word.get(idx).expect("invariant: idx < slot_word.len()");
        if unpack_generation(word) != handle.generation() {
            return None;
        }
        Some(idx)
    }

    /// Resolves `handle` to its row index, requiring the row to be
    /// `Loading` or `Failed` — the shared precondition [`Self::fill`] and
    /// [`Self::fail`] both check before mutating a row.
    #[inline]
    fn resolve_reserved(&self, handle: Handle<T>) -> Option<usize> {
        let idx = self.resolve_index(handle)?;
        let word = self.slot_word.get(idx).expect("invariant: idx < slot_word.len()");
        matches!(unpack_state(word), STATE_LOADING | STATE_FAILED).then_some(idx)
    }

    /// Resolves `handle` to its row index, requiring the row to be `Loaded`
    /// — the shared precondition [`Self::get`] / [`Self::get_mut`] /
    /// [`Self::contains`] all check.
    #[inline]
    fn resolve_occupied(&self, handle: Handle<T>) -> Option<usize> {
        let idx = self.resolve_index(handle)?;
        let word = self.slot_word.get(idx).expect("invariant: idx < slot_word.len()");
        (unpack_state(word) == STATE_LOADED).then_some(idx)
    }

    /// Fills a row previously minted by [`Self::reserve`], transitioning it
    /// from `Loading`/`Failed` to [`Loaded`](AssetLoadState::Loaded).
    ///
    /// # Errors
    /// Returns `(`[`AssetError::StaleHandle`]`, value)` — WITH `value`
    /// returned to the caller (never silently dropped: a future
    /// device-owning asset has no safe bare-drop path) — if `handle` does not
    /// resolve to a `Loading`/`Failed` row with a matching generation: an
    /// already-`Loaded` row (a double-fill), a `Vacant` row, an
    /// out-of-range index, or a stale generation are all rejected.
    pub fn fill(&mut self, handle: Handle<T>, value: T) -> Result<(), (AssetError, T)> {
        let Some(idx) = self.resolve_reserved(handle) else {
            return Err((AssetError::StaleHandle, value));
        };
        let word = self.slot_word.get(idx).expect("invariant: idx < slot_word.len()");
        let generation = unpack_generation(word);
        // SAFETY: `idx` resolved to a Loading/Failed row (`resolve_reserved`
        // above) — its data-column bytes are inert scratch, never a live `T`
        // (the `reserve()`/`fail()` discipline); `write_value_at` overwrites
        // them with `value`.
        unsafe { self.write_value_at(idx, value) };
        self.slot_word.set(idx, pack_slot_word(generation, STATE_LOADED));
        self.live.set(idx);
        self.live_count += 1;
        self.dirty_gen += 1;
        Ok(())
    }

    /// Marks a row previously minted by [`Self::reserve`] as
    /// [`Failed`](AssetLoadState::Failed) — the load did not produce a
    /// value. A no-op if `handle` does not resolve to a `Loading`/`Failed`
    /// row with a matching generation (see [`Self::fill`]'s error cases).
    pub fn fail(&mut self, handle: Handle<T>) {
        if let Some(idx) = self.resolve_reserved(handle) {
            let word = self.slot_word.get(idx).expect("invariant: idx < slot_word.len()");
            let generation = unpack_generation(word);
            self.slot_word.set(idx, pack_slot_word(generation, STATE_FAILED));
        }
    }

    /// Returns a shared reference to the value `handle` addresses, or `None`
    /// if `handle` is out of range, stale, or the row is not `Loaded`.
    #[inline]
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        let idx = self.resolve_occupied(handle)?;
        let ptr = self.col.get_raw(idx).expect("invariant: a Loaded slot's row must be < col.count()");
        // SAFETY: `idx`'s packed state is Loaded (checked above), so the
        // data-column row holds a valid, initialized `T` written by
        // `push_value`/`write_value_at`. `&self` guarantees no exclusive
        // access exists for the returned reference's lifetime.
        Some(unsafe { &*ptr.cast::<T>() })
    }

    /// Mutable variant of [`Self::get`]. Bumps [`dirty_gen`](Self::dirty_gen)
    /// on every call that resolves to a live row.
    #[inline]
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        let idx = self.resolve_occupied(handle)?;
        self.dirty_gen += 1;
        let ptr = self
            .col
            .get_raw_mut(idx)
            .expect("invariant: a Loaded slot's row must be < col.count()");
        // SAFETY: same as `get` — `idx`'s packed state is Loaded, so the row
        // holds a valid `T`; `&mut self` guarantees exclusive access for the
        // returned reference's lifetime.
        Some(unsafe { &mut *ptr.cast::<T>() })
    }

    /// Frees the row `handle` addresses, returning its value if the row was
    /// `Loaded`, or `None` if it was `Loading`/`Failed` (there was never a
    /// value to return).
    ///
    /// Synchronous (F1 semantics, unchanged from the pre-rewrite `Vec`-backed
    /// store): the value is moved out and the row recycled immediately — the
    /// deferred, fence-gated `Retiring` teardown is a later rung (F2/F6).
    /// Bumps the row's generation so the just-freed `handle` is rejected from
    /// this point on, including after the row is reused.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        let idx = self.resolve_index(handle)?;
        let word = self.slot_word.get(idx).expect("invariant: idx < slot_word.len()");
        let generation = unpack_generation(word);
        let state = unpack_state(word);

        let value = match state {
            STATE_LOADED => {
                // Clear `live` BEFORE the move-out — the exactly-once
                // discipline `take_at`'s contract relies on: no later drop
                // path (this store's terminal `Drop`) can re-touch a slot
                // once its `live` bit is clear.
                self.live.clear(idx);
                self.live_count -= 1;
                // SAFETY: `idx` was Loaded (checked above) with a matching
                // generation, so the data column's row holds a valid `T`
                // written by `push_value`/`write_value_at`; `live` was just
                // cleared, so no other path reads or drops this slot again
                // before it is rewritten. `take_at` moves the value out via
                // `ptr::read` without running drop — the caller now owns it.
                Some(unsafe { self.col.take_at::<T>(idx) })
            }
            STATE_LOADING | STATE_FAILED => {
                // No value was ever written here — a Loading/Failed row
                // carries only inert scratch bytes — so there is nothing to
                // move out (mirrors the old `Slot::Reserved` → `None` path).
                None
            }
            _ => unreachable!(
                "invariant: a matching generation implies a Loading/Loaded/Failed slot"
            ),
        };

        self.slot_word.set(idx, pack_slot_word(generation.wrapping_add(1), STATE_VACANT));
        self.refcount.set(idx, 0);
        self.free.push(idx as u32);
        self.free_epoch = self.free_epoch.wrapping_add(1);
        self.dirty.set(idx);
        value
    }

    /// `true` if `handle` resolves to a live (`Loaded`) row.
    #[inline]
    pub fn contains(&self, handle: Handle<T>) -> bool {
        self.resolve_occupied(handle).is_some()
    }

    /// Number of live (`Loaded`) rows. O(1) — backed by a running counter.
    #[inline]
    pub fn len(&self) -> usize {
        self.live_count
    }

    /// `true` if there are no live rows.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.live_count == 0
    }

    /// The slot-row count INCLUDING freed holes — the high-water mark of
    /// every row index ever minted. O(1) (`== col.count()`, the data
    /// column's own append-only high-water mark — every minted slot,
    /// `add`'d or `reserve`'d, appends exactly one data-column row exactly
    /// once).
    ///
    /// This is the size an INDEX-ADDRESSED GPU mirror must allocate: `len()`
    /// is the LIVE count, but a still-live [`Handle`]'s `index()` can exceed
    /// `len() - 1` once a hole exists.
    #[inline]
    pub fn high_water(&self) -> usize {
        self.col.count()
    }

    /// Resolves a dense row `index` DIRECTLY, bypassing the generation check
    /// — the sanctioned way a render carrier that stores only a raw `u32`
    /// index resolves back to its record. Returns `None` if `index` is out
    /// of range or the row is not `Loaded`.
    #[inline]
    pub fn get_by_index(&self, index: u32) -> Option<&T> {
        let idx = index as usize;
        if idx >= self.slot_word.len() {
            return None;
        }
        let word = self.slot_word.get(idx).expect("invariant: idx < slot_word.len()");
        if unpack_state(word) != STATE_LOADED {
            return None;
        }
        let ptr = self.col.get_raw(idx).expect("invariant: a Loaded slot's row must be < col.count()");
        // SAFETY: same as `get`.
        Some(unsafe { &*ptr.cast::<T>() })
    }

    /// Returns the [`AssetLoadState`] of the row `handle` addresses, or
    /// `None` if `handle` is out of range or stale.
    #[inline]
    pub fn state(&self, handle: Handle<T>) -> Option<AssetLoadState> {
        let idx = self.resolve_index(handle)?;
        let word = self.slot_word.get(idx).expect("invariant: idx < slot_word.len()");
        match unpack_state(word) {
            STATE_LOADING => Some(AssetLoadState::Loading),
            STATE_LOADED => Some(AssetLoadState::Loaded),
            STATE_FAILED => Some(AssetLoadState::Failed),
            _ => unreachable!(
                "invariant: a matching generation implies a Loading/Loaded/Failed slot"
            ),
        }
    }

    /// Monotonically increasing counter bumped by every [`Self::get_mut`] /
    /// [`Self::fill`] call that resolves to a live row.
    #[inline]
    pub fn dirty_gen(&self) -> u64 {
        self.dirty_gen
    }

    /// Iterates every live (`Loaded`) `(Handle<T>, &T)` pair, skipping
    /// `Loading`/`Failed`/`Vacant` rows.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> + '_ {
        (0..self.slot_word.len()).filter_map(move |idx| {
            let word = self.slot_word.get(idx).expect("invariant: idx < slot_word.len()");
            if unpack_state(word) != STATE_LOADED {
                return None;
            }
            let ptr = self.col.get_raw(idx).expect("invariant: a Loaded slot's row must be < col.count()");
            // SAFETY: `idx`'s packed state is Loaded (checked above), so the
            // data column's row holds a valid, initialized `T`; the `&self`
            // borrow this iterator holds keeps it alive for the yielded
            // reference's lifetime.
            let value = unsafe { &*ptr.cast::<T>() };
            Some((Handle::new(idx as u32, unpack_generation(word)), value))
        })
    }

    /// The generation currently stamped on dense row `slot` (streaming
    /// plumbing — F2's ref-gen validation reads this). Inert in F1.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn generation(&self, slot: u32) -> u32 {
        let word = self.slot_word.get(slot as usize).expect("invariant: slot < slot_word.len()");
        unpack_generation(word)
    }

    /// The [`AssetLoadState`] currently stamped on dense row `slot`, bypassing
    /// the generation check (streaming plumbing — F2's ref-gen validation
    /// reads this via a bare dense id). Inert in F1.
    ///
    /// # Panics (debug only)
    /// If `slot`'s packed state is `Vacant`/`Retiring` — neither has an
    /// `AssetLoadState` mapping until F2 wires `Retiring`-aware validation;
    /// callers today only ever reach a `Loading`/`Loaded`/`Failed` row.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn state_of(&self, slot: u32) -> AssetLoadState {
        let word = self.slot_word.get(slot as usize).expect("invariant: slot < slot_word.len()");
        match unpack_state(word) {
            STATE_LOADING => AssetLoadState::Loading,
            STATE_LOADED => AssetLoadState::Loaded,
            STATE_FAILED => AssetLoadState::Failed,
            other => {
                debug_assert!(
                    false,
                    "state_of: slot {slot} is Vacant/Retiring (packed state {other}) — no \
                     AssetLoadState mapping exists until F2 wires Retiring-aware validation"
                );
                AssetLoadState::Failed
            }
        }
    }

    /// Monotonic counter bumped on every free/generation-advance (streaming
    /// plumbing — F5's validation early-out reads this). Inert in F1 (bumped
    /// by [`Self::remove`], read by nothing yet).
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn free_epoch(&self) -> u64 {
        self.free_epoch
    }

    /// Marks dense row `slot` NEVER-RETIRE (streaming plumbing — F2 pins slot
    /// 0, the default asset). Inert in F1: no retire path reads `pinned` yet.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn pin(&mut self, slot: u32) {
        self.pinned.set(slot as usize);
    }
}

impl<T: AssetBacking> Default for Assets<T> {
    /// An empty table with no rows reserved.
    fn default() -> Self {
        Self::with_reserved(0)
    }
}

impl<T: AssetBacking + Send + Sync> Resource for Assets<T> {
    // The `ResourceId` is minted through the `TypeId`-keyed process-global
    // registry, NOT a `static ID: OnceLock<_>` in this generic body — see
    // `resource_type_registry`'s rust#22991 rationale.
    #[inline]
    fn resource_id() -> ResourceId {
        resource_id_for::<Assets<T>>()
    }
}

// Any `Assets<T>` may be registered as a NonSend resource, regardless of
// whether `T` also happens to be `Send + Sync` (in which case it may ALSO be
// registered as a plain `Resource`, at the caller's choice). This impl must
// live HERE, not in a downstream crate: a downstream crate implementing this
// for a concrete `Assets<MeshGpu>` would hit the orphan rule (E0117).
impl<T: AssetBacking> NonSendResource for Assets<T> {}

// SAFETY: `Assets<T>` reproduces the `Vec<T>` auto-trait profile.
// `ComponentPool`/`VmColumn` hold a `NonNull<u8>`/`NonNull<u32>`, which is
// `!Send`/`!Sync` by default even though the memory they address is
// exclusively owned by this struct. Every mutation (`add`/`reserve`/`fill`/
// `fail`/`remove`/`get_mut`/`pin`) goes through `&mut self` (single-owner,
// no concurrent access), and every shared read (`get`/`get_by_index`/`iter`/
// `contains`/`state`/`generation`/`state_of`/`free_epoch`) exposes only `&T`
// and POD bookkeeping values — no `Cell`/`RefCell`/atomic interior mutability
// anywhere in the pool's own bookkeeping. So `Assets<T>` is `Send` whenever
// its element `T` is `Send`, exactly matching `Vec<T>: Send where T: Send`.
unsafe impl<T: AssetBacking + Send> Send for Assets<T> {}

// SAFETY: mirrors the `Send` impl above — shared `&Assets<T>` access exposes
// only `&T` reads and POD bookkeeping reads, so `Assets<T>` is `Sync`
// whenever `T` is `Sync`, matching `Vec<T>: Sync where T: Sync`.
unsafe impl<T: AssetBacking + Sync> Sync for Assets<T> {}

impl<T: AssetBacking> Drop for Assets<T> {
    /// Mirrors `DenseStore::drop`: the column's own terminal `Drop` runs
    /// `drop_fn` blindly over `[0, count)`, which would double-drop a
    /// tombstoned/never-filled row (`Vacant`/`Loading`/`Failed` rows hold no
    /// live `T`, only inert scratch/moved-from bytes). So this store drops
    /// each LIVE (`live`-bitmap-set) slot itself via `drop_at`, then walks
    /// the column's length down to 0 WITHOUT dropping — the now-empty
    /// column's terminal `Drop` is then a no-op and never re-touches a slot.
    fn drop(&mut self) {
        let len = self.col.count();
        for slot in 0..len {
            if self.live.test(slot) {
                // SAFETY: `slot < col.count()` and `live.test(slot)` — the
                // row was written by `push_value`/`write_value_at` and never
                // moved out (`remove` clears the `live` bit before
                // `take_at`). `&mut self` (Drop receives `&mut self`) gives
                // exclusive access. `drop_at` runs the registered `drop_fn`
                // exactly once on this live slot.
                unsafe { self.col.drop_at(slot) };
            }
        }
        while self.col.count() != 0 {
            self.col.pop_entity_no_drop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::impl_asset_pod_backing!(u64, u32);

    /// Stale-handle rejection across a full add → remove → reuse cycle
    /// (plan §A0 unit: generational stale-handle rejection).
    #[test]
    fn stale_handle_rejected_after_remove_and_reuse() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let old = assets.add(111);
        assert_eq!(assets.remove(old), Some(111));

        assert_eq!(assets.get(old), None, "removed handle must not resolve");
        assert!(
            assets.get_mut(old).is_none(),
            "removed handle must not resolve mutably"
        );
        assert!(!assets.contains(old), "removed handle must report absent");

        let fresh = assets.add(222);
        assert_eq!(
            fresh.index(),
            old.index(),
            "the freed row must be reused (same index)"
        );
        assert_eq!(
            fresh.generation(),
            old.generation() + 1,
            "reuse must bump the generation by exactly one"
        );

        assert_eq!(assets.get(fresh), Some(&222));
        assert_eq!(
            assets.get(old),
            None,
            "the OLD handle must still fail after the row was reused"
        );
        assert!(!assets.contains(old));
    }

    /// The free list is LIFO and rows are correctly reused without ever
    /// producing two live handles for the same row (plan §A0 unit:
    /// free-list LIFO reuse).
    #[test]
    fn free_list_reuses_lifo_without_duplication() {
        let mut assets = Assets::<u64>::with_reserved(8);
        let handles: Vec<_> = (0..5u64).map(|v| assets.add(v)).collect();
        assert_eq!(assets.len(), 5);

        // Remove rows 1 and 3 (in that order) — LIFO reuse must hand back
        // row 3 first, then row 1.
        assert_eq!(assets.remove(handles[1]), Some(1));
        assert_eq!(assets.remove(handles[3]), Some(3));
        assert_eq!(assets.len(), 3);

        let reuse_a = assets.add(301);
        assert_eq!(
            reuse_a.index(),
            handles[3].index(),
            "LIFO: the most recently freed row is reused first"
        );
        let reuse_b = assets.add(302);
        assert_eq!(
            reuse_b.index(),
            handles[1].index(),
            "LIFO: the second-most recently freed row is reused second"
        );

        assert_eq!(assets.len(), 5, "len() must reflect exactly the 5 live rows");
        assert_eq!(assets.get(handles[0]), Some(&0));
        assert_eq!(assets.get(handles[2]), Some(&2));
        assert_eq!(assets.get(handles[4]), Some(&4));
        assert_eq!(assets.get(reuse_a), Some(&301));
        assert_eq!(assets.get(reuse_b), Some(&302));
    }

    /// `iter()` visits exactly the `Loaded` rows, in row order (plan §A0
    /// unit: `iter()` visits exactly the Occupied slots).
    #[test]
    fn iter_visits_only_occupied_rows() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h0 = assets.add(10);
        let h1 = assets.add(20);
        let h2 = assets.add(30);
        assets.remove(h1);

        let visited: Vec<(Handle<u64>, u64)> = assets.iter().map(|(h, v)| (h, *v)).collect();
        assert_eq!(
            visited,
            vec![(h0, 10), (h2, 30)],
            "iter() must skip the vacated row and preserve the rest in order"
        );
    }

    /// `get_by_index` resolves a dense row bypassing generation, returns `None`
    /// out of range, and correctly stops resolving a freed row while a still-live
    /// higher index remains resolvable (plan A2 unit: the render-carrier indexed
    /// accessor added for `MeshHandle`-shaped consumers).
    #[test]
    fn get_by_index_resolves_dense_row_bypassing_generation() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h0 = assets.add(10);
        let h1 = assets.add(20);

        assert_eq!(assets.get_by_index(h1.index()), Some(&20), "index 1 is the 2nd row");
        assert_eq!(assets.get_by_index(5), None, "an out-of-range index must be None");

        assets.remove(h0);
        assert_eq!(
            assets.get_by_index(h0.index()),
            None,
            "a freed row must resolve to None even though the index is in range"
        );
        assert_eq!(
            assets.get_by_index(h1.index()),
            Some(&20),
            "a still-live higher index remains resolvable after a lower row is freed"
        );
    }

    /// `len()` / `is_empty()` track live rows through interleaved add/remove.
    #[test]
    fn len_and_is_empty_track_live_count() {
        let mut assets = Assets::<u64>::with_reserved(2);
        assert!(assets.is_empty());
        let h = assets.add(1);
        assert_eq!(assets.len(), 1);
        assert!(!assets.is_empty());
        assets.remove(h);
        assert_eq!(assets.len(), 0);
        assert!(assets.is_empty());
    }

    /// `get_mut` bumps `dirty_gen`; `get` does not.
    #[test]
    fn get_mut_bumps_dirty_gen_get_does_not() {
        let mut assets = Assets::<u64>::with_reserved(1);
        let h = assets.add(7);
        assert_eq!(assets.dirty_gen(), 0);
        let _ = assets.get(h);
        assert_eq!(assets.dirty_gen(), 0, "get() must not bump dirty_gen");
        *assets.get_mut(h).expect("h is live") += 1;
        assert_eq!(assets.dirty_gen(), 1);
        assert_eq!(assets.get(h), Some(&8));
    }

    /// `reserve()` mints a Loading row with no value: `get`/`get_by_index`
    /// must not resolve it, `contains` must report absent, `len()` must NOT
    /// count it, and `high_water()` must grow by one (plan §A3a unit:
    /// reserve mints an unresolvable, uncounted row).
    #[test]
    fn reserve_mints_unresolvable_row_without_bumping_live() {
        let mut assets = Assets::<u64>::with_reserved(4);
        assert_eq!(assets.high_water(), 0);

        let h = assets.reserve();

        assert_eq!(assets.state(h), Some(AssetLoadState::Loading), "a fresh reservation is Loading");
        assert_eq!(assets.get(h), None, "a Reserved row has no value to return");
        assert!(assets.get_mut(h).is_none(), "a Reserved row has no value to return mutably");
        assert_eq!(assets.get_by_index(h.index()), None, "get_by_index must also skip a Reserved row");
        assert!(!assets.contains(h), "contains() checks occupancy, not just generation");
        assert_eq!(assets.len(), 0, "reserve() must not bump the live count");
        assert!(assets.is_empty());
        assert_eq!(assets.high_water(), 1, "reserve() still mints a fresh row index");
    }

    /// `fill()` on a matching `Reserved` row transitions it to `Occupied` +
    /// `Loaded`, bumps `live` and `dirty_gen`, and makes the value resolvable
    /// (plan §A3a unit: fill resolves a reservation).
    #[test]
    fn fill_resolves_reserved_row_to_loaded() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.reserve();
        assert_eq!(assets.dirty_gen(), 0);

        let result = assets.fill(h, 42);

        assert_eq!(result, Ok(()), "fill on a matching Reserved row must succeed");
        assert_eq!(assets.state(h), Some(AssetLoadState::Loaded));
        assert_eq!(assets.get(h), Some(&42));
        assert_eq!(assets.len(), 1, "fill() must bump live exactly once");
        assert_eq!(assets.dirty_gen(), 1, "fill() must bump dirty_gen");
        assert!(assets.contains(h));
    }

    /// The row index `reserve()` returns is the SAME row `fill()` writes —
    /// the reserve/fill handle-binding contract (plan §A3a unit:
    /// reserve→fill round-trips the same row).
    #[test]
    fn reserve_then_fill_round_trips_the_same_row() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.reserve();
        let idx = h.index();

        assets.fill(h, 7).expect("fill on a fresh reservation must succeed");

        assert_eq!(assets.get_by_index(idx), Some(&7), "fill must write the exact row reserve() minted");
    }

    /// `fill()` on an already-`Occupied` row (a double-fill) errors WITHOUT
    /// overwriting the existing value or double-counting `live`, and RETURNS
    /// the rejected value (F1: `fill` no longer silently drops it) (plan
    /// §A3a unit: double-fill is rejected, not silently applied).
    #[test]
    fn fill_on_occupied_row_errors_and_does_not_overwrite() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(111);
        assert_eq!(assets.len(), 1);

        let result = assets.fill(h, 222);

        assert_eq!(
            result,
            Err((AssetError::StaleHandle, 222)),
            "double-fill on an Occupied row must error and return the rejected value"
        );
        assert_eq!(assets.get(h), Some(&111), "the original value must be untouched");
        assert_eq!(assets.len(), 1, "live must not be double-counted");
    }

    /// `fill()` on a STALE handle (its row was removed and its generation
    /// bumped) errors, returns the value, and touches nothing (plan §A3a
    /// unit: fill rejects a stale handle).
    #[test]
    fn fill_on_stale_handle_errors() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.reserve();
        assets.remove(h);

        let result = assets.fill(h, 9);

        assert_eq!(
            result,
            Err((AssetError::StaleHandle, 9)),
            "a stale (removed) handle must be rejected by fill, returning the value"
        );
        assert_eq!(assets.len(), 0);
    }

    /// `fill()` on a handle that was never reserved/added (out-of-range
    /// index) errors and returns the value (plan §A3a unit: fill rejects an
    /// unminted handle).
    #[test]
    fn fill_on_never_reserved_handle_errors() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let phantom = Handle::new(99, 0);

        let result = assets.fill(phantom, 1);

        assert_eq!(result, Err((AssetError::StaleHandle, 1)));
        assert_eq!(assets.len(), 0);
    }

    /// `fail()` on a matching `Reserved` row marks it `Failed` WITHOUT ever
    /// producing a value (plan §A3a unit: fail marks Failed with no value).
    #[test]
    fn fail_marks_reserved_row_failed_without_value() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.reserve();

        assets.fail(h);

        assert_eq!(assets.state(h), Some(AssetLoadState::Failed));
        assert_eq!(assets.get(h), None, "a Failed row still has no value");
        assert_eq!(assets.len(), 0, "a Failed row is never counted in live");
        assert!(!assets.contains(h));
    }

    /// `fail()` on an already-`Occupied` row is a no-op — its state and
    /// value are untouched (plan §A3a unit: fail is a no-op on a non-Reserved
    /// row).
    #[test]
    fn fail_on_occupied_row_is_noop() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(5);

        assets.fail(h);

        assert_eq!(assets.state(h), Some(AssetLoadState::Loaded), "fail() must not touch an Occupied row's state");
        assert_eq!(assets.get(h), Some(&5));
        assert_eq!(assets.len(), 1);
    }

    /// `fill()` after `fail()` on the SAME still-`Reserved` row succeeds:
    /// `fill`'s precondition checks only Loading/Failed occupancy, not a
    /// further sub-state (plan §A3a unit: fill after fail resurrects the
    /// row).
    #[test]
    fn fill_after_fail_resurrects_the_row_to_loaded() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.reserve();
        assets.fail(h);
        assert_eq!(assets.state(h), Some(AssetLoadState::Failed));

        let result = assets.fill(h, 5);

        assert_eq!(result, Ok(()), "fill only checks Loading/Failed occupancy, not a further sub-state");
        assert_eq!(assets.state(h), Some(AssetLoadState::Loaded));
        assert_eq!(assets.get(h), Some(&5));
        assert_eq!(assets.len(), 1);
    }

    /// The C1 case: `remove()` of a `Reserved` (still-`Loading`) row must
    /// return `None` WITHOUT underflowing `live` (plan §A3a unit: remove of
    /// a Loading row is sound).
    #[test]
    fn remove_of_loading_row_returns_none_without_underflowing_live() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.reserve();
        assert_eq!(assets.len(), 0);

        let removed = assets.remove(h);

        assert_eq!(removed, None, "removing a Loading row yields no value");
        assert_eq!(assets.len(), 0, "live must stay 0 — it was never bumped for a Reserved row");
    }

    /// `remove()` of a `Reserved` row recycles its index: a following
    /// `reserve()` reuses the SAME index with the generation bumped by
    /// exactly one (plan §A3a unit: remove of a Loading row recycles the
    /// slot).
    #[test]
    fn remove_of_loading_row_recycles_index_with_bumped_generation() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.reserve();
        assets.remove(h);

        let reused = assets.reserve();

        assert_eq!(reused.index(), h.index(), "the freed row must be reused (same index)");
        assert_eq!(reused.generation(), h.generation() + 1, "reuse must bump the generation by exactly one");
        assert_eq!(assets.state(reused), Some(AssetLoadState::Loading));
    }

    /// `remove()` of a `Failed` row behaves identically to a `Loading` row:
    /// no value, no underflow, and the slot recycles (plan §A3a unit: remove
    /// of a Failed row is sound).
    #[test]
    fn remove_of_failed_row_returns_none_without_underflowing_live() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.reserve();
        assets.fail(h);
        assert_eq!(assets.len(), 0);

        let removed = assets.remove(h);

        assert_eq!(removed, None);
        assert_eq!(assets.len(), 0, "live must stay 0 across a Failed row's removal");

        let reused = assets.reserve();
        assert_eq!(reused.index(), h.index(), "a Failed row's index must recycle exactly like a Loading row's");
        assert_eq!(reused.generation(), h.generation() + 1);
    }

    /// `remove()` of a `Vacant`/stale handle is a no-op: `None`, and `live`
    /// is untouched — a double-remove must not double-decrement (plan §A3a
    /// unit: double-remove is safe).
    #[test]
    fn remove_of_already_removed_handle_is_noop() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(1);
        assert_eq!(assets.remove(h), Some(1));
        assert_eq!(assets.len(), 0);

        let second = assets.remove(h);

        assert_eq!(second, None, "removing an already-removed handle must be a no-op");
        assert_eq!(assets.len(), 0, "live must not underflow on a double-remove");
    }

    /// An interleaved add → reserve → fill → fail → remove sequence never
    /// underflows `live`/`len()` — a concrete regression pin for the C1
    /// rewrite, complementing the extended proptest oracle below (plan §A3a
    /// unit: interleaved sequence never underflows).
    #[test]
    fn interleaved_add_reserve_fill_fail_remove_never_underflows_live() {
        let mut assets = Assets::<u64>::with_reserved(8);

        let a = assets.add(1); // Occupied
        let b = assets.reserve(); // Loading
        let c = assets.reserve();
        assets.fail(c); // Failed
        let d = assets.reserve();
        assets.fill(d, 4).expect("fresh reservation must fill"); // Loaded

        assert_eq!(assets.len(), 2, "only a and d are live (Occupied)");

        assets.remove(b); // Loading row — must not underflow
        assets.remove(c); // Failed row — must not underflow
        assert_eq!(assets.len(), 2, "removing non-Occupied rows must not change live");

        assets.remove(a);
        assets.remove(d);
        assert_eq!(assets.len(), 0, "removing both Occupied rows brings live back to 0, not below");
    }

    /// `add()`'s existing behavior is unchanged by the rewrite: it
    /// transitions straight to `Occupied` + `Loaded` (regression guard for
    /// plan §A3a: add() path unchanged).
    #[test]
    fn add_transitions_directly_to_occupied_loaded() {
        let mut assets = Assets::<u64>::with_reserved(1);

        let h = assets.add(99);

        assert_eq!(assets.state(h), Some(AssetLoadState::Loaded));
        assert_eq!(assets.get(h), Some(&99));
        assert_eq!(assets.len(), 1);
        assert!(assets.contains(h));
    }

    /// F1 slot-id parity (FIX-F): a deterministic add/reserve/remove/add
    /// sequence mints the SAME slot ids the OLD Vec-backed store did (append
    /// order + LIFO free-list reuse) — proving the ComponentPool-backed
    /// rewrite is a pure storage-container swap, invisible to any consumer
    /// keyed on `Handle::index()` (e.g. a GPU-mirror gather) or a golden's
    /// byte-identity.
    #[test]
    fn slot_ids_match_the_old_vec_backed_append_and_lifo_reuse_order() {
        let mut assets = Assets::<u64>::with_reserved(4);

        let h0 = assets.add(10);
        let h1 = assets.add(20);
        let h2 = assets.add(30);
        let h3 = assets.reserve();
        let h4 = assets.add(40);
        assert_eq!(
            [h0.index(), h1.index(), h2.index(), h3.index(), h4.index()],
            [0, 1, 2, 3, 4],
            "append order must be monotonic (matches Vec::push's index sequence)"
        );

        assets.remove(h1);
        assets.remove(h3);
        let reuse_a = assets.add(301);
        let reuse_b = assets.add(302);
        assert_eq!(reuse_a.index(), 3, "the most recently freed slot (3) must be reused first (LIFO)");
        assert_eq!(reuse_b.index(), 1, "the second-most recently freed slot (1) must be reused second (LIFO)");
        assert_eq!(reuse_a.generation(), h3.generation() + 1);
        assert_eq!(reuse_b.generation(), h1.generation() + 1);

        let h5 = assets.add(50);
        assert_eq!(h5.index(), 5, "a fresh mint past every freed slot continues the monotonic append sequence");
    }

    /// Miri-TB: `ComponentPool::take_at` (via `remove`) moves the value out
    /// EXACTLY ONCE — never drops it in place — and the store's terminal
    /// `Drop` drops exactly the still-live slots, never a removed
    /// (tombstoned) one (asset-streaming plan F1: the take_at/terminal-Drop
    /// exactly-once contract).
    #[test]
    fn take_at_and_terminal_drop_are_exactly_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        struct RecordDrop;
        impl Drop for RecordDrop {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }

        // SAFETY: caller (`ComponentPool::drop_at` / `take_at` / the
        // terminal `Assets::drop`) guarantees `ptr` points at a valid,
        // aligned, fully-initialized `RecordDrop`, exclusively owned and not
        // accessed again after this call — the standard `DropFn` contract.
        unsafe fn record_drop_glue(ptr: *mut u8) {
            unsafe { core::ptr::drop_in_place(ptr.cast::<RecordDrop>()) }
        }

        impl AssetBacking for RecordDrop {
            const NEEDS_TEARDOWN: bool = true;
            fn register_layout() -> ComponentId {
                crate::ecs::core::asset::backing::register_asset_layout::<RecordDrop>(Some(record_drop_glue))
            }
        }

        DROP_COUNT.store(0, Ordering::Relaxed);
        {
            let mut assets = Assets::<RecordDrop>::with_reserved(4);
            let h1 = assets.add(RecordDrop);
            let h2 = assets.add(RecordDrop);
            let _h3 = assets.add(RecordDrop);

            let taken = assets.remove(h1);
            assert!(taken.is_some(), "remove() of a Loaded row must return the moved-out value");
            assert_eq!(
                DROP_COUNT.load(Ordering::Relaxed),
                0,
                "remove()'s take_at must move the value out WITHOUT dropping it in place"
            );
            drop(taken);
            assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 1, "dropping the returned owned value runs Drop exactly once");

            let taken2 = assets.remove(h2);
            drop(taken2);
            assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 2);

            // `_h3` remains live; the store's terminal Drop (below, at scope
            // exit) must drop exactly this one remaining slot.
        }
        assert_eq!(
            DROP_COUNT.load(Ordering::Relaxed),
            3,
            "terminal Drop for Assets<T> must drop exactly the one still-live slot — no \
             double-drop of the two already-removed (tombstoned) ones"
        );
    }

    /// Miri-TB complement: a `Loading`/`Failed` row's inert scratch bytes are
    /// NEVER interpreted as a live `T` by the terminal `Drop` (no
    /// drop-of-uninit) — only the genuinely `Loaded` row is dropped.
    #[test]
    fn drop_never_touches_a_loading_or_failed_row() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static DROP_COUNT_2: AtomicUsize = AtomicUsize::new(0);

        struct RecordDrop2;
        impl Drop for RecordDrop2 {
            fn drop(&mut self) {
                DROP_COUNT_2.fetch_add(1, Ordering::Relaxed);
            }
        }

        // SAFETY: same `DropFn` contract as `record_drop_glue` above.
        unsafe fn record_drop_glue_2(ptr: *mut u8) {
            unsafe { core::ptr::drop_in_place(ptr.cast::<RecordDrop2>()) }
        }

        impl AssetBacking for RecordDrop2 {
            const NEEDS_TEARDOWN: bool = true;
            fn register_layout() -> ComponentId {
                crate::ecs::core::asset::backing::register_asset_layout::<RecordDrop2>(Some(record_drop_glue_2))
            }
        }

        DROP_COUNT_2.store(0, Ordering::Relaxed);
        {
            let mut assets = Assets::<RecordDrop2>::with_reserved(4);
            let _loading = assets.reserve();
            let failed = assets.reserve();
            assets.fail(failed);
            let _live = assets.add(RecordDrop2);
            // Terminal Drop below must drop ONLY `_live`.
        }
        assert_eq!(
            DROP_COUNT_2.load(Ordering::Relaxed),
            1,
            "terminal Drop must drop exactly the one Loaded row, never a Loading/Failed row's inert scratch bytes"
        );
    }

    /// proptest oracle: a random sequence of add/reserve/fill/fail/remove/get
    /// against a model `HashMap<Handle<u64>, u64>` (Occupied rows) + a
    /// `HashSet<Handle<u64>>` (rows currently Loading/Failed) (plan §A0
    /// proptest, extended at §A3a, ported verbatim at F1). `Assets` must
    /// never return a stale value, never resolve to the wrong row, and its
    /// live count must always match the model's.
    mod oracle {
        use std::collections::{HashMap, HashSet};

        use proptest::prelude::*;

        use super::*;

        #[derive(Clone, Debug)]
        enum Op {
            Add(u64),
            Reserve,
            FillAt(usize, u64),
            FailAt(usize),
            RemoveAt(usize),
            GetAt(usize),
        }

        fn op_strategy() -> impl Strategy<Value = Op> {
            prop_oneof![
                any::<u64>().prop_map(Op::Add),
                Just(Op::Reserve),
                (any::<usize>(), any::<u64>()).prop_map(|(i, v)| Op::FillAt(i, v)),
                any::<usize>().prop_map(Op::FailAt),
                any::<usize>().prop_map(Op::RemoveAt),
                any::<usize>().prop_map(Op::GetAt),
            ]
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(256))]
            #[test]
            fn assets_matches_hashmap_oracle(ops in proptest::collection::vec(op_strategy(), 1..300)) {
                let mut assets = Assets::<u64>::with_reserved(16);
                let mut oracle: HashMap<Handle<u64>, u64> = HashMap::new();
                let mut reserved_pending: HashSet<Handle<u64>> = HashSet::new();
                let mut minted: Vec<Handle<u64>> = Vec::new();

                for op in ops {
                    match op {
                        Op::Add(value) => {
                            let handle = assets.add(value);
                            oracle.insert(handle, value);
                            minted.push(handle);
                        }
                        Op::Reserve => {
                            let handle = assets.reserve();
                            reserved_pending.insert(handle);
                            minted.push(handle);
                        }
                        Op::FillAt(pick, value) => {
                            if let Some(&handle) = minted.get(pick % minted.len().max(1)) {
                                let expected_ok = reserved_pending.contains(&handle);
                                let real = assets.fill(handle, value);
                                if expected_ok {
                                    prop_assert!(real.is_ok(), "fill on a Reserved row must succeed");
                                    oracle.insert(handle, value);
                                    reserved_pending.remove(&handle);
                                } else {
                                    prop_assert!(real.is_err(), "fill on a non-Reserved/stale handle must error");
                                }
                            }
                        }
                        Op::FailAt(pick) => {
                            if let Some(&handle) = minted.get(pick % minted.len().max(1)) {
                                assets.fail(handle);
                            }
                        }
                        Op::RemoveAt(pick) => {
                            if let Some(&handle) = minted.get(pick % minted.len().max(1)) {
                                let was_occupied = oracle.contains_key(&handle);
                                let real = assets.remove(handle);
                                let model = if was_occupied { oracle.remove(&handle) } else { None };
                                prop_assert_eq!(
                                    real, model,
                                    "remove() must match the oracle exactly, including a Reserved row's \
                                     None with no underflow"
                                );
                                reserved_pending.remove(&handle);
                            }
                        }
                        Op::GetAt(pick) => {
                            if let Some(&handle) = minted.get(pick % minted.len().max(1)) {
                                let real = assets.get(handle).copied();
                                let model = oracle.get(&handle).copied();
                                prop_assert_eq!(real, model, "get() must match the oracle exactly (never a stale or wrong value)");
                            }
                        }
                    }
                    prop_assert_eq!(
                        assets.len(), oracle.len(),
                        "live count must always match the oracle — the C1 invariant: never \
                         under/over-counts across reserve/fill/fail/remove"
                    );
                }
            }
        }
    }
}
