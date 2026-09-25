//! UG-04 owner attribution (unified plan KC-01; allocator G2b): every commit route counts its
//! bytes under the owner it names, and under no other.
//!
//! Four commits, one per route, each read as a per-owner delta:
//!
//! | step | route | expected delta `[Column, Chunk, Table]` |
//! |---|---|---|
//! | 1 | a default `VmColumn` pushes its first element | `[COMMIT_PAGE, 0, 0]` |
//! | 2 | a `VmColumn<_, TableOwner>` pushes its first element | `[0, 0, COMMIT_PAGE]` |
//! | 3 | `VmReservation::commit` of one page | `[COMMIT_PAGE, 0, 0]` |
//! | 4 | `raw::commit_at::<ChunkOwner>` of one page | `[0, COMMIT_PAGE, 0]` |
//!
//! Step 1 and step 2 each commit one `POOL_MIN_SLAB` (= `COMMIT_PAGE`), the first rung of the
//! column's commit ladder. Step 2 is the case a column that routed every commit through the
//! default owner would get wrong: it would count the table's page under `Column`.
//!
//! The fallback arm (Miri) commits nothing at the OS but counts the same bytes, so these numbers
//! hold under Miri too. One `#[test]` in this binary, on purpose: the counter is process-global,
//! and a second test would commit concurrently into every window.

use boyko_memory::constants::{COMMIT_GRANULE, COMMIT_PAGE, POOL_MIN_SLAB};
use boyko_memory::vm::VmReservation;
use boyko_memory::vm_column::VmColumn;
use boyko_memory::{ChunkOwner, ColumnOwner, TableOwner, committed_bytes, raw};

/// `[Column, Chunk, Table]` committed bytes, now.
fn snapshot() -> [usize; 3] {
    [
        committed_bytes::<ColumnOwner>(),
        committed_bytes::<ChunkOwner>(),
        committed_bytes::<TableOwner>(),
    ]
}

/// Per-owner bytes committed since `before` (the counters never decrease).
fn since(before: [usize; 3]) -> [usize; 3] {
    let now = snapshot();
    [now[0] - before[0], now[1] - before[1], now[2] - before[2]]
}

#[test]
fn every_commit_route_counts_under_its_own_owner() {
    assert_eq!(POOL_MIN_SLAB, COMMIT_PAGE, "the first ladder rung is one commit page");

    let s = snapshot();
    let mut column = VmColumn::<u64>::new("ug04.column", 1024);
    column.push(1);
    assert_eq!(since(s), [COMMIT_PAGE, 0, 0], "step 1: a default VmColumn commits as Column");

    let s = snapshot();
    let mut table = VmColumn::<u64, TableOwner>::with_owner("ug04.table", 1024);
    table.push(2);
    assert_eq!(since(s), [0, 0, COMMIT_PAGE], "step 2: a VmColumn<_, TableOwner> commits as Table");

    let s = snapshot();
    let reservation = VmReservation::reserve(2 * COMMIT_GRANULE);
    reservation.commit(0, COMMIT_PAGE);
    assert_eq!(since(s), [COMMIT_PAGE, 0, 0], "step 3: VmReservation::commit is the Column route");

    let s = snapshot();
    // SAFETY: `off + len = 2 * COMMIT_PAGE <= 2 * COMMIT_GRANULE <= os_len()` (a granule is a
    // whole number of pages, and the reservation was asked for two granules), so the range lies
    // inside the reservation: `commit_at`'s whole contract.
    unsafe { raw::commit_at::<ChunkOwner>(&reservation, COMMIT_PAGE, COMMIT_PAGE) };
    assert_eq!(since(s), [0, COMMIT_PAGE, 0], "step 4: raw::commit_at::<ChunkOwner> commits as Chunk");

    assert_eq!((column.get(0), table.get(0)), (Some(1), Some(2)), "both columns hold their element");
}
