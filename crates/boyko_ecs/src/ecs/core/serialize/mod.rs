//! Serialization I/O boundary types (Phase S0 substrate + Phase S1 cursors).
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` (§3.7, §3.8, §5 C1–C4, §7 Phase S0/S1).
//! This module holds the **cursor / error / entity-map types** that the cold
//! per-component serialize fn-ptr table (`component_registry::SERIALIZE`)
//! references in its `SerializeFn` / `DeserializeFn` / `LoadMapEntitiesFn` type
//! aliases.
//!
//! # Why these live here (crate-boundary decision)
//!
//! The registry's `SerializeFn` / `DeserializeFn` / `LoadMapEntitiesFn` aliases
//! name these concrete types, so they are `boyko_ecs` types — they cannot move to
//! the `boyko_serialize` crate without inverting the dependency edge. S1 fleshes
//! out the real [`SaveCursor`] / [`LoadCursor`] / [`DecodeError`] bodies here (the
//! position-independent owning encoding + the "validate, never transmute blindly"
//! read contract, plan §3.8). [`LoadEntityMap`] stays a placeholder until S2
//! (`load.rs` builds it from the saved entity table).
//!
//! These types are NOT a hot path — they exist only on the cold save/load path.

pub mod load_writer;
pub mod wire;

pub use load_writer::{
    LoadColumn, LoadWriteError, load_archetype, load_dense_store, load_dense_store_via_fn,
    remap_loaded_entities,
};
pub use wire::{Wire, WireRefTuple, WireTuple};

// Re-export the registry's per-component deserialize fn-ptr alias here so the
// loader's `LoadColumn::Decode` variant (and the `boyko_serialize` parser) name it
// from the serialize boundary module rather than reaching into the registry.
pub use crate::ecs::core::component::component_registry::{DeserializeFn, RequiredCtor};

use crate::ecs::core::component::component_registry;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::ComponentId;

/// Resolves the capture-free `#[require]` ctor that materializes `target_id` within
/// the transitive require-closure of `base_ids` (plan §3.11 step 4 default-construct
/// path, C2). Returns `None` when `target_id` is not reachable as a required
/// component from any base id — the loader then EXCLUDES that no-data column from
/// the fresh archetype (it has no default value to construct).
///
/// A thin `pub` boundary over the crate-private
/// `component_registry::required_ctor_for` so the `boyko_serialize` file-format
/// parser can decide construct-vs-exclude for a no-data column without reaching into
/// the registry. COLD — called only from `boyko_serialize::load_world` (the C1
/// 0%-gate: never a per-frame path).
#[inline]
pub fn required_ctor_in_set(
    base_ids: &[ComponentId],
    target_id: ComponentId,
) -> Option<RequiredCtor> {
    component_registry::required_ctor_for(base_ids, target_id)
}

/// Append-only write cursor over a preallocated byte buffer (plan §3.8).
///
/// Wraps a `&mut Vec<u8>` and a `base_pos` snapshot taken at construction. The
/// writers ([`write_bytes`](Self::write_bytes) /
/// [`write_u32`](Self::write_u32) / [`write_u64`](Self::write_u64) /
/// [`write_len_prefixed`](Self::write_len_prefixed)) all **append** to the buffer
/// — the cursor never seeks or overwrites prior bytes. All multi-byte integers are
/// little-endian (postcard/rkyv convention — no byteswap on the common target,
/// plan §2.2).
///
/// # Position independence (the rkyv technique, plan §2.2 / §3.9)
///
/// [`pos`](Self::pos) reports the offset **relative to `base_pos`**, so an owning
/// `serialize_fn` can record a self-relative offset for a heap region instead of an
/// absolute file position. The resulting blob is position-independent and
/// mmap-castable at any base.
///
/// [`SerializeFn`]: crate::ecs::core::component::component_registry::SerializeFn
pub struct SaveCursor<'a> {
    /// The destination buffer being appended to (grown ahead of time by the
    /// two-pass save, so appends do not realloc mid-fill, plan §3.11 W3 — though
    /// `Vec::extend_from_slice` will still grow it if a caller appends past the
    /// reservation, which the owning-sizing pass relies on).
    out: &'a mut Vec<u8>,
    /// The buffer length at construction. [`pos`](Self::pos) and the relative
    /// offsets the owning encoding writes are measured against this, so a cursor
    /// handed a partially-filled buffer still reports offsets from its own origin.
    base_pos: usize,
}

