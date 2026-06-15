//! Enable-bit (sparse, non-fragmenting) tag storage backend.
//!
//! This module hosts the per-archetype, row-indexed, **paged** bitset storage
//! for `EnableTag` components (Decision D1) plus the per-world per-tag
//! archetype-presence oracle used by the query cull (Decision D2).
//!
//! Unlike signature (table) storage, an `EnableTag` never enters an archetype
//! signature mask and owns no `ComponentPool`; toggling a flag is a single
//! atomic read-modify-write at `(archetype, row)` with no migration and no
//! structural-generation bump.
//!
//! - [`enable_store`] — `EnablePage` / `EnableColumn` / `EnableStore` (the
//!   paged bitset columns living on each `Archetype`).
//! - [`enable_presence`] — `EnablePresence` (the per-tag archetype bitset cull
//!   oracle; filled by Step 3).

pub(crate) mod enable_store;
pub(crate) mod enable_presence;
