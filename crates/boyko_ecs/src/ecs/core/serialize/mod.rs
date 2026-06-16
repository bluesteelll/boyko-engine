//! Serialization registry substrate (Phase S0).
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` (§3.7, §5 C1–C4, §7 Phase S0). This module
//! holds the **placeholder I/O boundary types** that the cold per-component
//! serialize fn-ptr table (`component_registry::SERIALIZE`) references in its
//! `SerializeFn` / `DeserializeFn` / `LoadMapEntitiesFn` type aliases.
//!
//! # Why these live here in S0
//!
//! Phase S0 ships ONLY the registry + derive substrate + classification — NO
//! format, save/load, or cursor logic (that is S1/S2, in a future
//! `boyko_serialize` crate, §3.13). But the fn-ptr type aliases in
//! `component_registry` must name concrete types to type-check today. Rather than
//! creating the `boyko_serialize` crate prematurely (forbidden in S0), the cursor
//! / error / entity-map types are forward-declared here as minimal, opaque
//! substrate. S1 (`cursor.rs`) and S2 (`entity_map.rs`) flesh out the bodies; the
//! public shapes (`SaveCursor<'a>`, `LoadCursor<'a>`, `DecodeError`,
//! `LoadEntityMap`) are pinned now so the registry surface is stable.
//!
//! These types are NOT a hot path — they exist only on the cold save/load path.
//!
//! # Forward-seam dead code (S0)
//!
//! The cursor fields + the crate-internal `new` constructors + `LoadEntityMap`'s
//! `sparse` backing have NO in-crate caller until S1/S2 wire save/load. This is
//! the same ahead-of-consumer pattern the `component::enable` module uses; the
//! module-level `#[allow(dead_code)]` is removed when those phases land. (The fn-ptr
//! TYPE aliases referencing these types DO compile-exercise the public shapes now.)
#![allow(dead_code)]

use crate::ecs::core::entity::entity::Entity;

/// Append-only write cursor over a preallocated byte buffer (S1 `cursor.rs`).
///
/// In S0 this is a minimal placeholder so the cold [`SerializeFn`] alias
/// type-checks; S1 fills in the relative-offset / `base_pos` bookkeeping the
/// position-independent owning encoding needs (plan §3.8). The lifetime `'a`
/// borrows the destination buffer.
///
/// [`SerializeFn`]: crate::ecs::core::component::component_registry::SerializeFn
pub struct SaveCursor<'a> {
    /// The destination buffer being appended to. S1 replaces the `Vec<u8>` body
    /// with the exact-sized two-pass buffer + `base_pos` (plan §3.11 W3).
    pub(crate) out: &'a mut Vec<u8>,
}

impl<'a> SaveCursor<'a> {
    /// Wraps a destination buffer. Crate-internal S0 constructor — S1 widens the
    /// public surface (offset tracking, sizing sink) as the format lands.
    #[inline]
    pub(crate) fn new(out: &'a mut Vec<u8>) -> Self {
        Self { out }
    }
}

/// Bounds-checked read cursor over the file bytes (S1 `cursor.rs`).
///
/// In S0 this is a minimal placeholder so the cold [`DeserializeFn`] alias
/// type-checks; S1 fills in the `pos`-against-`bytes.len()` validation the
/// "validate, never transmute blindly" contract requires (plan §3.8). The
/// lifetime `'a` borrows the source bytes.
///
/// [`DeserializeFn`]: crate::ecs::core::component::component_registry::DeserializeFn
pub struct LoadCursor<'a> {
    /// The source bytes being read. S1 adds the `pos` read head + bounds checks.
    pub(crate) bytes: &'a [u8],
}

impl<'a> LoadCursor<'a> {
    /// Wraps a source byte slice. Crate-internal S0 constructor — S1 widens the
    /// public surface (read head, bounds-checked readers) as the format lands.
    #[inline]
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}

/// A malformed-stream decode failure (S1/S2 `error.rs`).
///
/// Returned by [`DeserializeFn`] on a malformed stream so the loader rolls back
/// (the W5 partial-row contract, mirroring `CloneFn`'s panic-leaves-uninit rule).
/// S0 declares the kind so the fn-ptr alias and the rollback contract are pinned;
/// S2 extends the variants (truncated, bad length prefix, fingerprint mismatch,
/// invalid bit pattern) as the loader lands.
///
/// [`DeserializeFn`]: crate::ecs::core::component::component_registry::DeserializeFn
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum DecodeError {
    /// The stream ended before a full value could be read.
    UnexpectedEof,
    /// A field carried a bit pattern invalid for its type (e.g. a `bool` byte
    /// other than `0|1`, an out-of-range enum discriminant) — the C3
    /// validate-on-read obligation.
    InvalidBitPattern,
}

/// Load-direction entity remap table: saved `EntityId.0` → freshly-allocated
/// `Entity` (S2 `entity_map.rs`).
///
/// Mirrors the clone subsystem's `EntityCloneMap` template (`SparseMap<Entity>`
/// keyed by `EntityId.0`, plan §3.13). S0 declares the lookup shape so the cold
/// [`LoadMapEntitiesFn`] alias type-checks; S2 fills in the build-on-load
/// population from the saved entity table.
///
/// [`LoadMapEntitiesFn`]: crate::ecs::core::component::component_registry::LoadMapEntitiesFn
#[derive(Default)]
pub struct LoadEntityMap {
    /// `saved EntityId.0` → freshly-allocated `Entity`. S2 backs this with a
    /// `SparseMap<Entity>` (the `EntityCloneMap` template); S0 keeps it empty.
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

    /// Returns the freshly-allocated `Entity` for a saved `EntityId.0`, or `None`
    /// when the saved id was never registered in this load (an unmapped reference —
    /// the C4 loud-error path the loader turns into a release error).
    #[inline]
    pub fn get(&self, saved_entity_id: usize) -> Option<Entity> {
        self.sparse.get(saved_entity_id).copied()
    }
}
