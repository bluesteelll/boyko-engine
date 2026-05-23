use std::alloc::{alloc, dealloc, Layout};
use std::cell::UnsafeCell;
use std::ptr::NonNull;

use crate::ecs::constants::{CACHE_LINE_SIZE, DEFAULT_ARENA_SIZE};
use crate::ecs::memory::free_mem_block::MemFreeBlockMaster;
use crate::ecs::memory::utils::align_up;

/// Pre-allocated, cache-line-aligned memory arena used as backing store for
/// every `ComponentPool` in the ECS.
///
/// # Ownership and lifetime
///
/// The arena is meant to live inside `EcsMaster` behind a `Box<Arena>` so its
/// address is stable across moves of the owning `EcsMaster` (audit finding
/// C-001). Children (`Archetype`, `ComponentPool`, ...) store a `NonNull<Arena>`
/// derived from that `Box` and rely on the address remaining valid for the
/// entire lifetime of the owning `EcsMaster`.
///
/// # Thread-safety
///
/// `Arena` is intentionally **single-threaded**: it contains an `UnsafeCell`
/// (free-block tracker), and there is no `Send`/`Sync` `impl`. The auto-derive
/// excludes both because `NonNull<u8>` and `UnsafeCell<_>` are `!Sync`. Do not
/// share an `Arena` (or anything that transitively holds a `NonNull<Arena>`)
/// across threads.
///
/// # Aliasing invariant for `allocate_*` (audit finding M-003)
///
/// `allocate_layout` / `allocate_from_free_blocks` take `&self` and reach into
/// the `UnsafeCell` to obtain a `&mut MemFreeBlockMaster`. For that to be
/// sound, **no other `&MemFreeBlockMaster` may exist concurrently**. The
/// single-threaded design and the absence of any other entry point that hands
/// out a reference to `free_blocks` keeps this invariant. Concurrent
/// `allocate_*` calls from two threads would be UB — protected against by the
/// non-`Sync` marker.
pub struct Arena {
    /// Backing buffer pointer. Allocated in `with_capacity`, freed in `Drop`.
    ptr: NonNull<u8>,

    /// Capacity of the backing buffer in bytes (cache-line aligned).
    capacity: usize,

    /// Layout used for the original `alloc`. Kept so that `Drop` can pass the
    /// exact same `Layout` to `dealloc` (required by `GlobalAlloc` contract).
    layout: Layout,

    /// Free-block tracker. Lives inside `UnsafeCell` because both
    /// `allocate_from_free_blocks` and future deallocate paths need to mutate
    /// it through `&self`. See the type-level doc-comment for the aliasing
    /// invariant.
    free_blocks: UnsafeCell<MemFreeBlockMaster>,
}

impl Arena {
    /// Allocates a fresh arena with `capacity` bytes, rounded up to cache-line
    /// granularity. Panics on allocation failure.
    pub fn with_capacity(capacity: usize) -> Self {
        let aligned_capacity = align_up(capacity, CACHE_LINE_SIZE);

        let layout = Layout::from_size_align(aligned_capacity, CACHE_LINE_SIZE)
            .expect("Invalid layout for arena");

        // SAFETY: layout has non-zero size (DEFAULT_ARENA_SIZE > 0) and
        // alignment is a valid power of two (`CACHE_LINE_SIZE` is 64).
        let raw = unsafe { alloc(layout) };
        let ptr = NonNull::new(raw).expect("Failed to allocate memory for arena");

        Self {
            ptr,
            capacity: aligned_capacity,
            layout,
            free_blocks: UnsafeCell::new(MemFreeBlockMaster::new_init(aligned_capacity)),
        }
    }

