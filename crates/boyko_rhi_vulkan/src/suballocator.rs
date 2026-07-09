//! Free-list-with-coalescing sub-allocator over a single contiguous block.
//!
//! This is the pure-logic core of the `VkDeviceMemory` sub-allocator (the
//! plan's highest-risk item, §7 Phase 0b). It manages BYTE OFFSETS within one
//! externally-owned block of `capacity` bytes and contains **no Vulkan and no
//! `unsafe`** — every offset it returns is just an integer, validated and
//! tested in isolation. `memory.rs` layers the real `vkAllocateMemory` block
//! and `vkMapMemory` pointer on top.
//!
//! # Design
//!
//! A sorted `Vec<FreeRange>` of disjoint, non-adjacent free ranges (the
//! free-list lineage of the retired `MemFreeBlockMaster`, reimplemented
//! locally so this crate does not depend on `boyko_ecs`). Allocation is
//! first-fit with alignment-aware splitting; `free` re-inserts a range and
//! coalesces it with any adjacent neighbours so the list never holds two
//! touching free ranges. A side `Vec<Allocation>` records live allocations so
//! `free(offset)` can recover the exact `[offset, offset+size)` extent that was
//! handed out (the alignment padding in front of an allocation is reclaimed
//! too).
//!
//! # Complexity
//!
//! `alloc`/`free` are O(F) in the number of free ranges (a linear scan +
//! sorted insert). For the foundation's long-lived-resource pool this is fine;
//! the streaming ring pool (§4) is a separate structure added later.

/// A half-open free range `[start, start + size)` of byte offsets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct FreeRange {
    start: u64,
    size: u64,
}

impl FreeRange {
    #[inline]
    fn end(self) -> u64 {
        self.start + self.size
    }
}

/// A live allocation handed out by [`SubAllocator::alloc`].
///
/// `start` is the alignment-padding start (the byte reclaimed on `free`);
/// `offset` is the aligned offset returned to the caller. They differ only when
/// alignment forced padding in front of the block.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Allocation {
    /// Aligned offset returned to the caller (the map key).
    offset: u64,
    /// Reclaimable extent start (`<= offset`).
    start: u64,
    /// Reclaimable extent size (alignment pad + requested size).
    size: u64,
}

/// First-fit free-list sub-allocator with adjacent-range coalescing over a
/// single `capacity`-byte block.
pub struct SubAllocator {
    /// Total managed size in bytes.
    capacity: u64,
    /// Disjoint, non-adjacent free ranges, kept sorted ascending by `start`.
    /// Invariant: for any two consecutive ranges `a`, `b`: `a.end() < b.start`
    /// (strict — touching ranges are always coalesced).
    free: Vec<FreeRange>,
    /// Live allocations, indexed for `free(offset)` lookup. Unsorted; `free`
    /// is O(live) but live counts are small for long-lived resources.
    live: Vec<Allocation>,
}

impl SubAllocator {
    /// Creates a sub-allocator managing `[0, capacity)` as one free range.
    ///
    /// A zero capacity is permitted (every allocation then fails with `None`).
    pub fn new(capacity: u64) -> Self {
        let mut free = Vec::new();
        if capacity > 0 {
            free.push(FreeRange { start: 0, size: capacity });
        }
        Self { capacity, free, live: Vec::new() }
    }

    /// Total managed capacity in bytes.
    #[inline]
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Allocates `size` bytes aligned to `align` (a power of two, or 1).
    ///
    /// Returns the aligned byte offset on success, or `None` if no free range
    /// can satisfy the request (exhaustion / fragmentation). A `size` of zero
    /// or a non-power-of-two `align` is rejected with `None` (callers always
    /// pass `VkMemoryRequirements.{size, alignment}`, which are positive
    /// powers of two, but the guard keeps the unit tests honest).
    pub fn alloc(&mut self, size: u64, align: u64) -> Option<u64> {
        if size == 0 || align == 0 || !align.is_power_of_two() {
            return None;
        }

        // First-fit: the first free range whose aligned interior holds `size`.
        for i in 0..self.free.len() {
            let range = self.free[i];
            let aligned = align_up(range.start, align)?;
            // The aligned offset must still lie within the range, and the
            // padding-plus-size must fit. Guard against u64 overflow on the
            // additions before comparing against the range end.
            let needed_end = aligned.checked_add(size)?;
            if aligned >= range.start && needed_end <= range.end() {
                let pad = aligned - range.start;
                self.carve(i, pad, size);
                self.live.push(Allocation { offset: aligned, start: range.start, size: pad + size });
                return Some(aligned);
            }
        }
        None
    }

    /// Frees the allocation previously returned at `offset`, coalescing the
    /// reclaimed extent with any adjacent free ranges.
    ///
    /// Returns `true` if `offset` named a live allocation, `false` otherwise
    /// (a double-free or an unknown offset is a no-op caught by the bool).
    pub fn free(&mut self, offset: u64) -> bool {
        let Some(live_idx) = self.live.iter().position(|a| a.offset == offset) else {
            return false;
        };
        let alloc = self.live.swap_remove(live_idx);
        self.insert_and_coalesce(FreeRange { start: alloc.start, size: alloc.size });
        true
    }

