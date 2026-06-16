//! `boyko_serialize` — custom binary world save/load for the boyko ECS.
//!
//! Spec: `docs/SERIALIZATION-PLAN.md`. **Codegen, not reflection** (§1): the
//! shipping path drives serialization through the per-`ComponentId` fn-ptr table
//! in `boyko_ecs`'s cold registry (`SERIALIZE`) plus a raw-blit fast path for
//! `PlainOldBytes` columns. This crate never depends on `boyko_reflect`.
//!
//! # Phase S1 scope
//!
//! S1 ships the **file format + cursors + two-pass save**:
//! - [`format`] — the `#[repr(C)]` on-disk types ([`SaveHeader`],
//!   [`TypeTableEntry`], [`ArchetypeBlock`], [`ColumnRegion`], [`VarRef`]), with
//!   const-asserted layouts (the bytes ARE the wire contract).
//! - [`save_world`] / [`save_world_to_file`] — the two-pass save (Pass 1 sizes
//!   exactly + lays out offsets + grows the buffer once; Pass 2 blits POB columns
//!   and encodes `SerializeViaFn` columns).
//! - [`SaveOptions`] / [`SaveError`].
//!
//! The cursors driven by the per-component fn-ptrs ([`SaveCursor`] /
//! [`LoadCursor`] / [`DecodeError`]) live in `boyko_ecs`
//! (`boyko_ecs::ecs::core::serialize`) because the registry's fn-ptr aliases name
//! them; they are re-exported here for convenience.
//!
//! **Load / mmap / entity-remap are NOT in S1** (S2 / S3 / S4).
//!
//! [`SaveHeader`]: format::SaveHeader
//! [`TypeTableEntry`]: format::TypeTableEntry
//! [`ArchetypeBlock`]: format::ArchetypeBlock
//! [`ColumnRegion`]: format::ColumnRegion
//! [`VarRef`]: format::VarRef

pub mod error;
pub mod format;
pub mod save;

pub use error::SaveError;
pub use save::{SaveOptions, save_world, save_world_to_file};

// Re-export the cursor / error boundary types (they live in `boyko_ecs` for the
// registry-alias reason documented above) so downstream code has one import root.
pub use boyko_ecs::ecs::core::serialize::{DecodeError, LoadCursor, LoadEntityMap, SaveCursor};