    /// Allocates an arena of the default size (`DEFAULT_ARENA_SIZE`).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_ARENA_SIZE)
    }

    /// Total capacity in bytes (cache-line aligned).
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Allocates `layout` from the arena. Panics if the arena cannot satisfy
    /// the request (no free block large enough).
    pub fn allocate_layout(&self, layout: Layout) -> NonNull<u8> {
        debug_assert!(
            layout.size() <= self.capacity,
            "allocation request {} B exceeds arena capacity {} B",
            layout.size(),
            self.capacity
        );

        match self.allocate_from_free_blocks(layout) {
            Some(ptr) => ptr,
            None => panic!("Arena out of memory: no suitable free blocks available"),
        }
    }

    /// Attempts to allocate `layout` from the free-block tracker. Returns
    /// `None` when no suitable block exists.
    pub fn allocate_from_free_blocks(&self, layout: Layout) -> Option<NonNull<u8>> {
        let size = layout.size();
        let align = layout.align();

        // SAFETY: see the type-level doc-comment — the arena is single-threaded
        // and no other reference into `free_blocks` exists while we hold this
        // exclusive `&mut`. The `&mut` is dropped before this function returns.
        let free_blocks = unsafe { &mut *self.free_blocks.get() };
        let block = free_blocks.allocate_aligned(size, align)?;

        // SAFETY: `block.start` is within `[0, self.capacity)` and
        // `block.start + size <= self.capacity` — that is the contract of
        // `allocate_aligned`. The resulting pointer is therefore inside the
        // single allocated object that `self.ptr` heads.
        let ptr = unsafe { self.ptr.as_ptr().add(block.start) };
        NonNull::new(ptr)
    }

    /// Convenience wrapper around `allocate_layout` for a Sized type `T`.
    pub fn allocate<T: Sized>(&self) -> NonNull<T> {
        let layout = Layout::new::<T>();
        let ptr = self.allocate_layout(layout);
        ptr.cast()
    }
}