    /// Number of currently-live allocations (test/diagnostic helper).
    #[inline]
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    /// Number of disjoint free ranges (test/diagnostic helper — a fully
    /// coalesced empty allocator reports `1`, or `0` at zero capacity).
    #[inline]
    pub fn free_range_count(&self) -> usize {
        self.free.len()
    }

    /// Carves the span `[start, start + pad + size)` out of the free range at
    /// index `i`. The alignment `pad` is part of the allocation's extent (it is
    /// reclaimed on `free`, not left as a tiny free remnant), so only the tail
    /// `[start + pad + size, end)` can remain free.
    fn carve(&mut self, i: usize, pad: u64, size: u64) {
        let range = self.free[i];
        let consumed = pad + size;
        debug_assert!(consumed <= range.size, "carve overruns the free range");
        let tail_start = range.start + consumed;
        let tail_size = range.size - consumed;
        if tail_size == 0 {
            self.free.remove(i);
        } else {
            self.free[i] = FreeRange { start: tail_start, size: tail_size };
        }
    }

    /// Inserts `range` into the sorted free-list and coalesces it with any
    /// neighbour it touches, preserving the strict-non-adjacency invariant.
    ///
    /// The ranges being freed are always disjoint from every existing free
    /// range (they came from a live allocation), so the only work is a sorted
    /// insert followed by at most one left-merge and one right-merge.
    fn insert_and_coalesce(&mut self, range: FreeRange) {
        // Sorted-insert position: first existing range that starts after us.
        let pos = self.free.partition_point(|r| r.start < range.start);
        debug_assert!(
            pos == 0 || self.free[pos - 1].end() <= range.start,
            "freed range overlaps its left neighbour"
        );
        debug_assert!(
            pos == self.free.len() || range.end() <= self.free[pos].start,
            "freed range overlaps its right neighbour"
        );
        self.free.insert(pos, range);

        // Merge the right pair (pos, pos+1) first so the left merge below sees
        // the fully-grown range; then merge the left pair (pos-1, pos). Doing
        // right-then-left keeps `pos` valid as our range's index throughout
        // (a right merge removes index pos+1, never shifting pos).
        self.try_merge(pos);
        if pos > 0 {
            self.try_merge(pos - 1);
        }
    }

    /// Merges free range `i` with `i + 1` if they are exactly adjacent
    /// (`free[i].end() == free[i+1].start`). No-op otherwise.
    fn try_merge(&mut self, i: usize) {
        if i + 1 >= self.free.len() {
            return;
        }
        let a = self.free[i];
        let b = self.free[i + 1];
        debug_assert!(a.end() <= b.start, "free-list out of order");
        if a.end() == b.start {
            self.free[i] = FreeRange { start: a.start, size: a.size + b.size };
            self.free.remove(i + 1);
        }
    }
}

