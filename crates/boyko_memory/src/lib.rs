//! `boyko_memory` — the memory primitives one layer below the ECS pool (unified plan KC-01).
//!
//! - [`vm`]: [`VmReservation`](vm::VmReservation), the single per-OS reserve / commit / release
//!   primitive every kernel store is built on.
//! - [`vm_column`]: [`VmColumn`](vm_column::VmColumn), a typed, address-stable, growable column
//!   on one reservation.
//! - [`constants`]: the commit granularity (`COMMIT_GRANULE`, `COMMIT_PAGE`) and the slab bounds
//!   of the commit ladders.
//! - The commit owners ([`CommitOwner`]) and the per-owner committed-bytes counter
//!   ([`committed_bytes`]) that gate UG-04 reads: every commit is counted at one choke point,
//!   `raw::commit_at::<O>`, under the owner its store declares.
//!
//! # Why a crate of its own
//!
//! These files lived in `boyko_ecs::ecs::memory` until rung C1, which moved them here so that
//! a crate below the ECS can hold engine storage without taking a dependency on the ECS.
//! `boyko_ecs` re-exports every old path, so its consumers compile unchanged.
//!
//! # The public surface is the soundness boundary
//!
//! Inside `boyko_ecs` these types were `pub(crate)`, and `VmReservation::commit` checked its range
//! only with `debug_assert!`: every caller was in the crate and held to its invariants. Across a
//! crate boundary that no longer holds, so the range check that keeps the syscall inside the
//! reservation is a release `assert!` on the cold commit path, and the one route that skips it,
//! `raw::commit_at`, is an `unsafe fn`. Every other safe entry point was already release-checked
//! (`VmColumn`'s bounds and ceiling asserts) or cannot reach memory at all (`base`, `os_len`).

pub mod constants;
mod owner;
pub mod utils;
pub mod vm;
pub mod vm_column;

pub use owner::{COMMIT_OWNERS, ChunkOwner, ColumnOwner, CommitOwner, TableOwner, committed_bytes};

/// Raw reserve and commit, for a kernel store that commits under an owner other than
/// [`ColumnOwner`] (KC-01). Hidden from the docs: never part of a surface a mod can reach
/// (allocator K-MOD-7).
#[doc(hidden)]
pub mod raw {
    pub use crate::vm::commit_at;
    use crate::vm::VmReservation;

    /// Reserves `bytes` of address space, committing nothing (`VmReservation::reserve`).
    #[inline]
    pub fn reserve(bytes: usize) -> VmReservation {
        VmReservation::reserve(bytes)
    }
}
