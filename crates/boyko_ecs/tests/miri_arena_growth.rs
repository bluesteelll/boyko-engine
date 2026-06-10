//! Phase X.F — Miri suite for arena growth bookkeeping (plan §Test matrix M1).
//!
//! Run under `cargo +nightly miri test --test miri_arena_growth`. The file is
//! `#![cfg(miri)]`-gated: under Miri the arena compiles its FALLBACK arm
//! (eager global-allocator backing, commit = watermark bump), which exercises
//! ALL of the growth bookkeeping — step math, frontier inserts, coalescing,
//! offset/pointer derivation, the GROW1 retry — under Tree Borrows. What it
//! cannot prove (real reserve/commit syscall semantics) is covered natively
//! by the unit tests in `arena.rs` (U2-U10, the drop loops).
//!
//! # M1 trace (pinned)
//!
//! `with_reserve(8 MiB, 0)` + six 1 MiB allocations (64 B align). Each 1 MiB
//! free remainder is 63 B short of `required_size = 1 MiB + 63`, so allocs
//! #1, #2 and #4 take the grow path: committed watermarks 2 -> 4 -> 8 MiB
//! (three growth events: MIN_SLAB, then doubling). The six blocks land at
//! offsets 0..=5 MiB — contiguous, non-overlapping.

#![cfg(miri)]

use std::alloc::Layout;

use boyko_ecs::ecs::memory::arena::Arena;

const MIB: usize = 1024 * 1024;

#[test]
fn miri_growth_bookkeeping_watermarks_and_non_overlap() {
    let arena = Arena::with_reserve(8 * MIB, 0);
    assert_eq!(arena.capacity(), 8 * MIB);
    assert_eq!(arena.committed(), 0, "lazy arena starts uncommitted");

    let layout = Layout::from_size_align(MIB, 64).expect("valid layout");

    // Expected commit frontier AFTER each of the six allocations.
    let expected_watermarks = [2 * MIB, 4 * MIB, 4 * MIB, 8 * MIB, 8 * MIB, 8 * MIB];

    let mut ptrs = Vec::with_capacity(6);
    for (i, &expected) in expected_watermarks.iter().enumerate() {
        let p = arena.allocate_layout(layout);
        assert_eq!(
            arena.committed(),
            expected,
            "unexpected commit watermark after alloc #{}",
            i + 1
        );
        // Write the head and tail byte of every block: under the fallback arm
        // this validates provenance/initialization of the full span under
        // Tree Borrows.
        // SAFETY: `p` heads a fresh 1 MiB block inside the arena backing;
        // offsets 0 and MIB - 1 are in bounds.
        unsafe {
            p.as_ptr().write(i as u8);
            p.as_ptr().add(MIB - 1).write(0xF0 | i as u8);
        }
        ptrs.push(p);
    }

    // Non-overlap + containment: sorted block start ADDRESSES must be
    // >= 1 MiB apart and the whole span must fit the 8 MiB backing.
    // (Addresses are used for arithmetic only — reads below go through the
    // original pointers, preserving provenance.)
    let mut sorted: Vec<usize> = ptrs.iter().map(|p| p.as_ptr() as usize).collect();
    sorted.sort_unstable();
    for pair in sorted.windows(2) {
        assert!(
            pair[1] - pair[0] >= MIB,
            "blocks overlap: starts {:#x} and {:#x}",
            pair[0],
            pair[1]
        );
    }
    assert!(
        sorted[5] + MIB - sorted[0] <= 8 * MIB,
        "blocks span more than the reservation"
    );

    // Read-back witness: head/tail bytes survive all later growth events.
    for (i, p) in ptrs.iter().enumerate() {
        // SAFETY: same pointers (and provenance) as the writes above; the
        // arena is still alive, offsets in bounds.
        unsafe {
            assert_eq!(p.as_ptr().read(), i as u8, "head byte of block #{i}");
            assert_eq!(
                p.as_ptr().add(MIB - 1).read(),
                0xF0 | i as u8,
                "tail byte of block #{i}"
            );
        }
    }
}