/// Rounds `value` up to the next multiple of `align` (a power of two), or
/// `None` on overflow. Standalone + total so it is unit-testable.
fn align_up(value: u64, align: u64) -> Option<u64> {
    debug_assert!(align.is_power_of_two(), "align must be a power of two");
    let mask = align - 1;
    value.checked_add(mask).map(|v| v & !mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A1 — a fresh allocator is one free range covering the whole capacity.
    #[test]
    fn fresh_allocator_single_free_range() {
        let a = SubAllocator::new(1024);
        assert_eq!(a.capacity(), 1024);
        assert_eq!(a.free_range_count(), 1);
        assert_eq!(a.live_count(), 0);
    }

    /// A2 — sequential allocations carve from the front and never overlap.
    #[test]
    fn sequential_allocs_do_not_overlap() {
        let mut a = SubAllocator::new(1024);
        let o0 = a.alloc(100, 1).unwrap();
        let o1 = a.alloc(100, 1).unwrap();
        let o2 = a.alloc(100, 1).unwrap();
        assert_eq!(o0, 0);
        assert_eq!(o1, 100);
        assert_eq!(o2, 200);
        // Pairwise non-overlap of [offset, offset+100).
        let spans = [(o0, 100u64), (o1, 100), (o2, 100)];
        for i in 0..spans.len() {
            for j in (i + 1)..spans.len() {
                let (s0, l0) = spans[i];
                let (s1, l1) = spans[j];
                assert!(s0 + l0 <= s1 || s1 + l1 <= s0, "spans overlap");
            }
        }
        assert_eq!(a.live_count(), 3);
    }

    /// A3 — alignment is honored: every returned offset is a multiple of the
    /// requested alignment.
    #[test]
    fn alignment_honored() {
        let mut a = SubAllocator::new(4096);
        // Force a misaligned tail, then demand a 256-byte alignment.
        let _o0 = a.alloc(17, 1).unwrap();
        let o1 = a.alloc(64, 256).unwrap();
        assert_eq!(o1 % 256, 0, "offset not 256-aligned");
        let o2 = a.alloc(64, 64).unwrap();
        assert_eq!(o2 % 64, 0, "offset not 64-aligned");
        // o1 cannot overlap o0's [0,17).
        assert!(o1 >= 17);
    }

    /// A4 — exhaustion returns None (request larger than capacity).
    #[test]
    fn exhaustion_returns_none() {
        let mut a = SubAllocator::new(256);
        assert!(a.alloc(512, 1).is_none(), "oversized request must fail");
        // Exact fit succeeds, then any further byte fails.
        assert_eq!(a.alloc(256, 1), Some(0));
        assert!(a.alloc(1, 1).is_none(), "full allocator must reject");
    }

    /// A5 — free + coalesce + re-alloc reuses the space; a fully-emptied
    /// allocator collapses back to a single free range.
    #[test]
    fn free_coalesce_reuse() {
        let mut a = SubAllocator::new(1024);
        let o0 = a.alloc(256, 1).unwrap();
        let o1 = a.alloc(256, 1).unwrap();
        let o2 = a.alloc(256, 1).unwrap();
        assert_eq!((o0, o1, o2), (0, 256, 512));

        // Free the middle block — it becomes its own free range between the
        // live neighbours, so there is a hole plus the tail = 2 free ranges.
        assert!(a.free(o1));
        assert_eq!(a.free_range_count(), 2);

        // A 256-byte request refills the hole at offset 256 (first-fit).
        let reuse = a.alloc(256, 1).unwrap();
        assert_eq!(reuse, 256, "freed hole must be reused");

        // Free everything; adjacent ranges coalesce back to one.
        assert!(a.free(o0));
        assert!(a.free(reuse));
        assert!(a.free(o2));
        assert_eq!(a.free_range_count(), 1, "emptied allocator must coalesce to one range");
        assert_eq!(a.live_count(), 0);
        // The whole capacity is allocatable again as one block.
        assert_eq!(a.alloc(1024, 1), Some(0));
    }

    /// A6 — coalescing merges a freed range with BOTH neighbours at once.
    #[test]
    fn free_coalesces_both_neighbours() {
        let mut a = SubAllocator::new(900);
        let o0 = a.alloc(300, 1).unwrap();
        let o1 = a.alloc(300, 1).unwrap();
        let o2 = a.alloc(300, 1).unwrap();
        // Free the outer two first → two separate holes around the live middle.
        assert!(a.free(o0));
        assert!(a.free(o2));
        assert_eq!(a.free_range_count(), 2);
        // Freeing the middle bridges both holes into one full range.
        assert!(a.free(o1));
        assert_eq!(a.free_range_count(), 1);
        assert_eq!(a.alloc(900, 1), Some(0));
    }

    /// A7 — double-free / unknown-offset is a no-op reported as `false`.
    #[test]
    fn double_free_is_noop() {
        let mut a = SubAllocator::new(128);
        let o = a.alloc(64, 1).unwrap();
        assert!(a.free(o));
        assert!(!a.free(o), "second free of the same offset must report false");
        assert!(!a.free(9999), "unknown offset must report false");
    }

    /// A8 — degenerate requests are rejected without disturbing the free-list.
    #[test]
    fn degenerate_requests_rejected() {
        let mut a = SubAllocator::new(128);
        assert!(a.alloc(0, 1).is_none(), "zero size rejected");
        assert!(a.alloc(8, 0).is_none(), "zero align rejected");
        assert!(a.alloc(8, 3).is_none(), "non-power-of-two align rejected");
        // The free-list is untouched: a valid request still uses offset 0.
        assert_eq!(a.alloc(8, 1), Some(0));
    }

    /// A9 — zero-capacity allocator has no free ranges and fails every alloc.
    #[test]
    fn zero_capacity() {
        let mut a = SubAllocator::new(0);
        assert_eq!(a.free_range_count(), 0);
        assert!(a.alloc(1, 1).is_none());
    }

    /// A10 — alignment padding is reclaimed on free (no slow leak of pad
    /// bytes): an aligned alloc that wasted pad bytes, once freed, leaves the
    /// allocator able to re-serve the full capacity as one block.
    #[test]
    fn alignment_pad_reclaimed_on_free() {
        let mut a = SubAllocator::new(4096);
        let o0 = a.alloc(1, 1).unwrap(); // offset 0, len 1
        assert_eq!(o0, 0);
        let o1 = a.alloc(16, 256).unwrap(); // aligned to 256 → pad [1,256)
        assert_eq!(o1, 256);
        a.free(o0);
        a.free(o1);
        assert_eq!(a.free_range_count(), 1, "pad must coalesce away");
        assert_eq!(a.alloc(4096, 1), Some(0));
    }
}
