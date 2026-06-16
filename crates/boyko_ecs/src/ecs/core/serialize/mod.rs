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

pub use load_writer::{LoadColumn, load_archetype, remap_loaded_entities};
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
/// Mirrors the clone subsystem's `EntityCloneMap` template (`SparseMap<Entity>`
/// keyed by `EntityId.0`, plan §3.13). S1 keeps this a **placeholder**: the
/// `LoadMapEntitiesFn` alias type-checks against it and the saver never touches it
/// (the remap pass is load-direction). S2 fills in the build-on-load population
/// from the saved entity table.
///
/// [`LoadMapEntitiesFn`]: crate::ecs::core::component::component_registry::LoadMapEntitiesFn
//
#[derive(Default)]
pub struct LoadEntityMap {
    /// `saved EntityId.0` → freshly-allocated `Entity`. Backed by a
    /// `SparseMap<Entity>` (the `EntityCloneMap` template). S2's
    /// [`load_archetype`](load_writer::load_archetype) populates one entry per
    /// loaded entity; the S2.5 remap pass reads it through [`Self::get`].
    sparse: boyko_utils::sparse_map::sparse_map::SparseMap<Entity>,
}

impl LoadEntityMap {
    /// Creates an empty map.
    #[inline]
    pub fn new() -> Self {
        Self {
            sparse: boyko_utils::sparse_map::sparse_map::SparseMap::new(),
        }
    }

    /// Records that the saved `saved_entity_id` (`EntityId.0`) maps to the
    /// freshly-allocated `fresh` entity. Returns the previous mapping for that
    /// saved id, if any (a duplicate saved id in one file — a corrupt save).
    #[inline]
    pub fn insert(&mut self, saved_entity_id: usize, fresh: Entity) -> Option<Entity> {
        self.sparse.insert(saved_entity_id, fresh)
    }

    /// Returns the freshly-allocated `Entity` for a saved `EntityId.0`, or `None`
    /// when the saved id was never registered in this load (an unmapped reference —
    /// the C4 loud-error path the loader turns into a release error).
    #[inline]
    pub fn get(&self, saved_entity_id: usize) -> Option<Entity> {
        self.sparse.get(saved_entity_id).copied()
    }

    /// Number of saved→fresh mappings recorded.
    #[inline]
    pub fn len(&self) -> usize {
        self.sparse.len()
    }

    /// `true` when no mappings have been recorded.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.sparse.is_empty()
    }
}