impl Drop for Arena {
    /// Releases the backing buffer back to the global allocator (audit
    /// finding M-001 — without this, every `Arena::new()` leaks 64 MB).
    ///
    /// Note: type-erased component `Drop` is **not** invoked here. The
    /// components live inside this buffer, but their drop is the
    /// responsibility of `ComponentPool::Drop` (tracked separately as a
    /// Phase 1b refactor; today, components with non-trivial `Drop` will
    /// still leak their inner heap data when their `ComponentPool` is
    /// destroyed, which is a known limitation).
    fn drop(&mut self) {
        // SAFETY: `self.ptr` was returned by `alloc(self.layout)` in
        // `with_capacity` and has not been freed yet (`Drop` runs once). The
        // exact same `Layout` is passed back to `dealloc`, satisfying the
        // `GlobalAlloc` contract. After this point `self.ptr` must not be
        // used — which is fine because `Drop` is the last call on `self`.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- arena::with_capacity ---

    #[test]
    fn arena_capacity_is_cache_line_aligned() {
        // Request a non-multiple-of-64 value; capacity must be rounded up.
        let arena = Arena::with_capacity(100);
        assert!(
            arena.capacity() >= 100,
            "capacity must be at least the requested amount"
        );
        assert_eq!(
            arena.capacity() % CACHE_LINE_SIZE, 0,
            "capacity must be a multiple of CACHE_LINE_SIZE ({} B)",
            CACHE_LINE_SIZE
        );
        // Drop arena — M-001 fix: must not panic, backing memory is freed.
    }

    #[test]
    fn arena_default_size_matches_constant() {
        let arena = Arena::new();
        // DEFAULT_ARENA_SIZE is itself a multiple of CACHE_LINE_SIZE (64 MB / 64 = exact),
        // so after align_up the capacity must equal exactly DEFAULT_ARENA_SIZE.
        let expected = align_up(DEFAULT_ARENA_SIZE, CACHE_LINE_SIZE);
        assert_eq!(
            arena.capacity(),
            expected,
            "Arena::new() capacity should match align_up(DEFAULT_ARENA_SIZE, CACHE_LINE_SIZE)"
        );
    }

    #[test]
    fn arena_small_capacity_rounds_up_to_one_cache_line() {
        // Requesting 1 byte should yield exactly CACHE_LINE_SIZE bytes.
        let arena = Arena::with_capacity(1);
        assert_eq!(arena.capacity(), CACHE_LINE_SIZE);
    }

    #[test]
    fn arena_exact_multiple_of_cache_line_unchanged() {
        let cap = CACHE_LINE_SIZE * 16; // exact multiple
        let arena = Arena::with_capacity(cap);
        assert_eq!(arena.capacity(), cap, "exact multiple must not be changed");
    }

    // --- arena::allocate_layout ---

    #[test]
    fn arena_allocate_layout_returns_aligned_pointer() {
        let arena = Arena::with_capacity(4096);
        let align = 64usize;
        let layout =
            Layout::from_size_align(128, align).expect("valid layout");
        let ptr = arena.allocate_layout(layout);
        assert_eq!(
            ptr.as_ptr() as usize % align,
            0,
            "returned pointer must be {align}-byte aligned"
        );
    }

    #[test]
    fn arena_allocate_multiple_blocks_non_overlapping() {
        let arena = Arena::with_capacity(4096);
        let layout = Layout::from_size_align(64, 64).expect("valid layout");
        let p1 = arena.allocate_layout(layout);
        let p2 = arena.allocate_layout(layout);
        // The two allocations must not overlap: their distance must be >= size.
        let diff = (p2.as_ptr() as isize - p1.as_ptr() as isize).unsigned_abs();
        assert!(
            diff >= 64,
            "allocations must not overlap (distance {diff} < 64)"
        );
    }

    #[test]
    fn arena_allocate_beyond_capacity_panics() {
        // Arena of 64 bytes (one cache-line); request 128 B — must panic.
        // In debug mode the debug_assert fires first ("allocation request ... exceeds arena capacity").
        // In release mode the `panic!` in the None arm fires ("Arena out of memory").
        // Either way, `catch_unwind` must catch a panic.
        let result = std::panic::catch_unwind(|| {
            let arena = Arena::with_capacity(64);
            let big = Layout::from_size_align(128, 8).expect("valid layout");
            arena.allocate_layout(big);
        });
        assert!(result.is_err(), "allocating more than capacity must panic");
    }

    // --- arena::allocate<T> ---

    #[test]
    fn arena_allocate_typed_returns_correct_alignment() {
        #[repr(align(32))]
        struct Fat([u8; 32]);

        let arena = Arena::with_capacity(4096);
        let ptr: std::ptr::NonNull<Fat> = arena.allocate::<Fat>();
        assert_eq!(
            ptr.as_ptr() as usize % std::mem::align_of::<Fat>(),
            0,
            "typed allocation must respect T's alignment"
        );
    }

    // --- Drop (M-001) ---

    #[test]
    fn arena_drop_loop_does_not_crash() {
        // Creates and drops 50 arenas in a loop.
        // If M-001 is re-introduced (no Drop impl), this would double-free on the
        // second iteration via sanitizers / Miri, or leak detectable via Miri.
        // Under plain cargo test it at minimum verifies no panic.
        for _ in 0..50 {
            let _arena = Arena::with_capacity(1024);
        }
    }

    // --- allocate_from_free_blocks ---

    #[test]
    fn arena_allocate_from_free_blocks_returns_none_when_oom() {
        let arena = Arena::with_capacity(64);
        // Use up all memory with a single allocation.
        let full = Layout::from_size_align(64, 64).expect("valid layout");
        let _p = arena.allocate_from_free_blocks(full);
        // Now the arena is exhausted; second attempt must return None.
        let layout = Layout::from_size_align(64, 8).expect("valid layout");
        let result = arena.allocate_from_free_blocks(layout);
        assert!(result.is_none(), "exhausted arena must return None");
    }
}
