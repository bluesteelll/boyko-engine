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
//! # Streaming fields — refcount wired at F2, the rest still inert
//!
//! [`inc_ref`](Assets::inc_ref) / [`dec_ref`](Assets::dec_ref) (asset-streaming
//! plan F2, gen-checked as of F5) are the refcount lifetime driver: a carrier
//! hook (`on_insert` / `on_replace` on a `boyko_scene` `MeshHandle` /
//! `MaterialHandle`) pushes a delta, a `boyko_render` apply system folds it
//! in, and [`dec_ref`](Assets::dec_ref) reaching zero on an unpinned row marks
//! it `Retiring` (see [`STATE_RETIRING`]), bumps
//! [`free_epoch`](Assets::free_epoch), sets [`dirty`](Assets::dirty), and
//! returns a [`RetireTicket`] the caller enqueues into the (still-undrained)
//! deferred-free queue. [`pin`](Assets::pin) marks a row NEVER-RETIRE (slot 0,
//! the default asset).
//!
//! # F5 — the generation gen-check closes the stale-decrement/reuse hole
//!
//! [`inc_ref`](Assets::inc_ref) now refuses (returns `false`, no mutation) a
//! `Retiring`/`Vacant` target — the sole resurrection hazard (a carrier
//! binding an already-`Retiring` slot); see its doc for the F5/F6 boundary
//! this establishes. [`dec_ref`](Assets::dec_ref) now takes the carrier's
//! bind-time `slot` generation and no-ops (no mutation, no `RetireTicket`) on
//! a mismatch — a stale weak `Handle`/carrier decrementing a slot that was
//! since retired-and-reused would otherwise corrupt the new tenant's
//! refcount; [`GEN_UNSYNCED`] bypasses the check for the same-frame
//! bound-but-not-yet-synced window. The generation-checked validation (a
//! `MeshRefGen`/`MaterialRefGen` lane pair, `boyko_render`'s
//! `validate_asset_refs`) is the best-effort net over these two durable
//! guards; the actual fence-gated device teardown (`retire_deferred_frees`)
//! is F6.
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
/// Occupied-but-unreusable, awaiting a future fence-gated
/// `retire_deferred_frees` pass (F6). Written by [`Assets::dec_ref`] (F2)
/// when a row's refcount reaches zero on an unpinned `Loaded`/`Failed`/
/// `Loading` row. A `Loaded`→`Retiring` row holds a valid `T` (nothing frees
/// its `col` bytes until F6 actually retires it) and
/// [`Assets::get_by_index`] still resolves it; a `Loading`/`Failed`→`Retiring`
/// row never held a `T` at all (only inert scratch bytes) and does NOT
/// resolve — see [`Assets::get_by_index`]'s doc for the `live`-gated
/// discriminator between the two.
const STATE_RETIRING: u32 = 4;

/// Sentinel generation (asset-streaming plan F5): marks a `boyko_scene`
/// `MeshRefGen`/`MaterialRefGen` lane as "bound this frame, not yet
/// gen-stamped by `apply_refcount_deltas`", and is the value
/// [`Assets::dec_ref`] treats as "skip the gen-check" (the
/// bound-but-not-yet-synced window, where no reuse can have happened — reuse
/// requires a prior-frame sync). Real generations are 29-bit
/// (`< 2^29`, see [`pack_slot_word`]'s doc), so `u32::MAX` never collides
/// with one.
pub const GEN_UNSYNCED: u32 = u32::MAX;

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

/// Routing tag for the fence-gated deferred-free queue (F6) — names which
/// `Assets<T>` table a queued free entry belongs to. Wraps the type's own
/// [`ComponentId`] (already unique per asset type via
/// [`AssetBacking::register_layout`]) rather than minting a second registry.
///
/// `pub` (promoted from F1's `pub(crate)`) so [`RetireTicket::kind`] is
/// reachable from a cross-crate caller (asset-streaming plan F2: the
/// `boyko_render` apply system that calls [`Assets::dec_ref`]). The inner
/// [`ComponentId`] stays private — callers carry the value opaquely; only
/// this module constructs one (via [`Assets::with_reserved`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetKind(#[allow(dead_code)] ComponentId);

