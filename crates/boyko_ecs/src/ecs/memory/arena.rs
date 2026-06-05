// `Layout` is always needed: the public `allocate_*` signatures use it. The
// global-allocator `alloc`/`dealloc` are pulled in only for the fallback
// backing (Miri / wasm32 / exotic targets); the OS-syscall arms do not use
// them.
use std::alloc::Layout;
#[cfg(any(miri, not(any(windows, unix))))]
use std::alloc::{alloc, dealloc};
use std::cell::UnsafeCell;
use std::ptr::NonNull;

use crate::ecs::constants::{CACHE_LINE_SIZE, DEFAULT_ARENA_SIZE};
use crate::ecs::memory::free_mem_block::MemFreeBlockMaster;
use crate::ecs::memory::utils::align_up;

/// Windows backing: hand-declared `VirtualAlloc` / `VirtualFree` (no
/// `windows-sys` dependency). `kernel32.dll` is already linked transitively by
/// `std`, so no `#[link(name = "kernel32")]` is required.
///
/// If a future toolchain breaks bare-extern `kernel32` symbol resolution, the
/// fix is to add `#[link(name = "kernel32")]` above the `extern` block — no
/// other change.
///
/// ABI types locked for Win64: `LPVOID` -> `*mut c_void`, `SIZE_T` -> `usize`,
/// `DWORD` -> `u32`, `BOOL` -> `i32`.
#[cfg(all(not(miri), windows))]
mod win {
    use core::ffi::c_void;

    // SAFETY: signatures match the Win64 kernel32 ABI exactly (see the
    // type-mapping note above). `unsafe extern` is required by the Rust 2024
    // edition (extern blocks are unsafe to declare).
    unsafe extern "system" {
        pub fn VirtualAlloc(
            lpAddress: *mut c_void,
            dwSize: usize,
            flAllocationType: u32,
            flProtect: u32,
        ) -> *mut c_void;
        pub fn VirtualFree(lpAddress: *mut c_void, dwSize: usize, dwFreeType: u32) -> i32;
    }

    pub const MEM_COMMIT: u32 = 0x1000;
    pub const MEM_RESERVE: u32 = 0x2000;
    pub const MEM_RELEASE: u32 = 0x8000;
    pub const PAGE_READWRITE: u32 = 0x04;
}

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
///
/// # Backing store (Phase X.C)
///
/// The 64 MB buffer is acquired by a single OS-level reservation+commit call
/// (`VirtualAlloc` on Windows, `mmap` on Unix) so that `EcsMaster::new` no
/// longer pays the eager global-allocator memset; physical pages fault in
/// lazily on first touch. Miri / wasm32 / exotic targets keep the original
/// global-allocator path. The deallocator in `Drop` is chosen per cfg-arm to
/// match the acquisition (audit finding M-001: free exactly once, with the
/// matching deallocator). See [`Backing`] and `with_capacity`.
pub struct Arena {
    /// Backing buffer pointer. Allocated in `with_capacity`, freed in `Drop`.
    ptr: NonNull<u8>,

    /// Capacity of the backing buffer in bytes (cache-line aligned). Equals the
    /// logical `aligned_capacity`; on the syscall arms this is also the exact
    /// reservation/mapping length (no extra page rounding is performed here).
    capacity: usize,

    /// Per-target metadata required to release `ptr` in `Drop` with the
    /// **matching** deallocator (M-001):
    /// - fallback (global allocator): `dealloc(ptr, layout)`
    /// - Windows: `VirtualFree(ptr, 0, MEM_RELEASE)` (needs only the base ptr)
    /// - Unix: `munmap(ptr, map_len)` (needs the exact mapping length)
    ///
    /// Read only in `Drop` (and on Windows the deallocator needs nothing from
    /// it, hence the cfg-scoped `allow(dead_code)` — the zero-sized field is
    /// kept solely for cfg-arm symmetry of the struct shape).
    #[cfg_attr(all(not(miri), windows), allow(dead_code))]
    backing: Backing,