impl<'a> SaveCursor<'a> {
    /// Wraps a destination buffer, snapshotting its current length as `base_pos`.
    #[inline]
    pub fn new(out: &'a mut Vec<u8>) -> Self {
        let base_pos = out.len();
        Self { out, base_pos }
    }

    /// Returns the number of bytes written through this cursor so far — the offset
    /// of the next append **relative to `base_pos`** (plan §3.8 position
    /// independence). The owning encoding uses this to record self-relative offsets.
    #[inline]
    pub fn pos(&self) -> usize {
        self.out.len() - self.base_pos
    }

    /// Appends `bytes` verbatim.
    #[inline]
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    /// Appends a `u32` in little-endian.
    #[inline]
    pub fn write_u32(&mut self, value: u32) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }

    /// Appends a `u64` in little-endian.
    #[inline]
    pub fn write_u64(&mut self, value: u64) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }

    /// Appends a length-prefixed byte run: a `u64` LE length followed by the bytes.
    ///
    /// The fixed-width `u64` prefix (not a varint) keeps the reader's bounds check
    /// a single comparison and matches the file format's other length fields
    /// (plan §3.9). The owning `serialize_fn` path uses this for `String`/`Vec`
    /// payloads.
    #[inline]
    pub fn write_len_prefixed(&mut self, bytes: &[u8]) {
        self.write_u64(bytes.len() as u64);
        self.write_bytes(bytes);
    }
}

/// Bounds-checked read cursor over the file bytes (plan §3.8).
///
/// Wraps a `&[u8]` and a `pos` read head. **Every** read validates `pos` against
/// `bytes.len()` and returns [`DecodeError`] on a short read — the loader never
/// transmutes a malformed stream into a value (the C3 "validate, never transmute
/// blindly" obligation). All multi-byte integers are read little-endian.
///
/// [`DeserializeFn`]: crate::ecs::core::component::component_registry::DeserializeFn
pub struct LoadCursor<'a> {
    /// The source bytes being read.
    bytes: &'a [u8],
    /// The read head: the offset of the next byte to read. Invariant:
    /// `pos <= bytes.len()` (every reader advances it only after a bounds check).
    pos: usize,
}

impl<'a> LoadCursor<'a> {
    /// Wraps a source byte slice with the read head at the start.
    #[inline]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// The current read-head offset (bytes consumed so far).
    #[inline]
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Bytes remaining between the read head and the end of the slice.
    #[inline]
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    /// Reads exactly `len` bytes, advancing the head. Returns
    /// [`DecodeError::UnexpectedEof`] when fewer than `len` bytes remain.
    #[inline]
    pub fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(len).ok_or(DecodeError::BadLengthPrefix)?;
        if end > self.bytes.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// Reads a little-endian `u32`, advancing the head 4 bytes.
    #[inline]
    pub fn read_u32(&mut self) -> Result<u32, DecodeError> {
        let bytes = self.read_bytes(4)?;
        // `read_bytes(4)` returns exactly 4 bytes, so the array conversion is
        // infallible; `try_into` keeps it panic-free without an `unwrap`.
        let arr: [u8; 4] = bytes.try_into().map_err(|_| DecodeError::UnexpectedEof)?;
        Ok(u32::from_le_bytes(arr))
    }

    /// Reads a little-endian `u64`, advancing the head 8 bytes.
    #[inline]
    pub fn read_u64(&mut self) -> Result<u64, DecodeError> {
        let bytes = self.read_bytes(8)?;
        let arr: [u8; 8] = bytes.try_into().map_err(|_| DecodeError::UnexpectedEof)?;
        Ok(u64::from_le_bytes(arr))
    }

    /// Reads a length-prefixed byte run written by
    /// [`SaveCursor::write_len_prefixed`]: a `u64` LE length then that many bytes.
    ///
    /// Returns [`DecodeError::BadLengthPrefix`] when the prefix exceeds the
    /// remaining input (a hostile length that would index out of bounds) and
    /// [`DecodeError::UnexpectedEof`] on a truncated payload.
    #[inline]
    pub fn read_len_prefixed(&mut self) -> Result<&'a [u8], DecodeError> {
        let len = self.read_u64()?;
        // Reject a length that cannot fit in the remaining bytes BEFORE the usize
        // cast / slice — a 2^63 length on a 64-bit target would otherwise wrap the
        // `checked_add` in `read_bytes` into `BadLengthPrefix`, but checking here
        // gives the precise diagnostic and is robust on every target width.
        if len > self.remaining() as u64 {
            return Err(DecodeError::BadLengthPrefix);
        }
        self.read_bytes(len as usize)
    }
}

