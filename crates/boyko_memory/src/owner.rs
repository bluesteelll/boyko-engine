//! Commit owners and the per-owner committed-bytes counter (unified plan KC-01; gate UG-04,
//! allocator G2b).
//!
//! Every byte a kernel store commits goes through one choke point, `vm::commit_at::<O>`, and is
//! counted there under its owner. The owner is a marker type with an associated `INDEX`, so the
//! choice is monomorphised: attributing a commit costs no runtime instruction beyond the one
//! relaxed add, and that add sits on the cold commit path beside a syscall, never on a hot one.
//!
//! | owner | producers |
//! |---|---|
//! | [`ColumnOwner`] | every `ComponentPool`, every default `VmColumn`, `InlandStore`, the profiling store |
//! | [`ChunkOwner`] | the scope `ChunkArena` (KC-05), from rung D-M2 |
//! | [`TableOwner`] | the KC-18 kernel tables, declared `VmColumn<T, TableOwner>`, from rung D-M1 |
//!
//! The counters are statistics, not synchronisation: they only ever grow (nothing decommits), and
//! no reader orders any other memory access on them, so every access is `Relaxed` (KC-01, "a
//! bound, not a publication").

use core::sync::atomic::{AtomicUsize, Ordering};

/// The number of commit owners, and the length of the counter array.
pub const COMMIT_OWNERS: usize = 3;

mod sealed {
    /// Seals [`CommitOwner`](super::CommitOwner): only this module can name an owner, so every
    /// `INDEX` is in range by construction and no crate outside the kernel can mint one.
    pub trait Sealed {}
}

/// A kernel owner of committed virtual memory. Sealed; the three implementors are the whole set.
pub trait CommitOwner: sealed::Sealed {
    /// This owner's slot in the counter array; `< COMMIT_OWNERS`.
    const INDEX: usize;
}

/// The default owner: every `ComponentPool`, every default `VmColumn`, `InlandStore` and the
/// profiling store (their shared route is `VmReservation::commit`).
pub struct ColumnOwner;

/// The scope chunk arena (KC-05). No producer until rung D-M2.
pub struct ChunkOwner;

/// The KC-18 kernel tables, declared `VmColumn<T, TableOwner>`. No producer until rung D-M1.
pub struct TableOwner;

impl sealed::Sealed for ColumnOwner {}
impl sealed::Sealed for ChunkOwner {}
impl sealed::Sealed for TableOwner {}

impl CommitOwner for ColumnOwner {
    const INDEX: usize = 0;
}
impl CommitOwner for ChunkOwner {
    const INDEX: usize = 1;
}
impl CommitOwner for TableOwner {
    const INDEX: usize = 2;
}

/// Bytes committed so far, per owner, since process start. Never decremented: no route decommits.
static COMMITTED_BYTES: [AtomicUsize; COMMIT_OWNERS] = [const { AtomicUsize::new(0) }; COMMIT_OWNERS];

/// Counts `bytes` just committed under owner `O`. Called by `commit_at` after the commit
/// succeeded, on every backing arm.
#[inline]
pub(crate) fn record<O: CommitOwner>(bytes: usize) {
    const { assert!(O::INDEX < COMMIT_OWNERS) };
    // Relaxed: a statistic with no ordering role; see the module doc.
    COMMITTED_BYTES[O::INDEX].fetch_add(bytes, Ordering::Relaxed);
}

/// Bytes committed under owner `O` since process start (UG-04's census reads windows of this as
/// a delta). The counter is process-global, so a delta is exact only while no other thread
/// commits under the same owner.
#[inline]
pub fn committed_bytes<O: CommitOwner>() -> usize {
    const { assert!(O::INDEX < COMMIT_OWNERS) };
    // Relaxed: a statistic with no ordering role; see the module doc.
    COMMITTED_BYTES[O::INDEX].load(Ordering::Relaxed)
}