    /// Free-block tracker. Lives inside `UnsafeCell` because both
    /// `allocate_from_free_blocks` and future deallocate paths need to mutate
    /// it through `&self`. See the type-level doc-comment for the aliasing
    /// invariant.
    free_blocks: UnsafeCell<MemFreeBlockMaster>,
}

/// Per-target backing-store metadata for the arena's release path. cfg-gated
/// (no enum, no runtime tag) so each build sees exactly one shape and `Drop`
/// has zero branching overhead. Read only in `Arena::drop`.
#[cfg(any(miri, not(any(windows, unix))))]
struct Backing {
    /// Layout passed to `dealloc` in `Drop` (must equal the one given to
    /// `alloc`, per the `GlobalAlloc` contract).
    layout: Layout,
}

/// Per-target backing-store metadata for the arena's release path. cfg-gated
/// (no enum, no runtime tag). `VirtualFree(ptr, 0, MEM_RELEASE)` needs only the
/// base pointer and size 0, so no fields are required.
#[cfg(all(not(miri), windows))]
struct Backing {}

/// Per-target backing-store metadata for the arena's release path. cfg-gated
/// (no enum, no runtime tag). `munmap` requires the exact mapping length used
/// at `mmap` time.
#[cfg(all(not(miri), unix, not(windows)))]
struct Backing {
    /// Exact length passed to `mmap`, replayed to `munmap` in `Drop`.
    map_len: usize,
}

