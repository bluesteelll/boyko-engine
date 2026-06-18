//! `boyko_serialize` — custom binary world save/load for the boyko ECS.
//!
//! Spec: `docs/SERIALIZATION-PLAN.md`. **Codegen, not reflection** (§1): the
//! shipping path drives serialization through the per-`ComponentId` fn-ptr table
//! in `boyko_ecs`'s cold registry (`SERIALIZE`) plus a raw-blit fast path for
//! `PlainOldBytes` columns. This crate never depends on `boyko_reflect`.
//!
//! # Scope (Phases S1 / S1.5 / S2)
//!
//! - [`format`](mod@format) — the `#[repr(C)]` on-disk types ([`SaveHeader`],
//!   [`TypeTableEntry`], [`ArchetypeBlock`], [`ColumnRegion`], [`VarRef`]), with
//!   const-asserted layouts (the bytes ARE the wire contract).
//! - [`save_world`] / [`save_world_to_file`] — the two-pass save (Pass 1 sizes
//!   exactly + lays out offsets + grows the buffer once; Pass 2 blits POB columns
//!   and encodes `SerializeViaFn` columns).
//! - [`load_world`] / [`load_world_from_file`] (Phase S2) — the
//!   `CopyIntoWorld` + [`Remap`](LoadEntityPolicy::Remap) loader: validate header,
//!   resolve the type table once, then per FRESH archetype (W4)
//!   blit POB columns / decode `SerializeViaFn` columns / default-construct
//!   no-data columns, allocate a fresh entity batch, and record the saved→fresh
//!   map. Returns a [`LoadReport`]; rolls a partial archetype back to empty on a
//!   malformed stream.
//! - [`SaveOptions`] / [`SaveError`] / [`LoadError`].
//!
//! The actual row-write driver lives in `boyko_ecs`
//! (`boyko_ecs::ecs::core::serialize::load_writer`) because the pool/archetype/
//! entity-master write primitives are crate-private; `load.rs` here is the
//! file-format parser that resolves the file into per-archetype
//! [`LoadColumn`] instructions and
//! hands one archetype at a time to that driver.
//!
//! The cursors driven by the per-component fn-ptrs ([`SaveCursor`] /
//! [`LoadCursor`] / [`DecodeError`]) live in `boyko_ecs`
//! (`boyko_ecs::ecs::core::serialize`) because the registry's fn-ptr aliases name
//! them; they are re-exported here for convenience.
//!
//! # Phase S2 boundary
//!
//! S2 round-trips a world with NO cross-entity references. Entity fields load with
//! their RAW saved ids; the saved→fresh remap (`map_entities_fn`) is deferred to
//! S2.5. `MmapInPlace` (S3) and `PreserveIds` (W2) are out of scope.
//!
//! [`SaveHeader`]: format::SaveHeader
//! [`TypeTableEntry`]: format::TypeTableEntry
//! [`ArchetypeBlock`]: format::ArchetypeBlock
//! [`ColumnRegion`]: format::ColumnRegion
//! [`VarRef`]: format::VarRef

pub mod error;
pub mod format;
pub mod load;
pub mod save;

pub use error::{LoadError, SaveError};
pub use load::{LoadEntityPolicy, LoadReport, load_world, load_world_from_file};
pub use save::{SaveOptions, save_world, save_world_to_file};

// Re-export the cursor / error / loader boundary types (they live in `boyko_ecs`
// for the registry-alias reason documented above) so downstream code has one import
// root.
pub use boyko_ecs::ecs::core::serialize::{
    DecodeError, LoadColumn, LoadCursor, LoadEntityMap, SaveCursor,
};