/// Signal returned by [`Assets::dec_ref`] when a row's refcount just reached
/// zero and the row transitioned to `Retiring` (asset-streaming plan F2 §1).
/// The caller (the streaming apply system) enqueues this into the
/// fence-gated deferred-free queue; F2 defines the queue and enqueues, F6
/// drains it. Never re-issued for the same retire — [`Assets::dec_ref`] is
/// idempotent once a row is `Retiring`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetireTicket {
    /// Which `Assets<T>` table [`Self::slot`] belongs to.
    pub kind: AssetKind,
    /// The dense row that just transitioned to `Retiring`.
    pub slot: u32,
}

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
/// - [`install_epoch`](Self::install_epoch) — bumped by every [`Self::add`] /
///   [`Self::fill`] call that writes a live `T` into a row, INCLUDING a
///   free-list reuse, AND by every [`Self::retire`] (the terminal
///   Retiring->Vacant transition) — asset-streaming plan F6: the signal a
///   GPU-mirror table whose gate is keyed on row-count growth alone (e.g. the
///   hwrt TLAS `blas_addr` table) needs to detect BOTH a reused slot's
///   re-install AND a bare retire-to-Vacant, which neither
///   [`high_water`](Self::high_water) nor [`dirty_gen`](Self::dirty_gen)
///   captures.
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
    install_epoch: u64,
    // The deferred-free queue's routing key, copied into every `RetireTicket`
    // `dec_ref` returns (F2); F6 is the actual consumer of the routed value.
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
            install_epoch: 0,
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
    /// appending a fresh row. Returns the [`Handle`] addressing it. Bumps
    /// [`Self::install_epoch`] on both paths — a GPU-mirror table gated on
    /// row-count growth alone (e.g. the hwrt TLAS `blas_addr` table) cannot
    /// otherwise detect the free-list-reuse path, since it leaves
    /// [`high_water`](Self::high_water) unchanged.
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
            self.install_epoch = self.install_epoch.wrapping_add(1);
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
        self.install_epoch = self.install_epoch.wrapping_add(1);
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
    /// from `Loading`/`Failed` to [`Loaded`](AssetLoadState::Loaded). Bumps
    /// [`Self::install_epoch`] on success — same rationale as [`Self::add`].
    ///
    /// # Errors
    /// Returns `(`[`AssetError::StaleHandle`]`, value)` — WITH `value`
    /// returned to the caller (never silently dropped: a future
    /// device-owning asset has no safe bare-drop path) — if `handle` does not
    /// resolve to a `Loading`/`Failed` row with a matching generation: an
    /// already-`Loaded` row (a double-fill), a `Vacant` row, an
    /// out-of-range index, or a stale generation are all rejected.
    ///
    /// # Routing obligation on `Err`
    /// The store never took ownership of a rejected `value` — for a
    /// device-owning `T` (e.g. `MeshGpu`, which has no `Drop`) the caller
    /// MUST route it into that type's orphan teardown queue (asset-streaming
    /// plan F6: `OrphanedMeshGpu`) rather than dropping it bare, or the
    /// device buffers/BLAS it holds leak.
    #[must_use = "a rejected value (e.g. MeshGpu) holds live device buffers with no Drop — \
                  route the Err value into its orphan teardown queue (OrphanedMeshGpu) or it leaks"]
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
        self.install_epoch = self.install_epoch.wrapping_add(1);
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
    /// `Loaded`, or `None` if it was `Loading`/`Failed`/`Retiring` (there was
    /// never a value to return, or the row is not this method's to free —
    /// see below).
    ///
    /// Synchronous (F1 semantics, unchanged from the pre-rewrite `Vec`-backed
    /// store): the value is moved out and the row recycled immediately — the
    /// deferred, fence-gated `Retiring` teardown is a later rung (F2/F6).
    /// Bumps the row's generation so the just-freed `handle` is rejected from
    /// this point on, including after the row is reused.
    ///
    /// # `Retiring` is a no-op (F2 — reachable-panic fix)
    ///
    /// `dec_ref`'s `Retiring` transition does NOT bump the row's generation,
    /// so a still-matching `handle` for a just-retired row reaches this far.
    /// A `Retiring` row is owned by the deferred-retire path — only a future
    /// fence-gated `retire_deferred_frees` pass (F6) may `take_at` it. This
    /// returns `None` WITHOUT clearing `live`, bumping the generation,
    /// pushing the free-list, or touching `free_epoch`/`dirty`: doing any of
    /// those here would let a manual `remove` double-take (or double-free-
    /// list-push) the same row F6 will later retire through its own path.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        let idx = self.resolve_index(handle)?;
        let word = self.slot_word.get(idx).expect("invariant: idx < slot_word.len()");
        let generation = unpack_generation(word);
        let state = unpack_state(word);

        if state == STATE_RETIRING {
            return None;
        }

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
                "invariant: a matching generation implies a Loading/Loaded/Failed slot (Retiring handled above)"
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
    /// of range, or the row does not hold a valid, live `T`.
    ///
    /// # `Retiring` resolves ONLY when it holds a live `T` (F2 — UB fix)
    ///
    /// A `Loaded`→`Retiring` row (asset-streaming plan F2: [`Assets::dec_ref`]
    /// reached zero on what was a `Loaded` row) is occupied-but-unreusable,
    /// NOT freed — its `col` bytes are untouched until a future fence-gated
    /// `retire_deferred_frees` pass (F6) actually moves the value out. So a
    /// render carrier whose referent just became `Retiring` this frame must
    /// keep resolving the SAME value it always did (no visible change, no
    /// dangling read) until that later rung retires the row.
    ///
    /// A `Loading`/`Failed`→`Retiring` row, by contrast, NEVER held a real
    /// `T` — [`Assets::reserve`]'s inert all-zero scratch bytes are not a
    /// valid `T` (a zeroed niche, e.g. a `NonNull` field, is immediate UB the
    /// instant `&T` is formed over it). [`live`](Self::live) is the precise
    /// discriminator: only [`Assets::add`]/[`Assets::fill`] ever set it, so
    /// `live.test(idx)` is exactly the "does this Retiring row hold a real
    /// `T`" predicate — false for a `Loading`/`Failed`→`Retiring` row (which
    /// therefore correctly returns `None`, exactly as it did before F2), true
    /// for a `Loaded`→`Retiring` one.
    #[inline]
    pub fn get_by_index(&self, index: u32) -> Option<&T> {
        let idx = index as usize;
        if idx >= self.slot_word.len() {
            return None;
        }
        let word = self.slot_word.get(idx).expect("invariant: idx < slot_word.len()");
        let state = unpack_state(word);
        let resolvable = match state {
            STATE_LOADED => true,
            // A Retiring row resolves only if it is the Loaded->Retiring case
            // (holds a real T); a Loading/Failed->Retiring row's `live` bit
            // was never set (see the field doc), so this excludes it.
            STATE_RETIRING => self.live.test(idx),
            _ => false,
        };
        if !resolvable {
            return None;
        }
        let ptr = self
            .col
            .get_raw(idx)
            .expect("invariant: a resolvable slot's row must be < col.count()");
        // SAFETY: `resolvable` is true only for a Loaded row, or a Retiring
        // row with `live.test(idx)` true (a Loaded->Retiring row) — both
        // cases guarantee the data-column row holds a valid, initialized `T`
        // written by `push_value`/`write_value_at`. A Loading/Failed row
        // (never written a `T`, live=false) is excluded by `resolvable`
        // above whether it is Loading/Failed/Retiring, so this never reads
        // uninitialized scratch bytes as `&T`.
        Some(unsafe { &*ptr.cast::<T>() })
    }

    /// Returns the [`AssetLoadState`] of the row `handle` addresses, or
    /// `None` if `handle` is out of range, stale, or the row is `Retiring`.
    ///
    /// # `Retiring` does not resolve (F2)
    ///
    /// `dec_ref`'s `Retiring` transition does NOT bump the row's generation
    /// (only [`Self::remove`]'s terminal `Vacant` transition does), so a
    /// still-matching `handle` for a just-retired row reaches this far. A
    /// `Retiring` row has no [`AssetLoadState`] mapping — it does not resolve
    /// through `state`, mirroring [`Self::resolve_occupied`]'s
    /// `STATE_LOADED`-only contract for [`Self::get`].
    #[inline]
    pub fn state(&self, handle: Handle<T>) -> Option<AssetLoadState> {
        let idx = self.resolve_index(handle)?;
        let word = self.slot_word.get(idx).expect("invariant: idx < slot_word.len()");
        match unpack_state(word) {
            STATE_LOADING => Some(AssetLoadState::Loading),
            STATE_LOADED => Some(AssetLoadState::Loaded),
            STATE_FAILED => Some(AssetLoadState::Failed),
            STATE_RETIRING => None,
            _ => unreachable!(
                "invariant: a matching generation implies a Loading/Loaded/Failed/Retiring slot"
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

    /// The generation currently stamped on dense row `slot` — `boyko_render`'s
    /// `apply_refcount_deltas` reads this on a `+1` delta to re-sync the
    /// carrier's `MeshRefGen`/`MaterialRefGen` lane (asset-streaming plan F5).
    ///
    /// # Panics
    /// If `slot >= `[`Self::high_water`] — every caller resolves `slot` from a
    /// carrier that is (or just was) live, so this is an invariant violation,
    /// not a user error.
    #[inline]
    pub fn generation(&self, slot: u32) -> u32 {
        let word = self.slot_word.get(slot as usize).expect("invariant: slot < slot_word.len()");
        unpack_generation(word)
    }

    /// OOR-safe twin of [`Self::generation`] — `None` if `slot >=`
    /// [`Self::high_water`] (asset-streaming plan F5: `validate_asset_refs`
    /// probes a bare carrier-held `u32` index, which — unlike every other
    /// caller of [`Self::generation`] — is not guaranteed in-range for a
    /// malformed/stale carrier).
    #[inline]
    pub fn try_generation(&self, slot: u32) -> Option<u32> {
        self.slot_word.get(slot as usize).map(unpack_generation)
    }

    /// The [`AssetLoadState`] currently stamped on dense row `slot`, bypassing
    /// the generation check (streaming plumbing — F2's ref-gen validation
    /// reads this via a bare dense id). Inert in F1.
    ///
    /// # Panics (debug only)
    /// If `slot`'s packed state is `Vacant`/`Retiring` — neither has an
    /// `AssetLoadState` mapping. Cross-crate callers that cannot guarantee
    /// `slot` resolves to a `Loading`/`Loaded`/`Failed` row (e.g. a malformed
    /// carrier) MUST use [`Self::state_of_index`] instead, which has no such
    /// precondition.
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
                    "state_of: slot {slot} is Vacant/Retiring (packed state {other}) — \
                     no AssetLoadState mapping; cross-crate callers must use state_of_index"
                );
                AssetLoadState::Failed
            }
        }
    }

    /// OOR/`Vacant`/`Retiring`-safe twin of [`Self::state_of`] (asset-streaming
    /// plan F5): `None` for an out-of-range `slot`, a never-minted/freed
    /// `Vacant` row, or a `Retiring` row (neither has an `AssetLoadState`
    /// mapping); `Some` otherwise. The sanctioned cross-crate probe —
    /// `validate_asset_refs` calls this on a bare carrier-held index that may
    /// name any row state, including a malformed carrier's out-of-range one.
    #[inline]
    pub fn state_of_index(&self, slot: u32) -> Option<AssetLoadState> {
        let word = self.slot_word.get(slot as usize)?;
        match unpack_state(word) {
            STATE_LOADING => Some(AssetLoadState::Loading),
            STATE_LOADED => Some(AssetLoadState::Loaded),
            STATE_FAILED => Some(AssetLoadState::Failed),
            _ => None,
        }
    }

    /// Monotonic counter bumped on every free/generation-advance (streaming
    /// plumbing — F5's validation early-out reads this; bumped by
    /// [`Self::remove`] and, as of F2, by [`Self::dec_ref`] on a Retiring
    /// transition).
    #[inline]
    pub fn free_epoch(&self) -> u64 {
        self.free_epoch
    }

    /// Monotonic counter bumped by every [`Self::add`] / [`Self::fill`] call
    /// that writes a live `T` into a row, AND by every [`Self::retire`]
    /// (asset-streaming plan F6). Unlike [`Self::high_water`], this ALSO
    /// advances on a free-list-reuse install (which leaves `high_water`
    /// unchanged) and on a bare retire-to-Vacant — the signal a GPU-mirror
    /// table gated purely on row-count growth (e.g. the hwrt TLAS
    /// `blas_addr` table) needs to detect a reused slot's new content, or a
    /// retired slot's Vacant sentinel, and avoid reading a freed device
    /// resource's stale address.
    #[inline]
    pub fn install_epoch(&self) -> u64 {
        self.install_epoch
    }

    /// Marks dense row `slot` NEVER-RETIRE (asset-streaming plan F2: pins
    /// slot 0, the default asset). [`Self::dec_ref`] reaching zero on a
    /// pinned row leaves it `Loaded` at refcount 0 instead of transitioning
    /// to `Retiring`. `pub` (promoted from F1's `pub(crate)`) so a
    /// cross-crate boot sequence (`boyko_app::runner`) can pin the default
    /// material immediately after minting it.
    #[inline]
    pub fn pin(&mut self, slot: u32) {
        self.pinned.set(slot as usize);
    }

    /// Increments the live-refcount of dense row `slot` — the carrier ATTACH
    /// event (asset-streaming plan F2 §1: a `MeshHandle`/`MaterialHandle`
    /// `on_insert` hook pushes `+1`, folded in by the `boyko_render` apply
    /// system). Refcount is otherwise driven only by [`Self::dec_ref`];
    /// [`Self::add`]/[`Self::reserve`] always mint a fresh row at refcount 0
    /// (an unattached load has no owner yet).
    ///
    /// Returns `true` iff the refcount was actually incremented.
    ///
    /// # State-guarded (F5 Decision 5) — refuses `Retiring`/`Vacant`
    ///
    /// A no-op (`false`, refcount untouched) if `slot` is out of range, or its
    /// row is `Retiring`/`Vacant`. The `Retiring` refusal is the sole
    /// resurrection-hazard guard: a carrier binding an ALREADY-`Retiring` slot
    /// (refcount 0, already queued for a future fence-gated free) must not
    /// resurrect it — this call no-ops (the real refcount never rises above
    /// 0), the slot stays queued and always eventually dies, and
    /// `validate_asset_refs` (F5) disables/substitutes the carrier once it
    /// observes the non-`Loaded` state. This is the exact F5/F6 boundary: F5
    /// guarantees a `Retiring` slot's refcount can never rise again, so a
    /// future F6 `retire_deferred_frees` needs no retire-time refcount
    /// recheck — it can free unconditionally.
    #[inline]
    pub fn inc_ref(&mut self, slot: u32) -> bool {
        let idx = slot as usize;
        let Some(word) = self.slot_word.get(idx) else {
            return false;
        };
        if !matches!(unpack_state(word), STATE_LOADING | STATE_LOADED | STATE_FAILED) {
            return false;
        }
        let count = self
            .refcount
            .get(idx)
            .expect("invariant: refcount column is 1:1 with slot_word (see with_reserved)");
        debug_assert!(count < u32::MAX, "invariant: refcount overflow at slot {slot}");
        self.refcount.set(idx, count + 1);
        true
    }

    /// Decrements the live-refcount of dense row `slot` — the carrier DETACH
    /// event (asset-streaming plan F2 §1: a `MeshHandle`/`MaterialHandle`
    /// `on_replace` hook pushes `-1`, folded in by the `boyko_render` apply
    /// system). When the count reaches zero on an unpinned `Loaded`/
    /// `Failed`/`Loading` row, the row transitions to `Retiring` (occupied-
    /// but-unreusable, awaiting a future fence-gated `retire_deferred_frees`
    /// pass — F6), bumps [`Self::free_epoch`], marks the row's `dirty` bit,
    /// and this returns a [`RetireTicket`] the caller must enqueue into the
    /// deferred-free queue.
    ///
    /// # Idempotent
    ///
    /// A row already `Retiring` is a no-op — `None`, refcount untouched. A
    /// `Vacant` row (freed via [`Self::remove`], unreachable from the F2
    /// hook pipeline in well-formed use) is likewise a no-op. This idempotent
    /// guard is what F6's single-`take_at` contract relies on: a row can
    /// never be enqueued (and later freed) twice.
    ///
    /// A pinned row (slot 0, the default asset — see [`Self::pin`]) never
    /// transitions: reaching zero leaves it `Loaded` at refcount 0 and
    /// returns `None`.
    ///
    /// A no-op (`None`) if `slot` is out of range — same rationale as
    /// [`Self::inc_ref`].
    ///
    /// # Gen-checked (F5 Decision 4) — closes the stale-decrement/reuse hole
    ///
    /// `gen_` is the carrier's BIND-time generation (the sibling
    /// `MeshRefGen`/`MaterialRefGen` lane the `on_replace` hook captured while
    /// the row was still live — see `boyko_scene::render_caps`'s hook doc). If
    /// `gen_ != `[`GEN_UNSYNCED`]` and `gen_` does not match `slot`'s CURRENT
    /// generation, this is a no-op (`None`, refcount untouched): the slot was
    /// retired-and-reused underneath a lost/stale ref since this carrier last
    /// synced, so decrementing here would corrupt the NEW tenant's refcount
    /// (FIX-B / W2). `GEN_UNSYNCED` bypasses the check — the same-frame
    /// bound-but-not-yet-synced window, where no reuse can have happened (a
    /// reuse requires the slot to have gone `Retiring` then been freed by a
    /// PRIOR frame's `retire_deferred_frees`).
    ///
    /// # Only `on_replace` decrements (deviation from a literal 2-hook wire)
    ///
    /// The kernel's `on_replace` fires exactly once per value-departure event
    /// — an in-place overwrite (`add A, then insert B` on the same entity) OR
    /// a genuine removal/despawn — always reading the correct dying/old
    /// value (`migrate_entity_insert`/`migrate_entity_remove`). `on_remove`
    /// fires ADDITIONALLY, but only for a genuine removal, reading the SAME
    /// dying value `on_replace` already saw. Wiring an independent `dec_ref`
    /// call on BOTH hooks would double-decrement every genuine removal
    /// whenever the row's refcount is still `> 1` at that point (this
    /// idempotent-on-Retiring guard only absorbs the duplicate when the
    /// FIRST call already reached zero) — the common shared-handle case
    /// (many entities referencing one mesh/material slot). So only
    /// `on_replace` is wired to `dec_ref`; see `boyko_scene::render_caps`'s
    /// hook wiring.
    #[inline]
    pub fn dec_ref(&mut self, slot: u32, gen_: u32) -> Option<RetireTicket> {
        let idx = slot as usize;
        let word = self.slot_word.get(idx)?;
        let cur_gen = unpack_generation(word);
        if gen_ != GEN_UNSYNCED && gen_ != cur_gen {
            return None;
        }
        // Tripwire (F5 Decision 4): a proceeding dec (i.e. control flow reaching
        // this point, past the mismatch-return above) implies a gen match or the
        // UNSYNCED bypass — restated explicitly so a future refactor that
        // reorders this block cannot silently reintroduce the stale-decrement
        // hole this check exists to close.
        debug_assert!(
            gen_ == GEN_UNSYNCED || gen_ == cur_gen,
            "invariant: dec_ref must not proceed past the gen-check on a real mismatch \
             (slot {slot}, gen_ {gen_}, cur_gen {cur_gen})"
        );
        let state = unpack_state(word);
        if state == STATE_RETIRING || state == STATE_VACANT {
            return None;
        }
        let count = self
            .refcount
            .get(idx)
            .expect("invariant: refcount column is 1:1 with slot_word (see with_reserved)");
        debug_assert!(
            count > 0,
            "invariant: dec_ref underflow — refcount already 0 at slot {slot}"
        );
        let new_count = count.saturating_sub(1);
        self.refcount.set(idx, new_count);
        if new_count != 0 || self.pinned.test(idx) {
            return None;
        }
        // The `Retiring || Vacant` early-return above already excludes every
        // state but Loaded/Failed/Loading — this is a documentation-only
        // invariant check, not a live branch (O1 cleanup: the prior
        // `if !matches!(..) { return None }` guard here was dead code).
        debug_assert!(
            matches!(state, STATE_LOADED | STATE_FAILED | STATE_LOADING),
            "invariant: dec_ref's zero-crossing transition state must be Loaded/Failed/Loading \
             here — Retiring/Vacant are excluded by the early-return above"
        );
        let generation = unpack_generation(word);
        self.slot_word.set(idx, pack_slot_word(generation, STATE_RETIRING));
        self.free_epoch = self.free_epoch.wrapping_add(1);
        self.dirty.set(idx);
        Some(RetireTicket { kind: self.id, slot })
    }

    /// Terminal free of a `Retiring` row — the deferred-retire path's other
    /// half of [`Self::dec_ref`] ([`Self::remove`] deliberately no-ops on
    /// `Retiring`, see its doc). Called ONLY by the fence-gated drain
    /// (asset-streaming plan F6: `retire_deferred_frees`) once every GPU
    /// submit that could reference this row's `T` is provably complete (see
    /// the F6 design's fence-gate proof).
    ///
    /// Returns the value for a `Loaded`->`Retiring` row (held a live `T`,
    /// `live` bit set), or `None` for a `Loading`/`Failed`->`Retiring` row
    /// (a cancelled load — never held a `T`; the same `live`-gated
    /// discriminator [`Self::get_by_index`] uses). Transitions the row to
    /// `Vacant` (generation bumped so a stale `Handle` is rejected even after
    /// reuse), clears the refcount, pushes the free-list, bumps
    /// [`Self::free_epoch`], and marks the row [`Self::dirty`] — mirrors
    /// [`Self::remove`]'s terminal-`Vacant` bookkeeping exactly.
    ///
    /// # Resurrection is impossible by construction (no refcount recheck)
    /// [`Self::inc_ref`] refuses a `Retiring`/`Vacant` slot (F5 Decision 5),
    /// so a row's refcount is provably still 0 at retire time; a `Retiring`
    /// row is also never pushed onto [`Self::free`] until this call, so `add`
    /// cannot reuse (and thus re-tenant) it first. `dec_ref` is idempotent on
    /// `Retiring`, so exactly one [`RetireTicket`] is ever issued per row —
    /// this runs exactly once per row.
    ///
    /// # Panics (debug only)
    /// If `slot`'s packed state is not `Retiring` — a well-formed caller
    /// (draining the deferred-free queue exactly once per enqueued
    /// [`RetireTicket`]) never calls this on any other state. Also panics
    /// (unconditionally, an invariant violation) if `slot >= high_water` —
    /// same contract every other row accessor in this file states.
    pub fn retire(&mut self, slot: u32) -> Option<T> {
        let idx = slot as usize;
        let word = self
            .slot_word
            .get(idx)
            .expect("invariant: retire's slot must be < high_water (caller: deferred-free drain)");
        let state = unpack_state(word);
        debug_assert_eq!(
            state, STATE_RETIRING,
            "invariant: retire must target a Retiring row (slot {slot}) — dec_ref's idempotent \
             guard issues exactly one RetireTicket per row"
        );
        let generation = unpack_generation(word);

        let value = if self.live.test(idx) {
            // Clear `live` BEFORE the move-out — mirrors `remove`'s
            // exactly-once discipline: no later drop path (this store's
            // terminal `Drop`) can re-touch a slot once its `live` bit is
            // clear.
            self.live.clear(idx);
            self.live_count -= 1;
            // SAFETY: `live.test(idx)` was true — a Loaded->Retiring row
            // holds a valid `T` written by `push_value`/`write_value_at`
            // (the same discriminator `get_by_index` uses). `live` was just
            // cleared, so no other path reads or drops this slot again
            // before it is rewritten. `take_at` moves the value out via
            // `ptr::read` without running drop — the caller now owns it.
            Some(unsafe { self.col.take_at::<T>(idx) })
        } else {
            // A Loading/Failed->Retiring row never held a real `T` (only
            // inert scratch bytes) — nothing to move out.
            None
        };

        self.slot_word.set(idx, pack_slot_word(generation.wrapping_add(1), STATE_VACANT));
        self.refcount.set(idx, 0);
        self.free.push(idx as u32);
        self.free_epoch = self.free_epoch.wrapping_add(1);
        // Bumped here too (not just on `add`/`fill`, FIX-C1): a GPU-mirror table
        // gated on `install_epoch` (the hwrt TLAS `blas_addr` table) must also
        // resync on a bare retire-to-Vacant transition — even with no reuse yet,
        // this is what makes the NEXT `sync_blas_addr` overwrite the freed slot's
        // stale device address with the `Vacant` sentinel (0), rather than
        // leaving it stale until some future unrelated `add`/`fill` happens to
        // bump the epoch.
        self.install_epoch = self.install_epoch.wrapping_add(1);
        self.dirty.set(idx);
        value
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
    // Test oracle model: the std `HashMap`/`HashSet` here are the REFERENCE
    // implementation the VM-native `Assets<T>` store (ComponentPool column +
    // slot table) is differentially verified against. Compiled out of every
    // shipping build.
    #![allow(clippy::disallowed_types)]

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

    /// proptest oracle: a random sequence of add/reserve/fill/fail/remove/get
    /// PLUS `inc_ref`/`dec_ref` (asset-streaming plan F2 §1), modeling
    /// `(slot, gen, state, refcount)` JOINTLY — an extension of [`oracle`]'s
    /// `(slot, gen, state)`-only model.
    ///
    /// The refcount/`Retiring` model is keyed by the DENSE SLOT INDEX (not by
    /// `Handle`) and stays deliberately gen-oblivious (every `inc_ref`/
    /// `dec_ref` call below passes [`GEN_UNSYNCED`], bypassing F5's gen-check)
    /// — this proptest targets the refcount/idempotency/`free_epoch`
    /// properties, not the gen-mismatch behavior (covered by the dedicated F5
    /// unit tests), so a real apply-system fold can still be modeled as
    /// targeting ANY currently-minted slot regardless of which generation
    /// currently occupies it.
    ///
    /// `RemoveAt` on a handle whose slot the model has marked `Retiring` is
    /// deliberately SKIPPED (not routed to `assets.remove`), with a comment at
    /// the skip site — see that comment for why. This keeps this proptest
    /// focused exactly on the refcount/idempotency/`free_epoch` properties
    /// asset-streaming plan F2 §1 describes.
    mod refcount_oracle {
        use std::collections::{HashMap, HashSet};

        use proptest::prelude::*;

        use super::*;

        #[derive(Clone, Debug)]
        enum RefOp {
            Add(u64),
            Reserve,
            FillAt(usize, u64),
            FailAt(usize),
            RemoveAt(usize),
            GetAt(usize),
            IncRefAt(usize),
            DecRefAt(usize),
        }

        fn ref_op_strategy() -> impl Strategy<Value = RefOp> {
            prop_oneof![
                any::<u64>().prop_map(RefOp::Add),
                Just(RefOp::Reserve),
                (any::<usize>(), any::<u64>()).prop_map(|(i, v)| RefOp::FillAt(i, v)),
                any::<usize>().prop_map(RefOp::FailAt),
                any::<usize>().prop_map(RefOp::RemoveAt),
                any::<usize>().prop_map(RefOp::GetAt),
                any::<usize>().prop_map(RefOp::IncRefAt),
                any::<usize>().prop_map(RefOp::DecRefAt),
            ]
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(256))]
            #[test]
            fn assets_refcount_matches_model_oracle(ops in proptest::collection::vec(ref_op_strategy(), 1..300)) {
                let mut assets = Assets::<u64>::with_reserved(16);
                let mut oracle: HashMap<Handle<u64>, u64> = HashMap::new();
                let mut reserved_pending: HashSet<Handle<u64>> = HashSet::new();
                let mut minted: Vec<Handle<u64>> = Vec::new();
                // Refcount + Retiring, keyed by dense slot index (see module doc).
                let mut refcount_model: HashMap<u32, u32> = HashMap::new();
                let mut retiring_model: HashSet<u32> = HashSet::new();
                let mut vacant_model: HashSet<u32> = HashSet::new();
                // The CURRENT (matching-generation) `Handle` occupying each slot, if
                // any — needed because `dec_ref`/`inc_ref` (like the real F2 apply
                // path) address a bare `u32` slot, not a generation-checked
                // `Handle`: a `RefOp::*At(pick)` can pick a STALE `minted` entry
                // whose generation no longer matches the slot's real occupant, yet
                // `dec_ref`/`inc_ref` still act on whatever IS really there. This map
                // is how a `DecRefAt` crossing-to-zero knows WHICH `oracle`/
                // `reserved_pending` entry to retract, independent of which handle
                // (possibly stale) `pick` happened to land on.
                let mut current_occupant: HashMap<u32, Handle<u64>> = HashMap::new();

                for op in ops {
                    match op {
                        RefOp::Add(value) => {
                            let handle = assets.add(value);
                            oracle.insert(handle, value);
                            minted.push(handle);
                            refcount_model.insert(handle.index(), 0);
                            retiring_model.remove(&handle.index());
                            vacant_model.remove(&handle.index());
                            current_occupant.insert(handle.index(), handle);
                        }
                        RefOp::Reserve => {
                            let handle = assets.reserve();
                            reserved_pending.insert(handle);
                            minted.push(handle);
                            refcount_model.insert(handle.index(), 0);
                            retiring_model.remove(&handle.index());
                            vacant_model.remove(&handle.index());
                            current_occupant.insert(handle.index(), handle);
                        }
                        RefOp::FillAt(pick, value) => {
                            if let Some(&handle) = minted.get(pick % minted.len().max(1)) {
                                let slot = handle.index();
                                // `fill`'s precondition is `resolve_reserved`: state ∈
                                // {Loading, Failed}. A slot the model marked Retiring
                                // (via `dec_ref` reaching zero on what WAS a Loading row —
                                // dec_ref accepts Loading too) now fails that check safely
                                // (resolve_reserved's `matches!` has no arm for Retiring,
                                // so it returns `None`, not a panic) — model this as a
                                // forced-error case alongside the existing "not reserved"
                                // case.
                                let expected_ok =
                                    reserved_pending.contains(&handle) && !retiring_model.contains(&slot);
                                let real = assets.fill(handle, value);
                                if expected_ok {
                                    prop_assert!(real.is_ok(), "fill on a Reserved, non-Retiring row must succeed");
                                    oracle.insert(handle, value);
                                    reserved_pending.remove(&handle);
                                } else {
                                    prop_assert!(
                                        real.is_err(),
                                        "fill on a non-Reserved/stale/Retiring handle must error"
                                    );
                                }
                            }
                        }
                        RefOp::FailAt(pick) => {
                            if let Some(&handle) = minted.get(pick % minted.len().max(1)) {
                                // Same safe-on-Retiring precondition as `fill` (see above);
                                // a no-op either way, nothing to model.
                                assets.fail(handle);
                            }
                        }
                        RefOp::RemoveAt(pick) => {
                            if let Some(&handle) = minted.get(pick % minted.len().max(1)) {
                                let slot = handle.index();
                                if retiring_model.contains(&slot) {
                                    // KNOWN GAP (not this proptest's target — see the
                                    // dedicated `remove_on_retiring_row_after_dec_ref_reaches_zero`
                                    // probe below): `remove()`'s `resolve_index` only checks
                                    // generation, and `dec_ref`'s Retiring transition does
                                    // NOT bump generation — so the ORIGINAL handle still
                                    // resolves, and `remove()`'s state match has no arm for
                                    // Retiring, hitting `unreachable!()`. Skip calling
                                    // `remove` here so THIS proptest stays focused on the
                                    // refcount/idempotency/free_epoch properties it targets.
                                    continue;
                                }
                                // `handle` genuinely resolves (matching generation)
                                // against the slot's CURRENT occupant iff it is the
                                // handle presently tracked as Loaded (`oracle`) or
                                // Loading/Failed (`reserved_pending`) — the retiring
                                // case is already excluded above, so these two sets
                                // jointly cover every resolvable non-Retiring state
                                // (mirrors the [`oracle`] proptest's own invariant).
                                // A `pick` landing on a SUPERSEDED (stale-generation)
                                // `minted` entry for an already-reused slot resolves
                                // to neither set — a real, guaranteed no-op that must
                                // NOT perturb this slot's refcount/retiring/vacant
                                // model (the slot's true current occupant, at a
                                // DIFFERENT generation, is untouched by the call).
                                let is_current_occupant =
                                    oracle.contains_key(&handle) || reserved_pending.contains(&handle);
                                let was_occupied = oracle.contains_key(&handle);
                                let real = assets.remove(handle);
                                let model = if was_occupied { oracle.remove(&handle) } else { None };
                                prop_assert_eq!(
                                    real, model,
                                    "remove() must match the oracle exactly, including a Reserved row's \
                                     None with no underflow"
                                );
                                if is_current_occupant {
                                    reserved_pending.remove(&handle);
                                    refcount_model.insert(slot, 0);
                                    retiring_model.remove(&slot);
                                    vacant_model.insert(slot);
                                    current_occupant.remove(&slot);
                                }
                            }
                        }
                        RefOp::GetAt(pick) => {
                            if let Some(&handle) = minted.get(pick % minted.len().max(1)) {
                                let real = assets.get(handle).copied();
                                let model = oracle.get(&handle).copied();
                                prop_assert_eq!(real, model, "get() must match the oracle exactly (never a stale or wrong value)");
                            }
                        }
                        RefOp::IncRefAt(pick) => {
                            if let Some(&handle) = minted.get(pick % minted.len().max(1)) {
                                let slot = handle.index();
                                // F5 Decision 5: inc_ref now refuses a Retiring/Vacant slot
                                // (resurrection refusal) — the model already tracks both sets
                                // (maintained by DecRefAt/RemoveAt above), so this arm asserts
                                // the refusal is a true no-op alongside the live-slot success
                                // case, rather than skipping the call.
                                let is_retiring = retiring_model.contains(&slot);
                                let is_vacant = vacant_model.contains(&slot);
                                let real_before = assets.refcount.get(slot as usize);
                                let incremented = assets.inc_ref(slot);
                                let real_after = assets.refcount.get(slot as usize);
                                if is_retiring || is_vacant {
                                    prop_assert!(
                                        !incremented,
                                        "inc_ref must refuse a Retiring/Vacant slot (F5 Decision 5)"
                                    );
                                    prop_assert_eq!(
                                        real_after, real_before,
                                        "a refused inc_ref must not mutate the refcount"
                                    );
                                } else if let Some(before) = real_before {
                                    prop_assert!(
                                        incremented,
                                        "inc_ref on a live (Loading/Loaded/Failed) slot must succeed"
                                    );
                                    prop_assert_eq!(
                                        real_after, Some(before + 1),
                                        "inc_ref must increment the real refcount column by exactly 1"
                                    );
                                    if let Some(c) = refcount_model.get_mut(&slot) {
                                        *c += 1;
                                    }
                                }
                            }
                        }
                        RefOp::DecRefAt(pick) => {
                            if let Some(&handle) = minted.get(pick % minted.len().max(1)) {
                                let slot = handle.index();
                                // Only exercise `dec_ref` when it stays within its own
                                // documented precondition (never call it on a slot the
                                // model knows has refcount 0 AND is neither Retiring nor
                                // Vacant — that would trip `dec_ref`'s own
                                // `debug_assert!(count > 0, ...)`, an invariant violation
                                // by the CALLER, not a property of `dec_ref` itself; a
                                // real caller never does this because the carrier hooks
                                // always pair +1/-1). Calling it while Retiring or Vacant
                                // is exactly the idempotency property this test targets.
                                let count = refcount_model.get(&slot).copied().unwrap_or(0);
                                let is_retiring = retiring_model.contains(&slot);
                                let is_vacant = vacant_model.contains(&slot);
                                if count == 0 && !is_retiring && !is_vacant {
                                    continue;
                                }

                                let epoch_before = assets.free_epoch();
                                // GEN_UNSYNCED bypasses F5's gen-check — this proptest's model
                                // is gen-oblivious by design (see the module doc): it targets
                                // the refcount/idempotency/free_epoch properties, not the
                                // gen-mismatch behavior (covered by the dedicated F5 unit tests).
                                let ticket = assets.dec_ref(slot, GEN_UNSYNCED);

                                if is_retiring || is_vacant {
                                    prop_assert!(
                                        ticket.is_none(),
                                        "dec_ref on an already-Retiring/Vacant slot must be idempotent (None)"
                                    );
                                    prop_assert_eq!(
                                        assets.free_epoch(), epoch_before,
                                        "an idempotent dec_ref (Retiring/Vacant) must not bump free_epoch again"
                                    );
                                } else if count > 1 {
                                    prop_assert!(
                                        ticket.is_none(),
                                        "dec_ref must return None while the refcount stays above zero"
                                    );
                                    prop_assert_eq!(
                                        assets.free_epoch(), epoch_before,
                                        "a non-zero-crossing dec_ref must not bump free_epoch"
                                    );
                                    refcount_model.insert(slot, count - 1);
                                } else {
                                    // count == 1: this call crosses to zero.
                                    prop_assert!(
                                        ticket.is_some(),
                                        "dec_ref reaching zero on an unpinned, non-Retiring row must return a RetireTicket"
                                    );
                                    prop_assert_eq!(
                                        assets.free_epoch(), epoch_before + 1,
                                        "the zero-crossing dec_ref must bump free_epoch by exactly 1"
                                    );
                                    refcount_model.insert(slot, 0);
                                    retiring_model.insert(slot);
                                    // A Retiring row is no longer Loaded — `get()`
                                    // (`resolve_occupied`, STATE_LOADED-only) stops
                                    // resolving it even though `get_by_index` still
                                    // does (see `Assets::get_by_index`'s doc); drop it
                                    // from the value oracle to match. `dec_ref` (like
                                    // the real F2 apply path) addresses the bare slot,
                                    // NOT `handle` (which may be a STALE `minted` pick
                                    // superseded by a later Add/Reserve at this same
                                    // index) — retract via `current_occupant`'s
                                    // slot->CURRENT-handle mapping, not `handle` itself.
                                    if let Some(&cur) = current_occupant.get(&slot) {
                                        oracle.remove(&cur);
                                        reserved_pending.remove(&cur);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// FIX C1 (UB): `get_by_index` must NOT resolve a `Loading`->`Retiring`
    /// row — it never held a real `T` (only `reserve()`'s inert all-zero
    /// scratch), so forming `&T` over it would be UB. `live.test(idx)` is
    /// the discriminator (see `get_by_index`'s doc); this pins the `Loading`
    /// half of that contract.
    #[test]
    fn get_by_index_returns_none_for_loading_row_retired_via_dec_ref() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.reserve(); // Loading, live=false
        assert!(assets.inc_ref(h.index()), "inc_ref on a Loading row must succeed");
        let ticket = assets.dec_ref(h.index(), GEN_UNSYNCED);
        assert!(ticket.is_some(), "refcount 1->0 must retire the row");
        assert_eq!(
            assets.get_by_index(h.index()),
            None,
            "a Loading->Retiring row never held a real T (live=false) — get_by_index must return \
             None, not read the inert scratch bytes as &T"
        );
    }

    /// Sibling of the Loading case for a `Failed` row.
    #[test]
    fn get_by_index_returns_none_for_failed_row_retired_via_dec_ref() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.reserve();
        assets.fail(h); // Failed, live=false
        assert!(assets.inc_ref(h.index()), "inc_ref on a Failed row must succeed");
        let ticket = assets.dec_ref(h.index(), GEN_UNSYNCED);
        assert!(ticket.is_some(), "refcount 1->0 must retire the row");
        assert_eq!(
            assets.get_by_index(h.index()),
            None,
            "a Failed->Retiring row never held a real T (live=false) — get_by_index must return \
             None, not read the inert scratch bytes as &T"
        );
    }

    /// Miri-TB / debug-build documented-gap probe (NOT an F2 regression per
    /// se — F2's own module doc concedes `state`/`remove` are not yet
    /// Retiring-aware; F5/F6 are the rungs that add it): calling `remove()`
    /// with the SAME (unchanged-generation) `Handle` a `dec_ref`-driven
    /// Retiring transition just touched hits `remove()`'s exhaustive state
    /// match, which has no arm for `Retiring` — `unreachable!()`. Recorded
    /// here as an explicit, isolated, deterministic reproduction (found via
    /// the `refcount_oracle` proptest's design, deliberately routed AROUND
    /// this exact call above) rather than left silently undiscovered.
    #[test]
    fn remove_on_retiring_row_after_dec_ref_reaches_zero() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(1);
        assert!(assets.inc_ref(h.index()), "inc_ref on a Loaded row must succeed");
        let ticket = assets.dec_ref(h.index(), GEN_UNSYNCED);
        assert!(ticket.is_some(), "refcount 1->0 must retire the row");

        // The SAME handle (dec_ref never bumps generation) still resolves via
        // `resolve_index`; `remove()`'s state match has no `Retiring` arm.
        let _ = assets.remove(h);
    }

    /// Sibling of [`remove_on_retiring_row_after_dec_ref_reaches_zero`] for
    /// `state()`, which has the identical exhaustive-match-with-no-Retiring-arm
    /// shape.
    #[test]
    fn state_on_retiring_row_after_dec_ref_reaches_zero() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(1);
        assert!(assets.inc_ref(h.index()), "inc_ref on a Loaded row must succeed");
        let ticket = assets.dec_ref(h.index(), GEN_UNSYNCED);
        assert!(ticket.is_some(), "refcount 1->0 must retire the row");

        let _ = assets.state(h);
    }

    // ════════════════════════════════════════════════════════════════════
    // F5 store-level tests: dec_ref's gen-check, inc_ref's state guard,
    // try_generation / state_of_index's OOR-safe cross-crate probes.
    // ════════════════════════════════════════════════════════════════════

    /// `dec_ref` with a generation that does not match the slot's CURRENT
    /// generation must no-op entirely — no refcount mutation, no `free_epoch`
    /// bump, no `RetireTicket` (asset-streaming plan F5 Decision 4 — closes
    /// the stale-decrement/reuse corruption hole).
    #[test]
    fn dec_ref_refuses_stale_generation_and_leaves_refcount_and_free_epoch_untouched() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(1);
        let slot = h.index();
        assert!(assets.inc_ref(slot), "inc_ref on a fresh Loaded row must succeed");

        let cur_gen = assets.generation(slot);
        let stale_gen = cur_gen.wrapping_add(1);
        let epoch_before = assets.free_epoch();
        let refcount_before = assets.refcount.get(slot as usize);

        let ticket = assets.dec_ref(slot, stale_gen);

        assert!(ticket.is_none(), "a gen-mismatched dec_ref must no-op (return None)");
        assert_eq!(
            assets.refcount.get(slot as usize),
            refcount_before,
            "a gen-mismatched dec_ref must not mutate the refcount"
        );
        assert_eq!(
            assets.free_epoch(),
            epoch_before,
            "a gen-mismatched dec_ref must not bump free_epoch"
        );
        assert_eq!(
            assets.state_of_index(slot),
            Some(AssetLoadState::Loaded),
            "the row must remain exactly as it was — still Loaded, not Retiring"
        );
    }

    /// [`GEN_UNSYNCED`] bypasses the gen-check entirely — `dec_ref` must
    /// decrement unconditionally regardless of the slot's real current
    /// generation (the same-frame bound-but-not-yet-synced window).
    #[test]
    fn dec_ref_gen_unsynced_bypasses_the_gen_check() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(1);
        let slot = h.index();
        assert!(assets.inc_ref(slot), "inc_ref on a fresh Loaded row must succeed");

        let ticket = assets.dec_ref(slot, GEN_UNSYNCED);

        assert!(
            ticket.is_some(),
            "GEN_UNSYNCED must bypass the gen-check and decrement unconditionally, \
             retiring the row on its 1->0 crossing"
        );
    }

    /// `inc_ref` on an already-`Retiring` slot must refuse (the resurrection
    /// hazard F5 Decision 5 guards against) — `false`, refcount untouched.
    #[test]
    fn inc_ref_refuses_a_retiring_slot_without_mutating_refcount() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(1);
        let slot = h.index();
        assert!(assets.inc_ref(slot), "inc_ref on a fresh Loaded row must succeed");
        let ticket = assets.dec_ref(slot, GEN_UNSYNCED);
        assert!(ticket.is_some(), "refcount 1->0 must retire the row (test precondition)");

        let refcount_before = assets.refcount.get(slot as usize);
        let incremented = assets.inc_ref(slot);

        assert!(!incremented, "inc_ref must refuse a Retiring slot (F5 Decision 5)");
        assert_eq!(
            assets.refcount.get(slot as usize),
            refcount_before,
            "a refused inc_ref must not mutate the refcount"
        );
    }

    /// `inc_ref` on a `Vacant` slot (never minted, or freed via
    /// [`Assets::remove`]) must refuse — `false`, refcount untouched.
    #[test]
    fn inc_ref_refuses_a_vacant_slot_without_mutating_refcount() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(1);
        let slot = h.index();
        assert_eq!(assets.remove(h), Some(1), "must vacate the row (test precondition)");

        let refcount_before = assets.refcount.get(slot as usize);
        let incremented = assets.inc_ref(slot);

        assert!(!incremented, "inc_ref must refuse a Vacant slot (F5 Decision 5)");
        assert_eq!(
            assets.refcount.get(slot as usize),
            refcount_before,
            "a refused inc_ref must not mutate the refcount"
        );
    }

    /// [`Assets::try_generation`] must return `None` for an out-of-range slot
    /// (a malformed/stale carrier's bare index), never panic — the OOR-safe
    /// twin of the panicking [`Assets::generation`].
    #[test]
    fn try_generation_returns_none_out_of_range() {
        let assets = Assets::<u64>::with_reserved(4);
        assert_eq!(
            assets.try_generation(9_999),
            None,
            "an out-of-range slot must be None, not panic"
        );
    }

    /// [`Assets::try_generation`] must return `Some` matching the handle's own
    /// generation for a live slot.
    #[test]
    fn try_generation_returns_some_for_a_live_slot() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(1);
        assert_eq!(
            assets.try_generation(h.index()),
            Some(h.generation()),
            "try_generation must match the handle's own generation"
        );
    }

    /// [`Assets::state_of_index`] must return `None` for an out-of-range slot,
    /// never panic.
    #[test]
    fn state_of_index_returns_none_out_of_range() {
        let assets = Assets::<u64>::with_reserved(4);
        assert_eq!(
            assets.state_of_index(9_999),
            None,
            "an out-of-range slot must be None, not panic"
        );
    }

    /// [`Assets::state_of_index`] must return `Some(Loaded)` for a live,
    /// `Loaded` slot.
    #[test]
    fn state_of_index_returns_some_for_a_loaded_slot() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(1);
        assert_eq!(
            assets.state_of_index(h.index()),
            Some(AssetLoadState::Loaded),
            "a live Loaded slot must resolve"
        );
    }

    /// [`Assets::state_of_index`] must return `None` for a `Vacant` slot — no
    /// `AssetLoadState` mapping exists for it.
    #[test]
    fn state_of_index_returns_none_for_a_vacant_slot() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(1);
        let slot = h.index();
        assets.remove(h);
        assert_eq!(
            assets.state_of_index(slot),
            None,
            "a Vacant slot has no AssetLoadState mapping"
        );
    }

    /// [`Assets::state_of_index`] must return `None` for a `Retiring` slot —
    /// no `AssetLoadState` mapping exists for it either (mirrors the `Vacant`
    /// case; this is the sanctioned cross-crate probe `validate_asset_refs`
    /// relies on to detect a stale/dead carrier without panicking).
    #[test]
    fn state_of_index_returns_none_for_a_retiring_slot() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(1);
        let slot = h.index();
        assert!(assets.inc_ref(slot), "inc_ref on a fresh Loaded row must succeed");
        assert!(
            assets.dec_ref(slot, GEN_UNSYNCED).is_some(),
            "refcount 1->0 must retire the row (test precondition)"
        );
        assert_eq!(
            assets.state_of_index(slot),
            None,
            "a Retiring slot has no AssetLoadState mapping"
        );
    }

    // ════════════════════════════════════════════════════════════════════
    // F6 store-level tests: `Assets::retire` — the terminal, fence-gated free
    // of a `Retiring` row. Miri-TB: exactly-once move-out, no double-take on
    // an already-`Vacant` slot, and a fresh generation on reuse.
    // ════════════════════════════════════════════════════════════════════

    /// Miri-TB: `retire` moves a `Loaded`->`Retiring` row's value out via
    /// `take_at` WITHOUT dropping it in place (mirrors
    /// `take_at_and_terminal_drop_are_exactly_once`'s `RecordDrop` idiom
    /// exactly — the `impl_asset_pod_backing!` macro fixes `NEEDS_TEARDOWN =
    /// false` with no drop glue, which is unsound for a Drop-counting stand-in
    /// whose value the store's OWN terminal `Drop` must also be able to drop
    /// correctly if it were ever left un-retired). The returned value's Drop
    /// runs EXACTLY ONCE when the caller drops it; the store's own terminal
    /// `Drop` (at scope exit, over the now-`Vacant`/reused slot) must not
    /// touch it again.
    #[test]
    fn retire_moves_the_value_out_exactly_once_on_a_loaded_then_retiring_slot() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        struct RetireDrop;
        impl Drop for RetireDrop {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }

        // SAFETY: caller (`ComponentPool::drop_at` / the terminal `Assets::drop`)
        // guarantees `ptr` points at a valid, aligned, fully-initialized
        // `RetireDrop`, exclusively owned and not accessed again after this call.
        unsafe fn retire_drop_glue(ptr: *mut u8) {
            unsafe { core::ptr::drop_in_place(ptr.cast::<RetireDrop>()) }
        }

        impl AssetBacking for RetireDrop {
            const NEEDS_TEARDOWN: bool = true;
            fn register_layout() -> ComponentId {
                crate::ecs::core::asset::backing::register_asset_layout::<RetireDrop>(Some(
                    retire_drop_glue,
                ))
            }
        }

        DROP_COUNT.store(0, Ordering::Relaxed);
        {
            let mut assets = Assets::<RetireDrop>::with_reserved(4);
            let h = assets.add(RetireDrop);
            let slot = h.index();
            assert!(assets.inc_ref(slot), "inc_ref on a fresh Loaded row must succeed");
            let ticket = assets.dec_ref(slot, GEN_UNSYNCED);
            assert!(ticket.is_some(), "refcount 1->0 must retire the row (test precondition)");
            assert_eq!(
                DROP_COUNT.load(Ordering::Relaxed),
                0,
                "dec_ref's Retiring transition must not drop the value — it still holds it"
            );

            let taken = assets.retire(slot);
            assert!(taken.is_some(), "retire on a Loaded->Retiring row must return the value");
            assert_eq!(
                DROP_COUNT.load(Ordering::Relaxed),
                0,
                "retire's take_at must move the value out WITHOUT dropping it in place"
            );

            drop(taken);
            assert_eq!(
                DROP_COUNT.load(Ordering::Relaxed),
                1,
                "dropping the returned owned value runs Drop exactly once"
            );
            // The slot is now Vacant; the store's terminal Drop below must not
            // touch it again.
        }
        assert_eq!(
            DROP_COUNT.load(Ordering::Relaxed),
            1,
            "the store's terminal Drop must not re-drop a slot `retire` already moved out"
        );
    }

    /// Sibling of the above for a `Loading`/`Failed`->`Retiring` row (never
    /// held a real value — only `reserve()`'s inert scratch bytes): `retire`
    /// must return `None` and run Drop ZERO times.
    #[test]
    fn retire_on_a_loading_retiring_row_returns_none_and_drops_nothing() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static DROP_COUNT_LOADING: AtomicUsize = AtomicUsize::new(0);

        struct RetireDropLoading;
        impl Drop for RetireDropLoading {
            fn drop(&mut self) {
                DROP_COUNT_LOADING.fetch_add(1, Ordering::Relaxed);
            }
        }

        // SAFETY: same `DropFn` contract as `retire_drop_glue` above.
        unsafe fn retire_drop_loading_glue(ptr: *mut u8) {
            unsafe { core::ptr::drop_in_place(ptr.cast::<RetireDropLoading>()) }
        }

        impl AssetBacking for RetireDropLoading {
            const NEEDS_TEARDOWN: bool = true;
            fn register_layout() -> ComponentId {
                crate::ecs::core::asset::backing::register_asset_layout::<RetireDropLoading>(Some(
                    retire_drop_loading_glue,
                ))
            }
        }

        DROP_COUNT_LOADING.store(0, Ordering::Relaxed);
        let mut assets = Assets::<RetireDropLoading>::with_reserved(4);
        let h = assets.reserve(); // Loading, live=false — never held a real value.
        let slot = h.index();
        assert!(assets.inc_ref(slot), "inc_ref on a Loading row must succeed");
        let ticket = assets.dec_ref(slot, GEN_UNSYNCED);
        assert!(ticket.is_some(), "refcount 1->0 must retire the row (test precondition)");

        let taken = assets.retire(slot);
        assert!(
            taken.is_none(),
            "a Loading->Retiring row never held a real value — retire must return None"
        );
        assert_eq!(
            DROP_COUNT_LOADING.load(Ordering::Relaxed),
            0,
            "no value ever existed at this slot — Drop must never run"
        );
    }

    /// `retire` targets a `Retiring` row by construction (exactly one call per
    /// `RetireTicket` — see its own doc's "Resurrection is impossible"
    /// section). A SECOND `retire` call on the same slot (now `Vacant`) is a
    /// caller-contract violation, NOT a silent no-op: the debug assertion
    /// fires — deliberately harder-failing than a quiet `None`, so a
    /// double-drain bug (which would otherwise double-push the free-list and
    /// corrupt a later `add`'s slot assignment) is caught immediately rather
    /// than corrupting state silently.
    #[test]
    #[should_panic(expected = "invariant: retire must target a Retiring row")]
    fn retire_again_on_the_now_vacant_slot_panics_the_one_shot_contract() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h = assets.add(1);
        let slot = h.index();
        assert!(assets.inc_ref(slot), "inc_ref on a fresh Loaded row must succeed");
        assert!(
            assets.dec_ref(slot, GEN_UNSYNCED).is_some(),
            "refcount 1->0 must retire the row (test precondition)"
        );
        let _ = assets.retire(slot);

        // The slot is now Vacant — a second retire() must panic, not double-take.
        let _ = assets.retire(slot);
    }

    /// A retired-and-reused slot gets a FRESH generation (retire bumps it
    /// exactly once, mirroring `remove`'s terminal-`Vacant` bookkeeping): the
    /// old `Handle` never resolves again, even after the slot is handed back
    /// out by a later `add` (LIFO reuse).
    #[test]
    fn retire_then_add_reuses_the_slot_with_a_fresh_generation() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static DROP_COUNT_REUSE: AtomicUsize = AtomicUsize::new(0);

        struct RetireDropReuse;
        impl Drop for RetireDropReuse {
            fn drop(&mut self) {
                DROP_COUNT_REUSE.fetch_add(1, Ordering::Relaxed);
            }
        }

        // SAFETY: same `DropFn` contract as `retire_drop_glue` above.
        unsafe fn retire_drop_reuse_glue(ptr: *mut u8) {
            unsafe { core::ptr::drop_in_place(ptr.cast::<RetireDropReuse>()) }
        }

        impl AssetBacking for RetireDropReuse {
            const NEEDS_TEARDOWN: bool = true;
            fn register_layout() -> ComponentId {
                crate::ecs::core::asset::backing::register_asset_layout::<RetireDropReuse>(Some(
                    retire_drop_reuse_glue,
                ))
            }
        }

        DROP_COUNT_REUSE.store(0, Ordering::Relaxed);
        let mut assets = Assets::<RetireDropReuse>::with_reserved(4);
        let h = assets.add(RetireDropReuse);
        let slot = h.index();
        assert!(assets.inc_ref(slot), "inc_ref on a fresh Loaded row must succeed");
        assert!(
            assets.dec_ref(slot, GEN_UNSYNCED).is_some(),
            "refcount 1->0 must retire the row (test precondition)"
        );
        let taken = assets.retire(slot);
        drop(taken);
        assert_eq!(DROP_COUNT_REUSE.load(Ordering::Relaxed), 1);

        let reused = assets.add(RetireDropReuse);
        assert_eq!(reused.index(), slot, "the freed slot must be reused (LIFO)");
        assert_eq!(
            reused.generation(),
            h.generation() + 1,
            "reuse must bump the generation by exactly one"
        );

        assert!(assets.contains(reused), "the reused handle must resolve");
        assert!(
            !assets.contains(h),
            "the OLD (pre-retire) handle must never resolve again, even after reuse"
        );
        assert!(
            assets.get(h).is_none(),
            "the old handle's generation must not match the reused slot's new one"
        );
    }

    /// proptest: `dec_ref` with a generation that never matches the slot's
    /// current one must NEVER mutate the refcount, across a wide, randomized
    /// span of mismatched generations — a dedicated, focused companion to
    /// [`refcount_oracle`]'s joint model, which is deliberately gen-oblivious
    /// (see that module's doc: it targets the refcount/idempotency/
    /// `free_epoch` properties, explicitly deferring gen-mismatch coverage to
    /// "the dedicated F5 unit tests" — this proptest is that coverage, kept
    /// separate rather than folded into the joint oracle's `RefOp` enum, to
    /// preserve that module's own documented scope boundary).
    ///
    /// The no-mutation claim is proven INDIRECTLY (refcount is a private
    /// field, inaccessible outside this exact test module) via a
    /// non-corruption oracle: if the stale call had actually decremented,
    /// the row would already be `Retiring` by the time the immediately
    /// following CORRECTLY-gen'd `dec_ref` runs, and that call would then
    /// observe the `Retiring`-idempotent `None` arm instead of the expected
    /// zero-crossing `Some` — so requiring `Some` here transitively proves
    /// the stale call left the refcount exactly where it was.
    mod gen_check_proptest {
        use proptest::prelude::*;

        use super::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(256))]
            #[test]
            fn dec_ref_never_mutates_on_a_generation_mismatch(gen_offset in 1u32..=1_000_000) {
                let mut assets = Assets::<u64>::with_reserved(4);
                let h = assets.add(1);
                let slot = h.index();
                prop_assert!(assets.inc_ref(slot), "inc_ref on a fresh Loaded row must succeed");

                let cur_gen = assets.generation(slot);
                // `gen_offset` is always in 1..=1_000_000, so `stale_gen` can only
                // equal `cur_gen` via a wrap-around at exactly `u32::MAX + 1` steps —
                // unreachable at this offset span — and real generations are 29-bit
                // (see GEN_UNSYNCED's doc), so `stale_gen` lands on `GEN_UNSYNCED`
                // only in the astronomically unlikely case `cur_gen == u32::MAX -
                // gen_offset`; `prop_assume!` filters both edge cases defensively.
                let stale_gen = cur_gen.wrapping_add(gen_offset);
                prop_assume!(stale_gen != GEN_UNSYNCED && stale_gen != cur_gen);

                let epoch_before = assets.free_epoch();
                let ticket = assets.dec_ref(slot, stale_gen);

                prop_assert!(ticket.is_none(), "a mismatched generation must no-op");
                prop_assert_eq!(
                    assets.free_epoch(), epoch_before,
                    "a mismatched dec_ref must not bump free_epoch"
                );
                prop_assert_eq!(
                    assets.state_of_index(slot), Some(AssetLoadState::Loaded),
                    "the row must remain live & Loaded — the real decrement never happened"
                );

                // The transitive non-corruption proof — see this module's doc.
                let real_ticket = assets.dec_ref(slot, cur_gen);
                prop_assert!(
                    real_ticket.is_some(),
                    "the correctly gen'd dec_ref must still retire the row, proving the \
                     stale call never touched the refcount"
                );
            }
        }
    }
}