/// A malformed-stream decode failure (plan §3.8).
///
/// Returned by [`DeserializeFn`] and the [`LoadCursor`] readers on a malformed
/// stream so the loader rolls back (the W5 partial-row contract, mirroring
/// `CloneFn`'s panic-leaves-uninit rule). `#[non_exhaustive]` so S2/S3 can add
/// loader-specific variants (fingerprint mismatch, unmapped entity) without a
/// breaking change.
///
/// [`DeserializeFn`]: crate::ecs::core::component::component_registry::DeserializeFn
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum DecodeError {
    /// The stream ended before a full value could be read (a short read at any
    /// cursor reader).
    UnexpectedEof,
    /// A length prefix was larger than the remaining input (or overflowed the
    /// address space) — a hostile/corrupt length that would index out of bounds.
    BadLengthPrefix,
    /// A field carried a bit pattern invalid for its type (e.g. a `bool` byte
    /// other than `0|1`, an out-of-range enum discriminant) — the C3
    /// validate-on-read obligation. Surfaced by the per-element `deserialize_fn`
    /// (S2 derive emission), not by the cursor itself.
    InvalidBitPattern,
    /// The S2.5 entity-remap pass found a saved `Entity` reference whose id was
    /// not in the [`LoadEntityMap`] (a referenced entity absent from the file, or
    /// a corrupt id) — the C4 loud-error path. Surfaced ONLY by a
    /// [`LoadMapEntitiesFn`](crate::ecs::core::component::component_registry::LoadMapEntitiesFn)
    /// during the remap pass; never silently dropped into a dangling reference.
    UnmappedEntity,
}

/// Load-direction entity remap table: saved `EntityId.0` → freshly-allocated
/// `Entity` (S2 `entity_map.rs`).
///
/// # Why a sorted `Vec`, not a value-indexed sparse map (the F1 abort fix)
///
/// A saved `EntityId.0` is an UNTRUSTED value read straight from the file. An
/// earlier design backed this map with a value-indexed dense `SparseMap<Entity>`
/// (the clone-side `EntityCloneMap` template), whose backing store grows to
/// `O(max saved-id value)`: a single bit-flipped saved id (e.g. `~2^40`) drove a
/// multi-terabyte allocation that ABORTED the process — bypassing `catch_unwind`
/// and killing the loader / fuzzer outright.
///
/// This map stores `(saved id, fresh Entity)` PAIRS in a `Vec`, so its memory is
/// `O(entries)` — bounded by the total loaded entity count, which the loader has
/// already capped (W2). It is NEVER keyed on a saved id VALUE, so no untrusted
/// value can drive a pathological allocation.
///
/// # Two-phase contract: insert-all → `finalize` → `get`
///
/// The load path is insert-all-then-lookup. Every saved→fresh pair is appended via
/// [`insert`](Self::insert) (push-only, `O(1)` amortized, in load order) during the
/// per-archetype pass. Once every pair is recorded the loader calls
/// [`finalize`](Self::finalize) ONCE — it sorts the pairs by saved id so the
/// subsequent remap pass can resolve each lookup with a `binary_search`
/// ([`get`](Self::get), `O(log n)`). In debug builds the phase transition is
/// guarded: a `get` before `finalize`, or an `insert` after it, panics on a
/// `debug_assert`.
///
/// [`LoadMapEntitiesFn`]: crate::ecs::core::component::component_registry::LoadMapEntitiesFn
#[derive(Default)]
pub struct LoadEntityMap {
    /// `(saved EntityId.0, fresh Entity)` pairs — pushed in load order, then sorted
    /// by saved id ONCE in [`finalize`](Self::finalize) before the remap pass's
    /// first [`get`](Self::get). Memory is `O(entries)` (== total loaded entities,
    /// already W2-count-capped), NEVER keyed on a saved id VALUE — so no untrusted
    /// value can drive a pathological allocation (the F1 abort fix).
    entries: Vec<(u64, Entity)>,
    /// Debug-only phase guard: `false` until [`finalize`](Self::finalize) seals the
    /// table, then `true`. Enforces the insert-all → finalize → get contract.
    #[cfg(debug_assertions)]
    sealed: bool,
}

impl LoadEntityMap {
    /// Creates an empty map.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty map preallocated for `n` saved→fresh pairs. Use this when a
    /// pre-loop total entity count is cheaply available so the insert pass never
    /// reallocates; otherwise [`new`](Self::new) is fine.
    #[inline]
    pub fn with_capacity(n: usize) -> Self {
        Self {
            entries: Vec::with_capacity(n),
            #[cfg(debug_assertions)]
            sealed: false,
        }
    }