impl Arena {
    /// Allocates a fresh arena with `capacity` bytes, rounded up to cache-line
    /// granularity. Panics on allocation failure.
    ///
    /// # Backing store (Phase X.C)
    ///
    /// The buffer is acquired with a single OS reservation+commit
    /// (`VirtualAlloc(MEM_RESERVE | MEM_COMMIT)` on Windows,
    /// `mmap(PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS)` on Unix);
    /// the OS zero-fills pages lazily on first touch, so this call no longer
    /// pays the eager global-allocator memset. Miri / wasm32 / exotic targets
    /// fall back to `std::alloc::alloc`.
    ///
    /// Commit-whole charges the full size (e.g. 64 MB) of commit up front (same
    /// as today's eager `alloc`); on commit-limit exhaustion
    /// `VirtualAlloc` / `mmap` returns a failure value and the
    /// `expect` / `assert` panics — an identical failure surface to the
    /// previous code.
    pub fn with_capacity(capacity: usize) -> Self {
        let aligned_capacity = align_up(capacity, CACHE_LINE_SIZE);

        // Acquire `(ptr, backing)` from the per-target backing store. Exactly
        // one of these three arms is compiled (the cfg matrix is total and
        // disjoint by construction).
        #[cfg(all(not(miri), windows))]
        let (ptr, backing) = {
            // SAFETY: a NULL base lets the OS choose the address; `dwSize` is
            // the cache-line-aligned capacity (> 0 because
            // `DEFAULT_ARENA_SIZE > 0` and `align_up` only grows). The
            // `MEM_RESERVE | MEM_COMMIT` + `PAGE_READWRITE` flags request a
            // demand-zero readable/writable region. The result is null-checked
            // by `NonNull::new` before any use.
            let raw = unsafe {
                win::VirtualAlloc(
                    core::ptr::null_mut(),
                    aligned_capacity,
                    win::MEM_RESERVE | win::MEM_COMMIT,
                    win::PAGE_READWRITE,
                )
            };
            let ptr = NonNull::new(raw as *mut u8)
                .expect("VirtualAlloc failed to reserve+commit the arena");
            (ptr, Backing {})
        };

        #[cfg(all(not(miri), unix, not(windows)))]
        let (ptr, backing) = {
            // SAFETY: a NULL base lets the OS choose the address; `len` is the
            // cache-line-aligned capacity (> 0, see the Windows arm). The
            // `PROT_READ | PROT_WRITE` + `MAP_PRIVATE | MAP_ANONYMOUS` flags
            // request a private, demand-zero anonymous mapping (fd = -1,
            // offset = 0). The return is validated below.
            let raw = unsafe {
                libc::mmap(
                    core::ptr::null_mut(),
                    aligned_capacity,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };
            // `MAP_FAILED` is `(void*)-1` — NON-NULL — so it must be checked
            // BEFORE `NonNull::new`, which would otherwise wrongly accept it.
            assert!(raw != libc::MAP_FAILED, "mmap failed to map the arena");
            let ptr = NonNull::new(raw as *mut u8).expect("mmap returned null");
            (ptr, Backing { map_len: aligned_capacity })
        };

        #[cfg(any(miri, not(any(windows, unix))))]
        let (ptr, backing) = {
            let layout = Layout::from_size_align(aligned_capacity, CACHE_LINE_SIZE)
                .expect("Invalid layout for arena");

            // SAFETY: layout has non-zero size (DEFAULT_ARENA_SIZE > 0) and
            // alignment is a valid power of two (`CACHE_LINE_SIZE` is 64).
            let raw = unsafe { alloc(layout) };
            let ptr = NonNull::new(raw).expect("Failed to allocate memory for arena");
            (ptr, Backing { layout })
        };

        Self {
            ptr,
            capacity: aligned_capacity,
            backing,
            // O4: the free-block tracker spans the logical cache-line-rounded
            // size, not any (potentially larger) mapping length.
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
        // ALLOC1 (Phase 9 §2.7): no allocation may occur inside a system body.
        // The IN_SYSTEM_RUN TLS guard is set by `InSystemRunGuard` in the
        // scheduler's worker_main wrapper around `System::run_unsafe`. Release
        // builds rely on the §9.4 audit + the optional Step 24
        // `force_alloc_panic` cfg gate.
        debug_assert!(
            !boyko_threadpool::is_in_system_run(),
            "Phase 9 ALLOC1 violation: Arena::allocate_layout called inside System::run_unsafe. \
             All allocation must happen during the apply window (dispatcher-only) or at \
             ScheduleBuilder::build time. See plan §2.7, §9.2, §9.4."
        );
        // Step 24 / Round 3 O-NEW-1: opt-in release-mode escalation. When
        // built with `RUSTFLAGS="--cfg force_alloc_panic"`, the discipline
        // panics in release too — closing the dev/release gap. See
        // docs/PHASE-9-FORCE-ALLOC-PANIC.md.
        #[cfg(all(not(debug_assertions), force_alloc_panic))]
        {
            if boyko_threadpool::is_in_system_run() {
                panic!(
                    "Phase 9 ALLOC1 violation (force_alloc_panic): \
                     Arena::allocate_layout called inside System::run_unsafe."
                );
            }
        }
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
        // ALLOC1 (Phase 9 §2.7): mirror the guard in `allocate_layout` for the
        // direct entry point. `allocate_layout` already checks before
        // delegating here, but callers that invoke this function directly
        // (e.g. tests for the `None`-on-OOM path) must also be covered.
        debug_assert!(
            !boyko_threadpool::is_in_system_run(),
            "Phase 9 ALLOC1 violation: Arena::allocate_from_free_blocks called inside \
             System::run_unsafe. All allocation must happen during the apply window \
             (dispatcher-only) or at ScheduleBuilder::build time. See plan §2.7, §9.2, §9.4."
        );
        // Step 24 / Round 3 O-NEW-1: opt-in release-mode escalation. See
        // the matching block in `allocate_layout` and
        // docs/PHASE-9-FORCE-ALLOC-PANIC.md.
        #[cfg(all(not(debug_assertions), force_alloc_panic))]
        {
            if boyko_threadpool::is_in_system_run() {
                panic!(
                    "Phase 9 ALLOC1 violation (force_alloc_panic): \
                     Arena::allocate_from_free_blocks called inside System::run_unsafe."
                );
            }
        }

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
        // Release with the deallocator matching the acquisition arm in
        // `with_capacity` (M-001). Exactly one arm is compiled.
        #[cfg(all(not(miri), windows))]
        {
            // SAFETY: `self.ptr` is the exact base returned by `VirtualAlloc`
            // in `with_capacity`, freed exactly once (`Drop` runs once).
            // `MEM_RELEASE` requires `dwSize == 0` together with the original
            // base address — both are satisfied. After this point `self.ptr`
            // must not be used, which is fine because `Drop` is the last call
            // on `self`.
            let ok =
                unsafe { win::VirtualFree(self.ptr.as_ptr() as *mut core::ffi::c_void, 0, win::MEM_RELEASE) };
            debug_assert!(ok != 0, "VirtualFree(MEM_RELEASE) failed");
        }

        #[cfg(all(not(miri), unix, not(windows)))]
        {
            // SAFETY: `self.ptr` and `self.backing.map_len` are the exact base
            // and length returned by `mmap` in `with_capacity`, unmapped
            // exactly once (`Drop` runs once). After this point `self.ptr` must
            // not be used — fine, because `Drop` is the last call on `self`.
            let ret = unsafe {
                libc::munmap(self.ptr.as_ptr() as *mut core::ffi::c_void, self.backing.map_len)
            };
            debug_assert_eq!(ret, 0, "munmap failed");
        }

        #[cfg(any(miri, not(any(windows, unix))))]
        {
            // SAFETY: `self.ptr` was returned by `alloc(self.backing.layout)`
            // in `with_capacity` and has not been freed yet (`Drop` runs once).
            // The exact same `Layout` is passed back to `dealloc`, satisfying
            // the `GlobalAlloc` contract. After this point `self.ptr` must not
            // be used — which is fine because `Drop` is the last call on
            // `self`.
            unsafe { dealloc(self.ptr.as_ptr(), self.backing.layout) }
        }
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
        struct Fat(#[allow(dead_code)] [u8; 32]);

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
        //
        // Phase X.C: with the syscall-backed store, this now exercises 50 real
        // VirtualAlloc/VirtualFree (Windows) or mmap/munmap (Unix) round trips;
        // a mismatched or double free would crash here on the native host.
        for _ in 0..50 {
            let _arena = Arena::with_capacity(1024);
        }
    }

    #[test]
    fn arena_default_size_drop_loop_does_not_crash() {
        // Phase X.C — large-mapping path coverage. The existing drop-loop above
        // uses a 1 KB capacity, which on the syscall arms still maps at least one
        // page but does not stress the full 64 MB reserve+commit. This loop
        // creates and drops 20 *default-size* (64 MB) arenas back-to-back,
        // exercising VirtualAlloc(MEM_RESERVE | MEM_COMMIT) + VirtualFree
        // (Windows) / mmap + munmap (Unix) for the production-sized mapping.
        //
        // A mismatched deallocator, a double free, or a leak of the large
        // mapping would surface here as a crash (native), a borrow/leak error
        // (Miri fallback arm), or commit-limit exhaustion if a prior iteration's
        // mapping were not released. Asserting the logical capacity each
        // iteration also confirms `capacity()` stays the cache-line-rounded
        // size (the OS may over-map to page granularity, but the logical value
        // must not drift).
        let expected = align_up(DEFAULT_ARENA_SIZE, CACHE_LINE_SIZE);
        for _ in 0..20 {
            let arena = Arena::new();
            assert_eq!(
                arena.capacity(),
                expected,
                "each default-size arena must report the rounded logical capacity"
            );
            // Touch the first byte to force at least one page to fault in,
            // proving the mapping is genuinely readable/writable on every
            // iteration (a stale/released mapping would fault here).
            let p = arena.allocate_layout(
                Layout::from_size_align(CACHE_LINE_SIZE, CACHE_LINE_SIZE)
                    .expect("valid layout"),
            );
            // SAFETY: `p` is a fresh CACHE_LINE_SIZE-byte block inside the live
            // arena mapping; writing one byte at offset 0 is in bounds.
            unsafe {
                p.as_ptr().write_volatile(0xABu8);
            }
            // `arena` drops at end of scope — must release the 64 MB mapping.
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