    /// Records that the saved `saved_entity_id` (`EntityId.0`) maps to the
    /// freshly-allocated `fresh` entity.
    ///
    /// Push-only: the pair is appended in load order; [`finalize`](Self::finalize)
    /// sorts the table afterwards. Duplicate saved ids are not detected here (the
    /// loader does not rely on it). Must be called only in the insert phase, before
    /// [`finalize`](Self::finalize).
    #[inline]
    pub fn insert(&mut self, saved_entity_id: usize, fresh: Entity) {
        // `sealed` is `#[cfg(debug_assertions)]`-only, so the assert (whose
        // argument is type-checked in every profile) must itself be gated, or a
        // release build fails to resolve `self.sealed`.
        #[cfg(debug_assertions)]
        debug_assert!(!self.sealed, "LoadEntityMap: insert after finalize");
        self.entries.push((saved_entity_id as u64, fresh));
    }

    /// Seals the table for lookups: sorts the recorded pairs by saved id so
    /// [`get`](Self::get) can binary-search them. Call ONCE after the last
    /// [`insert`](Self::insert) and before the first [`get`](Self::get).
    #[inline]
    pub fn finalize(&mut self) {
        self.entries.sort_unstable_by_key(|&(k, _)| k);
        #[cfg(debug_assertions)]
        {
            self.sealed = true;
        }
    }

    /// Returns the freshly-allocated `Entity` for a saved `EntityId.0`, or `None`
    /// when the saved id was never registered in this load (an unmapped reference —
    /// the C4 loud-error path the loader turns into a release error).
    ///
    /// Must be called only after [`finalize`](Self::finalize) (the table must be
    /// sorted for the binary search to be correct).
    #[inline]
    pub fn get(&self, saved_entity_id: usize) -> Option<Entity> {
        // Gated like `insert`: `sealed` exists only under `debug_assertions`.
        #[cfg(debug_assertions)]
        debug_assert!(self.sealed, "LoadEntityMap: get before finalize");
        self.entries
            .binary_search_by_key(&(saved_entity_id as u64), |&(k, _)| k)
            .ok()
            .map(|i| self.entries[i].1)
    }

    /// Number of saved→fresh mappings recorded.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when no mappings have been recorded.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::identifiers::primitives::EntityId;

    /// Mints a distinct fresh `Entity` for a test fixture (generation 0, matching
    /// the loader's `Entity::new(EntityId(id), 0)` minting in `load_writer`).
    fn fresh(id: usize) -> Entity {
        Entity::new(EntityId(id), 0)
    }

    #[test]
    fn load_entity_map_get_after_finalize_roundtrips() {
        let mut map = LoadEntityMap::new();
        // Insert OUT OF ORDER to prove `finalize` sorts before the binary search.
        map.insert(40, fresh(100));
        map.insert(10, fresh(101));
        map.insert(30, fresh(102));
        map.insert(20, fresh(103));
        map.finalize();

        assert_eq!(map.get(40), Some(fresh(100)));
        assert_eq!(map.get(10), Some(fresh(101)));
        assert_eq!(map.get(30), Some(fresh(102)));
        assert_eq!(map.get(20), Some(fresh(103)));
        // An absent key resolves to `None` (the C4 unmapped-reference path).
        assert_eq!(map.get(25), None);
        assert_eq!(map.get(0), None);
        assert_eq!(map.len(), 4);
    }

    #[test]
    fn load_entity_map_huge_saved_id_no_abort() {
        // The exact F1 bug value: a value-indexed sparse map would try to allocate
        // `O(2^40)` slots here and abort. A sorted-Vec stores exactly one pair.
        let huge = 1usize << 40;
        let mut map = LoadEntityMap::new();
        map.insert(huge, fresh(7));
        map.finalize();

        assert_eq!(map.get(huge), Some(fresh(7)));
        // Memory is O(entries), not O(max key value): one insert => one entry.
        assert_eq!(map.len(), 1);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "LoadEntityMap: get before finalize")]
    fn load_entity_map_get_before_finalize_panics() {
        let mut map = LoadEntityMap::new();
        map.insert(1, fresh(1));
        // No `finalize` — the lookup phase guard must trip.
        let _ = map.get(1);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "LoadEntityMap: insert after finalize")]
    fn load_entity_map_insert_after_finalize_panics() {
        let mut map = LoadEntityMap::new();
        map.insert(1, fresh(1));
        map.finalize();
        // Inserting after sealing must trip the insert-phase guard.
        map.insert(2, fresh(2));
    }
}
